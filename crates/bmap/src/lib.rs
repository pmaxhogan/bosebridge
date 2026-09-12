//! Bose BMAP, the protocol the Bose app speaks to its headphones over the
//! Bluetooth Serial Port Profile (UUID 00001101). Everything here was derived
//! from the Bose Android app and confirmed against QC Ultra headphones; see
//! `docs/protocol.md` in the repository.
//!
//! Wire format: a 4-byte header `[fblock, function, (deviceId<<6)|(port<<4)|op, len]`
//! followed by `len` payload bytes. Replies use the same framing.

use std::fmt;

/// Function blocks (byte 0). Only the ones this crate touches are named.
pub mod fblock {
    pub const PRODUCT_INFO: u8 = 0;
    pub const DEVICE_MANAGEMENT: u8 = 4;
}

/// Device Management functions (byte 1 when `fblock == DEVICE_MANAGEMENT`).
pub mod dm {
    pub const FUNCTION_BLOCK_INFO: u8 = 0;
    pub const CONNECT: u8 = 1;
    pub const DISCONNECT: u8 = 2;
    pub const REMOVE_DEVICE: u8 = 3;
    pub const LIST_DEVICES: u8 = 4;
    pub const INFO: u8 = 5;
    pub const EXTENDED_INFO: u8 = 6;
    pub const PAIRING_MODE: u8 = 8;
}

/// Product Info functions.
pub mod product_info {
    pub const FUNCTION_BLOCK_INFO: u8 = 0;
    pub const BMAP_VERSION: u8 = 1;
}

/// Low nibble of header byte 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Operator {
    Set = 0,
    Get = 1,
    SetGet = 2,
    Status = 3,
    Error = 4,
    Start = 5,
    Result = 6,
    Processing = 7,
}

impl Operator {
    pub fn from_byte(b: u8) -> Option<Operator> {
        Some(match b & 0x0F {
            0 => Operator::Set,
            1 => Operator::Get,
            2 => Operator::SetGet,
            3 => Operator::Status,
            4 => Operator::Error,
            5 => Operator::Start,
            6 => Operator::Result,
            7 => Operator::Processing,
            _ => return None,
        })
    }
}

/// A 48-bit Bluetooth address, stored big-endian as printed (`AA:BB:CC:DD:EE:FF`).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    /// Parses `AA:BB:CC:DD:EE:FF`, `AA-BB-...`, or `AABBCCDDEEFF` (any case).
    pub fn parse(s: &str) -> Result<Mac, Error> {
        let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        let stripped: String = s.chars().filter(|c| !matches!(c, ':' | '-')).collect();
        if hex.len() != 12 || stripped.len() != 12 {
            return Err(Error::BadMac(s.to_string()));
        }
        let mut out = [0u8; 6];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| Error::BadMac(s.to_string()))?;
        }
        Ok(Mac(out))
    }

    /// Windows reports the local radio address as a u64.
    pub fn from_u64(v: u64) -> Mac {
        let b = v.to_be_bytes();
        Mac([b[2], b[3], b[4], b[5], b[6], b[7]])
    }

    pub fn to_u64(self) -> u64 {
        let mut b = [0u8; 8];
        b[2..].copy_from_slice(&self.0);
        u64::from_be_bytes(b)
    }

    /// Upper-case hex with no separators, the form used in Windows device instance ids.
    pub fn compact(self) -> String {
        self.0.iter().map(|b| format!("{b:02X}")).collect()
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|b| format!("{b:02X}")).collect();
        write!(f, "{}", parts.join(":"))
    }
}

impl fmt::Debug for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mac({self})")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Fewer than 4 header bytes.
    Truncated,
    /// Header says `len` payload bytes but fewer were given.
    ShortPayload {
        expected: usize,
        got: usize,
    },
    /// Operator nibble outside 0..=7.
    BadOperator(u8),
    BadMac(String),
    /// A reply did not have the shape we expected for that function.
    Malformed(&'static str),
    /// The headphones answered with the ERROR operator.
    Device(DeviceError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => write!(f, "frame shorter than the 4-byte header"),
            Error::ShortPayload { expected, got } => write!(f, "header promises {expected} payload bytes, got {got}"),
            Error::BadOperator(b) => write!(f, "operator nibble {b} is not defined"),
            Error::BadMac(s) => write!(f, "'{s}' is not a Bluetooth address"),
            Error::Malformed(what) => write!(f, "malformed reply: {what}"),
            Error::Device(e) => write!(f, "headphones returned error {e:?}"),
        }
    }
}

