//! FIX -> domain normalization (and domain -> FIX for orders).

use std::collections::HashMap;

use domain::{now_ns, Execution, Fixed, Level, Order, Quote};
use fix_codec::{
    now_timestamp, ExecutionReport, InstrumentRef, MarketDataIncremental, MarketDataSnapshot,
    MdEntryType, MdUpdateAction, NewOrderSingle,
};

use crate::config::GatewayConfig;

/// Per-instrument book built from W (full refresh) and X (incremental) messages.
#[derive(Default)]
pub struct Books {
    books: HashMap<String, (Vec<Level>, Vec<Level>)>,
}

fn sort(bids: &mut [Level], asks: &mut [Level]) {
    bids.sort_by_key(|l| std::cmp::Reverse(l.price));
    asks.sort_by_key(|l| l.price);
}

impl Books {
    pub fn snapshot(&mut self, cfg: &GatewayConfig, w: &MarketDataSnapshot) -> Option<Quote> {
        let instr = cfg.instrument_by_security_id(&w.instrument.security_id)?;
        let mut bids = Vec::new();
        let mut asks = Vec::new();
        for e in &w.entries {
            let l = Level {
                price: e.price,
                qty: e.size,
            };
            match e.entry_type {
                MdEntryType::Bid => bids.push(l),
                MdEntryType::Offer => asks.push(l),
                MdEntryType::Trade => {}
            }
        }
        sort(&mut bids, &mut asks);
        self.books
            .insert(instr.symbol.clone(), (bids.clone(), asks.clone()));
        Some(Quote {
            lp: cfg.lp.clone(),
            symbol: instr.symbol.clone(),
            bids,
            asks,
            ts_recv_ns: now_ns(),
        })
    }

    /// Applies an incremental refresh; returns one quote per touched instrument.
    /// Entries without SecurityID inherit the previous entry's instrument (FIX rule).
    pub fn incremental(&mut self, cfg: &GatewayConfig, x: &MarketDataIncremental) -> Vec<Quote> {
        let mut touched: Vec<String> = Vec::new();
        let mut current: Option<String> = None;
        for e in &x.entries {
            if let Some(i) = &e.instrument {
                current = cfg
                    .instrument_by_security_id(&i.security_id)
                    .map(|i| i.symbol.clone());
            }
            let Some(sym) = current.clone() else { continue };
            let (bids, asks) = self.books.entry(sym.clone()).or_default();
            let side = match e.entry_type {
                MdEntryType::Bid => bids,
                MdEntryType::Offer => asks,
                MdEntryType::Trade => continue,
            };
            let Some(price) = e.price else { continue };
            side.retain(|l| l.price != price);
            if e.action != MdUpdateAction::Delete {
                side.push(Level {
                    price,
                    qty: e.size.unwrap_or(Fixed::ZERO),
                });
            }
            if !touched.contains(&sym) {
                touched.push(sym);
            }
        }
        touched
            .into_iter()
            .filter_map(|sym| {
                let (bids, asks) = self.books.get_mut(&sym)?;
                sort(bids, asks);
                Some(Quote {
                    lp: cfg.lp.clone(),
                    symbol: sym,
                    bids: bids.clone(),
                    asks: asks.clone(),
                    ts_recv_ns: now_ns(),
                })
            })
            .collect()
    }
}

pub fn execution(cfg: &GatewayConfig, er: &ExecutionReport) -> Execution {
    let symbol = cfg
        .instrument_by_security_id(&er.instrument.security_id)
        .map(|i| i.symbol.clone())
        .unwrap_or_else(|| format!("SECID:{}", er.instrument.security_id));
    Execution {
        lp: cfg.lp.clone(),
        symbol,
        order_id: er.order_id.clone(),
        cl_ord_id: er.cl_ord_id.clone(),
        orig_cl_ord_id: er.orig_cl_ord_id.clone(),
        exec_id: er.exec_id.clone(),
        exec_type: er.exec_type,
        status: er.ord_status,
        side: er.side,
        last_qty: er.last_qty,
        last_px: er.last_px,
        cum_qty: er.cum_qty,
        leaves_qty: er.leaves_qty,
        avg_px: er.avg_px,
        text: er.text.clone(),
        ts_recv_ns: now_ns(),
    }
}

