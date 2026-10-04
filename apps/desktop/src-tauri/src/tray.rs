//! System tray: shows connection status, show/hide and quit.

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

use crate::commands::ConnectionStatus;

pub const TRAY_ID: &str = "main-tray";
const STATUS_ID: &str = "status";

pub fn status_text(status: ConnectionStatus, latency_ms: Option<u32>) -> String {
    let base = match status {
        ConnectionStatus::Connecting => "Connecting…",
        ConnectionStatus::Connected => "Connected",
        ConnectionStatus::Reconnecting => "Reconnecting…",
        ConnectionStatus::Disconnected => "Disconnected",
    };
    match (status, latency_ms) {
        (ConnectionStatus::Connected, Some(ms)) => format!("{base} ({ms} ms)"),
        _ => base.to_string(),
    }
}

pub fn toggle_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<TrayIcon<R>> {
    let status = MenuItem::with_id(
        app,
        STATUS_ID,
        status_text(ConnectionStatus::Connecting, None),
        false,
        None::<&str>,
    )?;
    let show = MenuItem::with_id(app, "show", "Show / hide terminal", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit fxvps", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&status, &sep, &show, &quit])?;
    app.manage(StatusItem(status));

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("fxvps — connecting")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => toggle_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)
}

struct StatusItem<R: Runtime>(MenuItem<R>);

pub fn set_status<R: Runtime>(
    app: &AppHandle<R>,
    status: ConnectionStatus,
    latency_ms: Option<u32>,
) -> tauri::Result<()> {
    let text = status_text(status, latency_ms);
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_tooltip(Some(format!("fxvps — {text}")))?;
    }
    if let Some(item) = app.try_state::<StatusItem<R>>() {
        item.0.set_text(text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_texts() {
        assert_eq!(
            status_text(ConnectionStatus::Connected, Some(12)),
            "Connected (12 ms)"
        );
        assert_eq!(
            status_text(ConnectionStatus::Disconnected, Some(12)),
            "Disconnected"
        );
    }
}
