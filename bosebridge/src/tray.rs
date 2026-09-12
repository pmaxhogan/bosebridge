//! The tray icon: a thin shell over the watch loop. The loop runs on a worker
//! thread; this thread owns the icon, menu, and hotkey and refreshes them from
//! the shared status once a second.

#![cfg_attr(not(windows), allow(dead_code))]

use crate::config::Config;
use crate::watch::{self, Resolved};
use anyhow::Result;

/// Tray icon colour states.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glyph {
    /// Link down: grey.
    Idle,
    /// Link up and audio present: green.
    Good,
    /// Link up, no audio, or nudging: orange.
    Working,
    /// Last action failed: red.
    Failed,
}

impl Glyph {
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Glyph::Idle => [128, 128, 128],
            Glyph::Good => [46, 160, 67],
            Glyph::Working => [230, 140, 20],
            Glyph::Failed => [200, 50, 50],
        }
    }
}

/// A filled circle with a thin dark outline, as RGBA. Generated at runtime so
/// the binary ships no image assets.
pub fn glyph_rgba(glyph: Glyph, size: u32) -> Vec<u8> {
    let [r, g, b] = glyph.rgb();
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    let c = (size as f32 - 1.0) / 2.0;
    let radius = c - 0.5;
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let (rr, gg, bb, a) = if d <= radius - 1.5 {
                (r, g, b, 255)
            } else if d <= radius {
                (30, 30, 30, 255)
            } else if d <= radius + 1.0 {
                (30, 30, 30, ((radius + 1.0 - d) * 255.0) as u8)
            } else {
                (0, 0, 0, 0)
            };
            px.extend_from_slice(&[rr, gg, bb, a]);
        }
    }
    px
}

/// Which glyph a status snapshot deserves.
pub fn glyph_for(status: &watch::Status) -> Glyph {
    if status.last_error.is_some() {
        Glyph::Failed
    } else if !status.link_up {
        Glyph::Idle
    } else if status.endpoint_present {
        Glyph::Good
    } else {
        Glyph::Working
    }
}

#[cfg(windows)]
pub fn run(config: Config, resolved: Resolved) -> Result<()> {
    use crate::watch::{Command, SharedStatus};
    use anyhow::Context;
    use global_hotkey::hotkey::HotKey;
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIconBuilder};

    let _guard = single_instance::acquire()?;

    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let status: SharedStatus = Arc::new(Mutex::new(Default::default()));
    {
        let config = config.clone();
        let resolved = resolved.clone();
        let status = status.clone();
        std::thread::Builder::new()
            .name("watch".into())
            .spawn(move || {
                if let Err(e) = watch::run(&config, &resolved, cmd_rx, status, None) {
                    log::error!("watch loop ended: {e:#}");
                }
            })
            .context("spawning the watch thread")?;
    }

    let event_loop = EventLoopBuilder::new().build();

    let menu = Menu::new();
    let status_item = MenuItem::new("starting", false, None);
    let connect_item = MenuItem::new("Connect headphones now", true, None);
    let auto_item = CheckMenuItem::new("Auto-reconnect", true, config.auto_reconnect, None);
    let log_item = MenuItem::new("Open log", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append_items(&[
        &status_item,
        &PredefinedMenuItem::separator(),
        &connect_item,
        &auto_item,
        &PredefinedMenuItem::separator(),
        &log_item,
        &quit_item,
    ])
    .context("building the tray menu")?;

    let make_icon = |g: Glyph| Icon::from_rgba(glyph_rgba(g, 32), 32, 32).context("building the tray icon");
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(format!("bosebridge {}", crate::cli::VERSION))
        .with_icon(make_icon(Glyph::Idle)?)
        .build()
        .context("creating the tray icon")?;

    let hotkey_manager = GlobalHotKeyManager::new().context("creating the hotkey manager")?;
    let hotkey: Option<HotKey> = match config.hotkey.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(spec) => match spec.parse::<HotKey>() {
            Ok(hk) => match hotkey_manager.register(hk) {
                Ok(()) => {
                    log::info!("hotkey {spec} registered");
                    Some(hk)
                }
                Err(e) => {
                    log::warn!("could not register hotkey {spec}: {e}");
                    None
                }
            },
            Err(e) => {
                log::warn!("hotkey '{spec}' does not parse: {e}");
                None
            }
        },
        None => None,
    };

    let menu_rx = MenuEvent::receiver();
    let hotkey_rx = GlobalHotKeyEvent::receiver();
    let mut shown_glyph = Glyph::Idle;
    let mut shown_summary = String::new();
    let mut next_refresh = Instant::now();

    event_loop.run(move |_event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_refresh);

        while let Ok(ev) = menu_rx.try_recv() {
            if ev.id == connect_item.id() {
                let _ = cmd_tx.send(Command::ConnectNow);
            } else if ev.id == auto_item.id() {
                let _ = cmd_tx.send(Command::SetAuto(auto_item.is_checked()));
            } else if ev.id == log_item.id() {
                let _ = std::process::Command::new("notepad.exe")
                    .arg(Config::log_path())
                    .spawn();
            } else if ev.id == quit_item.id() {
                let _ = cmd_tx.send(Command::Quit);
                *control_flow = ControlFlow::Exit;
            }
        }
        while let Ok(ev) = hotkey_rx.try_recv() {
            if Some(ev.id) == hotkey.map(|h| h.id()) && ev.state == HotKeyState::Pressed {
                log::info!("hotkey pressed");
                let _ = cmd_tx.send(Command::ConnectNow);
            }
        }

        if Instant::now() >= next_refresh {
            next_refresh = Instant::now() + Duration::from_secs(1);
            let s = status.lock().unwrap().clone();
            let glyph = glyph_for(&s);
            if glyph != shown_glyph {
                if let Ok(icon) = make_icon(glyph) {
                    let _ = tray.set_icon(Some(icon));
                }
                shown_glyph = glyph;
            }
            let summary = match &s.last_error {
                Some(e) => format!("{} ({})", s.summary, e),
                None => s.summary.clone(),
            };
            if summary != shown_summary {
                status_item.set_text(if summary.is_empty() { "starting" } else { &summary });
                let _ = tray.set_tooltip(Some(format!(
                    "bosebridge: {}",
                    if summary.is_empty() { "starting" } else { &summary }
                )));
                shown_summary = summary;
            }
            *control_flow = ControlFlow::WaitUntil(next_refresh);
        }
    });
}

