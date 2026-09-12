//! Talks BMAP to the headphones through the serial port Windows binds to
//! their SPP service. Opening the port makes Windows bring up the Bluetooth
//! link if it is down, which pages the headphones; callers decide when that
//! is acceptable.
//!
//! The exchange logic is generic over any `Read + Write` so it can be tested
//! with a scripted fake instead of real hardware.

use anyhow::{anyhow, Context, Result};
use bmap::{FrameDecoder, Mac, Packet};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub struct Headphones<P: Read + Write = Box<dyn serialport::SerialPort>> {
    port: P,
    decoder: FrameDecoder,
    /// Keep collecting reply frames until the line has been quiet this long.
    pub quiet: Duration,
    /// Give up waiting for any reply after this.
    pub reply_timeout: Duration,
}

const QUIET: Duration = Duration::from_millis(600);
const REPLY_TIMEOUT: Duration = Duration::from_secs(4);

impl Headphones<Box<dyn serialport::SerialPort>> {
    pub fn open(port_name: &str) -> Result<Self> {
        let t0 = Instant::now();
        let port = serialport::new(port_name, 115_200)
            .timeout(Duration::from_millis(300))
            .open()
            .with_context(|| format!("opening {port_name} (is the headset in range and paired?)"))?;
        log::info!("opened {port_name} in {:.1}s", t0.elapsed().as_secs_f32());
        Ok(Headphones::with_port(port))
    }
}

impl<P: Read + Write> Headphones<P> {
    /// Wrap an already open port (or a fake, in tests).
    pub fn with_port(port: P) -> Self {
        Headphones {
            port,
            decoder: FrameDecoder::new(),
            quiet: QUIET,
            reply_timeout: REPLY_TIMEOUT,
        }
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
            if !out.is_empty() && last.elapsed() >= self.quiet {
                break;
            }
            if out.is_empty() && start.elapsed() >= self.reply_timeout {
                return Err(anyhow!("no reply from the headphones within {:?}", self.reply_timeout));
            }
            if start.elapsed() >= self.reply_timeout + self.quiet {
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
pub fn describe_devices<P: Read + Write>(hp: &mut Headphones<P>) -> Result<Vec<bmap::PairedDevice>> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io;

    /// A port that answers each write with the next scripted reply, delivered
    /// in the chunks given, and times out when nothing is queued.
    struct Fake {
        written: Vec<Vec<u8>>,
        replies: VecDeque<Vec<Vec<u8>>>,
        pending: VecDeque<Vec<u8>>,
        fail_reads: bool,
        fail_writes: bool,
    }

    impl Fake {
        fn new(replies: Vec<Vec<Vec<u8>>>) -> Fake {
            Fake {
                written: Vec::new(),
                replies: replies.into_iter().collect(),
                pending: VecDeque::new(),
                fail_reads: false,
                fail_writes: false,
            }
        }
    }

    impl Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.fail_reads {
                return Err(io::Error::other("port vanished"));
            }
            match self.pending.pop_front() {
                Some(chunk) => {
                    buf[..chunk.len()].copy_from_slice(&chunk);
                    Ok(chunk.len())
                }
                None => {
                    std::thread::sleep(Duration::from_millis(5));
                    Err(io::Error::new(io::ErrorKind::TimedOut, "no data"))
                }
            }
        }
    }

