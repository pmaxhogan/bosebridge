//! Start the tray at logon via the per-user Run key. No admin rights needed.

#![cfg_attr(not(windows), allow(dead_code))]

use anyhow::Result;

pub const VALUE_NAME: &str = "bosebridge";

/// The command line the Run key should hold for this executable.
pub fn command_line(exe: &std::path::Path) -> String {
    format!("\"{}\" tray", exe.display())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::Context;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub fn install() -> Result<String> {
        let exe = std::env::current_exe().context("finding this executable")?;
        let cmd = command_line(&exe);
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_WRITE)
            .context("opening HKCU Run key for writing")?;
        key.set_value(VALUE_NAME, &cmd).context("writing the Run value")?;
        Ok(cmd)
    }

    pub fn uninstall() -> Result<bool> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_WRITE)
            .context("opening HKCU Run key for writing")?;
        match key.delete_value(VALUE_NAME) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e).context("deleting the Run value"),
        }
    }

    pub fn current() -> Result<Option<String>> {
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, KEY_READ)
            .context("opening HKCU Run key")?;
        match key.get_value::<String, _>(VALUE_NAME) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context("reading the Run value"),
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use anyhow::anyhow;

    pub fn install() -> Result<String> {
        Err(anyhow!("autostart needs Windows"))
    }
    pub fn uninstall() -> Result<bool> {
        Err(anyhow!("autostart needs Windows"))
    }
    pub fn current() -> Result<Option<String>> {
        Err(anyhow!("autostart needs Windows"))
    }
}

pub use imp::{current, install, uninstall};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_the_path_and_runs_the_tray() {
        let p = std::path::Path::new("C:\\Tools\\bosebridge.exe");
        assert_eq!(command_line(p), "\"C:\\Tools\\bosebridge.exe\" tray");
    }

    #[cfg(not(windows))]
    #[test]
    fn stubs_report_windows_only() {
        assert!(install().is_err());
        assert!(uninstall().is_err());
        assert!(current().is_err());
    }
}
