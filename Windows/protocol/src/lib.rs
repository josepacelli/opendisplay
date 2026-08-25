//! OpenDisplay wire-protocol crate.
//!
//! Ports the constants and framing rules that `Shared/Protocol.swift` and
//! `PROTOCOL.md` define, so `windows-core` speaks the exact same wire as
//! the Mac sender. Values here MUST match `Shared/Protocol.swift:12-27`
//! exactly — see `[[memory:AD-002]]`: this crate is hand-ported, not
//! linked, so any change to `Shared/Protocol.swift` must be mirrored here
//! by hand.

/// The protocol version this build speaks.
/// Mirrors `Shared/Protocol.swift` `WireProtocol.version`.
pub const WIRE_PROTOCOL_VERSION: u32 = 3;

/// Protocol version that introduced Apple Pencil / proximity wire messages.
/// Mirrors `Shared/Protocol.swift` `WireProtocol.pencilWireVersion`.
pub const PENCIL_WIRE_VERSION: u32 = 3;

/// Oldest peer protocol version this build still supports.
/// Mirrors `Shared/Protocol.swift` `WireProtocol.minSupportedPeer`.
pub const MIN_SUPPORTED_PEER: u32 = 1;

/// A peer that advertises no `pv` is defined as protocol 1.
/// Mirrors `Shared/Protocol.swift` `WireProtocol.assumedWhenAbsent`.
pub const ASSUMED_WHEN_ABSENT: u32 = 1;

/// Control-message `type` strings introduced with the handshake.
/// Mirrors `Shared/Protocol.swift`'s `WireMessage` enum.
pub mod wire_message {
    /// Mac -> phone: Mac's pv + min supported.
    pub const WELCOME: &str = "welcome";
    /// Mac -> phone: peer is below the Mac's floor.
    pub const UPDATE_REQUIRED: &str = "updateRequired";
    /// phone -> Mac: device locked, reconnect on wake.
    pub const SLEEPING: &str = "sleeping";
    /// phone -> Mac: app quit, end the session for good.
    pub const CLOSING: &str = "closing";
    /// Receiver -> sender: finger input (`PROTOCOL.md` §6.1).
    pub const TOUCH: &str = "touch";
    /// Receiver -> sender: two-finger scroll (`PROTOCOL.md` §6.1).
    pub const SCROLL: &str = "scroll";
    /// Sender -> receiver liveness beat, sent every 2 seconds while
    /// connected, per `PROTOCOL.md` §6.2/Appendix A and spec WSEND-05. Not
    /// named in `Shared/Protocol.swift`'s `WireMessage` enum (its doc
    /// comment: "existing types (hello, ping, pong, touch, ...) stay inline
    /// for now") — sourced directly from `PROTOCOL.md` instead.
    pub const PING: &str = "ping";
    /// Receiver -> sender (`pv` 3): Apple Pencil input. Out of scope for
    /// this sender (spec Touch/cursor AC 4) — accepted and ignored, never
    /// an error, per `PROTOCOL.md` Appendix A ("ignore every control
    /// message it does not care about"). Not named in `Shared/Protocol.swift`'s
    /// `WireMessage` enum — sourced directly from `PROTOCOL.md` §6.1/§7
    /// instead.
    pub const PENCIL: &str = "pencil";
    /// Receiver -> sender (`pv` 3): stylus hover enter/leave. Same
    /// out-of-scope/ignore treatment as [`PENCIL`].
    pub const PROXIMITY: &str = "proximity";
}

/// Frame length header size in bytes (PROTOCOL.md §3).
const LENGTH_PREFIX_BYTES: usize = 4;

/// The receiver-direction payload bound per `PROTOCOL.md` §3: "Receiver to
/// sender, the payload MUST be `1` to `2^20 - 1` bytes. The official sender
/// treats a length of 0 or `>= 2^20` as a protocol error." `windows-core`
/// plays the sender role, so this is the bound it must enforce both when
/// decoding frames from the receiver and when it emits its own frames
/// (Edge Case: frame-size bound applies to every frame this sender emits).
pub const MAX_PAYLOAD_LEN: usize = (1 << 20) - 1;

/// Errors that can occur while framing or unframing a payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// The payload was empty (length 0), which `PROTOCOL.md` §3 forbids.
    PayloadEmpty,
    /// The payload was `>= 2^20` bytes, which `PROTOCOL.md` §3 forbids.
    PayloadTooLarge { len: usize },
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::PayloadEmpty => write!(f, "frame payload must not be empty"),
            FrameError::PayloadTooLarge { len } => {
                write!(f, "frame payload of {len} bytes exceeds the {MAX_PAYLOAD_LEN}-byte bound")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Validates a payload length against the `PROTOCOL.md` §3 bound
/// (`1..=2^20-1` bytes). Shared by both encode and decode so the two sides
/// of the wire agree on what is valid.
fn validate_payload_len(len: usize) -> Result<(), FrameError> {
    if len == 0 {
        return Err(FrameError::PayloadEmpty);
    }
    if len >= (1 << 20) {
        return Err(FrameError::PayloadTooLarge { len });
    }
    Ok(())
}

/// Encodes `payload` as a length-prefixed frame: a 4-byte big-endian length
/// followed by the payload bytes, per `PROTOCOL.md` §3. Rejects a payload
/// that violates the length bound rather than emitting it.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    validate_payload_len(payload.len())?;

    let mut framed = Vec::with_capacity(LENGTH_PREFIX_BYTES + payload.len());
    framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    framed.extend_from_slice(payload);
    Ok(framed)
}

/// The result of successfully decoding one frame out of a buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame<'a> {
    /// The frame's payload (excludes the 4-byte length header).
    pub payload: &'a [u8],
    /// Total bytes consumed from `buf` (header + payload).
    pub consumed: usize,
}

