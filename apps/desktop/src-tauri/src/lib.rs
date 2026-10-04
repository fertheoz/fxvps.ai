//! fxvps desktop: Tauri 2 shell around the `apps/terminal` web build.
//!
//! Native features: detached chart windows, tray with connection status,
//! OS notifications, global shortcut, auto-updater (keys not committed),
//! OS-keychain credential storage and window-state persistence.

pub mod commands;
pub mod credentials;
pub mod tray;

/// Global shortcut that shows/focuses the terminal from anywhere.
pub const TOGGLE_SHORTCUT: &str = "CommandOrControl+Shift+F";

pub fn run() {
    let mut builder = tauri::Builder::default()
        // Must be first: focus the existing window instead of starting twice.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tray::toggle_main(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build());

    #[cfg(desktop)]
    {
        use tauri_plugin_global_shortcut::ShortcutState;
        builder = builder.plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([TOGGLE_SHORTCUT])
                .expect("valid shortcut")
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        tray::toggle_main(app);
                    }
                })
                .build(),
        );
    }

    builder
        .setup(|app| {
            tray::create(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_chart_window,
            commands::set_connection_status,
            commands::notify_event,
            commands::credential_set,
            commands::credential_get,
            commands::credential_delete,
        ])
        .run(tauri::generate_context!())
        .expect("error while running fxvps desktop");
}
