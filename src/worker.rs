//! One background thread per projector: polls status, runs commands, and
//! reconnects with backoff. The UI only ever reads the shared state snapshot.

use crate::model::*;
use crate::panasonic::PanasonicDriver;
use crate::pjlink::PjLinkDriver;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Repaint = Arc<dyn Fn() + Send + Sync>;

const POLL_STEADY: Duration = Duration::from_secs(4);
const POLL_TRANSITION: Duration = Duration::from_secs(1);
const POLL_AFTER_COMMAND: Duration = Duration::from_millis(350);
const BACKOFF_MAX: Duration = Duration::from_secs(20);
const AUTH_RETRY: Duration = Duration::from_secs(15);
/// A projector that has not answered yet is "checking", not "offline", for
/// this long after its worker starts. The first connections of a freshly
/// launched app can fail for reasons that pass within a moment (the system
/// still deciding on local network access, a link just coming up), and those
/// must not flash up as faults.
pub const STARTUP_GRACE: Duration = Duration::from_secs(4);
const STARTUP_RETRY: Duration = Duration::from_secs(1);

enum Msg {
    Cmd(Command),
    Stop,
}

pub struct Handle {
    pub cfg: ProjectorConfig,
    pub state: Arc<Mutex<ProjectorState>>,
    pub log: Log,
    tx: Sender<Msg>,
}

