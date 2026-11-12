//! A simulated projector that speaks PJLink (and optionally Panasonic
//! NTCONTROL), for demos and tests without hardware.

use crate::net::LineConn;
use crate::pjlink::{digest, md5_hex};
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct SimSpec {
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    /// TCP port for PJLink on 127.0.0.1; 0 picks a free one.
    pub port: u16,
    /// Also serve Panasonic NTCONTROL on this port (0 = free port).
    pub ntcontrol_port: Option<u16>,
    pub password: Option<String>,
    pub class: u8,
    /// fan, lamp, temperature, cover, filter, other (0 ok, 1 warning, 2 error).
    pub errors: [u8; 6],
    pub lamp_hours: u32,
    pub powered: bool,
    pub inputs: Vec<(String, String)>,
    pub warm_up: Duration,
    pub cool_down: Duration,
}

impl Default for SimSpec {
    fn default() -> Self {
        Self {
            name: "Simulated".into(),
            manufacturer: "ProjectorHUD".into(),
            model: "SIM-1".into(),
            port: 0,
            ntcontrol_port: None,
            password: None,
            class: 2,
            errors: [0; 6],
            lamp_hours: 120,
            powered: false,
            inputs: vec![
                ("31".into(), "HDMI 1".into()),
                ("32".into(), "HDMI 2".into()),
                ("33".into(), "SDI".into()),
                ("11".into(), "RGB".into()),
            ],
            warm_up: Duration::from_secs(6),
            cool_down: Duration::from_secs(8),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Power {
    Standby,
    On,
    Cooling(Instant),
    Warming(Instant),
}

struct State {
    spec: SimSpec,
    power: Power,
    input: String,
    video_mute: bool,
    audio_mute: bool,
    freeze: bool,
}

impl State {
    fn tick(&mut self) {
        match self.power {
            Power::Warming(until) if Instant::now() >= until => self.power = Power::On,
            Power::Cooling(until) if Instant::now() >= until => {
                self.power = Power::Standby;
                self.video_mute = false;
                self.audio_mute = false;
                self.freeze = false;
            }
            _ => {}
        }
    }
    fn power_on(&mut self) -> bool {
        match self.power {
            Power::Standby => {
                self.power = Power::Warming(Instant::now() + self.spec.warm_up);
                true
            }
            Power::On | Power::Warming(_) => true,
            Power::Cooling(_) => false,
        }
    }
    fn power_off(&mut self) -> bool {
        match self.power {
            Power::On => {
                self.power = Power::Cooling(Instant::now() + self.spec.cool_down);
                true
            }
            Power::Standby | Power::Cooling(_) => true,
            Power::Warming(_) => false,
        }
    }
}

pub struct SimHandle {
    pub port: u16,
    pub ntcontrol_port: Option<u16>,
    stop: Arc<AtomicBool>,
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn spawn(spec: SimSpec) -> io::Result<SimHandle> {
    let listener = TcpListener::bind(("127.0.0.1", spec.port))?;
    let port = listener.local_addr()?.port();
    let nt_listener = match spec.ntcontrol_port {
        Some(p) => Some(TcpListener::bind(("127.0.0.1", p))?),
        None => None,
    };
    let ntcontrol_port = match &nt_listener {
        Some(l) => Some(l.local_addr()?.port()),
        None => None,
    };

    let stop = Arc::new(AtomicBool::new(false));
    let state = Arc::new(Mutex::new(State {
        power: if spec.powered { Power::On } else { Power::Standby },
        input: spec.inputs.first().map(|i| i.0.clone()).unwrap_or_else(|| "31".into()),
        video_mute: false,
        audio_mute: false,
        freeze: false,
        spec,
    }));

    accept_loop(listener, stop.clone(), state.clone(), serve_pjlink);
    if let Some(l) = nt_listener {
        accept_loop(l, stop.clone(), state, serve_ntcontrol);
    }
    Ok(SimHandle { port, ntcontrol_port, stop })
}

fn accept_loop(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<State>>,
    serve: fn(TcpStream, Arc<Mutex<State>>, Arc<AtomicBool>),
) {
    listener.set_nonblocking(true).ok();
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).ok();
                    let state = state.clone();
                    let stop = stop.clone();
                    std::thread::spawn(move || serve(stream, state, stop));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(40)),
            }
        }
    });
}

fn nonce() -> String {
    use std::hash::{BuildHasher, Hasher};
    let h = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{:08x}", h as u32)
}

