mod commands;
pub mod core;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let paths = core::storage::Paths {
                data: app.path().app_data_dir()?,
                temp: app.path().app_local_data_dir()?.join("tmp"),
                documents: app.path().document_dir().ok(),
            };
            app.manage(commands::AppState::new(paths));

            // Debug-only UI automation bridge (see `npm run dev`). Never compiled into release builds.
            #[cfg(all(debug_assertions, feature = "mcp-bridge"))]
            app.add_capability(
                tauri::ipc::CapabilityBuilder::new("mcp-bridge-dev")
                    .window("main")
                    .permission("mcp-bridge:default"),
            )?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::lazer_status,
            commands::detected_lazer_paths,
            commands::launch_lazer,
            commands::recent_collections,
            commands::load_collection,
            commands::selection_preview,
            commands::start_job,
            commands::job_snapshot,
            commands::pause_job,
            commands::resume_job,
            commands::cancel_job,
            commands::retry_failed,
            commands::clear_cache,
            commands::clear_history,
            commands::history_size,
            commands::open_folder,
            commands::read_collection_db,
            commands::load_db_collection,
            commands::cancel_db_lookup,
            commands::scan_lazer,
            commands::installed_count,
            commands::export_info,
            commands::clear_export,
            commands::is_installed,
        ]);

    #[cfg(all(debug_assertions, feature = "mcp-bridge"))]
    let builder = builder.plugin(
        tauri_plugin_mcp_bridge::Builder::new()
            .bind_address("127.0.0.1")
            .build(),
    );

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
