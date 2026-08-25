//! Global plugin settings & favorite paths persistence (~/.config/sliceplayer/settings.json).

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GlobalSettings {
    pub favorites: [Option<PathBuf>; 5],
    pub last_dir: Option<PathBuf>,
    pub default_export_dir: Option<PathBuf>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            favorites: [None, None, None, None, None],
            last_dir: None,
            default_export_dir: None,
        }
    }
}

pub fn get_config_path() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".into())
    } else if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/Library/Application Support")
    } else {
        std::env::var("XDG_CONFIG_HOME")
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                format!("{home}/.config")
            })
    };
    PathBuf::from(base).join("sliceplayer").join("settings.json")
}

pub fn load_global_settings() -> GlobalSettings {
    let path = get_config_path();
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(settings) = serde_json::from_str::<GlobalSettings>(&content) {
                return settings;
            }
        }
    }
    GlobalSettings::default()
}

pub fn save_global_settings(settings: &GlobalSettings) {
    let path = get_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_settings_serde() {
        let mut settings = GlobalSettings::default();
        settings.last_dir = Some(PathBuf::from("/tmp/samples"));
        settings.favorites[0] = Some(PathBuf::from("/tmp/samples/jungle"));

        let json = serde_json::to_string(&settings).unwrap();
        let restored: GlobalSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.last_dir, Some(PathBuf::from("/tmp/samples")));
        assert_eq!(restored.favorites[0], Some(PathBuf::from("/tmp/samples/jungle")));
    }
}
