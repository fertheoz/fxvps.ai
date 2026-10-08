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
    /// Fill-or-kill instead of immediate-or-cancel.
    pub all_or_none: bool,
    /// Rests at the LP (GTC limit) instead of IOC; `revision` counts
    /// cancel/replaces (ClOrdID `LP-<id>` → `LP-<id>-r<n>`).
    #[serde(default)]
    pub resting: bool,
    #[serde(default)]
    pub revision: u32,
}

/// Outbound side of A-book routing. Fills come back as
/// `Command::LpFill` / `Command::LpReject` so they are journaled.
pub trait LpRouter: Send {
    fn send(&mut self, req: &LpOrderRequest);
    /// Cancel/replace of a resting order: `req.volume` is the new total
    /// quantity, `req.revision` the new revision.
    fn replace(&mut self, _req: &LpOrderRequest) {}
    /// Cancel of a resting order (`req` names the order, its symbol/side and
    /// the revision live at the LP — enough after a restart too).
    fn cancel(&mut self, _req: &LpOrderRequest) {}
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
    pub replaced: Arc<Mutex<Vec<LpOrderRequest>>>,
    pub cancelled: Arc<Mutex<Vec<LpOrderRequest>>>,
}

impl RecordingRouter {
    pub fn take(&self) -> Vec<LpOrderRequest> {
        std::mem::take(&mut *self.sent.lock().unwrap_or_else(|e| e.into_inner()))
    }
    pub fn take_replaced(&self) -> Vec<LpOrderRequest> {
        std::mem::take(&mut *self.replaced.lock().unwrap_or_else(|e| e.into_inner()))
    }
    pub fn take_cancelled(&self) -> Vec<(LpOrderId, u32)> {
        self.take_cancelled_reqs()
            .into_iter()
            .map(|r| (r.lp_order_id, r.revision))
            .collect()
    }
    pub fn take_cancelled_reqs(&self) -> Vec<LpOrderRequest> {
        std::mem::take(&mut *self.cancelled.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl LpRouter for RecordingRouter {
    fn send(&mut self, req: &LpOrderRequest) {
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
    }
    fn replace(&mut self, req: &LpOrderRequest) {
        self.replaced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
    }
    fn cancel(&mut self, req: &LpOrderRequest) {
        self.cancelled
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.clone());
    }
}
