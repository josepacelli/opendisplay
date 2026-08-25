//! The `hello`/`welcome` handshake, per `PROTOCOL.md` §6.
//!
//! `windows-core` plays the *sender* role: it waits for the receiver's
//! `hello` (the first message on every new connection), replies with
//! `welcome`, and — if the peer's `pv` is below `MIN_SUPPORTED_PEER` —
//! additionally sends `updateRequired` and refuses to stream, per the
//! spec's Edge Cases.
//!
//! Generic over `Read + Write` rather than tied to `TcpStream`, so the
//! handshake logic is exercised in tests against an in-memory fake stream
//! instead of a live socket.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// The receiver's `hello` payload, per `PROTOCOL.md` §6.1.
#[derive(Debug, Deserialize)]
struct HelloWire {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(rename = "pixelsWide")]
    pixels_wide: u32,
    #[serde(rename = "pixelsHigh")]
    pixels_high: u32,
    scale: f32,
    device: Option<String>,
    id: Option<String>,
    pv: Option<u32>,
}

#[derive(Debug, Serialize)]
struct WelcomeWire {
    #[serde(rename = "type")]
    msg_type: &'static str,
    pv: u32,
    min: u32,
}

#[derive(Debug, Serialize)]
struct PingWire {
    #[serde(rename = "type")]
    msg_type: &'static str,
}

/// Builds the outbound `ping` payload windows-core sends every 2 seconds
/// while connected (spec WSEND-05, `PROTOCOL.md` §6.2/Appendix A). No
/// optional telemetry fields (`drops`/`encDrops`/`netDrops`/...) are
/// populated — `PROTOCOL.md`'s ping table marks all of them optional, and
/// this sender does not yet track those stats. Whether/when to call this is
/// `session_state::should_send_ping`'s decision; this function only builds
/// the wire bytes.
pub fn build_ping_message() -> Vec<u8> {
    serde_json::to_vec(&PingWire {
        msg_type: protocol::wire_message::PING,
    })
    .expect("PingWire always serializes")
}

#[derive(Debug, Serialize)]
struct UpdateRequiredWire {
    #[serde(rename = "type")]
    msg_type: &'static str,
    target: &'static str,
    store: String,
    message: String,
}

/// The receiver's reported panel, straight from `hello` (before any
/// `DisplaySpec` derivation — that's T15's job).
#[derive(Debug, Clone, PartialEq)]
pub struct PanelReport {
    pub pixels_wide: u32,
    pub pixels_high: u32,
    pub scale: f32,
    pub device: Option<String>,
    pub id: Option<String>,
}

/// The result of a completed handshake.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// The peer's `hello.pv`, defaulted per `COMPATIBILITY.md` §2.
    pub peer_pv: u32,
    /// The lower of the peer's `pv` and this sender's own
    /// `protocol::WIRE_PROTOCOL_VERSION` — the effective version this
    /// session speaks.
    pub negotiated_pv: u32,
    pub panel: PanelReport,
}

/// Errors that can occur during the handshake.
#[derive(Debug)]
pub enum HandshakeError {
    Io(io::Error),
    MalformedHello(serde_json::Error),
    UnexpectedFirstMessage { got: String },
    Frame(protocol::FrameError),
    /// The peer's `pv` is below `protocol::MIN_SUPPORTED_PEER`. `welcome`
    /// and `updateRequired` have already been sent by the time this is
    /// returned; the caller MUST NOT proceed to streaming for this session.
    PeerBelowMinSupported { peer_pv: u32 },
}

impl std::fmt::Display for HandshakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeError::Io(err) => write!(f, "handshake I/O error: {err}"),
            HandshakeError::MalformedHello(err) => write!(f, "malformed hello: {err}"),
            HandshakeError::UnexpectedFirstMessage { got } => {
                write!(f, "expected \"hello\" as the first message, got {got:?}")
            }
            HandshakeError::Frame(err) => write!(f, "handshake framing error: {err}"),
            HandshakeError::PeerBelowMinSupported { peer_pv } => {
                write!(f, "peer pv {peer_pv} is below the sender's minimum supported peer")
            }
        }
    }
}

impl std::error::Error for HandshakeError {}

/// Reads bytes from `stream` until one complete length-prefixed frame (per
/// `PROTOCOL.md` §3) has arrived, and returns its payload.
fn read_one_frame<S: Read>(stream: &mut S) -> Result<Vec<u8>, HandshakeError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match protocol::decode_frame(&buf) {
            Ok(Some(frame)) => return Ok(frame.payload.to_vec()),
            Ok(None) => {
                let n = stream.read(&mut chunk).map_err(HandshakeError::Io)?;
                if n == 0 {
                    return Err(HandshakeError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed before a full frame arrived",
                    )));
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(err) => return Err(HandshakeError::Frame(err)),
        }
    }
}

fn write_frame<S: Write>(stream: &mut S, payload: &[u8]) -> Result<(), HandshakeError> {
    let framed = protocol::encode_frame(payload).map_err(HandshakeError::Frame)?;
    stream.write_all(&framed).map_err(HandshakeError::Io)
}

