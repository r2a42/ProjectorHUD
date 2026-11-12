//! PJLink Class 1 / Class 2 client.
//!
//! Wire format: `%<class><CMND> <param>\r` -> `%<class><CMND>=<value>\r`.
//! On connect the projector sends `PJLINK 0` (no authentication) or
//! `PJLINK 1 <random>`; in the latter case the first command is prefixed with
//! the hex MD5 of `<random><password>`.

use crate::model::*;
use crate::net::LineConn;
use md5::{Digest, Md5};

pub const DEFAULT_PORT: u16 = 4352;

pub fn md5_hex(input: &str) -> String {
    let mut h = Md5::new();
    h.update(input.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Authentication digest for a `PJLINK 1 <random>` greeting.
pub fn digest(random: &str, password: &str) -> String {
    md5_hex(&format!("{random}{password}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Value(String),
    /// ERR1: command not supported.
    Undefined,
    /// ERR2: parameter out of range.
    OutOfParameter,
    /// ERR3: unavailable right now (e.g. input query while in standby).
    Unavailable,
    /// ERR4: projector/display failure.
    Failure,
}

/// Parse a response line for `cmd`. Returns `None` if the line belongs to a
/// different command (stale or unsolicited data).
pub fn parse_reply(line: &str, cmd: &str) -> Option<Reply> {
    let b = line.as_bytes();
    if b.len() < 7 || b[0] != b'%' || b[6] != b'=' {
        return None;
    }
    if !line.get(2..6)?.eq_ignore_ascii_case(cmd) {
        return None;
    }
    let value = line.get(7..)?.trim();
    Some(match value.to_ascii_uppercase().as_str() {
        "ERR1" => Reply::Undefined,
        "ERR2" => Reply::OutOfParameter,
        "ERR3" => Reply::Unavailable,
        "ERR4" => Reply::Failure,
        _ => Reply::Value(value.to_string()),
    })
}

pub fn parse_power(v: &str) -> Power {
    match v {
        "0" => Power::Standby,
        "1" => Power::On,
        "2" => Power::Cooling,
        "3" => Power::Warming,
        _ => Power::Unknown,
    }
}

pub fn parse_errors(v: &str) -> Option<Errors> {
    let d: Vec<Severity> = v
        .chars()
        .map(|c| match c {
            '1' => Severity::Warning,
            '2' => Severity::Error,
            _ => Severity::Ok,
        })
        .collect();
    if d.len() != 6 {
        return None;
    }
    Some(Errors {
        fan: d[0],
        lamp: d[1],
        temperature: d[2],
        cover: d[3],
        filter: d[4],
        other: d[5],
    })
}

pub fn parse_lamps(v: &str) -> Vec<Lamp> {
    let parts: Vec<&str> = v.split_whitespace().collect();
    parts
        .chunks(2)
        .filter_map(|c| {
            let hours = c.first()?.parse().ok()?;
            Some(Lamp {
                hours,
                on: c.get(1) == Some(&"1"),
            })
        })
        .collect()
}

/// (video muted, audio muted) from an `AVMT ?` reply.
pub fn parse_avmt(v: &str) -> Option<(bool, bool)> {
    match v {
        "11" => Some((true, false)),
        "21" => Some((false, true)),
        "31" => Some((true, true)),
        "30" | "10" | "20" => Some((false, false)),
        _ => None,
    }
}

/// Generic name for an input code, used until the projector tells us better (INNM).
pub fn default_input_name(code: &str) -> String {
    let mut ch = code.chars();
    let kind = match ch.next() {
        Some('1') => "RGB",
        Some('2') => "Video",
        Some('3') => "Digital",
        Some('4') => "Storage",
        Some('5') => "Network",
        Some('6') => "Internal",
        _ => return code.to_string(),
    };
    match ch.next() {
        Some(n) => format!("{kind} {n}"),
        None => kind.to_string(),
    }
}

pub struct PjLinkDriver {
    host: String,
    port: u16,
    password: String,
    log: Log,
    conn: Option<LineConn>,
    /// Digest to prepend to the first command on a fresh authenticated connection.
    pending_digest: Option<String>,
    class: u8,
    static_loaded: bool,
}

impl PjLinkDriver {
    pub fn new(host: &str, port: u16, password: &str, log: Log) -> Self {
        Self {
            host: host.to_string(),
            port,
            password: password.to_string(),
            log,
            conn: None,
            pending_digest: None,
            class: 1,
            static_loaded: false,
        }
    }

    fn connect(&mut self) -> Result<(), DriverError> {
        self.conn = None;
        self.pending_digest = None;
        let mut conn = LineConn::connect(&self.host, self.port)?;
        let greeting = conn.read_line()?;
        self.log.push(LogKind::Rx, redact_greeting(&greeting));
        let upper = greeting.to_ascii_uppercase();
        if !upper.starts_with("PJLINK") {
            return Err(DriverError::Protocol(format!(
                "Not a PJLink device (got \"{greeting}\")"
            )));
        }
        let mut parts = greeting.split_whitespace().skip(1);
        match parts.next() {
            Some("0") => {}
            Some("1") => {
                let random = parts.next().unwrap_or("");
                if self.password.is_empty() {
                    return Err(DriverError::Auth(
                        "This projector requires a PJLink password".into(),
                    ));
                }
                self.pending_digest = Some(digest(random, &self.password));
            }
            Some(other) if other.eq_ignore_ascii_case("ERRA") => {
                return Err(DriverError::Auth("Password rejected".into()));
            }
            _ => {
                return Err(DriverError::Protocol(format!(
                    "Unexpected greeting \"{greeting}\""
                )))
            }
        }
        self.conn = Some(conn);
        Ok(())
    }

    fn exchange_once(&mut self, class: u8, cmd: &str, param: &str) -> Result<Reply, DriverError> {
        if self.conn.is_none() {
            self.connect()?;
        }
        let body = format!("%{class}{cmd} {param}");
        let prefix = self.pending_digest.take().unwrap_or_default();
        let conn = self.conn.as_mut().expect("connected above");
        self.log.push(LogKind::Tx, body.clone());
        conn.send(&format!("{prefix}{body}\r"))?;
        // Tolerate a couple of stale/unsolicited lines before our answer.
        for _ in 0..4 {
            let line = conn.read_line()?;
            self.log.push(LogKind::Rx, line.clone());
            let upper = line.to_ascii_uppercase();
            if upper.starts_with("PJLINK") && upper.contains("ERRA") {
                self.conn = None;
                return Err(DriverError::Auth("Password rejected".into()));
            }
            if let Some(reply) = parse_reply(&line, cmd) {
                return Ok(reply);
            }
        }
        Err(DriverError::Protocol(format!("No valid answer to {cmd}")))
    }

    /// Send one command, transparently reconnecting once if the (possibly
    /// idle-closed) connection fails.
    fn exchange(&mut self, class: u8, cmd: &str, param: &str) -> Result<Reply, DriverError> {
        let had_conn = self.conn.is_some();
        match self.exchange_once(class, cmd, param) {
            Err(DriverError::Io(_)) if had_conn => {
                self.conn = None;
                self.log
                    .push(LogKind::Info, "Connection lost, reconnecting");
                let r = self.exchange_once(class, cmd, param);
                if r.is_err() {
                    self.conn = None;
                }
                r
            }
            Err(e) => {
                self.conn = None;
                Err(e)
            }
            ok => ok,
        }
    }

    /// Query that treats "unsupported" and "unavailable" as simply no value.
    fn query(&mut self, class: u8, cmd: &str) -> Result<Option<String>, DriverError> {
        self.query_param(class, cmd, "?")
    }

    fn query_param(
        &mut self,
        class: u8,
        cmd: &str,
        param: &str,
    ) -> Result<Option<String>, DriverError> {
        Ok(match self.exchange(class, cmd, param)? {
            Reply::Value(v) => Some(v),
            _ => None,
        })
    }

    fn set(&mut self, class: u8, cmd: &str, param: &str) -> Result<(), DriverError> {
        match self.exchange(class, cmd, param)? {
            Reply::Value(_) => Ok(()),
            Reply::Undefined => Err(DriverError::Protocol(
                "Not supported by this projector".into(),
            )),
            Reply::OutOfParameter => {
                Err(DriverError::Protocol("Projector rejected the value".into()))
            }
            Reply::Unavailable => Err(DriverError::Protocol(
                "Not available right now (standby, warming or busy)".into(),
            )),
            Reply::Failure => Err(DriverError::Protocol("Projector reports a failure".into())),
        }
    }

    fn load_static(&mut self, st: &mut ProjectorState) -> Result<(), DriverError> {
        self.class = self
            .query(1, "CLSS")?
            .and_then(|v| v.trim().parse::<u8>().ok())
            .filter(|c| *c >= 1)
            .unwrap_or(1)
            .min(2);
        st.class = self.class;
        st.device_name = self.query(1, "NAME")?.unwrap_or_default();
        st.manufacturer = self.query(1, "INF1")?.unwrap_or_default();
        st.model = self.query(1, "INF2")?.unwrap_or_default();
        st.info = self.query(1, "INFO")?.unwrap_or_default();

        let codes = self.query(self.class, "INST")?.unwrap_or_default();
        let mut inputs: Vec<InputSource> = codes
            .split_whitespace()
            .map(|c| InputSource {
                code: c.to_string(),
                name: default_input_name(c),
            })
            .collect();
        if self.class >= 2 {
            st.serial = self.query(2, "SNUM")?.unwrap_or_default();
            st.firmware = self.query(2, "SVER")?.unwrap_or_default();
            for input in inputs.iter_mut() {
                if let Some(name) = self.query_param(2, "INNM", &format!("?{}", input.code))? {
                    if !name.trim().is_empty() {
                        input.name = name.trim().to_string();
                    }
                }
            }
        }
        st.inputs = inputs;
        self.static_loaded = true;
        Ok(())
    }
}

fn redact_greeting(g: &str) -> String {
    // The random challenge is not secret, but there is no reason to keep it.
    if g.to_ascii_uppercase().starts_with("PJLINK 1") {
        "PJLINK 1 (authentication required)".into()
    } else {
        g.to_string()
    }
}

impl Driver for PjLinkDriver {
    fn poll(
        &mut self,
        st: &mut ProjectorState,
        publish: &dyn Fn(&ProjectorState),
    ) -> Result<(), DriverError> {
        // Power first: it is the cheapest proof of life and what operators look at.
        let power = match self.exchange(1, "POWR", "?")? {
            Reply::Value(v) => parse_power(&v),
            _ => Power::Unknown,
        };
        st.power = power;
        st.link = Link::Online;
        st.link_error = None;
        publish(st);

        if !self.static_loaded {
            self.load_static(st)?;
            publish(st);
        }

        st.errors = self.query(1, "ERST")?.and_then(|v| parse_errors(&v));
        if let Some(v) = self.query(1, "LAMP")? {
            st.lamps = parse_lamps(&v);
        }

        if power == Power::On {
            st.input = self
                .query(self.class, "INPT")?
                .map(|v| v.trim().to_string());
            match self.query(1, "AVMT")?.and_then(|v| parse_avmt(&v)) {
                Some((video, audio)) => {
                    st.video_mute = Some(video);
                    st.audio_mute = Some(audio);
                }
                None => {
                    st.video_mute = None;
                    st.audio_mute = None;
                }
            }
            st.freeze = if self.class >= 2 {
                self.query(2, "FREZ")?.map(|v| v.trim() == "1")
            } else {
                None
            };
        } else {
            st.video_mute = None;
            st.audio_mute = None;
            st.freeze = None;
        }
        Ok(())
    }

    fn command(&mut self, cmd: &Command) -> Result<(), DriverError> {
        match cmd {
            Command::PowerOn => self.set(1, "POWR", "1"),
            Command::PowerOff => self.set(1, "POWR", "0"),
            Command::VideoMute(on) => self.set(1, "AVMT", if *on { "11" } else { "10" }),
            Command::AudioMute(on) => self.set(1, "AVMT", if *on { "21" } else { "20" }),
            Command::Input(code) => self.set(self.class, "INPT", code),
            Command::Freeze(on) => self.set(2, "FREZ", if *on { "1" } else { "0" }),
            Command::VolumeUp => self.set(2, "SVOL", "1"),
            Command::VolumeDown => self.set(2, "SVOL", "0"),
            Command::Refresh => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_digest_example() {
        // Example from the PJLink specification.
        assert_eq!(
            digest("498e4a67", "JBMIAProjectorLink"),
            "5d8409bc1c3fa39749434aa3a5c38682"
        );
    }

    #[test]
    fn replies() {
        assert_eq!(
            parse_reply("%1POWR=1", "POWR"),
            Some(Reply::Value("1".into()))
        );
        assert_eq!(parse_reply("%1powr=ERR3", "POWR"), Some(Reply::Unavailable));
        assert_eq!(
            parse_reply("%2INNM=HDMI 1", "INNM"),
            Some(Reply::Value("HDMI 1".into()))
        );
        assert_eq!(parse_reply("%1INPT=31", "POWR"), None);
        assert_eq!(parse_reply("junk", "POWR"), None);
        assert_eq!(
            parse_reply("%1NAME=", "NAME"),
            Some(Reply::Value(String::new()))
        );
    }

    #[test]
    fn status_fields() {
        let e = parse_errors("002010").unwrap();
        assert_eq!(e.temperature, Severity::Error);
        assert_eq!(e.filter, Severity::Warning);
        assert_eq!(e.worst(), Severity::Error);
        assert!(parse_errors("00").is_none());
        assert_eq!(
            parse_lamps("1234 1 80 0"),
            vec![
                Lamp {
                    hours: 1234,
                    on: true
                },
                Lamp {
                    hours: 80,
                    on: false
                }
            ]
        );
        assert_eq!(parse_avmt("31"), Some((true, true)));
        assert_eq!(parse_avmt("30"), Some((false, false)));
        assert_eq!(default_input_name("32"), "Digital 2");
    }
}
