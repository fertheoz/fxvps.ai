//! Allocation of (partial) LP fills to client orders.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum AllocationMode {
    /// Proportional to remaining quantity; leftovers by largest remainder
    /// (ties: earlier order first).
    #[default]
    ProRata,
    /// First order filled completely before the next.
    Fifo,
}

/// Splits `fill` (raw qty units) over `(id, remaining)` children. The sum of
/// allocations equals `min(fill, Σ remaining)` and no child receives more
/// than its remaining quantity.
pub fn allocate<Id: Copy>(
    fill: i64,
    children: &[(Id, i64)],
    mode: AllocationMode,
) -> Vec<(Id, i64)> {
    let total: i128 = children.iter().map(|c| c.1.max(0) as i128).sum();
    let fill = (fill.max(0) as i128).min(total);
    if fill == 0 {
        return Vec::new();
    }
    let mut out: Vec<(Id, i64)> = Vec::with_capacity(children.len());
    match mode {
        AllocationMode::Fifo => {
            let mut left = fill;
            for &(id, rem) in children {
                let q = left.min(rem.max(0) as i128);
                if q > 0 {
                    out.push((id, q as i64));
                    left -= q;
                }
            }
        }
        AllocationMode::ProRata => {
            let mut base: Vec<(usize, i128, i128)> = children
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let n = fill * c.1.max(0) as i128;
                    (i, n / total, n % total)
                })
                .collect();
            let mut left = fill - base.iter().map(|b| b.1).sum::<i128>();
            let mut order: Vec<usize> = (0..base.len()).collect();
            order.sort_by(|&a, &b| base[b].2.cmp(&base[a].2).then(a.cmp(&b)));
            for i in order {
                if left == 0 {
                    break;
                }
                if base[i].1 < children[i].1 as i128 {
                    base[i].1 += 1;
                    left -= 1;
                }
            }
            for (i, q, _) in base {
                if q > 0 {
                    out.push((children[i].0, q as i64));
                }
            }
        }
    }
    out
}
