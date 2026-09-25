use media_core::{Library, MediaItem, MediaType};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app::organize::{ShowMergePair, ShowMergePlanDto};
use crate::app::{err_string, Events};
use crate::config::AppConfig;
use crate::state::{AppState, AppStatusDto};
use crate::task_queue::TaskSnapshot;
use sula_core::batch_rename::RenamerOutcome;
use sula_core::files::DirectoryEntryDto;
use sula_core::media::{MediaDetailDto, MediaListPayload};

async fn ui_locale(state: &AppState) -> String {
    state.config().await.ui_locale
}

/// Forwards core events to the webview (`Events` belongs to sula-core, so the
/// handle is wrapped).
pub(crate) struct UiEvents(pub AppHandle);

impl Events for UiEvents {
    fn library_updated(&self) {
        let _ = self.0.emit("library-updated", ());
    }

    fn task_updated(&self, task: &TaskSnapshot) {
        let _ = self.0.emit("task-updated", task);
    }

    fn config_changed(&self, config: &AppConfig) {
        crate::tray::set_enabled(&self.0, config.tray_enabled);
        crate::tray::set_locale(&self.0, &config.ui_locale);
        #[cfg(target_os = "macos")]
        crate::app_menu::set_locale(&self.0, &config.ui_locale);
        // Other windows (main ↔ settings) keep their config in sync from this event.
        let _ = self.0.emit("config-changed", config);
    }
}

#[tauri::command]
pub async fn app_status(state: State<'_, AppState>) -> Result<AppStatusDto, String> {
    state.status().await
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config().await)
}

#[tauri::command]
pub async fn save_config(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig, String> {
    state.save_config(config).await
}

#[tauri::command]
pub async fn list_libraries(state: State<'_, AppState>) -> Result<Vec<Library>, String> {
    state.libraries().await
}

#[tauri::command]
pub async fn add_library(
    state: State<'_, AppState>,
    name: String,
    root_path: String,
    media_type: MediaType,
) -> Result<Library, String> {
    state.add_library(name, root_path, media_type).await
}

#[tauri::command]
pub async fn rename_library(state: State<'_, AppState>, id: String, name: String) -> Result<Library, String> {
    state.rename_library(id, name).await
}

#[tauri::command]
pub async fn delete_library(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.delete_library(id).await
}

#[tauri::command]
pub async fn rebind_library(state: State<'_, AppState>, id: String, root_path: String) -> Result<Library, String> {
    state.rebind_library(id, root_path).await
}

#[tauri::command]
pub async fn path_is_dir(path: String) -> Result<bool, String> {
    sula_core::files::path_is_dir(path).await
}

#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<DirectoryEntryDto>, String> {
    sula_core::files::list_directory(path).await
}

#[tauri::command]
pub async fn clear_thumbnail_cache(state: State<'_, AppState>) -> Result<usize, String> {
    state.clear_image_caches().await
}

#[tauri::command]
pub async fn resolve_actor_avatar(state: State<'_, AppState>, url: String) -> Result<Option<String>, String> {
    Ok(state.actor_avatar(url).await?.map(|path| path.display().to_string()))
}

#[tauri::command]
pub async fn resolve_poster_thumbnail(
    state: State<'_, AppState>,
    folder_path: String,
    poster_path: String,
    width: Option<u32>,
    height: Option<u32>,
    allow_fallbacks: Option<bool>,
) -> Result<Option<String>, String> {
    // A cache file path; the frontend loads it through convertFileSrc.
    let path = state.poster_thumbnail(folder_path, poster_path, width, height, allow_fallbacks).await?;
    Ok(path.map(|path| path.display().to_string()))
}

#[tauri::command]
pub async fn list_media_items(state: State<'_, AppState>, library_id: String) -> Result<Vec<MediaItem>, String> {
    state.media_items(library_id).await
}

#[tauri::command]
pub async fn list_media_page(
    state: State<'_, AppState>, library_id: String, offset: Option<u32>, limit: Option<u32>,
) -> Result<MediaListPayload, String> {
    state.media_page(library_id, offset, limit).await
}

#[tauri::command]
pub async fn get_media_detail(state: State<'_, AppState>, id: String) -> Result<MediaDetailDto, String> {
    state.media_detail(id).await
}

#[tauri::command]
pub async fn plan_show_merges(
    state: State<'_, AppState>,
    library_id: Option<String>,
    item_ids: Option<Vec<String>>,
) -> Result<Vec<ShowMergePlanDto>, String> {
    state.plan_show_merges(library_id, item_ids).await
}

#[tauri::command]
pub async fn merge_planned_shows(state: State<'_, AppState>, pairs: Vec<ShowMergePair>) -> Result<u32, String> {
    state.merge_planned_shows(pairs).await
}

