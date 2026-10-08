//! Order handling: IOC / FOK / market / limit fills against the simulated book,
//! partial fills, rejects, cancel and cancel/replace of resting limit orders.
//!
//! Simplifications (documented gaps): resting limit orders are not re-matched when
//! the market moves, and fills do not consume book liquidity.

use std::collections::{HashMap, HashSet};

use domain::{ExecType, Fixed, OrderStatus, OrderType, Price, Qty, Side, TimeInForce};
use fix_codec::{
    now_timestamp, Body, CxlRejResponseTo, ExecutionReport, InstrumentRef, NewOrderSingle,
    OrderCancelReject, OrderCancelReplaceRequest, OrderCancelRequest,
};

use crate::market::Book;

/// OrdRejReason(103) values used by the simulator.
pub mod rej {
    pub const UNKNOWN_SYMBOL: u64 = 1;
    pub const DUPLICATE_ORDER: u64 = 6;
    pub const INCORRECT_QTY: u64 = 13;
    pub const OTHER: u64 = 99;
}

#[derive(Clone, Debug)]
struct Resting {
    order_id: String,
    instrument: InstrumentRef,
    side: Side,
    qty: Qty,
    price: Price,
    cum: Qty,
    notional: Fixed,
    tif: TimeInForce,
}

#[derive(Default)]
pub struct Engine {
    next_order: u64,
    next_exec: u64,
    seen_cl_ord_ids: HashSet<String>,
    resting: HashMap<String, Resting>,
}

struct Fill {
    qty: Qty,
    px: Price,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    fn exec_id(&mut self) -> String {
        self.next_exec += 1;
        format!("E{}", self.next_exec)
    }

    fn order_id(&mut self) -> String {
        self.next_order += 1;
        format!("SIM{}", self.next_order)
    }

    #[allow(clippy::too_many_arguments)]
    fn er(
        &mut self,
        order_id: &str,
        nos: &NewOrderSingle,
        exec_type: ExecType,
        status: OrderStatus,
        last: Option<&Fill>,
        cum: Qty,
        notional: Fixed,
        text: Option<String>,
    ) -> ExecutionReport {
        ExecutionReport {
            order_id: order_id.to_owned(),
            cl_ord_id: Some(nos.cl_ord_id.clone()),
            orig_cl_ord_id: None,
            exec_id: self.exec_id(),
            exec_type,
            ord_status: status,
            instrument: nos.instrument.clone(),
            side: nos.side,
            order_qty: Some(nos.order_qty),
            ord_type: Some(nos.ord_type),
            price: nos.price,
            time_in_force: nos.time_in_force,
            last_qty: last.map(|f| f.qty),
            last_px: last.map(|f| f.px),
            leaves_qty: if status.is_terminal() {
                Fixed::ZERO
            } else {
                nos.order_qty - cum
            },
            cum_qty: cum,
            avg_px: if cum.is_positive() {
                notional.checked_div(cum)
            } else {
                None
            },
            ord_rej_reason: None,
            text,
            transact_time: Some(now_timestamp()),
        }
    }

    pub fn reject(&mut self, nos: &NewOrderSingle, reason: u64, text: &str) -> Vec<Body> {
        let mut er = self.er(
            "NONE",
            nos,
            ExecType::Rejected,
            OrderStatus::Rejected,
            None,
            Fixed::ZERO,
            Fixed::ZERO,
            Some(text.into()),
        );
        er.ord_rej_reason = Some(reason);
        vec![Body::ExecutionReport(er)]
    }

