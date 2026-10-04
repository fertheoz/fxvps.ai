const COMMANDS: &[&str] = &[
    "open_chart_window",
    "set_connection_status",
    "notify_event",
    "credential_set",
    "credential_get",
    "credential_delete",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