#[cfg(not(windows))]
pub fn run(_config: Config, _resolved: Resolved) -> Result<()> {
    Err(anyhow::anyhow!("the tray needs Windows"))
}

#[cfg(windows)]
mod single_instance {
    use anyhow::{anyhow, Result};
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::CreateMutexW;

    pub struct Guard(HANDLE);

    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Fails if another bosebridge tray is already running for this user.
    pub fn acquire() -> Result<Guard> {
        unsafe {
            let handle = CreateMutexW(None, false, w!("Local\\bosebridge-tray"))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Err(anyhow!("bosebridge is already running in the tray"));
            }
            Ok(Guard(handle))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_reflects_status() {
        let mut s = watch::Status::default();
        assert_eq!(glyph_for(&s), Glyph::Idle);
        s.link_up = true;
        assert_eq!(glyph_for(&s), Glyph::Working);
        s.endpoint_present = true;
        assert_eq!(glyph_for(&s), Glyph::Good);
        s.last_error = Some("x".into());
        assert_eq!(glyph_for(&s), Glyph::Failed);
    }

    #[test]
    fn icon_is_opaque_in_the_middle_and_clear_at_corners() {
        let px = glyph_rgba(Glyph::Good, 32);
        assert_eq!(px.len(), 32 * 32 * 4);
        let center = (16 * 32 + 16) * 4;
        assert_eq!(&px[center..center + 4], &[46, 160, 67, 255]);
        assert_eq!(px[3], 0);
        let last = (31 * 32 + 31) * 4;
        assert_eq!(px[last + 3], 0);
        for g in [Glyph::Idle, Glyph::Working, Glyph::Failed] {
            let p = glyph_rgba(g, 16);
            let c = (8 * 16 + 8) * 4;
            assert_eq!(&p[c..c + 3], &g.rgb());
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn tray_is_windows_only() {
        let r = Resolved {
            headphones_mac: bmap::Mac([0; 6]),
            port: "COM0".into(),
            local_mac: bmap::Mac([0; 6]),
            endpoint_match: "(x)".into(),
            headphones_name: "x".into(),
        };
        assert!(run(Config::default(), r).is_err());
    }
}
