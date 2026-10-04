//! Abstraction over the trading gateway towards liquidity providers.

use crate::types::LpOrderId;
use money::{Price, Qty};
use risk::Side;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Order sent from the omnibus account to the LP.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub struct LpOrderRequest {
    pub lp_order_id: LpOrderId,
    pub symbol: String,
    pub side: Side,
    pub volume: Qty,
    pub limit: Option<Price>,
}

/// Outbound side of A-book routing. Fills come back as
/// `Command::LpFill` / `Command::LpReject` so they are journaled.
pub trait LpRouter: Send {
    fn send(&mut self, req: &LpOrderRequest);
}

/// Drops everything (used during replay: LP orders were already sent).
#[derive(Default, Debug)]
pub struct NullRouter;

impl LpRouter for NullRouter {
    fn send(&mut self, _req: &LpOrderRequest) {}
}

/// Records requests; handy for tests and simulators.
#[derive(Default, Clone, Debug)]
pub struct RecordingRouter {
    pub sent: Arc<Mutex<Vec<LpOrderRequest>>>,
}

impl RecordingRouter {
    pub fn take(&self) -> Vec<LpOrderRequest> {
        std::mem::take(&mut *self.sent.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl LpRouter for RecordingRouter {
    fn send(&mut self, req: &LpOrderRequest) {
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
    }
}
