//! Realised volatility from the tick warehouse → engine VaR input.
//!
//! Every hour, for every symbol with tick files, the last 5 days of
//! one-minute mids give a realised daily σ; with enough samples it is
//! pushed to the engine as `Command::SetVolatility` (journaled, replay
//! safe, expires after two days so a stalled job falls back to the EWMA).
use std::time::Duration;

use super::AdminCtx;

pub const DAYS: usize = 5;
pub const MIN_SAMPLES: usize = 300;

/// One pass: returns (symbol, σ_daily, samples) for what was pushed.
pub async fn run_once(ctx: &AdminCtx) -> Vec<(String, f64, usize)> {
    let dir = ctx.data_dir.join("ticks");
    let mut out = Vec::new();
    for symbol in crate::ticks::symbols(&dir) {
        let Some((sigma, n)) = crate::ticks::realised_daily_sigma(&dir, &symbol, DAYS, MIN_SAMPLES)
        else {
            continue;
        };
        if !sigma.is_finite() || sigma <= 0.0 {
            continue;
        }
        let cmd = oms::Command::SetVolatility {
            symbol: symbol.clone(),
            daily_sigma_e8: (sigma * 1e8).round() as u64,
        };
        if ctx.cmd(cmd).await.is_ok() {
            out.push((symbol, sigma, n));
        }
    }
    out
}

pub fn spawn(ctx: AdminCtx) {
    tokio::spawn(async move {
        // first pass soon after start, then hourly
        tokio::time::sleep(Duration::from_secs(90)).await;
        let mut iv = tokio::time::interval(Duration::from_secs(3600));
        loop {
            iv.tick().await;
            let pushed = run_once(&ctx).await;
            if !pushed.is_empty() {
                tracing::info!(
                    symbols = pushed.len(),
                    "warehouse volatility pushed to the engine"
                );
            }
        }
    });
}
