use crate::config::Config;
use crate::transport::{describe_devices, Headphones};
use crate::watch::{self, Command, SharedStatus};
use crate::win;
use anyhow::{anyhow, Context, Result};
use bmap::Mac;
use clap::{Parser, Subcommand};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

pub const VERSION: &str = match option_env!("BOSEBRIDGE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser)]
#[command(name = "bosebridge", version = VERSION, about = "Keeps Bose multipoint headphones' audio on this PC without the Bose app")]
pub struct Cli {
    /// Log debug detail (raw frames) too.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// What Windows and the headphones report right now.
    Status {
        /// Talk to the headphones even if that means paging them over Bluetooth.
        #[arg(long)]
        force: bool,
    },
    /// List the devices paired to the headphones, as the headphones see them.
    Devices {
        #[arg(long)]
        force: bool,
    },
    /// Tell the headphones to bring audio up to this PC (what the Bose toggle does).
    Connect {
        /// Target a different paired device instead of this PC.
        #[arg(long)]
        mac: Option<String>,
    },
    /// Tell the headphones to drop audio to this PC (the toggle switched off).
    Disconnect {
        #[arg(long)]
        mac: Option<String>,
    },
    /// Run the watcher in the foreground without a tray icon.
    Watch {
        /// Observe this many times, then exit (for testing).
        #[arg(long)]
        ticks: Option<u32>,
    },
    /// Run in the system tray (the default when no subcommand is given).
    Tray,
    /// Detect the headphones, port, and addresses, and write the config file.
    Detect,
    /// Print the config file path and contents.
    Config,
    /// Start the tray at logon (per-user Run key, no admin needed).
    InstallAutostart,
    /// Remove the logon entry.
    UninstallAutostart,
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    init_logging(cli.verbose)?;
    let mut config = Config::load()?;

    match cli.command.unwrap_or(Cmd::Tray) {
        Cmd::Tray => {
            let r = watch::resolve(&mut config)?;
            crate::tray::run(config, r)
        }
        Cmd::InstallAutostart => {
            let cmd = crate::autostart::install()?;
            println!("installed: {cmd}");
            Ok(())
        }
        Cmd::UninstallAutostart => {
            let removed = crate::autostart::uninstall()?;
            println!("{}", if removed { "removed" } else { "was not installed" });
            Ok(())
        }
        Cmd::Config => {
            println!("{}", Config::path().display());
            print!("{}", toml::to_string_pretty(&config)?);
            match crate::autostart::current() {
                Ok(Some(cmd)) => println!("autostart = {cmd}"),
                Ok(None) => println!("autostart = (not installed)"),
                Err(_) => {}
            }
            Ok(())
        }
        Cmd::Detect => {
            let r = watch::resolve(&mut config)?;
            println!("headphones: {} ({})", r.headphones_name, r.headphones_mac);
            println!("port:       {}", r.port);
            println!("this PC:    {}", r.local_mac);
            println!("endpoint:   any render endpoint containing '{}'", r.endpoint_match);
            println!("saved to    {}", Config::path().display());
            Ok(())
        }
        Cmd::Status { force } => status(&mut config, force),
        Cmd::Devices { force } => {
            let r = watch::resolve(&mut config)?;
            guard_link(&r, force)?;
            let mut hp = Headphones::open(&r.port)?;
            println!("BMAP {}", hp.bmap_version()?);
            for d in describe_devices(&mut hp)? {
                println!(
                    "{}  {:<20} {}{}{} flags={:#04x} extra={:02x?}",
                    d.mac,
                    d.name,
                    if d.connected { "connected" } else { "not connected" },
                    if d.is_local { ", this PC" } else { "" },
                    if d.is_bose_product { ", Bose product" } else { "" },
                    d.flags,
                    d.extra
                );
            }
            Ok(())
        }
        Cmd::Connect { mac } => {
            let r = watch::resolve(&mut config)?;
            let target = target(&r, mac)?;
            let mut hp = Headphones::open(&r.port)?;
            let res = hp.connect(target)?;
            println!(
                "connect {} acknowledged (result bytes {})",
                res.mac,
                bmap::hex(&res.extra)
            );
            Ok(())
        }
        Cmd::Disconnect { mac } => {
            let r = watch::resolve(&mut config)?;
            let target = target(&r, mac)?;
            let mut hp = Headphones::open(&r.port)?;
            let res = hp.disconnect(target)?;
            println!(
                "disconnect {} acknowledged (result bytes {})",
                res.mac,
                bmap::hex(&res.extra)
            );
            Ok(())
        }
        Cmd::Watch { ticks } => {
            let r = watch::resolve(&mut config)?;
            let (_tx, rx) = mpsc::channel::<Command>();
            let status: SharedStatus = Arc::new(Mutex::new(Default::default()));
            watch::run(&config, &r, rx, status, ticks)
        }
    }
}

fn target(r: &watch::Resolved, mac: Option<String>) -> Result<Mac> {
    match mac {
        Some(s) => Mac::parse(&s).with_context(|| format!("--mac '{s}'")),
        None => Ok(r.local_mac),
    }
}

fn guard_link(r: &watch::Resolved, force: bool) -> Result<()> {
    if force {
        return Ok(());
    }
    if win::link_up(r.headphones_mac)? {
        Ok(())
    } else {
        Err(anyhow!(
            "Windows says the headphones are not connected; opening {} would page them. Use --force if that is what you want.",
            r.port
        ))
    }
}

fn status(config: &mut Config, force: bool) -> Result<()> {
    let r = watch::resolve(config)?;
    println!("headphones: {} ({}) on {}", r.headphones_name, r.headphones_mac, r.port);
    println!("this PC:    {}", r.local_mac);
    let link = win::link_up(r.headphones_mac)?;
    println!("link:       {}", if link { "connected" } else { "not connected" });
    let endpoints = win::render_endpoints()?;
    let present = win::endpoint_matches(&endpoints, &r.endpoint_match);
    println!(
        "audio:      {} (looking for '{}' among {} render endpoints)",
        if present { "endpoint present" } else { "no endpoint" },
        r.endpoint_match,
        endpoints.len()
    );
    for e in endpoints
        .iter()
        .filter(|e| e.name.to_lowercase().contains(&r.endpoint_match.to_lowercase()))
    {
        println!(
            "            {} [{}]",
            e.name,
            if e.enabled { "enabled" } else { "disabled" }
        );
    }
    if !link && !force {
        println!("headphones: not asked (link down; --force to page them)");
        return Ok(());
    }
    let mut hp = Headphones::open(&r.port)?;
    println!("BMAP:       {}", hp.bmap_version()?);
    let me = hp.info(r.local_mac)?;
    println!(
        "headphones say this PC is {} (flags {:#04x}, extra {:02x?}, name '{}')",
        if me.connected { "connected" } else { "not connected" },
        me.flags,
        me.extra,
        me.name
    );
    Ok(())
}

fn init_logging(verbose: bool) -> Result<()> {
    use simplelog::{ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, TermLogger, TerminalMode, WriteLogger};
    let level = if verbose { LevelFilter::Debug } else { LevelFilter::Info };
    let cfg = ConfigBuilder::new().set_time_format_rfc3339().build();
    let mut loggers: Vec<Box<dyn simplelog::SharedLogger>> = vec![TermLogger::new(
        level,
        cfg.clone(),
        TerminalMode::Stderr,
        ColorChoice::Auto,
    )];
    if let Ok(()) = std::fs::create_dir_all(Config::dir()) {
        if let Ok(file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Config::log_path())
        {
            loggers.push(WriteLogger::new(level, cfg, file));
        }
    }
    CombinedLogger::init(loggers).context("initialising logging")
}
