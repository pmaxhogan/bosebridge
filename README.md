# bosebridge

Keeps Bose multipoint headphones' audio connected to a Windows PC, without
opening the Bose app.

## The annoyance

Bose QuietComfort Ultra headphones do multipoint, but when the PC drops off
(you walk away, the PC reboots) the headphones flip that PC's switch off in the
Bose app's "Devices" screen. When Windows reconnects, the headphones come back
as "Connected" but not "Connected voice, music": no audio endpoint appears in
Windows, and nothing on the PC side fixes it. The only cure was the Bose app on
a phone: wait for it to find the headphones, open Devices, flip the PC's switch
back on.

## What this does

That switch is a four-byte command over the headphones' Bluetooth serial port,
and the headphones accept it from any paired device with no authentication.
bosebridge sends it from the PC itself.

- **Watcher**: every few seconds, checks whether Windows reports the Bluetooth
  link up and whether the headphones' audio endpoint exists. If the link is up
  with no audio for 8 seconds, it sends the headphones `Connect(this PC)`, at
  most once per 30 seconds, and gives up after 4 tries until the headphones
  reconnect.
- **Tray icon**: grey when the headphones are away, green when audio is up,
  orange while it is fixing things, red if the last attempt failed. Menu:
  connect now, auto-reconnect toggle, open log, quit.
- **Hotkey**: `Ctrl+Alt+Shift+H` sends a connect on demand (configurable).
- **CLI**: `status`, `devices`, `connect`, `disconnect`, `watch`, `detect`,
  `config`, `install-autostart`, `uninstall-autostart`.

It never opens the serial port while Windows says the link is down, because
opening it would page the headphones and pull them onto the PC whenever they
are in range. The manual button and hotkey do force the link.

## Install

Download `bosebridge-<version>.exe` from
[Releases](https://github.com/pmaxhogan/bosebridge/releases), put it somewhere
permanent, then:

```
bosebridge detect              # finds the headphones, port, and addresses; writes the config
bosebridge status              # what Windows and the headphones think right now
bosebridge install-autostart   # tray at logon (per-user Run key, no admin)
bosebridge                     # run the tray now
```

Requirements: Windows 11 (10 should work), the headphones paired, and the
"Standard Serial over Bluetooth link" COM port Windows creates for them (it does
this automatically on pairing).

Config lives at `%APPDATA%\bosebridge\config.toml`; the log at
`%APPDATA%\bosebridge\bosebridge.log`. Everything in the config is optional and
auto-detected; the fields worth touching:

| key | default | meaning |
|---|---|---|
| `endpoint_match` | `(<headphone name>)` | substring that identifies the headphones' Windows audio endpoint |
| `debounce_secs` | 8 | link up without audio for this long before the first nudge |
| `cooldown_secs` | 30 | minimum gap between nudges |
| `max_attempts` | 4 | nudges per link session before giving up |
| `auto_reconnect` | true | whether the watcher acts on its own |
| `hotkey` | `Ctrl+Alt+Shift+H` | global "connect now" hotkey; empty to disable |

## Failure model

Verified on a Pixel-plus-desktop multipoint setup with QC Ultra (2nd gen):

- The headphones' own status byte reports the PC as "connected" whenever any
  Bluetooth link (including this tool's serial port) is up, so it cannot be
  used to detect the missing-audio state. Windows' audio endpoint list can.
- In one observed case the desktop merely opening the serial port was enough for
  the headphones to bring audio up on their own; the explicit `Connect` is the
  belt to that suspender.

See [docs/protocol.md](docs/protocol.md) for the wire protocol.

## Development

```
cargo test --workspace                       # pure crates run anywhere
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release                        # on Windows
```

Layout: `crates/bmap` (protocol codec, no I/O), `crates/decision` (the
watcher's state machine with an injected clock), `bosebridge` (serial
transport, Windows queries, CLI, tray). CI runs tests with a coverage gate on
Linux and builds plus tests on Windows; every push to `main` publishes a
release tagged `v0.1.<commit count>`. Dependabot minor and patch bumps
auto-merge once CI passes.

## Credits

Protocol details come from reading the Bose Android app (10.2.4). Nothing from
it is included here.

## License

MIT, see [LICENSE](LICENSE).