fn serve_pjlink(stream: TcpStream, state: Arc<Mutex<State>>, stop: Arc<AtomicBool>) {
    // Real projectors drop idle PJLink sessions after 30 seconds.
    let Ok(mut conn) = LineConn::from_stream(stream, Duration::from_secs(30)) else { return };
    let password = state.lock().unwrap().spec.password.clone();
    let random = nonce();
    let mut expected = password.as_ref().map(|p| digest(&random, p));
    let greeting = match expected {
        Some(_) => format!("PJLINK 1 {random}\r"),
        None => "PJLINK 0\r".to_string(),
    };
    if conn.send(&greeting).is_err() {
        return;
    }
    while !stop.load(Ordering::Relaxed) {
        let Ok(mut line) = conn.read_line() else { return };
        if let Some(d) = expected.take() {
            match line.strip_prefix(d.as_str()) {
                Some(rest) => line = rest.to_string(),
                None => {
                    let _ = conn.send("PJLINK ERRA\r");
                    return;
                }
            }
        }
        let reply = pjlink_reply(&line, &mut state.lock().unwrap());
        if conn.send(&format!("{reply}\r")).is_err() {
            return;
        }
    }
}

fn pjlink_reply(line: &str, st: &mut State) -> String {
    st.tick();
    let b = line.as_bytes();
    if b.len() < 8 || b[0] != b'%' || b[6] != b' ' {
        return "PJLINK ERR1".into(); // not a valid frame; any error text will do
    }
    let class = (b[1] as char).to_digit(10).unwrap_or(1) as u8;
    let cmd = line[2..6].to_ascii_uppercase();
    let param = line[7..].trim();
    let head = format!("%{class}{cmd}=");
    if class > st.spec.class {
        return format!("{head}ERR1");
    }
    let on = st.power == Power::On;
    let body: String = match (cmd.as_str(), param) {
        ("POWR", "?") => match st.power {
            Power::Standby => "0",
            Power::On => "1",
            Power::Cooling(_) => "2",
            Power::Warming(_) => "3",
        }
        .into(),
        ("POWR", "1") => if st.power_on() { "OK" } else { "ERR3" }.into(),
        ("POWR", "0") => if st.power_off() { "OK" } else { "ERR3" }.into(),
        ("POWR", _) => "ERR2".into(),
        ("INPT", "?") => if on { st.input.clone() } else { "ERR3".into() },
        ("INPT", code) => {
            if !on {
                "ERR3".into()
            } else if st.spec.inputs.iter().any(|i| i.0 == code) {
                st.input = code.to_string();
                "OK".into()
            } else {
                "ERR2".into()
            }
        }
        ("AVMT", "?") => {
            if !on {
                "ERR3".into()
            } else {
                match (st.video_mute, st.audio_mute) {
                    (true, true) => "31",
                    (true, false) => "11",
                    (false, true) => "21",
                    (false, false) => "30",
                }
                .into()
            }
        }
        ("AVMT", p) => {
            if !on {
                "ERR3".into()
            } else {
                let ok = match p {
                    "10" => { st.video_mute = false; true }
                    "11" => { st.video_mute = true; true }
                    "20" => { st.audio_mute = false; true }
                    "21" => { st.audio_mute = true; true }
                    "30" => { st.video_mute = false; st.audio_mute = false; true }
                    "31" => { st.video_mute = true; st.audio_mute = true; true }
                    _ => false,
                };
                if ok { "OK" } else { "ERR2" }.into()
            }
        }
        ("ERST", "?") => st.spec.errors.iter().map(|d| d.to_string()).collect(),
        ("LAMP", "?") => format!("{} {}", st.spec.lamp_hours, on as u8),
        ("INST", "?") => st.spec.inputs.iter().map(|i| i.0.clone()).collect::<Vec<_>>().join(" "),
        ("NAME", "?") => st.spec.name.clone(),
        ("INF1", "?") => st.spec.manufacturer.clone(),
        ("INF2", "?") => st.spec.model.clone(),
        ("INFO", "?") => "ProjectorHUD simulator".into(),
        ("CLSS", "?") => st.spec.class.to_string(),
        ("SNUM", "?") => format!("SIM{:05}", st.spec.lamp_hours),
        ("SVER", "?") => "1.0.0".into(),
        ("INNM", p) => match st.spec.inputs.iter().find(|i| Some(i.0.as_str()) == p.strip_prefix('?')) {
            Some(i) => i.1.clone(),
            None => "ERR2".into(),
        },
        ("FREZ", "?") => if on { (st.freeze as u8).to_string() } else { "ERR3".into() },
        ("FREZ", p @ ("0" | "1")) => {
            if on {
                st.freeze = p == "1";
                "OK".into()
            } else {
                "ERR3".into()
            }
        }
        ("SVOL" | "MVOL", "0" | "1") => if on { "OK" } else { "ERR3" }.into(),
        _ => "ERR1".into(),
    };
    format!("{head}{body}")
}