#[tauri::command]
pub async fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskSnapshot>, String> {
    Ok(state.tasks().await)
}

#[tauri::command]
pub async fn enqueue_smoke_task(state: State<'_, AppState>, title: Option<String>) -> Result<TaskSnapshot, String> {
    Ok(state.enqueue_smoke_task(title).await)
}

#[tauri::command]
pub async fn cancel_task(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    Ok(state.cancel_task(id).await)
}

#[tauri::command]
pub async fn refresh_library(state: State<'_, AppState>, library_id: String) -> Result<TaskSnapshot, String> {
    state.refresh_library(library_id).await
}

#[tauri::command]
pub async fn refresh_media_items(state: State<'_, AppState>, item_ids: Vec<String>) -> Result<TaskSnapshot, String> {
    state.refresh_items(item_ids).await
}

#[tauri::command]
pub async fn scrape_library(state: State<'_, AppState>, library_id: String) -> Result<TaskSnapshot, String> {
    state.scrape_library(library_id).await
}

#[tauri::command]
pub async fn scrape_items(state: State<'_, AppState>, item_ids: Vec<String>) -> Result<TaskSnapshot, String> {
    state.scrape_items(item_ids).await
}

#[tauri::command]
pub async fn rescrape_items(state: State<'_, AppState>, item_ids: Vec<String>) -> Result<TaskSnapshot, String> {
    state.rescrape_items(item_ids).await
}

#[tauri::command]
pub async fn scrape_season(
    state: State<'_, AppState>,
    media_item_id: String,
    season_number: i32,
) -> Result<TaskSnapshot, String> {
    state.scrape_season(media_item_id, season_number).await
}

#[tauri::command]
pub async fn search_match_candidates(
    state: State<'_, AppState>,
    query: String,
    media_type: MediaType,
) -> Result<Vec<scraper_kit::SearchResult>, String> {
    state.search_match_candidates(query, media_type).await
}

#[tauri::command]
pub async fn apply_manual_match(state: State<'_, AppState>, item_id: String, source_id: String) -> Result<TaskSnapshot, String> {
    state.apply_manual_match(item_id, source_id).await
}

#[tauri::command]
pub async fn apply_rename_templates(state: State<'_, AppState>, item_ids: Vec<String>) -> Result<TaskSnapshot, String> {
    state.apply_rename_templates(item_ids).await
}

#[tauri::command]
pub async fn organize_season_folders(state: State<'_, AppState>, item_ids: Vec<String>) -> Result<TaskSnapshot, String> {
    state.organize_season_folders(item_ids).await
}

#[tauri::command]
pub async fn scan_media_residuals(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<Vec<media_core::ResidualCandidate>, String> {
    state.scan_residuals(item_ids).await
}

#[tauri::command]
pub async fn cleanup_media_residuals(state: State<'_, AppState>, paths: Vec<String>) -> Result<TaskSnapshot, String> {
    state.cleanup_residuals(paths).await
}

#[tauri::command]
pub async fn delete_media_items(state: State<'_, AppState>, item_ids: Vec<String>, also_trash: bool) -> Result<usize, String> {
    state.delete_media_items(item_ids, also_trash).await
}

#[tauri::command]
pub async fn open_renamer_window(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let locale = ui_locale(&state).await;
    if let Some(existing) = app.get_webview_window("renamer") {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }
    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        "renamer",
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title(crate::ui_i18n::t(&locale, "window.renamer"))
    .inner_size(1040.0, 740.0)
    .min_inner_size(800.0, 560.0);

    // Same chrome as the main window: the toolbar hosts the traffic lights on
    // macOS; Windows is undecorated and draws its own caption buttons.
    // tao keeps the buttons' own offset (~9pt on macOS 26), so y = 28 centres
    // them on the 52pt toolbar.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(16.0, 28.0));
    #[cfg(target_os = "windows")]
    let builder = builder.decorations(false);

    builder.build().map_err(err_string)?;
    Ok(())
}

const SETTINGS_LABEL: &str = "settings";

#[tauri::command]
pub async fn open_settings_window(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    let locale = ui_locale(&state).await;
    open_settings(&app, &locale)
}

/// Menu-bar entry point (⌘,): menu callbacks run on the event loop, so the
/// window is built from an async task like the command does.
pub(crate) fn spawn_open_settings(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let locale = match app.try_state::<AppState>() {
            Some(state) => ui_locale(&state).await,
            None => crate::ui_i18n::system_locale().to_string(),
        };
        if let Err(error) = open_settings(&app, &locale) {
            tracing::warn!(%error, "failed to open settings window");
        }
    });
}

fn focus_settings(app: &AppHandle) -> bool {
    let Some(existing) = app.get_webview_window(SETTINGS_LABEL) else {
        return false;
    };
    let _ = existing.unminimize();
    let _ = existing.show();
    let _ = existing.set_focus();
    true
}