    /// Handles NewOrderSingle. `book` is `None` for unknown instruments.
    pub fn new_order(&mut self, nos: &NewOrderSingle, book: Option<&Book>) -> Vec<Body> {
        let Some(book) = book else {
            return self.reject(nos, rej::UNKNOWN_SYMBOL, "unknown instrument");
        };
        if !self.seen_cl_ord_ids.insert(nos.cl_ord_id.clone()) {
            return self.reject(nos, rej::DUPLICATE_ORDER, "duplicate ClOrdID");
        }
        if !nos.order_qty.is_positive() {
            return self.reject(nos, rej::INCORRECT_QTY, "OrderQty must be positive");
        }
        let tif = match (nos.ord_type, nos.time_in_force) {
            // Assumption (docs/02): takers use IOC/FOK; market defaults to IOC.
            (OrderType::Market, None | Some(TimeInForce::ImmediateOrCancel)) => {
                TimeInForce::ImmediateOrCancel
            }
            (OrderType::Market, Some(TimeInForce::FillOrKill)) => TimeInForce::FillOrKill,
            (OrderType::Market, Some(_)) => {
                return self.reject(nos, rej::OTHER, "market orders must be IOC or FOK");
            }
            (OrderType::Limit, tif) => tif.unwrap_or(TimeInForce::Day),
            (OrderType::Stop, _) => {
                return self.reject(nos, rej::OTHER, "stop orders are not supported here");
            }
        };
        let limit = match (nos.ord_type, nos.price) {
            (OrderType::Limit, Some(p)) if p.is_positive() => Some(p),
            (OrderType::Limit, _) => {
                return self.reject(nos, rej::OTHER, "limit order requires positive Price");
            }
            (OrderType::Market | OrderType::Stop, _) => None,
        };

        let marketable: Vec<_> = book
            .opposite(nos.side)
            .iter()
            .filter(|l| match (limit, nos.side) {
                (None, _) => true,
                (Some(p), Side::Buy) => l.price <= p,
                (Some(p), Side::Sell) => l.price >= p,
            })
            .copied()
            .collect();
        let available = marketable.iter().fold(Fixed::ZERO, |a, l| a + l.qty);

        let order_id = self.order_id();
        let mut out = Vec::new();
        if tif == TimeInForce::FillOrKill && available < nos.order_qty {
            out.push(Body::ExecutionReport(self.er(
                &order_id,
                nos,
                ExecType::Canceled,
                OrderStatus::Canceled,
                None,
                Fixed::ZERO,
                Fixed::ZERO,
                Some("FOK: insufficient liquidity".into()),
            )));
            return out;
        }
        out.push(Body::ExecutionReport(self.er(
            &order_id,
            nos,
            ExecType::New,
            OrderStatus::New,
            None,
            Fixed::ZERO,
            Fixed::ZERO,
            None,
        )));

        let mut cum = Fixed::ZERO;
        let mut notional = Fixed::ZERO;
        for level in marketable {
            let remaining = nos.order_qty - cum;
            if !remaining.is_positive() {
                break;
            }
            let qty = remaining.min(level.qty);
            cum = cum + qty;
            notional = notional + qty.checked_mul(level.price).unwrap_or(Fixed::ZERO);
            let status = if cum == nos.order_qty {
                OrderStatus::Filled
            } else {
                OrderStatus::PartiallyFilled
            };
            let fill = Fill {
                qty,
                px: level.price,
            };
            out.push(Body::ExecutionReport(self.er(
                &order_id,
                nos,
                ExecType::Trade,
                status,
                Some(&fill),
                cum,
                notional,
                None,
            )));
        }

        if cum < nos.order_qty {
            match tif {
                TimeInForce::ImmediateOrCancel | TimeInForce::FillOrKill => {
                    out.push(Body::ExecutionReport(self.er(
                        &order_id,
                        nos,
                        ExecType::Canceled,
                        OrderStatus::Canceled,
                        None,
                        cum,
                        notional,
                        Some("IOC remainder canceled".into()),
                    )));
                }
                TimeInForce::Day | TimeInForce::GoodTillCancel => {
                    self.resting.insert(
                        nos.cl_ord_id.clone(),
                        Resting {
                            order_id,
                            instrument: nos.instrument.clone(),
                            side: nos.side,
                            qty: nos.order_qty,
                            price: limit.unwrap_or(Fixed::ZERO),
                            cum,
                            notional,
                            tif,
                        },
                    );
                }
            }
        }
        out
    }

    fn cancel_reject(&self, cl: &str, orig: &str, to: CxlRejResponseTo, text: &str) -> Body {
        Body::OrderCancelReject(OrderCancelReject {
            order_id: "NONE".into(),
            cl_ord_id: cl.into(),
            orig_cl_ord_id: orig.into(),
            ord_status: OrderStatus::Rejected,
            response_to: to,
            reason: Some(1),
            text: Some(text.into()),
        })
    }

    fn resting_er(
        &mut self,
        r: &Resting,
        cl: &str,
        orig: &str,
        et: ExecType,
        st: OrderStatus,
    ) -> Body {
        Body::ExecutionReport(ExecutionReport {
            order_id: r.order_id.clone(),
            cl_ord_id: Some(cl.into()),
            orig_cl_ord_id: Some(orig.into()),
            exec_id: self.exec_id(),
            exec_type: et,
            ord_status: st,
            instrument: r.instrument.clone(),
            side: r.side,
            order_qty: Some(r.qty),
            ord_type: Some(OrderType::Limit),
            price: Some(r.price),
            time_in_force: Some(r.tif),
            last_qty: None,
            last_px: None,
            leaves_qty: if st.is_terminal() {
                Fixed::ZERO
            } else {
                r.qty - r.cum
            },
            cum_qty: r.cum,
            avg_px: if r.cum.is_positive() {
                r.notional.checked_div(r.cum)
            } else {
                None
            },
            ord_rej_reason: None,
            text: None,
            transact_time: Some(now_timestamp()),
        })
    }