impl Handle {
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Cmd(cmd));
    }
    pub fn snapshot(&self) -> ProjectorState {
        self.state.lock().unwrap().clone()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

pub fn make_driver(cfg: &ProjectorConfig, log: Log) -> Box<dyn Driver> {
    match cfg.protocol {
        Protocol::PjLink => Box::new(PjLinkDriver::new(&cfg.host, cfg.port, &cfg.password, log)),
        Protocol::Panasonic => Box::new(PanasonicDriver::new(
            &cfg.host,
            cfg.port,
            &cfg.username,
            &cfg.password,
            log,
        )),
    }
}

pub fn spawn(cfg: ProjectorConfig, repaint: Repaint) -> Handle {
    let state = Arc::new(Mutex::new(ProjectorState::default()));
    let log = Log::default();
    let (tx, rx) = mpsc::channel::<Msg>();

    let shared = state.clone();
    let thread_log = log.clone();
    let thread_cfg = cfg.clone();
    std::thread::Builder::new()
        .name(format!("projector-{}", cfg.id))
        .spawn(move || {
            let mut driver = make_driver(&thread_cfg, thread_log.clone());
            let mut st = ProjectorState::default();
            let publish = |s: &ProjectorState| {
                *shared.lock().unwrap() = s.clone();
                repaint();
            };
            let mut failures: u32 = 0;
            let started = Instant::now();
            let mut next_poll = Instant::now();

            loop {
                let wait = next_poll.saturating_duration_since(Instant::now());
                match rx.recv_timeout(wait) {
                    Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                    Ok(Msg::Cmd(Command::Refresh)) => {}
                    Ok(Msg::Cmd(cmd)) => {
                        st.busy = true;
                        st.command_error = None;
                        publish(&st);
                        thread_log.push(LogKind::Info, cmd.label());
                        if let Err(e) = driver.command(&cmd) {
                            let msg = format!("{} failed: {e}", cmd.label());
                            thread_log.push(LogKind::Error, msg.clone());
                            st.command_error = Some((msg, Instant::now()));
                        }
                        st.busy = false;
                        publish(&st);
                        // Let the projector act, then confirm the new state.
                        next_poll = Instant::now() + POLL_AFTER_COMMAND;
                        continue;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }

                let interval = match driver.poll(&mut st, &publish) {
                    Ok(()) => {
                        if failures > 0 || st.link != Link::Online {
                            thread_log.push(LogKind::Info, "Online");
                        }
                        failures = 0;
                        st.link = Link::Online;
                        st.link_error = None;
                        match st.power {
                            Power::Warming | Power::Cooling => POLL_TRANSITION,
                            _ => POLL_STEADY,
                        }
                    }
                    Err(DriverError::Auth(msg)) => {
                        if st.link != Link::AuthFailed {
                            thread_log.push(LogKind::Error, msg.clone());
                        }
                        st.link = Link::AuthFailed;
                        st.link_error = Some(msg);
                        st.power = Power::Unknown;
                        AUTH_RETRY
                    }
                    Err(e) => {
                        failures += 1;
                        // One missed poll is tolerated before declaring a
                        // previously healthy projector offline, and one that
                        // has never answered gets the startup grace.
                        let settling =
                            st.link == Link::Connecting && started.elapsed() < STARTUP_GRACE;
                        let tolerated = st.link == Link::Online && failures < 2;
                        if settling {
                            thread_log.push(LogKind::Info, format!("No answer yet: {e}"));
                            publish(&st);
                            next_poll = Instant::now() + STARTUP_RETRY;
                            continue;
                        }
                        if !tolerated {
                            if st.link != Link::Offline {
                                thread_log.push(LogKind::Error, format!("Offline: {e}"));
                            }
                            st.link = Link::Offline;
                            st.link_error = Some(e.to_string());
                            st.power = Power::Unknown;
                            st.video_mute = None;
                            st.audio_mute = None;
                            st.freeze = None;
                        }
                        Duration::from_secs(1u64 << failures.min(5)).min(BACKOFF_MAX)
                    }
                };
                publish(&st);
                next_poll = Instant::now() + interval;
            }
        })
        .expect("spawn projector worker");

    Handle {
        cfg,
        state,
        log,
        tx,
    }
}

/// Who a group action or schedule applies to.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum Target {
    All,
    Group(String),
    Projector(u64),
}

impl Target {
    pub fn matches(&self, cfg: &ProjectorConfig) -> bool {
        match self {
            Target::All => true,
            Target::Group(g) => cfg.group.as_deref() == Some(g.as_str()),
            Target::Projector(id) => cfg.id == *id,
        }
    }
}

/// The set of live projector workers.
pub struct Hub {
    handles: Vec<Handle>,
    repaint: Repaint,
}

impl Hub {
    pub fn new(repaint: Repaint) -> Self {
        Self {
            handles: Vec::new(),
            repaint,
        }
    }

    /// Start (or restart, if the config changed) the worker for `cfg`.
    pub fn upsert(&mut self, cfg: ProjectorConfig) {
        if let Some(h) = self.handles.iter_mut().find(|h| h.cfg.id == cfg.id) {
            let same_connection = h.cfg.host == cfg.host
                && h.cfg.port == cfg.port
                && h.cfg.protocol == cfg.protocol
                && h.cfg.username == cfg.username
                && h.cfg.password == cfg.password;
            if same_connection {
                h.cfg = cfg;
            } else {
                *h = spawn(cfg, self.repaint.clone());
            }
        } else {
            self.handles.push(spawn(cfg, self.repaint.clone()));
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.handles.retain(|h| h.cfg.id != id);
    }

    pub fn get(&self, id: u64) -> Option<&Handle> {
        self.handles.iter().find(|h| h.cfg.id == id)
    }

    pub fn send(&self, id: u64, cmd: Command) {
        if let Some(h) = self.get(id) {
            h.send(cmd);
        }
    }

    pub fn send_target(&self, target: &Target, cmd: Command) -> usize {
        let mut n = 0;
        for h in self.handles.iter().filter(|h| target.matches(&h.cfg)) {
            h.send(cmd.clone());
            n += 1;
        }
        n
    }

    pub fn snapshot(&self) -> Vec<(ProjectorConfig, ProjectorState)> {
        self.handles
            .iter()
            .map(|h| (h.cfg.clone(), h.snapshot()))
            .collect()
    }
}
