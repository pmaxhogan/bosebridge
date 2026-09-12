// Release builds hide the console so the tray does not spawn a window at
// logon; CLI subcommands reattach to the parent console so output still shows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod autostart;
mod cli;
mod config;
mod transport;
mod tray;
mod watch;
mod win;

fn main() {
    attach_parent_console();
    if let Err(e) = cli::run() {
        eprintln!("error: {e:#}");
        log::error!("{e:#}");
        std::process::exit(1);
    }
}

#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // Fails harmlessly when there is no parent console (launched from the Run key).
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(all(windows, not(debug_assertions))))]
fn attach_parent_console() {}
