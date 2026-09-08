//! The turn loop's terminal-state carrier — one enum instead of
//! interleaved booleans.
//!
//! The engine's stream loop sets exactly one signal when a terminal
//! condition is observed mid-loop (`Responded`, `DelegationYield`) or
//! checks it before each poll (`Stop` is a hard `Err` return; `TurnLimit`
//! terminates via the limit hook's error arm). The loop exits when the
//! signal is anything other than [`TurnSignal::Continue`].

/// Why the turn loop stopped iterating, or that it hasn't.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum TurnSignal {
    /// Keep polling the stream.
    #[default]
    Continue,
    /// A successful `respond` completed this execution.
    Responded,
    /// A sequential `delegate_to_agent` yielded to the child; no `Done`
    /// is emitted so the gateway does not report a final completion.
    DelegationYield,
}
