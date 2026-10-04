//! LMAX-like FIX 4.4 liquidity provider simulator.
//!
//! Everything LMAX-specific here (CompIDs, `SecurityIDSource=8`, IOC/FOK rules,
//! credentials in Logon) is an **assumption** until checked against the LMAX spec.

pub mod config;
pub mod engine;
pub mod market;
pub mod server;

pub use config::{EndpointConfig, SimConfig, SimInstrument};
pub use server::{start, SimHandle};
