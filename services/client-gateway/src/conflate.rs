//! Latest-wins quote conflation: between two flushes only the newest quote per
//! symbol is kept, so a client never receives more than `max_hz` batches per second
//! regardless of the upstream tick rate.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::hub::ClientQuote;

#[derive(Default)]
pub struct Conflator {
    pending: BTreeMap<Arc<str>, Arc<ClientQuote>>,
    /// Quotes overwritten before being flushed (metrics).
    pub conflated: u64,
}

impl Conflator {
    pub fn push(&mut self, q: Arc<ClientQuote>) {
        if self.pending.insert(q.symbol.clone(), q).is_some() {
            self.conflated += 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Drops pending quotes for a symbol (on unsubscribe).
    pub fn remove(&mut self, symbol: &str) {
        self.pending.remove(symbol);
    }

    /// Takes all pending quotes (one per symbol, symbol order).
    pub fn take(&mut self) -> Vec<Arc<ClientQuote>> {
        std::mem::take(&mut self.pending).into_values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Fixed;

    fn q(sym: &str, bid: i64) -> Arc<ClientQuote> {
        Arc::new(ClientQuote {
            symbol: sym.into(),
            bid: Some(Fixed::from_raw(bid)),
            ask: None,
            bid_size: None,
            ask_size: None,
            ts_ns: bid as u64,
        })
    }

    #[test]
    fn latest_wins_per_symbol() {
        let mut c = Conflator::default();
        for i in 0..100 {
            c.push(q("EURUSD", i));
        }
        c.push(q("GBPUSD", 7));
        assert_eq!(c.len(), 2);
        assert_eq!(c.conflated, 99);
        let out = c.take();
        assert_eq!(out.len(), 2);
        assert_eq!(&*out[0].symbol, "EURUSD");
        assert_eq!(out[0].bid, Some(Fixed::from_raw(99)));
        assert!(c.is_empty());
        c.push(q("EURUSD", 1));
        c.remove("EURUSD");
        assert!(c.is_empty());
    }
}