impl std::error::Error for Error {}

/// Error codes carried in an ERROR reply's first payload byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceError {
    Length,
    Checksum,
    FblockNotSupported,
    FuncNotSupported,
    OpNotSupported,
    InvalidData,
    DataUnavailable,
    Runtime,
    Timeout,
    InvalidState,
    DeviceNotFound,
    Busy,
    NoConnTimeout,
    NoConnKey,
    InsecureTransport,
    Other(u8),
}

impl DeviceError {
    pub fn from_byte(b: u8) -> DeviceError {
        match b {
            1 => DeviceError::Length,
            2 => DeviceError::Checksum,
            3 => DeviceError::FblockNotSupported,
            4 => DeviceError::FuncNotSupported,
            5 => DeviceError::OpNotSupported,
            6 => DeviceError::InvalidData,
            7 => DeviceError::DataUnavailable,
            8 => DeviceError::Runtime,
            9 => DeviceError::Timeout,
            10 => DeviceError::InvalidState,
            11 => DeviceError::DeviceNotFound,
            12 => DeviceError::Busy,
            13 => DeviceError::NoConnTimeout,
            14 => DeviceError::NoConnKey,
            20 => DeviceError::InsecureTransport,
            other => DeviceError::Other(other),
        }
    }
}

/// One BMAP frame, either direction.
#[derive(Clone, PartialEq, Eq)]
pub struct Packet {
    pub fblock: u8,
    pub function: u8,
    pub operator: Operator,
    /// Bits 7..6 of header byte 2. Zero in every frame seen so far.
    pub device_id: u8,
    /// Bits 5..4 of header byte 2. Zero unless DeviceManagement.Routing set up a port.
    pub port: u8,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn new(fblock: u8, function: u8, operator: Operator, payload: impl Into<Vec<u8>>) -> Packet {
        Packet {
            fblock,
            function,
            operator,
            device_id: 0,
            port: 0,
            payload: payload.into(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.payload.len());
        out.push(self.fblock);
        out.push(self.function);
        out.push(((self.device_id & 0x3) << 6) | ((self.port & 0x3) << 4) | (self.operator as u8));
        out.push(self.payload.len() as u8);
        out.extend_from_slice(&self.payload);
        out
    }

    /// Decodes exactly one frame from the front of `bytes`; returns it and the
    /// number of bytes consumed.
    pub fn decode(bytes: &[u8]) -> Result<(Packet, usize), Error> {
        if bytes.len() < 4 {
            return Err(Error::Truncated);
        }
        let len = bytes[3] as usize;
        if bytes.len() < 4 + len {
            return Err(Error::ShortPayload {
                expected: len,
                got: bytes.len() - 4,
            });
        }
        let operator = Operator::from_byte(bytes[2]).ok_or(Error::BadOperator(bytes[2] & 0x0F))?;
        Ok((
            Packet {
                fblock: bytes[0],
                function: bytes[1],
                operator,
                device_id: bytes[2] >> 6,
                port: (bytes[2] >> 4) & 0x3,
                payload: bytes[4..4 + len].to_vec(),
            },
            4 + len,
        ))
    }

    pub fn is_error(&self) -> bool {
        self.operator == Operator::Error
    }

    /// Turns an ERROR frame into `Err(Error::Device(..))`, passes others through.
    pub fn check(self) -> Result<Packet, Error> {
        if self.is_error() {
            let code = self.payload.first().copied().unwrap_or(0);
            Err(Error::Device(DeviceError::from_byte(code)))
        } else {
            Ok(self)
        }
    }

    pub fn payload_hex(&self) -> String {
        hex(&self.payload)
    }
}

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Packet(fb={} fn={} op={:?} payload=[{}])",
            self.fblock,
            self.function,
            self.operator,
            self.payload_hex()
        )
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

