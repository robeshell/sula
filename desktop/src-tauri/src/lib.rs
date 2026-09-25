mod commands;
mod config;
mod log_store;
mod state;
mod task_queue;
mod tray;
mod ui_i18n;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use state::AppState;
use tauri::Manager;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let logs = log_store::LogStore::new();
    init_tracing(logs.clone());

    let state = AppState::bootstrap(logs.clone()).expect("failed to bootstrap sula app state");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            let keep_running = window
                .try_state::<AppState>()
                .map(|state| state.keep_running_on_close.load(Ordering::Relaxed))
                .unwrap_or(true);
            if keep_running {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(move |app| {
            logs.attach_app(app.handle().clone());
            tray::setup(app.handle())?;
            // Windows has no Overlay titlebar; drop native chrome so content
            // can draw edge-to-edge like macOS Overlay + traffic lights.
            #[cfg(target_os = "windows")]
            if let Some(window) = app.get_webview_window("main") {
                window.set_decorations(false)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::get_config,
            commands::save_config,
            commands::list_libraries,
            commands::add_library,
            commands::rename_library,
            commands::delete_library,
            commands::rebind_library,
            commands::path_is_dir,
            commands::clear_thumbnail_cache,
            commands::resolve_actor_avatar,
            commands::list_media_items,
            commands::list_media_page,
            commands::consolidate_library_shows,
            commands::consolidate_media_items,
            commands::get_media_detail,
            commands::resolve_poster_thumbnail,
            commands::refresh_library,
            commands::refresh_media_items,
            commands::scrape_library,
            commands::scrape_items,
            commands::rescrape_items,
            commands::scrape_season,
            commands::apply_rename_templates,
            commands::organize_season_folders,
            commands::scan_media_residuals,
            commands::cleanup_media_residuals,
            commands::delete_media_items,
            commands::search_match_candidates,
            commands::apply_manual_match,
            commands::list_tasks,
            commands::enqueue_smoke_task,
            commands::cancel_task,
            commands::open_renamer_window,
            commands::renamer_collect_files,
            commands::renamer_preview,
            commands::renamer_execute,
            commands::renamer_undo_last,
            commands::renamer_snapshot_count,
            commands::renamer_list_presets,
            commands::renamer_save_preset,
            commands::renamer_load_preset,
            commands::renamer_delete_preset,
            commands::renamer_auto_save_pipeline,
            commands::renamer_auto_load_pipeline,
            commands::list_logs,
            commands::clear_logs,
            commands::list_directory,
            commands::reveal_in_file_manager,
        ])
        .build(tauri::generate_context!())
        .expect("error while building sula");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } = event
        {
            tray::show_main_window(app);
        }
    });
}

fn init_tracing(logs: Arc<log_store::LogStore>) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .with(log_store::AppLogLayer::new(logs))
        .init();
    tracing::info!("sula tracing initialized");
}

mod credentials;