fn open_settings(app: &AppHandle, locale: &str) -> Result<(), String> {
    if focus_settings(app) {
        return Ok(());
    }
    let builder = tauri::WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title(crate::ui_i18n::t(locale, "window.settings"))
    .inner_size(700.0, 610.0)
    .resizable(false)
    .maximizable(false)
    .center();

    // macOS overlay title bar with the lights centred in the 38pt title row
    // (see open_renamer_window for the offset); Windows undecorated.
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true)
        .traffic_light_position(tauri::LogicalPosition::new(14.0, 21.0));
    #[cfg(target_os = "windows")]
    let builder = builder.decorations(false);

    match builder.build() {
        Ok(_) => Ok(()),
        // A concurrent request (menu + shortcut) may have created it first.
        Err(_) if focus_settings(app) => Ok(()),
        Err(error) => Err(err_string(error)),
    }
}

#[tauri::command]
pub async fn renamer_collect_files(paths: Vec<String>) -> Result<Vec<renamer::FileEntry>, String> {
    sula_core::batch_rename::collect_files(paths).await
}

#[tauri::command]
pub async fn renamer_preview(
    files: Vec<renamer::FileEntry>,
    pipeline: renamer::RulePipeline,
) -> Result<Vec<renamer::PreviewResult>, String> {
    Ok(sula_core::batch_rename::preview(&files, &pipeline))
}

#[tauri::command]
pub async fn renamer_execute(
    state: State<'_, AppState>,
    files: Vec<renamer::FileEntry>,
    pipeline: renamer::RulePipeline,
) -> Result<RenamerOutcome, String> {
    state.rename_files(files, pipeline).await
}

#[tauri::command]
pub async fn renamer_undo_last(state: State<'_, AppState>) -> Result<RenamerOutcome, String> {
    state.undo_last_rename().await
}

#[tauri::command]
pub async fn renamer_snapshot_count(state: State<'_, AppState>) -> Result<usize, String> {
    state.rename_snapshot_count().await
}

#[tauri::command]
pub async fn renamer_list_presets(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state.rename_presets().await
}

#[tauri::command]
pub async fn renamer_save_preset(
    state: State<'_, AppState>,
    name: String,
    pipeline: renamer::RulePipeline,
) -> Result<(), String> {
    state.save_rename_preset(name, pipeline).await
}

#[tauri::command]
pub async fn renamer_load_preset(state: State<'_, AppState>, name: String) -> Result<Option<renamer::RulePipeline>, String> {
    state.load_rename_preset(name).await
}

#[tauri::command]
pub async fn renamer_delete_preset(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.delete_rename_preset(name).await
}

#[tauri::command]
pub async fn renamer_auto_save_pipeline(state: State<'_, AppState>, pipeline: renamer::RulePipeline) -> Result<(), String> {
    state.save_last_rename_pipeline(pipeline).await
}

#[tauri::command]
pub async fn renamer_auto_load_pipeline(state: State<'_, AppState>) -> Result<Option<renamer::RulePipeline>, String> {
    state.last_rename_pipeline().await
}

#[tauri::command]
pub async fn list_logs(state: State<'_, AppState>) -> Result<Vec<crate::log_store::LogEntry>, String> {
    Ok(state.logs.list())
}

#[tauri::command]
pub async fn clear_logs(state: State<'_, AppState>) -> Result<(), String> {
    state.logs.clear();
    Ok(())
}


/// Reveal a file/folder in the OS file manager (Explorer / Finder / …).
///
/// Strips Windows `\\?\` prefixes from DB paths so Explorer can open them.
#[tauri::command]
pub async fn reveal_in_file_manager(path: String) -> Result<(), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("empty path".into());
    }
    let cleaned = media_core::scanner::strip_windows_verbatim_prefix(std::path::Path::new(trimmed));
    let target = if cleaned.exists() {
        cleaned
    } else if let Some(parent) = cleaned.parent().filter(|p| p.exists()) {
        parent.to_path_buf()
    } else {
        return Err(format!("path does not exist: {}", cleaned.display()));
    };
    reveal_path_impl(&target)
}

#[cfg(windows)]
fn reveal_path_impl(path: &std::path::Path) -> Result<(), String> {
    // `explorer /select,<path>` is the reliable Windows reveal path;
    // opener's ILCreateFromPathW rejects some canonicalized/verbatim forms.
    std::process::Command::new("explorer")
        .arg("/select,")
        .arg(path)
        .spawn()
        .map_err(err_string)?;
    Ok(())
}

#[cfg(not(windows))]
fn reveal_path_impl(path: &std::path::Path) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(err_string)
}