/// Attempts to decode one length-prefixed frame from the front of `buf`.
///
/// TCP gives no message boundaries (`PROTOCOL.md` §3), so this is an
/// incremental parser over whatever bytes have arrived so far:
/// - `Ok(None)` means `buf` does not yet contain a complete frame — the
///   caller should read more bytes and try again.
/// - `Ok(Some(frame))` means a complete, valid frame was found.
/// - `Err(_)` means the header declares a length that violates the
///   `PROTOCOL.md` §3 bound.
pub fn decode_frame(buf: &[u8]) -> Result<Option<DecodedFrame<'_>>, FrameError> {
    if buf.len() < LENGTH_PREFIX_BYTES {
        return Ok(None);
    }

    let len_bytes: [u8; LENGTH_PREFIX_BYTES] = buf[..LENGTH_PREFIX_BYTES]
        .try_into()
        .expect("slice length checked above");
    let len = u32::from_be_bytes(len_bytes) as usize;

    validate_payload_len(len)?;

    let total = LENGTH_PREFIX_BYTES + len;
    if buf.len() < total {
        return Ok(None);
    }

    Ok(Some(DecodedFrame {
        payload: &buf[LENGTH_PREFIX_BYTES..total],
        consumed: total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Constants: must match Shared/Protocol.swift:12-27 exactly. ---

    #[test]
    fn wire_protocol_version_matches_shared_protocol_swift() {
        assert_eq!(WIRE_PROTOCOL_VERSION, 3);
    }

    #[test]
    fn pencil_wire_version_matches_shared_protocol_swift() {
        assert_eq!(PENCIL_WIRE_VERSION, 3);
    }

    #[test]
    fn min_supported_peer_matches_shared_protocol_swift() {
        assert_eq!(MIN_SUPPORTED_PEER, 1);
    }

    #[test]
    fn assumed_when_absent_matches_shared_protocol_swift() {
        assert_eq!(ASSUMED_WHEN_ABSENT, 1);
    }

    #[test]
    fn wire_message_strings_match_shared_protocol_swift() {
        assert_eq!(wire_message::WELCOME, "welcome");
        assert_eq!(wire_message::UPDATE_REQUIRED, "updateRequired");
        assert_eq!(wire_message::SLEEPING, "sleeping");
        assert_eq!(wire_message::CLOSING, "closing");
    }

    #[test]
    fn touch_and_scroll_wire_strings_match_protocol_md() {
        assert_eq!(wire_message::TOUCH, "touch");
        assert_eq!(wire_message::SCROLL, "scroll");
    }

    #[test]
    fn ping_wire_string_matches_protocol_md() {
        assert_eq!(wire_message::PING, "ping");
    }

    #[test]
    fn pencil_and_proximity_wire_strings_match_protocol_md() {
        assert_eq!(wire_message::PENCIL, "pencil");
        assert_eq!(wire_message::PROXIMITY, "proximity");
    }

    // --- Framing: PROTOCOL.md §3. ---

    #[test]
    fn encode_decode_round_trips_a_payload() {
        let payload = b"{\"type\":\"hello\"}";
        let framed = encode_frame(payload).expect("valid payload encodes");

        // 4-byte big-endian length prefix, per PROTOCOL.md §3.
        assert_eq!(&framed[0..4], &(payload.len() as u32).to_be_bytes());

        let decoded = decode_frame(&framed).expect("valid frame decodes").expect("frame is complete");
        assert_eq!(decoded.payload, payload);
        assert_eq!(decoded.consumed, framed.len());
    }

    #[test]
    fn decode_returns_none_when_buffer_has_incomplete_header() {
        let buf = [0u8, 0, 0]; // fewer than 4 header bytes
        assert_eq!(decode_frame(&buf), Ok(None));
    }

    #[test]
    fn decode_returns_none_when_payload_not_fully_arrived_yet() {
        let payload = b"hello world";
        let framed = encode_frame(payload).expect("valid payload encodes");
        // Simulate a partial TCP read: header plus only part of the payload.
        let partial = &framed[..framed.len() - 3];
        assert_eq!(decode_frame(partial), Ok(None));
    }

    #[test]
    fn decode_stops_at_the_first_frame_when_two_are_packed_together() {
        let first = encode_frame(b"first").unwrap();
        let second = encode_frame(b"second").unwrap();
        let mut both = first.clone();
        both.extend_from_slice(&second);

        let decoded = decode_frame(&both).unwrap().unwrap();
        assert_eq!(decoded.payload, b"first");
        assert_eq!(decoded.consumed, first.len());
    }

    #[test]
    fn encode_rejects_a_payload_of_2_pow_20_bytes_or_larger() {
        let oversized = vec![0u8; 1 << 20]; // exactly 2^20 bytes
        assert_eq!(
            encode_frame(&oversized),
            Err(FrameError::PayloadTooLarge { len: 1 << 20 })
        );
    }

    #[test]
    fn encode_accepts_the_largest_valid_payload_of_2_pow_20_minus_1_bytes() {
        let max_valid = vec![0u8; MAX_PAYLOAD_LEN];
        assert!(encode_frame(&max_valid).is_ok());
    }

    #[test]
    fn encode_rejects_an_empty_payload() {
        assert_eq!(encode_frame(&[]), Err(FrameError::PayloadEmpty));
    }

    #[test]
    fn decode_rejects_a_header_declaring_a_length_of_2_pow_20_or_larger() {
        let mut buf = ((1u32 << 20).to_be_bytes()).to_vec();
        buf.extend_from_slice(&[0u8; 8]); // trailing bytes irrelevant; header alone is invalid
        assert_eq!(
            decode_frame(&buf),
            Err(FrameError::PayloadTooLarge { len: 1 << 20 })
        );
    }
}