/// Reassembles frames from a byte stream that may split or coalesce writes.
#[derive(Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> FrameDecoder {
        FrameDecoder::default()
    }

    /// Feed bytes in; returns every complete frame now available.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Result<Packet, Error>> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if self.buf.len() < 4 {
                break;
            }
            let len = self.buf[3] as usize;
            if self.buf.len() < 4 + len {
                break;
            }
            let frame: Vec<u8> = self.buf.drain(..4 + len).collect();
            out.push(Packet::decode(&frame).map(|(p, _)| p));
        }
        out
    }

    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

// ---- Device Management ---------------------------------------------------

/// Requests. Each returns the exact bytes to write to the serial port.
pub mod request {
    use super::*;

    pub fn product_block_info() -> Packet {
        Packet::new(
            fblock::PRODUCT_INFO,
            product_info::FUNCTION_BLOCK_INFO,
            Operator::Get,
            [],
        )
    }

    pub fn bmap_version() -> Packet {
        Packet::new(fblock::PRODUCT_INFO, product_info::BMAP_VERSION, Operator::Get, [])
    }

    pub fn list_devices() -> Packet {
        Packet::new(fblock::DEVICE_MANAGEMENT, dm::LIST_DEVICES, Operator::Get, [])
    }

    pub fn info(mac: Mac) -> Packet {
        Packet::new(fblock::DEVICE_MANAGEMENT, dm::INFO, Operator::Get, mac.0)
    }

    /// Ask the headphones to connect their audio profiles to `mac`. This is what
    /// the Bose app's per-device toggle sends when switched on.
    pub fn connect(mac: Mac) -> Packet {
        let mut payload = vec![0u8];
        payload.extend_from_slice(&mac.0);
        Packet::new(fblock::DEVICE_MANAGEMENT, dm::CONNECT, Operator::Start, payload)
    }

    /// The toggle switched off. Unlike CONNECT there is no leading flag byte;
    /// the headphones answer InvalidData if one is included.
    pub fn disconnect(mac: Mac) -> Packet {
        Packet::new(fblock::DEVICE_MANAGEMENT, dm::DISCONNECT, Operator::Start, mac.0)
    }
}

/// One entry from `DeviceManagement.Info`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedDevice {
    pub mac: Mac,
    /// Bit 0 of the flags byte. Note: an open RFCOMM link from the querying
    /// device sets this for that device even when no audio profile is up.
    pub connected: bool,
    /// Bit 1: the device that asked.
    pub is_local: bool,
    /// Bit 2: another Bose product (name layout differs).
    pub is_bose_product: bool,
    pub flags: u8,
    /// Two bytes after the flags whose meaning is not yet decoded (seen: 02 03).
    pub extra: [u8; 2],
    pub name: String,
}

/// Parses a `DeviceManagement.Info` STATUS reply.
pub fn parse_info(p: &Packet) -> Result<PairedDevice, Error> {
    if p.fblock != fblock::DEVICE_MANAGEMENT || p.function != dm::INFO {
        return Err(Error::Malformed("not a DeviceManagement.Info reply"));
    }
    if p.operator != Operator::Status {
        return Err(Error::Malformed("Info reply is not a STATUS"));
    }
    let d = &p.payload;
    if d.len() < 9 {
        return Err(Error::Malformed("Info payload shorter than mac + flags + 2"));
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&d[0..6]);
    let flags = d[6];
    let is_bose_product = flags & 0b100 != 0;
    // Bose products carry a product id in bytes 7..8 and a variant byte at 9;
    // everything else has two small ints and the name from byte 9.
    let name_start = if is_bose_product { 10 } else { 9 };
    let name = if d.len() > name_start {
        String::from_utf8_lossy(&d[name_start..]).into_owned()
    } else {
        String::new()
    };
    Ok(PairedDevice {
        mac: Mac(mac),
        connected: flags & 0b1 != 0,
        is_local: flags & 0b10 != 0,
        is_bose_product,
        flags,
        extra: [d[7], d[8]],
        name,
    })
}

