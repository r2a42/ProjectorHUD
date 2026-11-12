//! Shared data types: projector configuration, live state, commands.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protocol {
    /// PJLink Class 1 / Class 2 (TCP 4352). Works with nearly every brand.
    PjLink,
    /// Panasonic native NTCONTROL (TCP 1024).
    Panasonic,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Protocol::PjLink => "PJLink",
            Protocol::Panasonic => "Panasonic native",
        }
    }
    pub fn default_port(self) -> u16 {
        match self {
            Protocol::PjLink => 4352,
            Protocol::Panasonic => 1024,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProjectorConfig {
    pub id: u64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub protocol: Protocol,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub demo: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Power {
    #[default]
    Unknown,
    Standby,
    On,
    Cooling,
    Warming,
}

impl Power {
    pub fn label(self) -> &'static str {
        match self {
            Power::Unknown => "—",
            Power::Standby => "STANDBY",
            Power::On => "ON",
            Power::Cooling => "COOLING",
            Power::Warming => "WARMING",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Link {
    #[default]
    Connecting,
    Online,
    Offline,
    AuthFailed,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Severity {
    #[default]
    Ok,
    Warning,
    Error,
}

/// The six PJLink error categories.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Errors {
    pub fan: Severity,
    pub lamp: Severity,
    pub temperature: Severity,
    pub cover: Severity,
    pub filter: Severity,
    pub other: Severity,
}

impl Errors {
    pub fn items(&self) -> [(&'static str, Severity); 6] {
        [
            ("Fan", self.fan),
            ("Lamp", self.lamp),
            ("Temperature", self.temperature),
            ("Cover", self.cover),
            ("Filter", self.filter),
            ("Other", self.other),
        ]
    }
    pub fn worst(&self) -> Severity {
        self.items()
            .iter()
            .map(|(_, s)| *s)
            .max()
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Lamp {
    pub hours: u32,
    pub on: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InputSource {
    pub code: String,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectorState {
    pub link: Link,
    pub power: Power,
    pub input: Option<String>,
    pub inputs: Vec<InputSource>,
    pub video_mute: Option<bool>,
    pub audio_mute: Option<bool>,
    pub freeze: Option<bool>,
    /// `None` when the protocol/projector does not report error status.
    pub errors: Option<Errors>,
    pub lamps: Vec<Lamp>,
    pub class: u8,
    pub device_name: String,
    pub manufacturer: String,
    pub model: String,
    pub info: String,
    pub serial: String,
    pub firmware: String,
    /// Why the projector is unreachable (when it is).
    pub link_error: Option<String>,
    /// Result of the last failed command, shown briefly in the UI.
    pub command_error: Option<(String, std::time::Instant)>,
    pub busy: bool,
}

impl ProjectorState {
    pub fn input_name(&self) -> Option<String> {
        let code = self.input.as_ref()?;
        Some(
            self.inputs
                .iter()
                .find(|i| &i.code == code)
                .map(|i| i.name.clone())
                .unwrap_or_else(|| code.clone()),
        )
    }

    /// Everything about this projector that deserves an operator's attention.
    pub fn issues(&self) -> Vec<(String, Severity)> {
        let mut out = Vec::new();
        match self.link {
            Link::Offline => out.push(("Offline".to_string(), Severity::Error)),
            Link::AuthFailed => out.push(("Password rejected".to_string(), Severity::Error)),
            _ => {}
        }
        if self.link == Link::Online {
            if let Some(e) = &self.errors {
                for (name, sev) in e.items() {
                    match sev {
                        Severity::Error => out.push((format!("{name} error"), sev)),
                        Severity::Warning => out.push((format!("{name} warning"), sev)),
                        Severity::Ok => {}
                    }
                }
            }
        }
        out.sort_by_key(|issue| std::cmp::Reverse(issue.1));
        out
    }

    pub fn severity(&self) -> Severity {
        self.issues()
            .iter()
            .map(|(_, s)| *s)
            .max()
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    PowerOn,
    PowerOff,
    /// Picture mute / shutter. `true` blanks the image.
    VideoMute(bool),
    AudioMute(bool),
    Input(String),
    Freeze(bool),
    VolumeUp,
    VolumeDown,
    Refresh,
}

impl Command {
    pub fn label(&self) -> String {
        match self {
            Command::PowerOn => "Power on".into(),
            Command::PowerOff => "Power off".into(),
            Command::VideoMute(true) => "Close shutter".into(),
            Command::VideoMute(false) => "Open shutter".into(),
            Command::AudioMute(true) => "Mute audio".into(),
            Command::AudioMute(false) => "Unmute audio".into(),
            Command::Input(c) => format!("Select input {c}"),
            Command::Freeze(true) => "Freeze".into(),
            Command::Freeze(false) => "Unfreeze".into(),
            Command::VolumeUp => "Volume up".into(),
            Command::VolumeDown => "Volume down".into(),
            Command::Refresh => "Refresh".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriverError {
    /// Network-level failure: unreachable, timeout, dropped.
    Io(String),
    /// The projector rejected our credentials (or needs some).
    Auth(String),
    /// The projector answered but refused or garbled the request.
    Protocol(String),
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::Io(s) | DriverError::Auth(s) | DriverError::Protocol(s) => f.write_str(s),
        }
    }
}

impl From<std::io::Error> for DriverError {
    fn from(e: std::io::Error) -> Self {
        DriverError::Io(crate::net::describe(&e))
    }
}

/// A protocol implementation. One instance per projector, owned by its worker thread.
pub trait Driver: Send {
    /// Refresh `st` from the projector. `publish` may be called mid-poll so the
    /// UI sees power state before the slower queries complete.
    fn poll(
        &mut self,
        st: &mut ProjectorState,
        publish: &dyn Fn(&ProjectorState),
    ) -> Result<(), DriverError>;
    fn command(&mut self, cmd: &Command) -> Result<(), DriverError>;
}

// ---------------------------------------------------------------- command log

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogKind {
    Tx,
    Rx,
    Info,
    Error,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub time: chrono::DateTime<chrono::Local>,
    pub kind: LogKind,
    pub text: String,
}

/// Per-projector ring buffer of protocol traffic, shared with the UI.
#[derive(Clone, Default)]
pub struct Log(Arc<Mutex<VecDeque<LogEntry>>>);

impl Log {
    const CAP: usize = 400;

    pub fn push(&self, kind: LogKind, text: impl Into<String>) {
        let mut q = self.0.lock().unwrap();
        if q.len() >= Self::CAP {
            q.pop_front();
        }
        q.push_back(LogEntry {
            time: chrono::Local::now(),
            kind,
            text: text.into(),
        });
    }
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.0.lock().unwrap().iter().cloned().collect()
    }
    pub fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}