    impl Write for Fake {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.fail_writes {
                return Err(io::Error::other("write failed"));
            }
            self.written.push(buf.to_vec());
            if let Some(reply) = self.replies.pop_front() {
                self.pending.extend(reply);
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn b(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    fn hp(replies: Vec<Vec<Vec<u8>>>) -> Headphones<Fake> {
        let mut h = Headphones::with_port(Fake::new(replies));
        h.quiet = Duration::from_millis(30);
        h.reply_timeout = Duration::from_millis(150);
        h
    }

    const DESKTOP: Mac = Mac([0xc8, 0x94, 0x02, 0x70, 0x6e, 0x56]);

    #[test]
    fn version_round_trip_writes_the_request_and_parses_the_reply() {
        let mut h = hp(vec![vec![b("00 01 03 05 31 2e 32 2e 30")]]);
        assert_eq!(h.bmap_version().unwrap(), "1.2.0");
        assert_eq!(h.port.written, vec![b("00 01 01 00")]);
    }

    #[test]
    fn reply_split_across_reads_is_reassembled() {
        let full = b("04 05 03 10 c8 94 02 70 6e 56 03 02 03 4d 41 58 2d 50 52 4f");
        let chunks = vec![full[..3].to_vec(), full[3..12].to_vec(), full[12..].to_vec()];
        let mut h = hp(vec![chunks]);
        let d = h.info(DESKTOP).unwrap();
        assert_eq!(d.name, "MAX-PRO");
        assert!(d.connected && d.is_local);
    }

    #[test]
    fn connect_collects_processing_then_result() {
        let mut h = hp(vec![vec![
            b("04 01 07 06 c8 94 02 70 6e 56"),
            b("04 01 06 08 c8 94 02 70 6e 56 0f 00"),
        ]]);
        let r = h.connect(DESKTOP).unwrap();
        assert_eq!(r.mac, DESKTOP);
        assert_eq!(r.extra, vec![0x0f, 0x00]);
        assert_eq!(h.port.written, vec![b("04 01 05 07 00 c8 94 02 70 6e 56")]);
    }

    #[test]
    fn disconnect_uses_function_two() {
        let mut h = hp(vec![vec![b("04 02 06 06 c8 94 02 70 6e 56")]]);
        assert!(h.disconnect(DESKTOP).is_ok());
        assert_eq!(h.port.written[0][1], 2);
    }

    #[test]
    fn connect_with_only_processing_is_an_error() {
        let mut h = hp(vec![vec![b("04 01 07 06 c8 94 02 70 6e 56")]]);
        let e = h.connect(DESKTOP).unwrap_err().to_string();
        assert!(e.contains("no result"), "{e}");
    }

    #[test]
    fn device_error_reply_surfaces() {
        let mut h = hp(vec![vec![b("04 01 04 01 0b")]]);
        let e = h.connect(DESKTOP).unwrap_err().to_string();
        assert!(e.contains("DeviceNotFound"), "{e}");
        let mut h = hp(vec![vec![b("00 01 04 01 0c")]]);
        let e = h.bmap_version().unwrap_err().to_string();
        assert!(e.contains("Busy"), "{e}");
    }

    #[test]
    fn silence_times_out() {
        let mut h = hp(vec![]);
        let e = h.bmap_version().unwrap_err().to_string();
        assert!(e.contains("no reply"), "{e}");
    }

    #[test]
    fn io_errors_propagate() {
        let mut h = hp(vec![]);
        h.port.fail_writes = true;
        assert!(h.bmap_version().unwrap_err().to_string().contains("writing"));
        let mut h = hp(vec![]);
        h.port.fail_reads = true;
        assert!(h.bmap_version().unwrap_err().to_string().contains("reading"));
    }

    #[test]
    fn undecodable_bytes_are_dropped_but_good_frames_kept() {
        // First frame has an invalid operator nibble; second is fine.
        let mut h = hp(vec![vec![b("00 01 0f 00 00 01 03 05 31 2e 32 2e 30")]]);
        assert_eq!(h.bmap_version().unwrap(), "1.2.0");
    }

    #[test]
    fn describe_devices_lists_each_paired_device_and_skips_failures() {
        let list = b("04 04 03 0d 03 94 45 60 2f cc 9b c8 94 02 70 6e 56");
        let phone = b("04 05 03 10 94 45 60 2f cc 9b 01 02 03 50 69 78 65 6c 20 38");
        let bad = b("04 05 04 01 0b");
        let mut h = hp(vec![vec![list], vec![phone], vec![bad]]);
        let devs = describe_devices(&mut h).unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].name, "Pixel 8");
        assert_eq!(h.port.written.len(), 3);
    }
}