/// Parses a `DeviceManagement.ListDevices` STATUS reply into its MAC list.
/// The leading byte is returned separately; it was `03` with two devices and
/// is probably a count or bitmask of connected devices.
pub fn parse_list_devices(p: &Packet) -> Result<(u8, Vec<Mac>), Error> {
    if p.fblock != fblock::DEVICE_MANAGEMENT || p.function != dm::LIST_DEVICES {
        return Err(Error::Malformed("not a DeviceManagement.ListDevices reply"));
    }
    if p.operator != Operator::Status {
        return Err(Error::Malformed("ListDevices reply is not a STATUS"));
    }
    let d = &p.payload;
    if d.is_empty() || !(d.len() - 1).is_multiple_of(6) {
        return Err(Error::Malformed("ListDevices payload is not 1 + 6n bytes"));
    }
    let macs = d[1..]
        .chunks(6)
        .map(|c| {
            let mut m = [0u8; 6];
            m.copy_from_slice(c);
            Mac(m)
        })
        .collect();
    Ok((d[0], macs))
}

/// Outcome of a CONNECT or DISCONNECT exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectResult {
    pub mac: Mac,
    /// Bytes after the MAC in the RESULT frame. Seen: `0f 00` on an already
    /// connected device. The Bose app ignores them; we keep them for the log.
    pub extra: Vec<u8>,
}

/// Parses the RESULT frame of a CONNECT/DISCONNECT. PROCESSING frames are
/// acknowledged with `Ok(None)`.
pub fn parse_connect_result(p: &Packet) -> Result<Option<ConnectResult>, Error> {
    if p.fblock != fblock::DEVICE_MANAGEMENT || !(p.function == dm::CONNECT || p.function == dm::DISCONNECT) {
        return Err(Error::Malformed("not a Connect/Disconnect reply"));
    }
    match p.operator {
        Operator::Processing => Ok(None),
        Operator::Result | Operator::Status => {
            if p.payload.len() < 6 {
                return Err(Error::Malformed("Connect result shorter than a MAC"));
            }
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&p.payload[0..6]);
            Ok(Some(ConnectResult {
                mac: Mac(mac),
                extra: p.payload[6..].to_vec(),
            }))
        }
        Operator::Error => Err(Error::Device(DeviceError::from_byte(
            p.payload.first().copied().unwrap_or(0),
        ))),
        other => Err(Error::BadOperator(other as u8)),
    }
}

