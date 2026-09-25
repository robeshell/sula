mod accent;
mod app;
#[cfg(target_os = "macos")]
mod app_menu;
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

    let app = tauri::Builder::default()
        // Must stay first: a second launch exits here, before bootstrap takes the library lock.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main_window(app)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|window, event| {
            // Users change the accent in System Settings, then come back to the app.
            if let tauri::WindowEvent::Focused(true) = event {
                accent::refresh(window.app_handle());
            }
            if window.label() != "main" {
                return;
            }
            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            let Some(state) = window.try_state::<AppState>() else {
                return;
            };
            // Without a tray icon only macOS (Dock reopen) can bring a hidden window back.
            let reachable = cfg!(target_os = "macos") || state.tray_enabled.load(Ordering::Relaxed);
            if reachable && state.keep_running_on_close.load(Ordering::Relaxed) {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(move |app| {
            let state = AppState::bootstrap(logs.clone()).unwrap_or_else(|error| startup_failed(app.handle(), &error));
            app.manage(state);
            logs.attach_app(app.handle().clone());
            tray::setup(app.handle())?;
            accent::setup(app.handle());
            #[cfg(target_os = "macos")]
            {
                let locale = app.state::<AppState>().config.try_lock().map(|store| store.config.ui_locale.clone());
                app_menu::setup(app.handle(), locale.as_deref().unwrap_or("zh-Hans"))?;
            }
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
            commands::plan_show_merges,
            commands::merge_planned_shows,
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
            commands::open_settings_window,
            accent::get_system_accent,
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

fn startup_failed(app: &tauri::AppHandle, error: &anyhow::Error) -> ! {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    let locale = state::app_data_dir().ok().and_then(|dir| config::peek_ui_locale(&dir.join("config.toml")));
    let locale = locale.as_deref().and_then(ui_i18n::from_tag).unwrap_or_else(ui_i18n::system_locale);
    let detail = format!("{error:#}");
    let already_running = error.downcast_ref::<state::AlreadyRunning>().is_some();
    let (level, description) = if already_running {
        tracing::warn!(error = %detail, "another sula instance holds the library lock");
        (rfd::MessageLevel::Info, ui_i18n::t(locale, "startup.alreadyRunning"))
    } else {
        tracing::error!(error = %detail, "failed to bootstrap sula app state");
        (rfd::MessageLevel::Error, ui_i18n::tf(locale, "startup.failedBody", &[("err", &detail)]))
    };
    rfd::MessageDialog::new()
        .set_level(level)
        .set_title(if already_running { "Sula".into() } else { ui_i18n::t(locale, "startup.failedTitle") })
        .set_description(description)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    std::process::exit(if already_running { 0 } else { 1 });
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
