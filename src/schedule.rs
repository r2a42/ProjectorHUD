//! Timed actions (e.g. "power everything off at 23:00 on weekdays").
//! Schedules run only while the app is open.

use crate::model::Command;
use crate::worker::{Hub, Target};
use chrono::{Datelike, Local, NaiveDate, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    PowerOn,
    PowerOff,
    ShutterClose,
    ShutterOpen,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::PowerOn, Action::PowerOff, Action::ShutterClose, Action::ShutterOpen];

    pub fn label(self) -> &'static str {
        match self {
            Action::PowerOn => "Power on",
            Action::PowerOff => "Power off",
            Action::ShutterClose => "Close shutter",
            Action::ShutterOpen => "Open shutter",
        }
    }
    pub fn command(self) -> Command {
        match self {
            Action::PowerOn => Command::PowerOn,
            Action::PowerOff => Command::PowerOff,
            Action::ShutterClose => Command::VideoMute(true),
            Action::ShutterOpen => Command::VideoMute(false),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub id: u64,
    pub enabled: bool,
    pub hour: u32,
    pub minute: u32,
    /// Monday first.
    pub days: [bool; 7],
    pub action: Action,
    pub target: Target,
}

impl Schedule {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            enabled: true,
            hour: 9,
            minute: 0,
            days: [true, true, true, true, true, false, false],
            action: Action::PowerOn,
            target: Target::All,
        }
    }

    /// Should this fire at the given local weekday (0 = Monday) and time?
    pub fn due(&self, weekday: usize, hour: u32, minute: u32) -> bool {
        self.enabled && self.days[weekday % 7] && self.hour == hour && self.minute == minute
    }
}

/// Shared with the UI so it can show when something last ran.
pub type LastRun = Arc<Mutex<Option<String>>>;

pub fn start(schedules: Arc<Mutex<Vec<Schedule>>>, hub: Arc<Mutex<Hub>>, last_run: LastRun) {
    std::thread::Builder::new()
        .name("scheduler".into())
        .spawn(move || {
            // Remember the minute each schedule last fired so it runs exactly once.
            let mut fired: HashMap<u64, (NaiveDate, u32)> = HashMap::new();
            loop {
                std::thread::sleep(Duration::from_secs(5));
                let now = Local::now();
                let stamp = (now.date_naive(), now.hour() * 60 + now.minute());
                let weekday = now.weekday().num_days_from_monday() as usize;
                let due: Vec<Schedule> = schedules
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|s| s.due(weekday, now.hour(), now.minute()))
                    .filter(|s| fired.get(&s.id) != Some(&stamp))
                    .cloned()
                    .collect();
                for s in due {
                    fired.insert(s.id, stamp);
                    let n = hub.lock().unwrap().send_target(&s.target, s.action.command());
                    *last_run.lock().unwrap() = Some(format!(
                        "{} · {} sent to {} projector{}",
                        now.format("%a %H:%M"),
                        s.action.label(),
                        n,
                        if n == 1 { "" } else { "s" }
                    ));
                }
            }
        })
        .expect("spawn scheduler");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_respects_day_time_and_enabled() {
        let mut s = Schedule::new(1);
        assert!(s.due(0, 9, 0));
        assert!(!s.due(5, 9, 0), "weekend is off by default");
        assert!(!s.due(0, 9, 1));
        s.enabled = false;
        assert!(!s.due(0, 9, 0));
    }
}
