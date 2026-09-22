//! Authoritative `AgentSession` state machine.
//!
//! Rules:
//!  - Callers propose a transition via `apply`.
//!  - If the transition is valid, the session's state is updated and
//!    `TransitionResult::Changed` is returned.
//!  - If the transition is invalid, state is unchanged and
//!    `TransitionResult::Rejected` is returned — the caller must record a
//!    `TransitionRejected` event for diagnostics but must NOT mutate state.

use crate::types::{AgentSession, SessionState};

/// Result of attempting a state transition.
#[derive(Debug, PartialEq, Eq)]
pub enum TransitionResult {
    /// Transition was valid; state was updated.
    Changed { from: SessionState, to: SessionState },
    /// Transition was invalid for the current state; state unchanged.
    Rejected { current: SessionState, attempted: SessionState },
}

/// The set of transitions that the state machine enforces.
/// Returns true when `from → to` is a legal edge in the transition table.
pub fn is_valid_transition(from: &SessionState, to: &SessionState) -> bool {
    use SessionState::*;
    matches!(
        (from, to),
        // Creation → start
        (Idle, Starting)
        // Idle can be stopped directly (no adapter running)
        | (Idle, Stopped)
        // Adapter ready / start failed
        | (Starting, Working)
        | (Starting, Failed)
        // Normal operation → waiting
        | (Working, WaitingForHuman)
        // Human reply or policy decision delivered
        | (WaitingForHuman, Working)
        // Pause / resume
        | (Working, Paused)
        | (WaitingForHuman, Paused)
        | (Paused, Working)
        | (RateLimited, Working)
        // Rate-limiting
        | (Working, RateLimited)
        | (WaitingForHuman, RateLimited)
        // Graceful stop from any non-terminal active state
        | (Working, Stopping)
        | (WaitingForHuman, Stopping)
        | (Paused, Stopping)
        | (RateLimited, Stopping)
        | (Starting, Stopping)
        // Stopping → terminal
        | (Stopping, Stopped)
        | (Stopping, HandedOff)
        // Crashes
        | (Working, Crashed)
        | (WaitingForHuman, Crashed)
        | (Paused, Crashed)
        | (Starting, Crashed)
        // Restart cycle
        | (Crashed, Restarting)
        | (Restarting, Starting)
        | (Crashed, Failed)   // restart limit exceeded
    )
}