    pub fn cancel(&mut self, req: &OrderCancelRequest) -> Body {
        match self.resting.remove(&req.orig_cl_ord_id) {
            Some(r) => self.resting_er(
                &r,
                &req.cl_ord_id,
                &req.orig_cl_ord_id,
                ExecType::Canceled,
                OrderStatus::Canceled,
            ),
            None => self.cancel_reject(
                &req.cl_ord_id,
                &req.orig_cl_ord_id,
                CxlRejResponseTo::Cancel,
                "unknown order",
            ),
        }
    }

    pub fn replace(&mut self, req: &OrderCancelReplaceRequest) -> Body {
        let valid = req.ord_type == OrderType::Limit
            && req.price.is_some_and(Fixed::is_positive)
            && req.order_qty.is_positive();
        let Some(mut r) = self.resting.remove(&req.orig_cl_ord_id) else {
            return self.cancel_reject(
                &req.cl_ord_id,
                &req.orig_cl_ord_id,
                CxlRejResponseTo::Replace,
                "unknown order",
            );
        };
        if !valid || req.order_qty <= r.cum {
            let body = self.cancel_reject(
                &req.cl_ord_id,
                &req.orig_cl_ord_id,
                CxlRejResponseTo::Replace,
                "invalid replace",
            );
            self.resting.insert(req.orig_cl_ord_id.clone(), r);
            return body;
        }
        r.qty = req.order_qty;
        r.price = req.price.unwrap_or(r.price);
        let st = if r.cum.is_positive() {
            OrderStatus::PartiallyFilled
        } else {
            OrderStatus::New
        };
        let body = self.resting_er(
            &r,
            &req.cl_ord_id,
            &req.orig_cl_ord_id,
            ExecType::Replaced,
            st,
        );
        self.seen_cl_ord_ids.insert(req.cl_ord_id.clone());
        self.resting.insert(req.cl_ord_id.clone(), r);
        body
    }

    pub fn resting_count(&self) -> usize {
        self.resting.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Level;

    fn px(v: i64) -> Fixed {
        Fixed::from_parts(v, 5)
    }

    fn book() -> Book {
        let m = Fixed::from_int(1_000_000);
        Book {
            bids: vec![
                Level {
                    price: px(108_499),
                    qty: m,
                },
                Level {
                    price: px(108_498),
                    qty: m + m,
                },
            ],
            asks: vec![
                Level {
                    price: px(108_501),
                    qty: m,
                },
                Level {
                    price: px(108_502),
                    qty: m + m,
                },
            ],
        }
    }

    fn nos(
        id: &str,
        side: Side,
        qty: i64,
        ot: OrderType,
        price: Option<Fixed>,
        tif: Option<TimeInForce>,
    ) -> NewOrderSingle {
        NewOrderSingle {
            cl_ord_id: id.into(),
            instrument: InstrumentRef {
                security_id: "4001".into(),
                security_id_source: "8".into(),
            },
            side,
            transact_time: now_timestamp(),
            order_qty: Fixed::from_int(qty),
            ord_type: ot,
            price,
            stop_px: None,
            time_in_force: tif,
        }
    }

    fn ers(bodies: Vec<Body>) -> Vec<ExecutionReport> {
        bodies
            .into_iter()
            .map(|b| match b {
                Body::ExecutionReport(e) => e,
                other => panic!("expected ER, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn market_order_fills_at_best() {
        let mut e = Engine::new();
        let r = ers(e.new_order(
            &nos("a", Side::Buy, 100_000, OrderType::Market, None, None),
            Some(&book()),
        ));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].exec_type, ExecType::New);
        assert_eq!(r[1].exec_type, ExecType::Trade);
        assert_eq!(r[1].ord_status, OrderStatus::Filled);
        assert_eq!(r[1].last_px, Some(px(108_501)));
        assert_eq!(r[1].avg_px, Some(px(108_501)));
        assert_eq!(r[1].leaves_qty, Fixed::ZERO);
    }

    #[test]
    fn market_sweeps_levels_with_partial_fills() {
        let mut e = Engine::new();
        let r = ers(e.new_order(
            &nos("a", Side::Sell, 2_500_000, OrderType::Market, None, None),
            Some(&book()),
        ));
        let trades: Vec<_> = r
            .iter()
            .filter(|x| x.exec_type == ExecType::Trade)
            .collect();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].ord_status, OrderStatus::PartiallyFilled);
        assert_eq!(trades[0].last_qty, Some(Fixed::from_int(1_000_000)));
        assert_eq!(trades[1].ord_status, OrderStatus::Filled);
        assert_eq!(trades[1].last_qty, Some(Fixed::from_int(1_500_000)));
        // avg = (1e6*1.08499 + 1.5e6*1.08498) / 2.5e6 = 1.084984
        assert_eq!(trades[1].avg_px, Some(Fixed::from_parts(1_084_984, 6)));
    }

