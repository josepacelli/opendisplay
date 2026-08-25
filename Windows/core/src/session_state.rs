//! The reconnect/retry state machine for one device's session.
//!
//! Mirrors the Mac sender's existing retry behavior
//! (`Mac/MacSender.swift`): a flat 1.0s redial interval (no exponential
//! backoff, `scheduleReconnect()`), and — since the wire protocol has no
//! explicit "you were replaced" signal (`PROTOCOL.md` §1: "a receiver
//! serves one sender at a time", but no message announces it) — the Mac
//! sender infers "another sender took over" from `refusalsBeforeGivingUp`
//! consecutive `ECONNREFUSED` dials (`dialRefused()`), rather than from any
//! wire message. `windows-core` reuses that same detection: the receiver
//! that already has a sender simply refuses new connections, so repeated
//! dial refusals are the only observable signal that fits spec WiFi AC 7.
//!
//! This module is the pure state-transition logic only; the actual 1.0s
//! timer and the actual redial call are runtime concerns for whatever
//! drives this state machine (not testable here without a live
//! network/device — see the Test Coverage Matrix).

/// How many consecutive dial refusals are treated as "another sender has
/// taken over this device", not a transient failure. Mirrors
/// `Mac/MacSender.swift`'s `refusalsBeforeGivingUp = 3`.
pub const REFUSALS_BEFORE_ANOTHER_SENDER: u32 = 3;

/// Mirrors `Mac/MacSender.swift`'s flat `scheduleReconnect()` redial
/// interval — no exponential backoff. Documented here for parity; the
/// actual timer lives in whatever drives this state machine at runtime.
pub const RETRY_INTERVAL_SECS: f64 = 1.0;

/// How often `windows-core` sends a `ping` control message while connected,
/// per `PROTOCOL.md` §6.2/Appendix A and spec WSEND-05 ("every 2 seconds").
pub const PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Decides whether a `ping` should be sent right now, given the session's
/// current state and when the last one went out (spec WSEND-05: "send a
/// `ping` control message every 2 seconds while connected").
///
/// Gated on [`SessionState::Connected`] only — never while retrying,
/// paused, or terminal. `last_ping_at` of `None` means no ping has gone out
/// yet this session, which always fires immediately. `now` is passed in
/// rather than read via `Instant::now()` inside this function, the same
/// seam this module's other pure logic uses (real timer/OS-clock calls stay
/// in whatever runtime loop drives this state machine, per the module-level
/// doc comment); that keeps this decision itself plain, injectable-clock
/// testable Rust.
pub fn should_send_ping(
    state: SessionState,
    last_ping_at: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    if state != SessionState::Connected {
        return false;
    }
    match last_ping_at {
        None => true,
        Some(last) => now.duration_since(last) >= PING_INTERVAL,
    }
}

/// Why a session reached `Terminal` — no further auto-retry follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalReason {
    /// The receiver's `closing` control message: user quit the app.
    Closing,
    /// Repeated dial refusals inferred as "the receiver adopted a
    /// different sender" (spec WiFi AC 7).
    AnotherSenderConnected,
}

/// One device's session state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Handshake complete, streaming.
    Connected,
    /// Reconnecting after an ordinary drop; counts consecutive dial
    /// refusals to detect the "another sender" case.
    Retrying { consecutive_refusals: u32 },
    /// The device reported `sleeping`; the display was torn down and the
    /// session waits to redial on wake.
    Paused,
    /// No further auto-retry for this device.
    Terminal { reason: TerminalReason },
}

impl SessionState {
    /// The initial state once a handshake completes.
    pub fn new_connected() -> Self {
        SessionState::Connected
    }
}

/// Inputs that drive a session state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// An ordinary connection drop (socket closed, no known reason).
    Dropped,
    /// A redial attempt is beginning (used to leave `Paused`).
    DialAttempted,
    /// A dial attempt was refused (`ECONNREFUSED`).
    DialRefused,
    /// A redial attempt succeeded.
    DialSucceeded,
    /// The receiver's `sleeping` control message (`PROTOCOL.md` §6.1).
    ReceivedSleeping,
    /// The receiver's `closing` control message (`PROTOCOL.md` §6.1).
    ReceivedClosing,
}