fn serve_ntcontrol(stream: TcpStream, state: Arc<Mutex<State>>, _stop: Arc<AtomicBool>) {
    let Ok(mut conn) = LineConn::from_stream(stream, Duration::from_secs(5)) else { return };
    let password = state.lock().unwrap().spec.password.clone();
    let random = nonce();
    let expected = password.as_ref().map(|p| md5_hex(&format!("admin1:{p}:{random}")));
    let greeting = match expected {
        Some(_) => format!("NTCONTROL 1 {random}\r"),
        None => "NTCONTROL 0\r".to_string(),
    };
    if conn.send(&greeting).is_err() {
        return;
    }
    let Ok(mut line) = conn.read_line() else { return };
    if let Some(d) = expected {
        match line.strip_prefix(d.as_str()) {
            Some(rest) => line = rest.to_string(),
            None => {
                let _ = conn.send("00ERRA\r");
                return;
            }
        }
    }
    let body = line.strip_prefix("00").unwrap_or(&line).to_string();
    let mut st = state.lock().unwrap();
    st.tick();
    let on = st.power == Power::On;
    let reply: String = match body.as_str() {
        "QPW" => if matches!(st.power, Power::On | Power::Warming(_)) { "001" } else { "000" }.into(),
        "Q$S" => match st.power {
            Power::Standby => "0",
            Power::Warming(_) => "1",
            Power::On => "2",
            Power::Cooling(_) => "3",
        }
        .into(),
        "PON" => if st.power_on() { "PON" } else { "ERR3" }.into(),
        "POF" => if st.power_off() { "POF" } else { "ERR3" }.into(),
        "QSH" => (st.video_mute as u8).to_string(),
        "OSH:0" | "OSH:1" if on => {
            st.video_mute = body.ends_with('1');
            body.clone()
        }
        "QFZ" => (st.freeze as u8).to_string(),
        "OFZ:0" | "OFZ:1" if on => {
            st.freeze = body.ends_with('1');
            body.clone()
        }
        "QIN" => "HD1".into(),
        "Q$L:1" => format!("{:04}", st.spec.lamp_hours),
        b if b.starts_with("IIS:") && on => body.clone(),
        _ => "ERR1".into(),
    };
    let _ = conn.send(&format!("00{reply}\r"));
}

/// The demo fleet shown by the app's "Try the demo" button.
pub fn demo_fleet() -> Vec<SimSpec> {
    let base = SimSpec::default();
    let pana_inputs = vec![
        ("31".to_string(), "HDMI 1".to_string()),
        ("32".to_string(), "HDMI 2".to_string()),
        ("33".to_string(), "Digital Link".to_string()),
        ("34".to_string(), "SDI".to_string()),
        ("11".to_string(), "RGB".to_string()),
    ];
    vec![
        SimSpec {
            name: "Main Stage Left".into(),
            manufacturer: "Panasonic".into(),
            model: "PT-RZ120".into(),
            port: 14352,
            powered: true,
            lamp_hours: 3412,
            inputs: pana_inputs.clone(),
            ..base.clone()
        },
        SimSpec {
            name: "Main Stage Right".into(),
            manufacturer: "Panasonic".into(),
            model: "PT-RZ120".into(),
            port: 14353,
            powered: true,
            lamp_hours: 3398,
            inputs: pana_inputs.clone(),
            ..base.clone()
        },
        SimSpec {
            name: "Rear Projection".into(),
            manufacturer: "Panasonic".into(),
            model: "PT-MZ880".into(),
            port: 14354,
            powered: true,
            lamp_hours: 9120,
            errors: [0, 0, 0, 0, 1, 0],
            inputs: pana_inputs,
            ..base.clone()
        },
        SimSpec {
            name: "Lobby".into(),
            manufacturer: "EPSON".into(),
            model: "EB-L1075U".into(),
            port: 14355,
            lamp_hours: 640,
            ..base.clone()
        },
        SimSpec {
            name: "Rehearsal Room".into(),
            manufacturer: "NEC".into(),
            model: "PA803U".into(),
            port: 14356,
            powered: true,
            class: 1,
            lamp_hours: 1875,
            errors: [0, 0, 2, 0, 0, 0],
            ..base.clone()
        },
        SimSpec {
            name: "Balcony".into(),
            manufacturer: "Christie".into(),
            model: "DWU1075-GS".into(),
            port: 14357,
            password: Some("demo".into()),
            lamp_hours: 210,
            ..base
        },
    ]
}
