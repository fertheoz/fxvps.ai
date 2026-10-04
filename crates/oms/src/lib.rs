//! Order management engine: order lifecycle, positions (hedging and
//! netting), SL/TP/trailing stops, OCO groups, A-book routing through an
//! [`LpRouter`] with allocation of LP fills back to client sub-accounts, and
//! B-book internalization. All money movements go through [`ledger`].
//!
//! The [`Engine`] is a deterministic state machine: it never reads a clock
//! or random source; every input is an [`Envelope`] (sequence number,
//! timestamp, [`Command`]). Replaying the same envelopes yields the same
//! state ([`Engine::state_digest`]).

pub mod alloc;
pub mod engine;
pub mod router;
pub mod types;

pub use alloc::{allocate, AllocationMode};
pub use engine::{
    Engine, EngineConfig, EngineSnapshot, BANK, BROKER_BOOK, LP_COUNTERPARTY, OMNIBUS,
};
pub use router::{LpOrderRequest, LpRouter, NullRouter, RecordingRouter};
pub use types::*;

#[cfg(test)]
mod tests;