/// Performs the `hello`/`welcome` handshake on an already-connected
/// `stream` (see `transport::dial`, T12), per `PROTOCOL.md` §6.
pub fn handshake<S: Read + Write>(stream: &mut S) -> Result<Session, HandshakeError> {
    let hello_bytes = read_one_frame(stream)?;
    let hello: HelloWire =
        serde_json::from_slice(&hello_bytes).map_err(HandshakeError::MalformedHello)?;

    if hello.msg_type != "hello" {
        return Err(HandshakeError::UnexpectedFirstMessage { got: hello.msg_type });
    }

    let peer_pv = hello.pv.unwrap_or(protocol::ASSUMED_WHEN_ABSENT);

    let welcome = WelcomeWire {
        msg_type: protocol::wire_message::WELCOME,
        pv: protocol::WIRE_PROTOCOL_VERSION,
        min: protocol::MIN_SUPPORTED_PEER,
    };
    write_frame(
        stream,
        serde_json::to_string(&welcome)
            .expect("WelcomeWire always serializes")
            .as_bytes(),
    )?;

    if peer_pv < protocol::MIN_SUPPORTED_PEER {
        let update_required = UpdateRequiredWire {
            msg_type: protocol::wire_message::UPDATE_REQUIRED,
            target: "ios",
            store: "https://apps.apple.com/app/opendisplay".to_string(),
            message: "Please update the OpenDisplay app on your device.".to_string(),
        };
        write_frame(
            stream,
            serde_json::to_string(&update_required)
                .expect("UpdateRequiredWire always serializes")
                .as_bytes(),
        )?;
        return Err(HandshakeError::PeerBelowMinSupported { peer_pv });
    }

    Ok(Session {
        peer_pv,
        negotiated_pv: peer_pv.min(protocol::WIRE_PROTOCOL_VERSION),
        panel: PanelReport {
            pixels_wide: hello.pixels_wide,
            pixels_high: hello.pixels_high,
            scale: hello.scale,
            device: hello.device,
            id: hello.id,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// An in-memory stand-in for the dialed `TcpStream`, so the handshake
    /// is exercised without a live socket (per the task: "exercised via an
    /// injected fake peer").
    struct FakeStream {
        incoming: Cursor<Vec<u8>>,
        outgoing: Vec<u8>,
    }

    impl FakeStream {
        fn with_incoming(bytes: Vec<u8>) -> Self {
            Self { incoming: Cursor::new(bytes), outgoing: Vec::new() }
        }
    }

    impl Read for FakeStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.incoming.read(buf)
        }
    }

    impl Write for FakeStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.outgoing.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn hello_frame(json: &str) -> Vec<u8> {
        protocol::encode_frame(json.as_bytes()).unwrap()
    }

    /// Decodes every frame packed into `buf`, returning each payload as a
    /// JSON value.
    fn decode_all_frames(mut buf: &[u8]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        while let Ok(Some(frame)) = protocol::decode_frame(buf) {
            out.push(serde_json::from_slice(frame.payload).unwrap());
            buf = &buf[frame.consumed..];
        }
        out
    }

    #[test]
    fn successful_handshake_yields_session_with_negotiated_pv_and_panel_dimensions() {
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":1668,"pixelsHigh":2388,"scale":2.0,"device":"iPad","id":"00008030-ABC","pv":3}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        let session = handshake(&mut stream).expect("compatible peer completes the handshake");

        assert_eq!(session.peer_pv, 3);
        assert_eq!(session.negotiated_pv, 3.min(protocol::WIRE_PROTOCOL_VERSION));
        assert_eq!(
            session.panel,
            PanelReport {
                pixels_wide: 1668,
                pixels_high: 2388,
                scale: 2.0,
                device: Some("iPad".to_string()),
                id: Some("00008030-ABC".to_string()),
            }
        );
    }

    #[test]
    fn hello_with_absent_pv_defaults_to_assumed_when_absent() {
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":750,"pixelsHigh":1334,"scale":2.0}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        let session = handshake(&mut stream).expect("pre-pv-2 peer still completes the handshake");

        assert_eq!(session.peer_pv, protocol::ASSUMED_WHEN_ABSENT);
    }

    #[test]
    fn successful_handshake_writes_a_welcome_frame_with_this_senders_pv_and_min() {
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":1668,"pixelsHigh":2388,"scale":2.0,"pv":3}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        handshake(&mut stream).expect("compatible peer completes the handshake");

        let frames = decode_all_frames(&stream.outgoing);
        assert_eq!(frames.len(), 1, "only welcome, no updateRequired, for a compatible peer");
        assert_eq!(frames[0]["type"], "welcome");
        assert_eq!(frames[0]["pv"], protocol::WIRE_PROTOCOL_VERSION);
        assert_eq!(frames[0]["min"], protocol::MIN_SUPPORTED_PEER);
    }

    #[test]
    fn peer_below_min_supported_peer_receives_update_required_and_session_ends_without_streaming() {
        // The real MIN_SUPPORTED_PEER is 1, so this injects a fake peer
        // advertising pv 0 to exercise the below-floor path, per the
        // task's note ("since the real constant is 1").
        let hello = hello_frame(
            r#"{"type":"hello","pixelsWide":750,"pixelsHigh":1334,"scale":2.0,"pv":0}"#,
        );
        let mut stream = FakeStream::with_incoming(hello);

        let result = handshake(&mut stream);

        match result {
            Err(HandshakeError::PeerBelowMinSupported { peer_pv }) => assert_eq!(peer_pv, 0),
            other => panic!("expected PeerBelowMinSupported, got {other:?}"),
        }

        let frames = decode_all_frames(&stream.outgoing);
        assert_eq!(frames.len(), 2, "welcome, then updateRequired");
        assert_eq!(frames[0]["type"], "welcome");
        assert_eq!(frames[1]["type"], "updateRequired");
        assert_eq!(frames[1]["target"], "ios");
    }

    // --- build_ping_message: spec WSEND-05. ---

    #[test]
    fn build_ping_message_produces_the_ping_wire_type() {
        let bytes = build_ping_message();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["type"], "ping");
    }
}
