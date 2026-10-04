//! Connection caps (finding G9): global, per client IP and per authenticated
//! subject. Slots are RAII guards released when the connection ends.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Counts {
    total: usize,
    per_ip: HashMap<IpAddr, usize>,
    per_sub: HashMap<String, usize>,
}

/// Shared connection counters.
#[derive(Clone)]
pub struct ConnLimits {
    max_total: usize,
    max_per_ip: usize,
    max_per_subject: usize,
    counts: Arc<Mutex<Counts>>,
}

/// Why a connection was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum LimitError {
    Total,
    Ip,
    Subject,
}

/// Holds a connection slot (and optionally a subject slot) until dropped.
pub struct Slot {
    limits: ConnLimits,
    ip: Option<IpAddr>,
    sub: Option<String>,
}

impl ConnLimits {
    /// `0` disables the corresponding cap.
    pub fn new(max_total: usize, max_per_ip: usize, max_per_subject: usize) -> Self {
        ConnLimits {
            max_total,
            max_per_ip,
            max_per_subject,
            counts: Arc::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reserves a connection slot for a peer (before the WebSocket upgrade).
    pub fn acquire(&self, ip: Option<IpAddr>) -> Result<Slot, LimitError> {
        let mut c = self.lock();
        if self.max_total > 0 && c.total >= self.max_total {
            return Err(LimitError::Total);
        }
        if let Some(ip) = ip {
            let n = c.per_ip.get(&ip).copied().unwrap_or(0);
            if self.max_per_ip > 0 && n >= self.max_per_ip {
                return Err(LimitError::Ip);
            }
            c.per_ip.insert(ip, n + 1);
        }
        c.total += 1;
        Ok(Slot {
            limits: self.clone(),
            ip,
            sub: None,
        })
    }

    pub fn total(&self) -> usize {
        self.lock().total
    }
}

impl Slot {
    /// Binds the slot to an authenticated subject (after `Auth`).
    pub fn bind_subject(&mut self, sub: &str) -> Result<(), LimitError> {
        if self.sub.is_some() {
            return Ok(());
        }
        let l = &self.limits;
        let mut c = l.lock();
        let n = c.per_sub.get(sub).copied().unwrap_or(0);
        if l.max_per_subject > 0 && n >= l.max_per_subject {
            return Err(LimitError::Subject);
        }
        c.per_sub.insert(sub.to_string(), n + 1);
        drop(c);
        self.sub = Some(sub.to_string());
        Ok(())
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut c = self.limits.lock();
        c.total = c.total.saturating_sub(1);
        if let Some(ip) = self.ip {
            if let Some(n) = c.per_ip.get_mut(&ip) {
                *n -= 1;
                if *n == 0 {
                    c.per_ip.remove(&ip);
                }
            }
        }
        if let Some(s) = self.sub.take() {
            if let Some(n) = c.per_sub.get_mut(&s) {
                *n -= 1;
                if *n == 0 {
                    c.per_sub.remove(&s);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_and_release() {
        let l = ConnLimits::new(3, 2, 1);
        let a: IpAddr = "10.0.0.1".parse().unwrap();
        let b: IpAddr = "10.0.0.2".parse().unwrap();
        let mut s1 = l.acquire(Some(a)).unwrap();
        let s2 = l.acquire(Some(a)).unwrap();
        assert_eq!(l.acquire(Some(a)).err(), Some(LimitError::Ip));
        let mut s3 = l.acquire(Some(b)).unwrap();
        assert_eq!(l.acquire(Some(b)).err(), Some(LimitError::Total));
        s1.bind_subject("u").unwrap();
        assert_eq!(s3.bind_subject("u"), Err(LimitError::Subject));
        drop(s1);
        assert_eq!(l.total(), 2);
        s3.bind_subject("u").unwrap();
        drop((s2, s3));
        assert_eq!(l.total(), 0);
        assert!(l.lock().per_ip.is_empty() && l.lock().per_sub.is_empty());
        // 0 = unlimited
        let u = ConnLimits::new(0, 0, 0);
        let v: Vec<_> = (0..50).map(|_| u.acquire(Some(a)).unwrap()).collect();
        assert_eq!(u.total(), 50);
        drop(v);
    }
}
