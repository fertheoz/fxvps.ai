//! Random-walk order books.

use std::collections::HashMap;

use domain::{Fixed, Level, Price, Side};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::config::{SimConfig, SimInstrument};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Book {
    /// Best first.
    pub bids: Vec<Level>,
    /// Best first.
    pub asks: Vec<Level>,
}

impl Book {
    /// Levels a taker on `side` would trade against.
    pub fn opposite(&self, side: Side) -> &[Level] {
        match side {
            Side::Buy => &self.asks,
            Side::Sell => &self.bids,
        }
    }
}

struct InstrState {
    spec: SimInstrument,
    mid_ticks: i64,
}

pub struct Market {
    depth: usize,
    rng: SmallRng,
    instruments: Vec<InstrState>,
    books: HashMap<String, Book>,
}

impl Market {
    pub fn new(cfg: &SimConfig) -> Self {
        let rng = match cfg.seed {
            Some(s) => SmallRng::seed_from_u64(s),
            None => SmallRng::from_entropy(),
        };
        let instruments = cfg
            .instruments
            .iter()
            .map(|i| InstrState {
                mid_ticks: i.initial_mid.raw() / i.tick_size.raw(),
                spec: i.clone(),
            })
            .collect();
        let mut m = Market {
            depth: cfg.depth,
            rng,
            instruments,
            books: HashMap::new(),
        };
        m.rebuild();
        m
    }

    fn rebuild(&mut self) {
        for st in &self.instruments {
            let tick = st.spec.tick_size;
            let half_lo = st.spec.spread_ticks / 2;
            let half_hi = st.spec.spread_ticks - half_lo;
            let px = |ticks: i64| Fixed::from_raw(ticks * tick.raw());
            let size = |i: usize| Fixed::from_raw(st.spec.level_size.raw() * (i as i64 + 1));
            let bids = (0..self.depth)
                .map(|i| Level {
                    price: px(st.mid_ticks - half_lo - i as i64),
                    qty: size(i),
                })
                .collect();
            let asks = (0..self.depth)
                .map(|i| Level {
                    price: px(st.mid_ticks + half_hi + i as i64),
                    qty: size(i),
                })
                .collect();
            self.books
                .insert(st.spec.security_id.clone(), Book { bids, asks });
        }
    }

    /// Advances every instrument one random-walk step.
    pub fn step(&mut self) {
        for st in &mut self.instruments {
            let v = st.spec.volatility_ticks.max(0);
            let d = self.rng.gen_range(-v..=v);
            let floor = st.spec.spread_ticks + self.depth as i64 + 1;
            let dev = st.mid_ticks - st.spec.initial_mid.raw() / st.spec.tick_size.raw();
            let pull = -dev.signum() * dev.abs().min(st.spec.revert_ticks.max(0));
            st.mid_ticks = (st.mid_ticks + d + pull).max(floor);
        }
        self.rebuild();
    }

    /// Control hook (tests, demos): moves an instrument's mid to `mid`
    /// immediately, e.g. to simulate a price shock. `symbol` matches the
    /// instrument symbol or security id. Returns false if unknown.
    pub fn set_mid(&mut self, symbol: &str, mid: Price) -> bool {
        let Some(st) = self
            .instruments
            .iter_mut()
            .find(|i| i.spec.symbol == symbol || i.spec.security_id == symbol)
        else {
            return false;
        };
        let floor = st.spec.spread_ticks + self.depth as i64 + 1;
        st.mid_ticks = (mid.raw() / st.spec.tick_size.raw()).max(floor);
        self.rebuild();
        true
    }

    /// Current mid of an instrument (symbol or security id).
    pub fn mid(&self, symbol: &str) -> Option<Price> {
        self.instruments
            .iter()
            .find(|i| i.spec.symbol == symbol || i.spec.security_id == symbol)
            .map(|i| Fixed::from_raw(i.mid_ticks * i.spec.tick_size.raw()))
    }

    pub fn book(&self, security_id: &str) -> Option<&Book> {
        self.books.get(security_id)
    }

    pub fn security_ids(&self) -> impl Iterator<Item = &str> {
        self.instruments.iter().map(|i| i.spec.security_id.as_str())
    }

    pub fn tick_size(&self, security_id: &str) -> Option<Price> {
        self.instruments
            .iter()
            .find(|i| i.spec.security_id == security_id)
            .map(|i| i.spec.tick_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn books_are_well_formed_and_walk() {
        let cfg = SimConfig::for_tests();
        let mut m = Market::new(&cfg);
        let first = m.book("4001").unwrap().clone();
        assert_eq!(first.bids.len(), 3);
        assert!(first.bids[0].price < first.asks[0].price);
        assert!(first.bids.windows(2).all(|w| w[0].price > w[1].price));
        assert!(first.asks.windows(2).all(|w| w[0].price < w[1].price));
        let mut moved = false;
        for _ in 0..50 {
            m.step();
            let b = m.book("4001").unwrap();
            assert!(b.bids[0].price < b.asks[0].price);
            moved |= *b != first;
        }
        assert!(moved);
    }

    #[test]
    fn set_mid_shocks_the_book() {
        let cfg = SimConfig::for_tests();
        let mut m = Market::new(&cfg);
        let target: Price = "1.00000".parse().unwrap();
        assert!(m.set_mid("4001", target));
        assert_eq!(m.mid("4001"), Some(target));
        let b = m.book("4001").unwrap();
        assert!(b.bids[0].price < target && b.asks[0].price > target);
        assert!(!m.set_mid("NOPE", target));
    }
}