/// Parses a version string reply (Product Info block info or BMAP version).
pub fn parse_version(p: &Packet) -> Result<String, Error> {
    if p.operator != Operator::Status {
        return Err(Error::Malformed("version reply is not a STATUS"));
    }
    Ok(String::from_utf8_lossy(&p.payload).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    const DESKTOP: Mac = Mac([0xc8, 0x94, 0x02, 0x70, 0x6e, 0x56]);
    const PHONE: Mac = Mac([0x94, 0x45, 0x60, 0x2f, 0xcc, 0x9b]);

    #[test]
    fn mac_parsing_and_display() {
        assert_eq!(Mac::parse("C8:94:02:70:6E:56").unwrap(), DESKTOP);
        assert_eq!(Mac::parse("c8-94-02-70-6e-56").unwrap(), DESKTOP);
        assert_eq!(Mac::parse("C89402706E56").unwrap(), DESKTOP);
        assert_eq!(DESKTOP.to_string(), "C8:94:02:70:6E:56");
        assert_eq!(DESKTOP.compact(), "C89402706E56");
        assert!(Mac::parse("C8:94:02:70:6E").is_err());
        assert!(Mac::parse("zz:94:02:70:6e:56").is_err());
        assert!(Mac::parse("C8:94:02:70:6E:5").is_err());
    }

    #[test]
    fn mac_u64_round_trip_matches_windows_radio_address() {
        // Windows reported the RZ608 radio as this decimal value.
        let m = Mac::from_u64(220538021637718);
        assert_eq!(m, DESKTOP);
        assert_eq!(m.to_u64(), 220538021637718);
    }

    #[test]
    fn encodes_requests_exactly_as_captured() {
        assert_eq!(request::product_block_info().encode(), b("00 00 01 00"));
        assert_eq!(request::bmap_version().encode(), b("00 01 01 00"));
        assert_eq!(request::list_devices().encode(), b("04 04 01 00"));
        assert_eq!(request::info(DESKTOP).encode(), b("04 05 01 06 c8 94 02 70 6e 56"));
        assert_eq!(
            request::connect(DESKTOP).encode(),
            b("04 01 05 07 00 c8 94 02 70 6e 56")
        );
        assert_eq!(
            request::disconnect(DESKTOP).encode(),
            b("04 02 05 07 00 c8 94 02 70 6e 56")
        );
    }

    #[test]
    fn header_packs_device_id_and_port() {
        let mut p = Packet::new(4, 1, Operator::Start, []);
        p.device_id = 2;
        p.port = 1;
        assert_eq!(p.encode()[2], 0b1001_0101);
        let (back, n) = Packet::decode(&p.encode()).unwrap();
        assert_eq!(n, 4);
        assert_eq!(back.device_id, 2);
        assert_eq!(back.port, 1);
        assert_eq!(back.operator, Operator::Start);
    }

    #[test]
    fn decodes_version_replies() {
        let (p, n) = Packet::decode(&b("00 00 03 05 31 2e 31 2e 30")).unwrap();
        assert_eq!(n, 9);
        assert_eq!(parse_version(&p).unwrap(), "1.1.0");
        let (p, _) = Packet::decode(&b("00 01 03 05 31 2e 32 2e 30")).unwrap();
        assert_eq!(parse_version(&p).unwrap(), "1.2.0");
    }

    #[test]
    fn decodes_list_devices() {
        let (p, _) = Packet::decode(&b("04 04 03 0d 03 94 45 60 2f cc 9b c8 94 02 70 6e 56")).unwrap();
        let (lead, macs) = parse_list_devices(&p).unwrap();
        assert_eq!(lead, 3);
        assert_eq!(macs, vec![PHONE, DESKTOP]);
    }

    #[test]
    fn list_devices_rejects_bad_shapes() {
        let (p, _) = Packet::decode(&b("04 04 03 02 03 94")).unwrap();
        assert!(matches!(parse_list_devices(&p), Err(Error::Malformed(_))));
        let (p, _) = Packet::decode(&b("04 05 03 00")).unwrap();
        assert!(matches!(parse_list_devices(&p), Err(Error::Malformed(_))));
    }

    #[test]
    fn decodes_info_for_phone_and_desktop() {
        let (p, _) = Packet::decode(&b("04 05 03 10 94 45 60 2f cc 9b 01 02 03 50 69 78 65 6c 20 38")).unwrap();
        let phone = parse_info(&p).unwrap();
        assert_eq!(phone.mac, PHONE);
        assert!(phone.connected);
        assert!(!phone.is_local);
        assert!(!phone.is_bose_product);
        assert_eq!(phone.extra, [2, 3]);
        assert_eq!(phone.name, "Pixel 8");

        let (p, _) = Packet::decode(&b("04 05 03 10 c8 94 02 70 6e 56 03 02 03 4d 41 58 2d 50 52 4f")).unwrap();
        let desk = parse_info(&p).unwrap();
        assert_eq!(desk.mac, DESKTOP);
        assert!(desk.connected);
        assert!(desk.is_local);
        assert_eq!(desk.name, "MAX-PRO");
    }

    #[test]
    fn info_for_bose_product_reads_name_from_byte_ten() {
        // flags bit2 set: product id in 7..8, variant at 9, name from 10.
        let mut payload = b("aa bb cc dd ee ff 05 40 82 01");
        payload.extend_from_slice(b"Buds");
        let p = Packet::new(4, 5, Operator::Status, payload);
        let d = parse_info(&p).unwrap();
        assert!(d.is_bose_product);
        assert_eq!(d.name, "Buds");
        assert_eq!(d.extra, [0x40, 0x82]);
    }

    #[test]
    fn info_rejects_wrong_function_or_operator_or_length() {
        let (p, _) = Packet::decode(&b("04 04 03 00")).unwrap();
        assert!(parse_info(&p).is_err());
        let p = Packet::new(4, 5, Operator::Get, b("c8 94 02 70 6e 56 03 02 03"));
        assert!(parse_info(&p).is_err());
        let p = Packet::new(4, 5, Operator::Status, b("c8 94 02 70 6e 56 03"));
        assert!(parse_info(&p).is_err());
        // Exactly 9 bytes: valid, empty name.
        let p = Packet::new(4, 5, Operator::Status, b("c8 94 02 70 6e 56 03 02 03"));
        assert_eq!(parse_info(&p).unwrap().name, "");
    }

    #[test]
    fn decodes_connect_processing_then_result() {
        let mut dec = FrameDecoder::new();
        let frames = dec.feed(&b("04 01 07 06 c8 94 02 70 6e 56 04 01 06 08 c8 94 02 70 6e 56 0f 00"));
        assert_eq!(frames.len(), 2);
        let first = frames[0].as_ref().unwrap();
        assert_eq!(parse_connect_result(first).unwrap(), None);
        let second = frames[1].as_ref().unwrap();
        let r = parse_connect_result(second).unwrap().unwrap();
        assert_eq!(r.mac, DESKTOP);
        assert_eq!(r.extra, vec![0x0f, 0x00]);
        assert_eq!(dec.pending(), 0);
    }

    #[test]
    fn connect_result_surfaces_device_errors() {
        let p = Packet::new(4, 1, Operator::Error, [11u8]);
        assert_eq!(
            parse_connect_result(&p),
            Err(Error::Device(DeviceError::DeviceNotFound))
        );
        let p = Packet::new(4, 1, Operator::Get, []);
        assert!(parse_connect_result(&p).is_err());
        let p = Packet::new(4, 4, Operator::Result, []);
        assert!(parse_connect_result(&p).is_err());
        let p = Packet::new(4, 2, Operator::Result, [1u8, 2, 3]);
        assert!(matches!(parse_connect_result(&p), Err(Error::Malformed(_))));
    }

    #[test]
    fn check_converts_error_frames() {
        let p = Packet::new(4, 1, Operator::Error, [20u8]);
        assert_eq!(p.check(), Err(Error::Device(DeviceError::InsecureTransport)));
        let p = Packet::new(4, 1, Operator::Error, []);
        assert_eq!(p.check(), Err(Error::Device(DeviceError::Other(0))));
        let ok = Packet::new(0, 0, Operator::Status, [1u8]);
        assert!(ok.clone().check().is_ok());
        assert_eq!(DeviceError::from_byte(12), DeviceError::Busy);
        assert_eq!(DeviceError::from_byte(99), DeviceError::Other(99));
    }

    #[test]
    fn frame_decoder_handles_split_and_garbage_free_streams() {
        let mut dec = FrameDecoder::new();
        let full = b("04 05 03 10 c8 94 02 70 6e 56 03 02 03 4d 41 58 2d 50 52 4f");
        assert!(dec.feed(&full[..3]).is_empty());
        assert_eq!(dec.pending(), 3);
        assert!(dec.feed(&full[3..10]).is_empty());
        let frames = dec.feed(&full[10..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].as_ref().unwrap().payload, full[4..]);
        dec.feed(&[1, 2]);
        dec.clear();
        assert_eq!(dec.pending(), 0);
    }

    #[test]
    fn decode_errors() {
        assert_eq!(Packet::decode(&[1, 2, 3]), Err(Error::Truncated));
        assert_eq!(
            Packet::decode(&[4, 5, 3, 4, 1]),
            Err(Error::ShortPayload { expected: 4, got: 1 })
        );
        assert_eq!(Packet::decode(&[4, 5, 0x0f, 0]), Err(Error::BadOperator(15)));
        let mut dec = FrameDecoder::new();
        let out = dec.feed(&[4, 5, 0x0f, 0]);
        assert_eq!(out.len(), 1);
        assert!(out[0].is_err());
    }

    #[test]
    fn display_and_debug_are_readable() {
        let p = Packet::new(4, 1, Operator::Start, [0u8, 1]);
        assert_eq!(format!("{p:?}"), "Packet(fb=4 fn=1 op=Start payload=[00 01])");
        assert_eq!(format!("{DESKTOP:?}"), "Mac(C8:94:02:70:6E:56)");
        for e in [
            Error::Truncated,
            Error::ShortPayload { expected: 1, got: 0 },
            Error::BadOperator(9),
            Error::BadMac("x".into()),
            Error::Malformed("y"),
            Error::Device(DeviceError::Busy),
        ] {
            assert!(!e.to_string().is_empty());
        }
        assert_eq!(Operator::from_byte(0x35), Some(Operator::Start));
        assert_eq!(Operator::from_byte(8), None);
    }
}
