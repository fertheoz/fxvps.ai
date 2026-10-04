//! IPC commands exposed to the terminal webview. Every command here must also be
//! listed in `build.rs` and granted in `permissions/app.toml` + a capability.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_notification::NotificationExt;

use crate::{credentials, tray};

/// Validates a symbol for use in a window label / query string: `[A-Z0-9._-]{1,32}`.
pub fn sanitize_symbol(symbol: &str) -> Option<String> {
    let s = symbol.trim().to_ascii_uppercase();
    let ok = !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    ok.then_some(s)
}

/// Window label for a detached chart (matches the `chart-*` capability glob).
pub fn chart_label(symbol: &str) -> String {
    format!("chart-{}", symbol.replace('.', "_"))
}

/// Opens (or focuses) a detached chart window for `symbol`. The terminal build is
/// loaded with `?view=chart&symbol=...` so it can render a single chart.
#[tauri::command]
pub fn open_chart_window<R: Runtime>(
    app: AppHandle<R>,
    symbol: String,
    timeframe: Option<String>,
) -> Result<String, String> {
    let symbol = sanitize_symbol(&symbol).ok_or("invalid symbol")?;
    let tf = timeframe
        .as_deref()
        .and_then(sanitize_symbol)
        .unwrap_or_else(|| "M5".into());
    let label = chart_label(&symbol);
    if let Some(existing) = app.get_webview_window(&label) {
        existing.set_focus().map_err(|e| e.to_string())?;
        return Ok(label);
    }
    let url = format!("index.html?view=chart&symbol={symbol}&tf={tf}");
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.into()))
        .title(format!("{symbol} · {tf} — fxvps"))
        .inner_size(900.0, 600.0)
        .min_inner_size(480.0, 320.0)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(label)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

/// Mirrors the terminal's `ConnectionState` into the tray tooltip/menu.
#[tauri::command]
pub fn set_connection_status<R: Runtime>(
    app: AppHandle<R>,
    status: ConnectionStatus,
    latency_ms: Option<u32>,
) -> Result<(), String> {
    tray::set_status(&app, status, latency_ms).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotifyEvent {
    Fill {
        symbol: String,
        side: String,
        lots: String,
        price: String,
    },
    PriceAlert {
        symbol: String,
        price: String,
        message: Option<String>,
    },
}

impl NotifyEvent {
    pub fn render(&self) -> (String, String) {
        match self {
            NotifyEvent::Fill {
                symbol,
                side,
                lots,
                price,
            } => (
                format!("Order filled: {symbol}"),
                format!("{} {lots} {symbol} @ {price}", side.to_uppercase()),
            ),
            NotifyEvent::PriceAlert {
                symbol,
                price,
                message,
            } => (
                format!("Price alert: {symbol}"),
                message
                    .clone()
                    .unwrap_or_else(|| format!("{symbol} reached {price}")),
            ),
        }
    }
}

/// Native OS notification for fills and price alerts.
#[tauri::command]
pub fn notify_event<R: Runtime>(app: AppHandle<R>, event: NotifyEvent) -> Result<(), String> {
    let (title, body) = event.render();
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

/// Stores a secret (e.g. API token) in the OS keychain under `account`.
#[tauri::command]
pub fn credential_set(account: String, secret: String) -> Result<(), String> {
    credentials::set(&account, &secret).map_err(|e| e.to_string())
}

/// Reads a secret from the OS keychain; `None` when absent.
#[tauri::command]
pub fn credential_get(account: String) -> Result<Option<String>, String> {
    credentials::get(&account).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn credential_delete(account: String) -> Result<(), String> {
    credentials::delete(&account).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_are_sanitized() {
        assert_eq!(sanitize_symbol(" eurusd "), Some("EURUSD".into()));
        assert_eq!(sanitize_symbol("US30.cash"), Some("US30.CASH".into()));
        assert_eq!(sanitize_symbol(""), None);
        assert_eq!(sanitize_symbol("EUR/USD"), None);
        assert_eq!(sanitize_symbol("a&b=c"), None);
        assert_eq!(sanitize_symbol(&"X".repeat(33)), None);
    }

    #[test]
    fn chart_labels_match_capability_glob() {
        assert_eq!(chart_label("US30.CASH"), "chart-US30_CASH");
        assert!(chart_label("EURUSD").starts_with("chart-"));
    }

    #[test]
    fn notifications_render() {
        let fill: NotifyEvent = serde_json::from_str(
            r#"{"kind":"fill","symbol":"EURUSD","side":"buy","lots":"0.10","price":"1.08506"}"#,
        )
        .unwrap();
        assert_eq!(
            fill.render(),
            (
                "Order filled: EURUSD".into(),
                "BUY 0.10 EURUSD @ 1.08506".into()
            )
        );
        let alert: NotifyEvent =
            serde_json::from_str(r#"{"kind":"price_alert","symbol":"XAUUSD","price":"2400"}"#)
                .unwrap();
        assert_eq!(alert.render().1, "XAUUSD reached 2400");
    }
}