/// Advances `state` in response to `event`, per the transitions documented
/// on [`SessionState`]. `Terminal` never transitions further — once a
/// device's session is terminal, no event revives it, matching "no further
/// auto-retry for that device" (spec WiFi AC 7 / Edge Case `closing`).
pub fn transition(state: SessionState, event: SessionEvent) -> SessionState {
    match (state, event) {
        // Terminal is sticky: no event moves it.
        (SessionState::Terminal { reason }, _) => SessionState::Terminal { reason },

        // A closing message ends the session for good, from any non-terminal state.
        (_, SessionEvent::ReceivedClosing) => {
            SessionState::Terminal { reason: TerminalReason::Closing }
        }

        // Connected: an ordinary drop starts retrying; sleeping pauses.
        (SessionState::Connected, SessionEvent::Dropped) => {
            SessionState::Retrying { consecutive_refusals: 0 }
        }
        (SessionState::Connected, SessionEvent::ReceivedSleeping) => SessionState::Paused,

        // Retrying: refusals accumulate toward the another-sender floor;
        // a successful dial returns to Connected.
        (SessionState::Retrying { consecutive_refusals }, SessionEvent::DialRefused) => {
            let refusals = consecutive_refusals + 1;
            if refusals >= REFUSALS_BEFORE_ANOTHER_SENDER {
                SessionState::Terminal { reason: TerminalReason::AnotherSenderConnected }
            } else {
                SessionState::Retrying { consecutive_refusals: refusals }
            }
        }
        (SessionState::Retrying { .. }, SessionEvent::DialSucceeded) => SessionState::Connected,

        // Paused: the next dial attempt resumes retrying (not directly to
        // Connected — it still has to succeed).
        (SessionState::Paused, SessionEvent::DialAttempted) => {
            SessionState::Retrying { consecutive_refusals: 0 }
        }

        // Any other (state, event) pair is a no-op for this state machine's
        // documented transitions.
        (state, _) => state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_is_the_initial_happy_path_state() {
        assert_eq!(SessionState::new_connected(), SessionState::Connected);
    }

    #[test]
    fn ordinary_drop_transitions_from_connected_to_retrying() {
        let next = transition(SessionState::Connected, SessionEvent::Dropped);
        assert_eq!(next, SessionState::Retrying { consecutive_refusals: 0 });
    }

    #[test]
    fn closing_message_transitions_to_terminal_with_no_further_retry() {
        let next = transition(SessionState::Connected, SessionEvent::ReceivedClosing);
        assert_eq!(next, SessionState::Terminal { reason: TerminalReason::Closing });
    }

    #[test]
    fn fewer_than_the_refusal_floor_stays_in_retrying() {
        let mut state = SessionState::Retrying { consecutive_refusals: 0 };
        for _ in 0..(REFUSALS_BEFORE_ANOTHER_SENDER - 1) {
            state = transition(state, SessionEvent::DialRefused);
        }
        assert_eq!(
            state,
            SessionState::Retrying { consecutive_refusals: REFUSALS_BEFORE_ANOTHER_SENDER - 1 }
        );
    }

    #[test]
    fn reaching_the_refusal_floor_transitions_to_terminal_as_another_sender_connected() {
        let mut state = SessionState::Retrying { consecutive_refusals: 0 };
        for _ in 0..REFUSALS_BEFORE_ANOTHER_SENDER {
            state = transition(state, SessionEvent::DialRefused);
        }
        assert_eq!(
            state,
            SessionState::Terminal { reason: TerminalReason::AnotherSenderConnected }
        );
    }

    #[test]
    fn sleeping_message_transitions_from_connected_to_paused() {
        let next = transition(SessionState::Connected, SessionEvent::ReceivedSleeping);
        assert_eq!(next, SessionState::Paused);
    }

    #[test]
    fn paused_returns_to_retrying_on_the_next_dial_attempt() {
        let next = transition(SessionState::Paused, SessionEvent::DialAttempted);
        assert_eq!(next, SessionState::Retrying { consecutive_refusals: 0 });
    }

    #[test]
    fn terminal_state_ignores_further_events_no_auto_retry_for_that_device() {
        let terminal = SessionState::Terminal { reason: TerminalReason::AnotherSenderConnected };
        let next = transition(terminal, SessionEvent::DialSucceeded);
        assert_eq!(next, terminal);
    }

    // --- should_send_ping: spec WSEND-05 ("every 2 seconds while connected"). ---

    #[test]
    fn ping_fires_immediately_on_first_connect_with_no_prior_ping() {
        let now = std::time::Instant::now();
        assert!(should_send_ping(SessionState::Connected, None, now));
    }

    #[test]
    fn ping_does_not_fire_before_the_interval_elapses() {
        let last = std::time::Instant::now();
        let now = last + std::time::Duration::from_millis(500);
        assert!(!should_send_ping(SessionState::Connected, Some(last), now));
    }

    #[test]
    fn ping_fires_once_the_full_interval_has_elapsed() {
        let last = std::time::Instant::now();
        let now = last + PING_INTERVAL;
        assert!(should_send_ping(SessionState::Connected, Some(last), now));
    }

    #[test]
    fn ping_never_fires_outside_the_connected_state() {
        let last = std::time::Instant::now();
        let now = last + PING_INTERVAL;
        assert!(!should_send_ping(
            SessionState::Retrying { consecutive_refusals: 0 },
            Some(last),
            now
        ));
        assert!(!should_send_ping(SessionState::Paused, Some(last), now));
        assert!(!should_send_ping(
            SessionState::Terminal { reason: TerminalReason::Closing },
            Some(last),
            now
        ));
    }
}
