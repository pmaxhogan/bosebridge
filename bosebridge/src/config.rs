//! `%APPDATA%\bosebridge\config.toml`. Every field is optional; anything
//! missing is auto-detected or defaulted at startup and written back so the
//! file documents what the tool decided.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Bluetooth address of the headphones, e.g. "68:F2:1F:37:02:82".
    /// Auto-detected from the Bose serial port if absent.
    pub headphones_mac: Option<String>,
    /// Serial port bound to the headphones' SPP service, e.g. "COM6".
    /// Resolved from the registry by MAC if absent.
    pub port: Option<String>,
    /// Substring of the Windows audio render endpoint name that proves audio
    /// is up, e.g. "phones". Defaults to the headphones' Bluetooth name.
    pub endpoint_match: Option<String>,
    /// This PC's Bluetooth address. Read from the adapter if absent.
    pub local_mac: Option<String>,
    /// How often the watcher looks (seconds).
    pub poll_secs: u64,
    /// Link up without audio for this long before the first nudge.
    pub debounce_secs: u64,
    /// Minimum gap between nudges.
    pub cooldown_secs: u64,
    /// Nudges per link session before giving up.
    pub max_attempts: u32,
    /// Whether the watcher nudges automatically (the tray toggle).
    pub auto_reconnect: bool,
    /// Keep the serial port open between nudges. Off by default: holding the
    /// link may defeat the headphones' auto-off and costs battery.
    pub keep_port_open: bool,
    /// Global hotkey for "connect now", in global-hotkey syntax.
    pub hotkey: Option<String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            headphones_mac: None,
            port: None,
            endpoint_match: None,
            local_mac: None,
            poll_secs: 3,
            debounce_secs: 8,
            cooldown_secs: 30,
            max_attempts: 4,
            auto_reconnect: true,
            keep_port_open: false,
            hotkey: Some("Ctrl+Alt+Shift+H".to_string()),
        }
    }
}

impl Config {
    pub fn dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("bosebridge")
    }

    pub fn path() -> PathBuf {
        Config::dir().join("config.toml")
    }

    pub fn log_path() -> PathBuf {
        Config::dir().join("bosebridge.log")
    }

    pub fn load() -> Result<Config> {
        let path = Config::path();
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        Config::parse(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        Ok(toml::from_str(text)?)
    }

    pub fn save(&self) -> Result<()> {
        let dir = Config::dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let text = toml::to_string_pretty(self)?;
        std::fs::write(Config::path(), text).with_context(|| format!("writing {}", Config::path().display()))
    }

    pub fn policy(&self) -> decision::Policy {
        decision::Policy {
            debounce: std::time::Duration::from_secs(self.debounce_secs),
            cooldown: std::time::Duration::from_secs(self.cooldown_secs),
            max_attempts: self.max_attempts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_is_defaults() {
        assert_eq!(Config::parse("").unwrap(), Config::default());
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let c = Config::parse("port = \"COM6\"\npoll_secs = 10\nauto_reconnect = false\n").unwrap();
        assert_eq!(c.port.as_deref(), Some("COM6"));
        assert_eq!(c.poll_secs, 10);
        assert!(!c.auto_reconnect);
        assert_eq!(c.debounce_secs, 8);
        assert_eq!(c.hotkey.as_deref(), Some("Ctrl+Alt+Shift+H"));
    }

    #[test]
    fn round_trips_through_toml() {
        let c = Config {
            headphones_mac: Some("68:F2:1F:37:02:82".into()),
            endpoint_match: Some("phones".into()),
            keep_port_open: true,
            ..Config::default()
        };
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), c);
    }

    #[test]
    fn policy_reflects_config() {
        let c = Config {
            debounce_secs: 1,
            cooldown_secs: 2,
            max_attempts: 9,
            ..Config::default()
        };
        let p = c.policy();
        assert_eq!(p.debounce.as_secs(), 1);
        assert_eq!(p.cooldown.as_secs(), 2);
        assert_eq!(p.max_attempts, 9);
    }

    #[test]
    fn bad_toml_is_an_error() {
        assert!(Config::parse("poll_secs = \"lots\"").is_err());
    }

    #[test]
    fn paths_live_under_a_bosebridge_dir() {
        assert!(
            Config::path().ends_with("bosebridge/config.toml") || Config::path().ends_with("bosebridge\\config.toml")
        );
        assert_eq!(Config::log_path().file_name().unwrap(), "bosebridge.log");
    }
}
