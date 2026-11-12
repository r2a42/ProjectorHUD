//! Persistent configuration, stored as JSON in the platform config directory.

use crate::model::ProjectorConfig;
use crate::schedule::Schedule;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub projectors: Vec<ProjectorConfig>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub schedules: Vec<Schedule>,
    #[serde(default)]
    pub next_id: u64,
    #[serde(default)]
    pub demo: bool,
}

impl AppConfig {
    pub fn path() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("PROJECTORHUD_CONFIG") {
            return Some(PathBuf::from(p));
        }
        directories::ProjectDirs::from("", "", "ProjectorHUD")
            .map(|d| d.config_dir().join("config.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Write atomically (temp file + rename) so a crash can't truncate the config.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        #[cfg(unix)]
        {
            // The file may hold projector passwords: owner-only.
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(tmp, path)
    }

    pub fn alloc_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}