/// Attempt to transition `session` to `to`.
/// Updates `session.state` and `session.updated_at` on success.
pub fn apply(session: &mut AgentSession, to: SessionState) -> TransitionResult {
    let from = session.state.clone();

    if is_valid_transition(&from, &to) {
        let now = chrono::Utc::now();
        session.state = to.clone();
        session.updated_at = now;

        // Update convenience timestamps
        if to == SessionState::Working && session.started_at.is_none() {
            session.started_at = Some(now);
        }
        if to.is_terminal() {
            session.stopped_at = Some(now);
        }
        if to == SessionState::Restarting {
            session.restart_count += 1;
        }

        TransitionResult::Changed { from, to }
    } else {
        TransitionResult::Rejected {
            current: from,
            attempted: to,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentSession, Id, SessionState};

    fn session() -> AgentSession {
        AgentSession::new(Id::new(), "test task".into(), "mock".into())
    }

    // ── Valid transitions ───────────────────────────────────────────────────

    #[test]
    fn idle_to_starting() {
        let mut s = session();
        assert_eq!(s.state, SessionState::Idle);
        let r = apply(&mut s, SessionState::Starting);
        assert!(matches!(r, TransitionResult::Changed { .. }));
        assert_eq!(s.state, SessionState::Starting);
    }

    #[test]
    fn starting_to_working() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        let r = apply(&mut s, SessionState::Working);
        assert!(matches!(r, TransitionResult::Changed { .. }));
        assert_eq!(s.state, SessionState::Working);
        assert!(s.started_at.is_some());
    }

    #[test]
    fn working_to_paused_and_resume() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        let r = apply(&mut s, SessionState::Paused);
        assert!(matches!(r, TransitionResult::Changed { .. }));
        let r2 = apply(&mut s, SessionState::Working);
        assert!(matches!(r2, TransitionResult::Changed { .. }));
    }

    #[test]
    fn working_to_waiting_to_working() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        apply(&mut s, SessionState::WaitingForHuman);
        let r = apply(&mut s, SessionState::Working);
        assert!(matches!(r, TransitionResult::Changed { .. }));
    }

    #[test]
    fn working_to_stopped() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        apply(&mut s, SessionState::Stopping);
        let r = apply(&mut s, SessionState::Stopped);
        assert!(matches!(r, TransitionResult::Changed { .. }));
        assert!(s.state.is_terminal());
        assert!(s.stopped_at.is_some());
    }

    #[test]
    fn crash_and_restart_cycle() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        apply(&mut s, SessionState::Crashed);
        assert_eq!(s.state, SessionState::Crashed);

        apply(&mut s, SessionState::Restarting);
        assert_eq!(s.restart_count, 1);

        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        assert_eq!(s.state, SessionState::Working);
    }

    #[test]
    fn crash_exceeds_limit_goes_to_failed() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        apply(&mut s, SessionState::Crashed);
        let r = apply(&mut s, SessionState::Failed);
        assert!(matches!(r, TransitionResult::Changed { .. }));
        assert!(s.state.is_terminal());
    }

    #[test]
    fn rate_limited_then_working() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        apply(&mut s, SessionState::RateLimited);
        let r = apply(&mut s, SessionState::Working);
        assert!(matches!(r, TransitionResult::Changed { .. }));
    }

    // ── Invalid transitions ─────────────────────────────────────────────────

    #[test]
    fn idle_cannot_go_to_working_directly() {
        let mut s = session();
        let r = apply(&mut s, SessionState::Working);
        assert!(matches!(r, TransitionResult::Rejected { .. }));
        assert_eq!(s.state, SessionState::Idle); // state unchanged
    }

    #[test]
    fn stopped_is_terminal() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        apply(&mut s, SessionState::Stopping);
        apply(&mut s, SessionState::Stopped);

        // Cannot transition out of a terminal state
        let r = apply(&mut s, SessionState::Working);
        assert!(matches!(r, TransitionResult::Rejected { .. }));
        assert_eq!(s.state, SessionState::Stopped);
    }

    #[test]
    fn failed_is_terminal() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Failed);
        let r = apply(&mut s, SessionState::Starting);
        assert!(matches!(r, TransitionResult::Rejected { .. }));
    }

    #[test]
    fn paused_cannot_go_to_waiting() {
        let mut s = session();
        apply(&mut s, SessionState::Starting);
        apply(&mut s, SessionState::Working);
        apply(&mut s, SessionState::Paused);
        let r = apply(&mut s, SessionState::WaitingForHuman);
        assert!(matches!(r, TransitionResult::Rejected { .. }));
    }

    #[test]
    fn all_valid_transitions_pass() {
        // Exhaustively check that every documented valid transition passes.
        use SessionState::*;
        let valid_edges = vec![
            (Idle, Starting),
            (Idle, Stopped),  // direct stop of unstarted session
            (Starting, Working),
            (Starting, Failed),
            (Working, WaitingForHuman),
            (WaitingForHuman, Working),
            (Working, Paused),
            (WaitingForHuman, Paused),
            (Paused, Working),
            (Working, RateLimited),
            (WaitingForHuman, RateLimited),
            (RateLimited, Working),
            (Working, Stopping),
            (WaitingForHuman, Stopping),
            (Paused, Stopping),
            (RateLimited, Stopping),
            (Starting, Stopping),
            (Stopping, Stopped),
            (Stopping, HandedOff),
            (Working, Crashed),
            (WaitingForHuman, Crashed),
            (Paused, Crashed),
            (Starting, Crashed),
            (Crashed, Restarting),
            (Restarting, Starting),
            (Crashed, Failed),
        ];
        for (from, to) in valid_edges {
            assert!(
                is_valid_transition(&from, &to),
                "Expected {from:?} → {to:?} to be valid"
            );
        }
    }
}
