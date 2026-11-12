//! Panasonic native LAN control ("NTCONTROL", TCP 1024).
//!
//! One command per connection. The projector greets with `NTCONTROL 0` (no
//! protection) or `NTCONTROL 1 <random>`; in protect mode each command is
//! prefixed with the hex MD5 of `<user>:<password>:<random>`. Commands and
//! replies are framed as `00<body>\r`.
//!
//! This protocol does not expose the fan/lamp/temperature error summary that
//! PJLink does, so PJLink remains the recommended protocol for monitoring.

use crate::model::*;
use crate::net::LineConn;
use crate::pjlink::md5_hex;

pub const DEFAULT_PORT: u16 = 1024;

/// Input codes common across Panasonic projector ranges.
const INPUTS: &[(&str, &str)] = &[
    ("HD1", "HDMI 1"),
    ("HD2", "HDMI 2"),
    ("DL1", "Digital Link"),
    ("SD1", "SDI 1"),
    ("SD2", "SDI 2"),
    ("DVI", "DVI-D"),
    ("DP1", "DisplayPort"),
    ("RG1", "RGB 1"),
    ("RG2", "RGB 2"),
    ("VID", "Video"),
];

pub struct PanasonicDriver {
    host: String,
    port: u16,
    username: String,
    password: String,
    log: Log,
}

impl PanasonicDriver {
    pub fn new(host: &str, port: u16, username: &str, password: &str, log: Log) -> Self {
        Self {
            host: host.to_string(),
            port,
            username: username.to_string(),
            password: password.to_string(),
            log,
        }
    }

    /// Send one command on a fresh connection and return the reply body.
    fn exchange(&mut self, body: &str) -> Result<String, DriverError> {
        let mut conn = LineConn::connect(&self.host, self.port)?;
        let greeting = conn.read_line()?;
        let mut parts = greeting.split_whitespace();
        if !parts.next().unwrap_or("").eq_ignore_ascii_case("NTCONTROL") {
            return Err(DriverError::Protocol(format!(
                "Not a Panasonic projector (got \"{greeting}\")"
            )));
        }
        let prefix = match parts.next() {
            Some("1") => {
                if self.password.is_empty() {
                    return Err(DriverError::Auth(
                        "This projector requires a user name and password".into(),
                    ));
                }
                let random = parts.next().unwrap_or("");
                md5_hex(&format!("{}:{}:{}", self.username, self.password, random))
            }
            _ => String::new(),
        };
        self.log.push(LogKind::Tx, body.to_string());
        conn.send(&format!("{prefix}00{body}\r"))?;
        let line = conn.read_line()?;
        self.log.push(LogKind::Rx, line.clone());
        let reply = line.strip_prefix("00").unwrap_or(&line).trim().to_string();
        match reply.to_ascii_uppercase().as_str() {
            "ERRA" => Err(DriverError::Auth("User name or password rejected".into())),
            "ERR1" => Err(DriverError::Protocol(
                "Not supported by this projector".into(),
            )),
            "ERR2" => Err(DriverError::Protocol("Projector rejected the value".into())),
            "ERR3" => Err(DriverError::Protocol("Projector is busy".into())),
            "ERR4" => Err(DriverError::Protocol("Projector timed out".into())),
            "ERR5" => Err(DriverError::Protocol(
                "Projector rejected the command length".into(),
            )),
            _ => Ok(reply),
        }
    }

    /// Query where a protocol-level refusal just means "no value".
    fn query(&mut self, body: &str) -> Result<Option<String>, DriverError> {
        match self.exchange(body) {
            Ok(v) => Ok(Some(v)),
            Err(DriverError::Protocol(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

impl Driver for PanasonicDriver {
    fn poll(
        &mut self,
        st: &mut ProjectorState,
        publish: &dyn Fn(&ProjectorState),
    ) -> Result<(), DriverError> {
        let on = self.exchange("QPW")?.trim() == "001";
        st.link = Link::Online;
        st.link_error = None;
        st.manufacturer = "Panasonic".into();
        // Q$S refines on/off into warm-up and cool-down where supported.
        st.power = match self.query("Q$S")?.as_deref().map(str::trim) {
            Some("0") => Power::Standby,
            Some("1") => Power::Warming,
            Some("2") => Power::On,
            Some("3") => Power::Cooling,
            _ if on => Power::On,
            _ => Power::Standby,
        };
        publish(st);

        if st.inputs.is_empty() {
            st.inputs = INPUTS
                .iter()
                .map(|(c, n)| InputSource {
                    code: c.to_string(),
                    name: n.to_string(),
                })
                .collect();
        }
        st.errors = None;
        if let Some(h) = self
            .query("Q$L:1")?
            .and_then(|v| v.trim().parse::<u32>().ok())
        {
            st.lamps = vec![Lamp {
                hours: h,
                on: st.power == Power::On,
            }];
        }
        if st.power == Power::On {
            st.input = self.query("QIN")?.map(|v| v.trim().to_string());
            st.video_mute = self.query("QSH")?.map(|v| v.trim() == "1");
            st.freeze = self.query("QFZ")?.map(|v| v.trim() == "1");
        } else {
            st.video_mute = None;
            st.freeze = None;
        }
        st.audio_mute = None;
        Ok(())
    }

    fn command(&mut self, cmd: &Command) -> Result<(), DriverError> {
        let body = match cmd {
            Command::PowerOn => "PON".to_string(),
            Command::PowerOff => "POF".to_string(),
            Command::VideoMute(on) => format!("OSH:{}", *on as u8),
            Command::Freeze(on) => format!("OFZ:{}", *on as u8),
            Command::Input(code) => format!("IIS:{code}"),
            Command::VolumeUp => "AUU".to_string(),
            Command::VolumeDown => "AUD".to_string(),
            Command::AudioMute(_) => {
                return Err(DriverError::Protocol(
                    "Audio mute is not available over Panasonic native control".into(),
                ))
            }
            Command::Refresh => return Ok(()),
        };
        self.exchange(&body).map(|_| ())
    }
}
