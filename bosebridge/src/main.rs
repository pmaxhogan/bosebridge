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
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};
    // A GUI-subsystem process gets no std handles when started from a console
    // window, so attach to the parent's console to show CLI output there. When
    // stdout is already a pipe (ssh, PowerShell capture) leave it alone, since
    // AttachConsole would replace the pipe with the console.
    unsafe {
        let has_stdout = matches!(GetStdHandle(STD_OUTPUT_HANDLE), Ok(h) if !h.is_invalid() && h != INVALID_HANDLE_VALUE);
        if !has_stdout {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

#[cfg(not(all(windows, not(debug_assertions))))]
fn attach_parent_console() {}