    #[test]
    fn ioc_remainder_canceled_and_fok_killed() {
        let mut e = Engine::new();
        let r = ers(e.new_order(
            &nos("a", Side::Buy, 5_000_000, OrderType::Market, None, None),
            Some(&book()),
        ));
        let last = r.last().unwrap();
        assert_eq!(last.exec_type, ExecType::Canceled);
        assert_eq!(last.cum_qty, Fixed::from_int(3_000_000));
        let r = ers(e.new_order(
            &nos(
                "b",
                Side::Buy,
                5_000_000,
                OrderType::Market,
                None,
                Some(TimeInForce::FillOrKill),
            ),
            Some(&book()),
        ));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].ord_status, OrderStatus::Canceled);
        assert_eq!(r[0].cum_qty, Fixed::ZERO);
        let r = ers(e.new_order(
            &nos(
                "c",
                Side::Buy,
                3_000_000,
                OrderType::Market,
                None,
                Some(TimeInForce::FillOrKill),
            ),
            Some(&book()),
        ));
        assert_eq!(r.last().unwrap().ord_status, OrderStatus::Filled);
    }

    #[test]
    fn limit_ioc_respects_price() {
        let mut e = Engine::new();
        let r = ers(e.new_order(
            &nos(
                "a",
                Side::Buy,
                2_000_000,
                OrderType::Limit,
                Some(px(108_501)),
                Some(TimeInForce::ImmediateOrCancel),
            ),
            Some(&book()),
        ));
        let trades: Vec<_> = r
            .iter()
            .filter(|x| x.exec_type == ExecType::Trade)
            .collect();
        assert_eq!(trades.len(), 1);
        assert_eq!(r.last().unwrap().exec_type, ExecType::Canceled);
    }

    #[test]
    fn day_limit_rests_then_cancel_and_replace() {
        let mut e = Engine::new();
        let r = ers(e.new_order(
            &nos(
                "a",
                Side::Buy,
                1_000,
                OrderType::Limit,
                Some(px(108_000)),
                None,
            ),
            Some(&book()),
        ));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].ord_status, OrderStatus::New);
        assert_eq!(e.resting_count(), 1);
        let rep = e.replace(&OrderCancelReplaceRequest {
            orig_cl_ord_id: "a".into(),
            cl_ord_id: "a2".into(),
            instrument: r[0].instrument.clone(),
            side: Side::Buy,
            transact_time: now_timestamp(),
            order_qty: Fixed::from_int(2_000),
            ord_type: OrderType::Limit,
            price: Some(px(108_100)),
            stop_px: None,
            time_in_force: None,
        });
        assert!(
            matches!(rep, Body::ExecutionReport(ref x) if x.exec_type == ExecType::Replaced && x.price == Some(px(108_100)))
        );
        let cxl = |orig: &str| OrderCancelRequest {
            orig_cl_ord_id: orig.into(),
            cl_ord_id: "c1".into(),
            instrument: r[0].instrument.clone(),
            side: Side::Buy,
            transact_time: now_timestamp(),
            order_qty: None,
        };
        assert!(matches!(e.cancel(&cxl("a")), Body::OrderCancelReject(_)));
        assert!(
            matches!(e.cancel(&cxl("a2")), Body::ExecutionReport(ref x) if x.ord_status == OrderStatus::Canceled)
        );
        assert_eq!(e.resting_count(), 0);
    }

    #[test]
    fn rejects() {
        let mut e = Engine::new();
        let r = ers(e.new_order(&nos("a", Side::Buy, 1, OrderType::Market, None, None), None));
        assert_eq!(r[0].ord_rej_reason, Some(rej::UNKNOWN_SYMBOL));
        let r = ers(e.new_order(
            &nos("b", Side::Buy, 0, OrderType::Market, None, None),
            Some(&book()),
        ));
        assert_eq!(r[0].ord_rej_reason, Some(rej::INCORRECT_QTY));
        let r = ers(e.new_order(
            &nos("c", Side::Buy, 1, OrderType::Limit, None, None),
            Some(&book()),
        ));
        assert_eq!(r[0].exec_type, ExecType::Rejected);
        e.new_order(
            &nos("d", Side::Buy, 1, OrderType::Market, None, None),
            Some(&book()),
        );
        let r = ers(e.new_order(
            &nos("d", Side::Buy, 1, OrderType::Market, None, None),
            Some(&book()),
        ));
        assert_eq!(r[0].ord_rej_reason, Some(rej::DUPLICATE_ORDER));
    }
}
