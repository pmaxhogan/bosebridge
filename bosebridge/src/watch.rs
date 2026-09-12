//! The background loop: observe Windows, ask the decision crate what to do,
//! and nudge the headphones when told. Runs on a worker thread; the tray and
//! CLI talk to it through `Command`s and read `Status` snapshots.

use crate::config::Config;
use crate::transport::Headphones;
use crate::win;
use anyhow::{anyhow, Context, Result};
use bmap::Mac;
use decision::{Action, Observation, Watcher};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Everything the loop needs, with every optional config field resolved.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub headphones_mac: Mac,
    pub port: String,
    pub local_mac: Mac,
    pub endpoint_match: String,
    pub headphones_name: String,
}

/// Fill in the blanks from Windows, and persist what was learned.
pub fn resolve(config: &mut Config) -> Result<Resolved> {
    let ports = win::spp_ports().context("looking up Bluetooth serial ports")?;
    let headphones_mac = match &config.headphones_mac {
        Some(s) => Mac::parse(s).with_context(|| format!("config headphones_mac '{s}'"))?,
        None => {
            let bose: Vec<_> = ports.iter().filter(|p| p.is_bose).collect();
            let chosen = bose.first().ok_or_else(|| {
                anyhow!(
                    "no Bose headphones with a serial service found; pair them, or set headphones_mac in {}",
                    Config::path().display()
                )
            })?;
            if bose.len() > 1 {
                log::warn!("{} Bose devices have serial ports; using {}", bose.len(), chosen.mac);
            }
            chosen.mac
        }
    };
    let port = match &config.port {
        Some(p) => p.clone(),
        None => ports
            .iter()
            .find(|p| p.mac == headphones_mac)
            .map(|p| p.port.clone())
            .ok_or_else(|| anyhow!("no serial port registered for {headphones_mac}"))?,
    };
    let local_mac = match &config.local_mac {
        Some(s) => Mac::parse(s).with_context(|| format!("config local_mac '{s}'"))?,
        None => win::local_mac().context("reading this PC's Bluetooth address")?,
    };
    let headphones_name = win::device_name(headphones_mac).unwrap_or_else(|e| {
        log::warn!("could not read the headphones' name: {e}");
        headphones_mac.to_string()
    });
    let endpoint_match = match &config.endpoint_match {
        Some(m) if !m.trim().is_empty() => m.clone(),
        _ => win::default_endpoint_match(&headphones_name),
    };

    let before = config.clone();
    config.headphones_mac = Some(headphones_mac.to_string());
    config.port = Some(port.clone());
    config.local_mac = Some(local_mac.to_string());
    config.endpoint_match = Some(endpoint_match.clone());
    if *config != before {
        if let Err(e) = config.save() {
            log::warn!("could not save config: {e}");
        }
    }
    Ok(Resolved {
        headphones_mac,
        port,
        local_mac,
        endpoint_match,
        headphones_name,
    })
}

/// Open the port, send CONNECT for this PC, close. Returns the headphones' result bytes.
pub fn nudge(r: &Resolved) -> Result<bmap::ConnectResult> {
    let mut hp = Headphones::open(&r.port)?;
    let res = hp.connect(r.local_mac)?;
    log::info!(
        "headphones acknowledged connect for {} (extra {})",
        res.mac,
        bmap::hex(&res.extra)
    );
    Ok(res)
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)] // the tray shell sends these; the CLI only drives Quit via channel drop
pub enum Command {
    ConnectNow,
    SetAuto(bool),
    Quit,
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub link_up: bool,
    pub endpoint_present: bool,
    pub auto: bool,
    pub summary: String,
    pub last_error: Option<String>,
    pub updated: Option<Instant>,
}

pub type SharedStatus = Arc<Mutex<Status>>;

/// Runs until `Command::Quit` (or, with `max_ticks`, that many observations).
pub fn run(
    config: &Config,
    resolved: &Resolved,
    commands: Receiver<Command>,
    status: SharedStatus,
    max_ticks: Option<u32>,
) -> Result<()> {
    let mut watcher = Watcher::new(config.policy());
    let mut auto = config.auto_reconnect;
    let poll = Duration::from_secs(config.poll_secs.max(1));
    let mut last_summary = String::new();
    let mut ticks = 0u32;
    log::info!(
        "watching {} ({}) on {}; audio endpoint match '{}'; auto-reconnect {}",
        resolved.headphones_name,
        resolved.headphones_mac,
        resolved.port,
        resolved.endpoint_match,
        if auto { "on" } else { "off" }
    );

    loop {
        let now = Instant::now();
        let obs = match observe(resolved) {
            Ok(o) => o,
            Err(e) => {
                log::warn!("observation failed: {e:#}");
                set_error(&status, format!("{e:#}"));
                Observation {
                    link_up: false,
                    endpoint_present: false,
                }
            }
        };
        let action = watcher.observe(obs, now);
        let summary = watcher.describe(now);
        if summary != last_summary {
            log::info!("{summary}");
            last_summary = summary.clone();
        }
        {
            let mut s = status.lock().unwrap();
            s.link_up = obs.link_up;
            s.endpoint_present = obs.endpoint_present;
            s.auto = auto;
            s.summary = summary;
            s.updated = Some(now);
        }
        if action == Action::Connect {
            if auto {
                log::info!("link up but no audio: sending connect");
                match nudge(resolved) {
                    Ok(_) => set_error(&status, String::new()),
                    Err(e) => {
                        log::error!("connect failed: {e:#}");
                        set_error(&status, format!("connect failed: {e:#}"));
                    }
                }
            } else {
                log::info!("would send connect, but auto-reconnect is off");
            }
        }

        ticks += 1;
        if let Some(max) = max_ticks {
            if ticks >= max {
                return Ok(());
            }
        }

        // Sleep for the poll interval, but wake early for commands.
        let deadline = Instant::now() + poll;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match commands.recv_timeout(remaining) {
                Ok(Command::Quit) => return Ok(()),
                Ok(Command::SetAuto(v)) => {
                    auto = v;
                    log::info!("auto-reconnect {}", if v { "on" } else { "off" });
                }
                Ok(Command::ConnectNow) => {
                    log::info!("manual connect requested");
                    watcher.manual_nudge(Instant::now());
                    match nudge(resolved) {
                        Ok(_) => set_error(&status, String::new()),
                        Err(e) => {
                            log::error!("connect failed: {e:#}");
                            set_error(&status, format!("connect failed: {e:#}"));
                        }
                    }
                    break;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }
    }
}

pub fn observe(r: &Resolved) -> Result<Observation> {
    let link_up = win::link_up(r.headphones_mac)?;
    let endpoint_present = win::endpoint_present(&r.endpoint_match)?;
    Ok(Observation {
        link_up,
        endpoint_present,
    })
}

fn set_error(status: &SharedStatus, msg: String) {
    let mut s = status.lock().unwrap();
    s.last_error = if msg.is_empty() { None } else { Some(msg) };
}
