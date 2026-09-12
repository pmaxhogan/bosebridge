//! Talks BMAP to the headphones through the serial port Windows binds to
//! their SPP service. Opening the port makes Windows bring up the Bluetooth
//! link if it is down, which pages the headphones; callers decide when that
//! is acceptable.

use anyhow::{anyhow, Context, Result};
use bmap::{FrameDecoder, Mac, Packet};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub struct Headphones {
    port: Box<dyn serialport::SerialPort>,
    decoder: FrameDecoder,
}

/// How long to keep collecting reply frames after the last byte arrived.
const QUIET: Duration = Duration::from_millis(600);
/// Give up waiting for any reply after this.
const REPLY_TIMEOUT: Duration = Duration::from_secs(4);

impl Headphones {
    pub fn open(port_name: &str) -> Result<Headphones> {
        let t0 = Instant::now();
        let port = serialport::new(port_name, 115_200)
            .timeout(Duration::from_millis(300))
            .open()
            .with_context(|| format!("opening {port_name} (is the headset in range and paired?)"))?;
        log::info!("opened {port_name} in {:.1}s", t0.elapsed().as_secs_f32());
        Ok(Headphones {
            port,
            decoder: FrameDecoder::new(),
        })
    }

    /// Write one request and collect every frame that comes back before the
    /// line goes quiet. Frames that fail to decode are logged and dropped.
    pub fn exchange(&mut self, request: &Packet) -> Result<Vec<Packet>> {
        let bytes = request.encode();
        log::debug!("> {}", bmap::hex(&bytes));
        self.decoder.clear();
        self.port.write_all(&bytes).context("writing to the headphones")?;
        self.port.flush().ok();

        let mut out = Vec::new();
        let start = Instant::now();
        let mut last = Instant::now();
        let mut buf = [0u8; 512];
        loop {
            match self.port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    log::debug!("< {}", bmap::hex(&buf[..n]));
                    last = Instant::now();
                    for frame in self.decoder.feed(&buf[..n]) {
                        match frame {
                            Ok(p) => out.push(p),
                            Err(e) => log::warn!("undecodable frame from headphones: {e}"),
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(anyhow!("reading from the headphones: {e}")),
            }
            if !out.is_empty() && last.elapsed() >= QUIET {
                break;
            }
            if out.is_empty() && start.elapsed() >= REPLY_TIMEOUT {
                return Err(anyhow!("no reply from the headphones within {REPLY_TIMEOUT:?}"));
            }
            if start.elapsed() >= REPLY_TIMEOUT + QUIET {
                break;
            }
        }
        Ok(out)
    }

    fn one(&mut self, request: &Packet) -> Result<Packet> {
        let replies = self.exchange(request)?;
        let first = replies.into_iter().next().ok_or_else(|| anyhow!("empty reply"))?;
        Ok(first.check()?)
    }

    pub fn bmap_version(&mut self) -> Result<String> {
        let p = self.one(&bmap::request::bmap_version())?;
        Ok(bmap::parse_version(&p)?)
    }

    pub fn list_devices(&mut self) -> Result<Vec<Mac>> {
        let p = self.one(&bmap::request::list_devices())?;
        let (lead, macs) = bmap::parse_list_devices(&p)?;
        log::debug!("list_devices lead byte {lead:#04x}");
        Ok(macs)
    }

    pub fn info(&mut self, mac: Mac) -> Result<bmap::PairedDevice> {
        let p = self.one(&bmap::request::info(mac))?;
        Ok(bmap::parse_info(&p)?)
    }

    /// Ask the headphones to bring up their audio profiles to `mac`.
    pub fn connect(&mut self, mac: Mac) -> Result<bmap::ConnectResult> {
        self.start_and_wait(bmap::request::connect(mac), "connect")
    }

    pub fn disconnect(&mut self, mac: Mac) -> Result<bmap::ConnectResult> {
        self.start_and_wait(bmap::request::disconnect(mac), "disconnect")
    }

    fn start_and_wait(&mut self, request: Packet, what: &str) -> Result<bmap::ConnectResult> {
        let replies = self.exchange(&request)?;
        let mut result = None;
        for p in replies {
            match bmap::parse_connect_result(&p) {
                Ok(Some(r)) => result = Some(r),
                Ok(None) => log::debug!("{what}: processing"),
                Err(e) => return Err(anyhow!("{what} failed: {e}")),
            }
        }
        result.ok_or_else(|| anyhow!("{what}: headphones acknowledged but sent no result"))
    }
}

/// Every paired device with its info, for `bosebridge devices`.
pub fn describe_devices(hp: &mut Headphones) -> Result<Vec<bmap::PairedDevice>> {
    let macs = hp.list_devices()?;
    let mut out = Vec::new();
    for mac in macs {
        match hp.info(mac) {
            Ok(d) => out.push(d),
            Err(e) => log::warn!("info for {mac} failed: {e}"),
        }
    }
    Ok(out)
}
