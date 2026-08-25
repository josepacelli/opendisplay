//! Wires encoded frames into length-prefixed wire frames on the session's
//! connection, per `PROTOCOL.md` §3, rejecting any payload that would
//! exceed the `2^20 - 1`-byte receiver-direction bound (Edge Case) rather
//! than emitting it truncated or split across multiple frames.
//!
//! Generic over `Write` rather than tied to `TcpStream` — same pattern as
//! `protocol_session::handshake` — so this is exercised in tests against an
//! in-memory sink instead of a live socket.

use std::io::{self, Write};

/// An error from [`FrameSender::send`]. `Oversized` means the payload was
/// rejected *before* touching the underlying stream at all — `protocol`'s
/// `encode_frame` validates the length up front, so this function never
/// gets far enough to write a partial/truncated frame.
#[derive(Debug)]
pub enum FrameSendError {
    Io(io::Error),
    /// The payload violates `PROTOCOL.md` §3's `1..=2^20-1`-byte bound.
    Oversized(protocol::FrameError),
}

impl std::fmt::Display for FrameSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameSendError::Io(err) => write!(f, "frame send I/O error: {err}"),
            FrameSendError::Oversized(err) => write!(f, "frame rejected before sending: {err}"),
        }
    }
}

impl std::error::Error for FrameSendError {}

/// Sends encoded frames (video or control) on `S` as length-prefixed wire
/// frames, per `PROTOCOL.md` §3.
pub struct FrameSender<S: Write> {
    stream: S,
}

impl<S: Write> FrameSender<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Encodes `payload` as one length-prefixed frame and writes it to the
    /// underlying stream. Rejects (without writing anything) a payload that
    /// would violate the `PROTOCOL.md` §3 bound, per the Edge Case
    /// ("SHALL NOT emit it as a single frame").
    pub fn send(&mut self, payload: &[u8]) -> Result<(), FrameSendError> {
        let framed = protocol::encode_frame(payload).map_err(FrameSendError::Oversized)?;
        self.stream.write_all(&framed).map_err(FrameSendError::Io)
    }

    /// Returns the underlying stream, consuming the sender.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_sized_frame_is_length_prefixed_and_sent_correctly() {
        let mut sender = FrameSender::new(Vec::<u8>::new());
        let payload = b"\x00\x00\x00\x01NALU-DATA";

        sender.send(payload).expect("a normal-sized payload sends");

        let sent = sender.into_inner();
        assert_eq!(&sent[0..4], &(payload.len() as u32).to_be_bytes());
        assert_eq!(&sent[4..], payload);
    }

    #[test]
    fn the_largest_valid_payload_sends_successfully() {
        let mut sender = FrameSender::new(Vec::<u8>::new());
        let payload = vec![0xABu8; protocol::MAX_PAYLOAD_LEN];

        sender.send(&payload).expect("the largest valid payload sends");

        let sent = sender.into_inner();
        assert_eq!(sent.len(), 4 + protocol::MAX_PAYLOAD_LEN);
    }

    #[test]
    fn an_oversized_frame_is_rejected_before_it_reaches_the_socket() {
        let mut sender = FrameSender::new(Vec::<u8>::new());
        let oversized = vec![0u8; 1 << 20]; // exactly 2^20 bytes: over the bound

        let result = sender.send(&oversized);

        match result {
            Err(FrameSendError::Oversized(protocol::FrameError::PayloadTooLarge { len })) => {
                assert_eq!(len, 1 << 20);
            }
            other => panic!("expected Oversized(PayloadTooLarge), got {other:?}"),
        }

        // Nothing reached the "socket": not truncated, not partially
        // written, not split into multiple frames.
        let sent = sender.into_inner();
        assert!(sent.is_empty(), "an oversized payload must write zero bytes, not a truncated frame");
    }

    #[test]
    fn an_empty_payload_is_rejected_before_it_reaches_the_socket() {
        let mut sender = FrameSender::new(Vec::<u8>::new());

        let result = sender.send(&[]);

        assert!(matches!(result, Err(FrameSendError::Oversized(protocol::FrameError::PayloadEmpty))));
        let sent = sender.into_inner();
        assert!(sent.is_empty());
    }
}
