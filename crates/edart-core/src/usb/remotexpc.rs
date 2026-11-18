//! RemoteXPC: XPC messages carried in HTTP/2 DATA frames, which is how a
//! Mac talks to a service on the iPhone.
//!
//! It is HTTP/2 in framing only. HEADERS frames are empty (no HPACK), and
//! everything real travels in DATA frames as an XPC envelope:
//!
//! ```text
//! magic 0x29B00B92 | flags u32 | payload size u64 | message id u64 | payload
//! payload = magic 0x42133742 | version 5 | XPC object
//! ```
//!
//! Ported from iphone-use's `RemoteXPCConnection.swift`, `XPCWrapper.swift`
//! and `HTTP2.swift` (MIT, Sunghyun Kim), where the frame order below was
//! matched against `devicectl` and verified on iOS 27.3.

use std::io::{self, Read, Write};

use super::xpc::XpcValue;

pub const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

pub const FRAME_DATA: u8 = 0x0;
pub const FRAME_HEADERS: u8 = 0x1;
pub const FRAME_RST_STREAM: u8 = 0x3;
pub const FRAME_SETTINGS: u8 = 0x4;
pub const FRAME_PING: u8 = 0x6;
pub const FRAME_GOAWAY: u8 = 0x7;
pub const FRAME_WINDOW_UPDATE: u8 = 0x8;

pub const FLAG_ACK: u8 = 0x1;
pub const FLAG_END_HEADERS: u8 = 0x4;

/// The stream requests go out on.
pub const ROOT_STREAM: u32 = 1;
/// The stream the phone answers on.
pub const REPLY_STREAM: u32 = 3;

const INITIAL_WINDOW: u32 = 16 * 1024 * 1024;
const SETTING_MAX_CONCURRENT_STREAMS: u16 = 0x3;
const SETTING_INITIAL_WINDOW_SIZE: u16 = 0x4;

const WRAPPER_MAGIC: u32 = 0x29B0_0B92;
const PAYLOAD_MAGIC: u32 = 0x4213_3742;
const PROTOCOL_VERSION: u32 = 5;

pub const XPC_ALWAYS_SET: u32 = 0x0000_0001;
pub const XPC_DATA_PRESENT: u32 = 0x0000_0100;
pub const XPC_WANTING_REPLY: u32 = 0x0001_0000;
pub const XPC_INIT_HANDSHAKE: u32 = 0x0040_0000;

/// The largest frame we will send or accept. Ours are far smaller; the cap
/// stops a corrupt length from allocating without bound.
const MAX_SEND_FRAME: usize = 16 * 1024;
const MAX_RECEIVE_FRAME: usize = 1024 * 1024;
/// Frames read while waiting for one specific frame, before giving up.
const FRAME_LIMIT: usize = 4096;

