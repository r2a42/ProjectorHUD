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
    #[serde(default)]
    pub sort: SortKey,
    /// Size of everything in the window; 1.0 is normal. Smaller shows more
    /// projectors at once.
    #[serde(default)]
    pub zoom: f32,
    /// Compact mode: one line per projector instead of a card.
    #[serde(default)]
    pub compact: bool,
}

pub const ZOOM_MIN: f32 = 0.5;
pub const ZOOM_MAX: f32 = 1.6;

/// A usable zoom from whatever was saved (older files have none).
pub fn clamp_zoom(z: f32) -> f32 {
    if z.is_finite() && z > 0.0 {
        ((z * 10.0).round() / 10.0).clamp(ZOOM_MIN, ZOOM_MAX)
    } else {
        1.0
    }
}

/// How projectors are ordered in the window.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Name,
    Address,
    Type,
}

impl SortKey {
    pub const ALL: [SortKey; 3] = [SortKey::Name, SortKey::Address, SortKey::Type];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Address => "IP address",
            SortKey::Type => "Type",
        }
    }
}

/// Orders hosts so that 10.0.0.9 comes before 10.0.0.10, with host names after addresses.
pub fn address_key(host: &str, port: u16) -> (u8, u32, String, u16) {
    match host.trim().parse::<std::net::Ipv4Addr>() {
        Ok(ip) => (0, u32::from(ip), String::new(), port),
        Err(_) => (1, 0, host.trim().to_lowercase(), port),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_sort_numerically() {
        let mut hosts = vec!["10.0.0.10", "projector.local", "10.0.0.9", "2.0.0.34"];
        hosts.sort_by_key(|h| address_key(h, 4352));
        assert_eq!(
            hosts,
            vec!["2.0.0.34", "10.0.0.9", "10.0.0.10", "projector.local"]
        );
    }
}
