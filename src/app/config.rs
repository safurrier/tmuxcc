use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::notifications::NotificationConfig;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Polling interval in milliseconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval_ms: u64,

    /// Number of lines to capture from pane
    #[serde(default = "default_capture_lines")]
    pub capture_lines: u32,

    /// Custom agent patterns (command -> agent type mapping)
    #[serde(default)]
    pub agent_patterns: Vec<AgentPattern>,

    /// PR polling interval in milliseconds (default 60s)
    #[serde(default = "default_pr_poll_interval")]
    pub pr_poll_interval_ms: u64,

    /// Whether PR monitoring is enabled
    #[serde(default = "default_pr_enabled")]
    pub pr_enabled: bool,

    /// Notification settings
    #[serde(default)]
    pub notifications: NotificationConfig,

    /// Running inside a tmux popup (auto-quit on focus/go)
    #[serde(skip)]
    pub popup: bool,
}

fn default_poll_interval() -> u64 {
    500
}

fn default_capture_lines() -> u32 {
    100
}

fn default_pr_poll_interval() -> u64 {
    60_000
}

fn default_pr_enabled() -> bool {
    true
}

/// Pattern for detecting agent types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPattern {
    /// Command pattern to match (regex)
    pub pattern: String,
    /// Name of the agent type
    pub agent_type: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval_ms: default_poll_interval(),
            capture_lines: default_capture_lines(),
            agent_patterns: Vec::new(),
            pr_poll_interval_ms: default_pr_poll_interval(),
            pr_enabled: default_pr_enabled(),
            notifications: NotificationConfig::default(),
            popup: false,
        }
    }
}

impl Config {
    /// Returns the default config file path
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("tmuxcc").join("config.toml"))
    }

    /// Loads config from the default path or returns defaults
    pub fn load() -> Self {
        Self::default_path()
            .and_then(|path| {
                if path.exists() {
                    Self::load_from(&path).ok()
                } else {
                    None
                }
            })
            .unwrap_or_default()
    }

    /// Loads config from a specific path
    pub fn load_from(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut config: Config = toml::from_str(&content)?;
        config.notifications.migrate_legacy();
        Ok(config)
    }

    /// Saves config to the default path
    pub fn save(&self) -> Result<()> {
        if let Some(path) = Self::default_path() {
            self.save_to(&path)?;
        }
        Ok(())
    }

    /// Saves config to a specific path
    pub fn save_to(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.poll_interval_ms, 500);
        assert_eq!(config.capture_lines, 100);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(config.poll_interval_ms, parsed.poll_interval_ms);
    }

    #[test]
    fn test_config_pr_defaults() {
        let config = Config::default();
        assert_eq!(config.pr_poll_interval_ms, 60_000);
        assert!(config.pr_enabled);
    }

    #[test]
    fn test_config_pr_roundtrip() {
        let mut config = Config::default();
        config.pr_poll_interval_ms = 60_000;
        config.pr_enabled = false;
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.pr_poll_interval_ms, 60_000);
        assert!(!parsed.pr_enabled);
    }

    #[test]
    fn test_config_missing_pr_fields_uses_defaults() {
        let toml_str = r#"poll_interval_ms = 500"#;
        let parsed: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.pr_poll_interval_ms, 60_000);
        assert!(parsed.pr_enabled);
    }

    #[test]
    fn test_config_with_profiles() {
        let toml_str = r#"
poll_interval_ms = 500

[notifications]
active_profile = "itysl"

[notifications.profiles.default]
completed = "Tink"
needs_input = "Ping"
cycle = "random"

[notifications.profiles.itysl]
completed = "~/.config/tmuxcc/sounds/completed/"
needs_input = "~/.config/tmuxcc/sounds/needs_input/"
cycle = "random"
"#;
        let parsed: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.notifications.active_profile, "itysl");
        assert!(parsed.notifications.profiles.contains_key("default"));
        assert!(parsed.notifications.profiles.contains_key("itysl"));
        let sounds = parsed.notifications.resolve_sounds();
        assert_eq!(sounds.completed, "~/.config/tmuxcc/sounds/completed/");
    }
}