/// One HTTP/2 frame: 9-byte header (24-bit length, type, flags, 31-bit
/// stream id, all big-endian) followed by the payload.
pub fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut out = Vec::with_capacity(9 + payload.len());
    out.extend_from_slice(&len.to_be_bytes()[1..]);
    out.push(kind);
    out.push(flags);
    out.extend_from_slice(&(stream & 0x7FFF_FFFF).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn settings_frame() -> Vec<u8> {
    let mut payload = Vec::new();
    for (id, value) in [
        (SETTING_MAX_CONCURRENT_STREAMS, 100u32),
        (SETTING_INITIAL_WINDOW_SIZE, INITIAL_WINDOW),
    ] {
        payload.extend_from_slice(&id.to_be_bytes());
        payload.extend_from_slice(&value.to_be_bytes());
    }
    frame(FRAME_SETTINGS, 0, 0, &payload)
}

/// An XPC envelope around a dictionary.
pub fn wrapper(body: &XpcValue, message_id: u64, wanting_reply: bool) -> Vec<u8> {
    let mut flags = XPC_ALWAYS_SET;
    let empty = matches!(body, XpcValue::Dictionary(entries) if entries.is_empty());
    if !empty {
        flags |= XPC_DATA_PRESENT;
    }
    if wanting_reply {
        flags |= XPC_WANTING_REPLY;
    }
    let mut payload = Vec::new();
    payload.extend_from_slice(&PAYLOAD_MAGIC.to_le_bytes());
    payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    payload.extend_from_slice(&body.encode());

    let mut out = control(flags, message_id);
    out[8..16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

/// An XPC envelope with no payload, used only while opening the channels.
pub fn control(flags: u32, message_id: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(24);
    out.extend_from_slice(&WRAPPER_MAGIC.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&message_id.to_le_bytes());
    out
}

/// Everything the Mac sends to open a connection, in the order the phone
/// insists on: if stream 3's HEADERS came after stream 1's closing envelope,
/// the phone would drop the connection.
pub fn handshake() -> Vec<u8> {
    let empty = XpcValue::dict::<&str>([]);
    let mut out = PREFACE.to_vec();
    out.extend(settings_frame());
    out.extend(frame(
        FRAME_WINDOW_UPDATE,
        0,
        0,
        &(INITIAL_WINDOW - 65_535).to_be_bytes(),
    ));
    out.extend(frame(FRAME_HEADERS, FLAG_END_HEADERS, ROOT_STREAM, &[]));
    out.extend(frame(
        FRAME_DATA,
        0,
        ROOT_STREAM,
        &wrapper(&empty, 0, false),
    ));
    out.extend(frame(FRAME_HEADERS, FLAG_END_HEADERS, REPLY_STREAM, &[]));
    out.extend(frame(FRAME_DATA, 0, ROOT_STREAM, &control(0x0201, 0)));
    out.extend(frame(
        FRAME_DATA,
        0,
        REPLY_STREAM,
        &control(XPC_ALWAYS_SET | XPC_INIT_HANDSHAKE, 0),
    ));
    out
}

/// Why a RemoteXPC exchange stopped.
#[derive(Debug)]
pub enum Error {
    /// The socket failed, timed out or closed.
    Io(io::Error),
    /// The phone closed the connection or a stream; the text says how.
    Closed(String),
    /// The phone sent something this code does not expect.
    Protocol(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                write!(f, "the iPhone closed the connection")
            }
            Error::Io(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                write!(f, "the iPhone did not answer in time")
            }
            Error::Io(e) => write!(f, "connection error: {e}"),
            Error::Closed(how) => write!(f, "the iPhone closed the connection ({how})"),
            Error::Protocol(what) => write!(f, "unexpected reply from the iPhone: {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

struct Frame {
    kind: u8,
    flags: u8,
    payload: Vec<u8>,
}

fn go_away_reason(payload: &[u8]) -> String {
    if payload.len() < 8 {
        return "GOAWAY".into();
    }
    let code = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    let debug = String::from_utf8_lossy(&payload[8..]);
    if debug.is_empty() {
        format!("GOAWAY error {code}")
    } else {
        format!("GOAWAY error {code}: {debug}")
    }
}

/// An open RemoteXPC connection to one service on the phone.
pub struct Connection<S> {
    stream: S,
    next_id: u64,
}

impl<S: Read + Write> Connection<S> {
    /// Open the channels on a socket that is already connected to a service.
    pub fn open(mut stream: S) -> Result<Self, Error> {
        stream.write_all(&handshake())?;
        let mut conn = Connection { stream, next_id: 1 };
        // The phone's SETTINGS ends the handshake; flow-control frames may
        // come first.
        for _ in 0..100 {
            let frame = conn.read_frame()?;
            match frame.kind {
                FRAME_SETTINGS if frame.flags & FLAG_ACK == 0 => {
                    conn.write_frame(FRAME_SETTINGS, FLAG_ACK, 0, &[])?;
                    return Ok(conn);
                }
                FRAME_SETTINGS | FRAME_WINDOW_UPDATE | FRAME_HEADERS | FRAME_DATA => {}
                FRAME_PING if frame.flags & FLAG_ACK == 0 => {
                    conn.write_frame(FRAME_PING, FLAG_ACK, 0, &frame.payload)?;
                }
                other => {
                    return Err(Error::Protocol(format!(
                        "frame type {other} during the handshake"
                    )));
                }
            }
        }
        Err(Error::Protocol("the handshake never finished".into()))
    }

    /// Send a message that gets no reply (every HID report is one).
    ///
    /// Message ids stay odd: the first request is 1 in both references, and
    /// iPhone Use found an even id can abort the phone's daemon.
    pub fn send(&mut self, body: &XpcValue) -> Result<(), Error> {
        let data = wrapper(body, self.next_id, false);
        self.next_id = self.next_id.wrapping_add(2);
        self.write_frame(FRAME_DATA, 0, ROOT_STREAM, &data)
    }

    /// One HTTP/2 PING round trip.
    ///
    /// Reports get no reply, so sending alone does not show they arrived.
    /// The phone reads frames in order: once the ACK is back, everything
    /// sent before it has been read, and closing can no longer discard it.
    /// It proves delivery to the service, not that the UI reacted.
    pub fn ping(&mut self, token: [u8; 8]) -> Result<(), Error> {
        self.write_frame(FRAME_PING, 0, 0, &token)?;
        for _ in 0..FRAME_LIMIT {
            let frame = self.read_frame()?;
            match frame.kind {
                FRAME_PING if frame.flags & FLAG_ACK != 0 => {
                    if frame.payload == token {
                        return Ok(());
                    }
                }
                FRAME_PING => self.write_frame(FRAME_PING, FLAG_ACK, 0, &frame.payload)?,
                FRAME_SETTINGS if frame.flags & FLAG_ACK == 0 => {
                    self.write_frame(FRAME_SETTINGS, FLAG_ACK, 0, &[])?;
                }
                // DATA and flow-control frames that arrive meanwhile.
                _ => {}
            }
        }
        Err(Error::Protocol("no PING acknowledgement".into()))
    }

    /// Give the socket back, to shut it down.
    pub fn into_inner(self) -> S {
        self.stream
    }

    fn write_frame(
        &mut self,
        kind: u8,
        flags: u8,
        stream: u32,
        payload: &[u8],
    ) -> Result<(), Error> {
        if payload.len() > MAX_SEND_FRAME {
            return Err(Error::Protocol("message too large for one frame".into()));
        }
        self.stream
            .write_all(&frame(kind, flags, stream, payload))?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Frame, Error> {
        let mut header = [0u8; 9];
        self.stream.read_exact(&mut header)?;
        let len =
            usize::from(header[0]) << 16 | usize::from(header[1]) << 8 | usize::from(header[2]);
        if len > MAX_RECEIVE_FRAME {
            return Err(Error::Protocol(format!("a {len}-byte frame")));
        }
        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload)?;
        let stream = u32::from_be_bytes([header[5] & 0x7F, header[6], header[7], header[8]]);
        match header[3] {
            FRAME_GOAWAY => Err(Error::Closed(go_away_reason(&payload))),
            FRAME_RST_STREAM => Err(Error::Closed(format!("stream {stream} reset"))),
            kind => Ok(Frame {
                kind,
                flags: header[4],
                payload,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A socket with scripted input that records what is written.
    struct Fake {
        input: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl Fake {
        fn new(input: Vec<u8>) -> Self {
            Fake {
                input: Cursor::new(input),
                written: Vec::new(),
            }
        }
    }

    impl Read for Fake {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Fake {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn peer_settings() -> Vec<u8> {
        frame(FRAME_SETTINGS, 0, 0, &[0, 4, 0, 0x10, 0, 0])
    }

    #[test]
    fn frames_have_a_nine_byte_big_endian_header() {
        assert_eq!(
            frame(FRAME_DATA, 0, 3, &[0xAA, 0xBB]),
            [0, 0, 2, 0, 0, 0, 0, 0, 3, 0xAA, 0xBB]
        );
        // The reserved top bit of the stream id is always clear.
        assert_eq!(frame(FRAME_PING, 1, 0x8000_0001, &[])[5..9], [0, 0, 0, 1]);
    }

    #[test]
    fn the_handshake_is_byte_for_byte_what_devicectl_sends() {
        let h = handshake();
        let mut want = PREFACE.to_vec();
        // SETTINGS: max concurrent streams 100, initial window 16 MiB.
        want.extend_from_slice(&[0, 0, 12, 4, 0, 0, 0, 0, 0]);
        want.extend_from_slice(&[0, 3, 0, 0, 0, 100, 0, 4, 0x01, 0, 0, 0]);
        // WINDOW_UPDATE on the connection: 16 MiB - 65535.
        want.extend_from_slice(&[0, 0, 4, 8, 0, 0, 0, 0, 0, 0x00, 0xFF, 0x00, 0x01]);
        // HEADERS on stream 1, empty, END_HEADERS.
        want.extend_from_slice(&[0, 0, 0, 1, 4, 0, 0, 0, 1]);
        // DATA on stream 1: an empty dictionary, flags 1, id 0.
        want.extend_from_slice(&[0, 0, 44, 0, 0, 0, 0, 0, 1]);
        want.extend_from_slice(&[0x92, 0x0B, 0xB0, 0x29, 1, 0, 0, 0]);
        want.extend_from_slice(&20u64.to_le_bytes());
        want.extend_from_slice(&0u64.to_le_bytes());
        want.extend_from_slice(&[0x42, 0x37, 0x13, 0x42, 5, 0, 0, 0]);
        want.extend_from_slice(&[0x00, 0xF0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0]);
        // HEADERS on stream 3.
        want.extend_from_slice(&[0, 0, 0, 1, 4, 0, 0, 0, 3]);
        // DATA on stream 1: control envelope, flags 0x201.
        want.extend_from_slice(&[0, 0, 24, 0, 0, 0, 0, 0, 1]);
        want.extend_from_slice(&[0x92, 0x0B, 0xB0, 0x29, 0x01, 0x02, 0, 0]);
        want.extend_from_slice(&[0; 16]);
        // DATA on stream 3: control envelope, flags 0x400001.
        want.extend_from_slice(&[0, 0, 24, 0, 0, 0, 0, 0, 3]);
        want.extend_from_slice(&[0x92, 0x0B, 0xB0, 0x29, 0x01, 0x00, 0x40, 0]);
        want.extend_from_slice(&[0; 16]);
        assert_eq!(h, want);
    }

    #[test]
    fn a_wrapper_sizes_its_payload_and_flags_data() {
        let body = XpcValue::dict([("k", XpcValue::Bool(true))]);
        let w = wrapper(&body, 7, true);
        assert_eq!(&w[0..4], &[0x92, 0x0B, 0xB0, 0x29]);
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 0x10101);
        let size = u64::from_le_bytes(w[8..16].try_into().unwrap()) as usize;
        assert_eq!(size, w.len() - 24);
        assert_eq!(u64::from_le_bytes(w[16..24].try_into().unwrap()), 7);
        assert_eq!(&w[32..], &body.encode()[..]);
        assert_eq!(
            u32::from_le_bytes(wrapper(&body, 1, false)[4..8].try_into().unwrap()),
            0x101
        );
    }

    #[test]
    fn opening_waits_for_settings_and_acknowledges_them() {
        let mut input = frame(FRAME_WINDOW_UPDATE, 0, 0, &[0, 0x0F, 0, 1]);
        input.extend(peer_settings());
        let conn = Connection::open(Fake::new(input)).unwrap();
        let fake = conn.into_inner();
        assert!(fake.written.starts_with(&handshake()));
        assert_eq!(
            &fake.written[handshake().len()..],
            &frame(FRAME_SETTINGS, FLAG_ACK, 0, &[])[..]
        );
    }

    #[test]
    fn opening_fails_cleanly_when_the_phone_hangs_up() {
        let err = Connection::open(Fake::new(vec![])).err().unwrap();
        assert_eq!(err.to_string(), "the iPhone closed the connection");
        let goaway = frame(FRAME_GOAWAY, 0, 0, &[0, 0, 0, 0, 0, 0, 0, 2, b'n', b'o']);
        let err = Connection::open(Fake::new(goaway)).err().unwrap();
        assert!(err.to_string().contains("GOAWAY error 2: no"), "{err}");
    }

    #[test]
    fn requests_go_on_stream_one_with_odd_ids() {
        let mut conn = Connection::open(Fake::new(peer_settings())).unwrap();
        let body = XpcValue::dict([("a", XpcValue::Uint64(1))]);
        conn.send(&body).unwrap();
        conn.send(&body).unwrap();
        let fake = conn.into_inner();
        let sent = &fake.written[handshake().len() + 9..];
        let first = frame(FRAME_DATA, 0, ROOT_STREAM, &wrapper(&body, 1, false));
        let second = frame(FRAME_DATA, 0, ROOT_STREAM, &wrapper(&body, 3, false));
        assert_eq!(sent, [first, second].concat());
    }

    #[test]
    fn ping_waits_for_its_own_token_and_answers_the_phones() {
        let token = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut input = peer_settings();
        input.extend(frame(FRAME_WINDOW_UPDATE, 0, 1, &[0, 0, 1, 0]));
        input.extend(frame(FRAME_PING, 0, 0, &[9; 8])); // the phone's own ping
        input.extend(frame(FRAME_PING, FLAG_ACK, 0, &[0; 8])); // someone else's ack
        input.extend(frame(FRAME_PING, FLAG_ACK, 0, &token));
        let mut conn = Connection::open(Fake::new(input)).unwrap();
        conn.ping(token).unwrap();
        let fake = conn.into_inner();
        let tail = &fake.written[handshake().len() + 9..];
        let want = [
            frame(FRAME_PING, 0, 0, &token),
            frame(FRAME_PING, FLAG_ACK, 0, &[9; 8]),
        ]
        .concat();
        assert_eq!(tail, want);
    }

    #[test]
    fn ping_reports_a_dead_connection() {
        let mut input = peer_settings();
        input.extend(frame(FRAME_RST_STREAM, 0, 1, &[0, 0, 0, 8]));
        let mut conn = Connection::open(Fake::new(input)).unwrap();
        let err = conn.ping([0; 8]).unwrap_err();
        assert!(err.to_string().contains("stream 1 reset"), "{err}");
    }
}