pub fn instrument_ref(cfg: &GatewayConfig, symbol: &str) -> Option<InstrumentRef> {
    cfg.instrument_by_symbol(symbol).map(|i| InstrumentRef {
        security_id: i.security_id.clone(),
        security_id_source: cfg.security_id_source.clone(),
    })
}

pub fn new_order_single(cfg: &GatewayConfig, o: &Order) -> Option<NewOrderSingle> {
    Some(NewOrderSingle {
        cl_ord_id: o.cl_ord_id.clone(),
        instrument: instrument_ref(cfg, &o.symbol)?,
        side: o.side,
        transact_time: now_timestamp(),
        order_qty: o.qty,
        ord_type: o.ord_type,
        price: o.limit_price,
        time_in_force: Some(o.tif),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fix_codec::{MdEntry, MdIncEntry};

    fn cfg() -> GatewayConfig {
        GatewayConfig::from_toml(include_str!("../config/default.toml")).unwrap()
    }

    fn px(v: i64) -> Fixed {
        Fixed::from_parts(v, 5)
    }

    #[test]
    fn snapshot_then_incremental() {
        let cfg = cfg();
        let mut b = Books::default();
        let eur = InstrumentRef {
            security_id: "4001".into(),
            security_id_source: "8".into(),
        };
        let q = b
            .snapshot(
                &cfg,
                &MarketDataSnapshot {
                    md_req_id: None,
                    instrument: eur.clone(),
                    entries: vec![
                        MdEntry {
                            entry_type: MdEntryType::Offer,
                            price: px(108_503),
                            size: Fixed::from_int(2),
                        },
                        MdEntry {
                            entry_type: MdEntryType::Bid,
                            price: px(108_499),
                            size: Fixed::from_int(1),
                        },
                        MdEntry {
                            entry_type: MdEntryType::Offer,
                            price: px(108_501),
                            size: Fixed::from_int(1),
                        },
                        MdEntry {
                            entry_type: MdEntryType::Bid,
                            price: px(108_500),
                            size: Fixed::from_int(1),
                        },
                    ],
                },
            )
            .unwrap();
        assert_eq!(q.symbol, "EUR/USD");
        assert_eq!(q.best_bid().unwrap().price, px(108_500));
        assert_eq!(q.best_ask().unwrap().price, px(108_501));
        let qs = b.incremental(
            &cfg,
            &MarketDataIncremental {
                md_req_id: None,
                entries: vec![
                    MdIncEntry {
                        action: MdUpdateAction::Delete,
                        entry_type: MdEntryType::Offer,
                        instrument: Some(eur),
                        price: Some(px(108_501)),
                        size: None,
                    },
                    MdIncEntry {
                        action: MdUpdateAction::New,
                        entry_type: MdEntryType::Bid,
                        instrument: None,
                        price: Some(px(108_502)),
                        size: Some(Fixed::from_int(5)),
                    },
                ],
            },
        );
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].best_ask().unwrap().price, px(108_503));
        assert_eq!(
            qs[0].best_bid().unwrap(),
            Level {
                price: px(108_502),
                qty: Fixed::from_int(5)
            }
        );
    }

    #[test]
    fn unknown_instruments_are_ignored() {
        let cfg = cfg();
        let mut b = Books::default();
        let w = MarketDataSnapshot {
            md_req_id: None,
            instrument: InstrumentRef {
                security_id: "9999".into(),
                security_id_source: "8".into(),
            },
            entries: vec![],
        };
        assert!(b.snapshot(&cfg, &w).is_none());
        assert!(instrument_ref(&cfg, "XAU/USD").is_none());
    }
}
