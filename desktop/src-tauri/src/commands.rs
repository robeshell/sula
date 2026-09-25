use std::sync::atomic::Ordering;
use std::sync::Arc;

use media_core::{AppDatabase, Library, MediaItem, MediaType, ScrapedStatus};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app::cleanup::SystemTrash;
use crate::app::library::RefreshService;
use crate::app::locks::LockScope;
use crate::app::organize::{OrganizeService, ShowMergePair, ShowMergePlanDto};
use crate::app::scrape::{localized_error, ScrapeService, ScrapeSettings};
use crate::app::{blocking, err_string, Events};
use crate::config::AppConfig;
use crate::state::{AppState, AppStatusDto};
use sula_core::files::DirectoryEntryDto;
use sula_core::media::{MediaDetailDto, MediaListPayload};
use crate::task_queue::{TaskKind, TaskSnapshot};

async fn ui_locale(state: &State<'_, AppState>) -> String {
    state.config.lock().await.config.ui_locale.clone()
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
}

/// Lock scope of the libraries the given items belong to.
async fn items_scope(db: &Arc<AppDatabase>, item_ids: &[String]) -> Result<LockScope, String> {
    let (db, ids) = (Arc::clone(db), item_ids.to_vec());
    blocking(move || LockScope::for_items(&db, &ids)).await
}

#[tauri::command]
pub async fn app_status(state: State<'_, AppState>) -> Result<AppStatusDto, String> {
    state.status().await
}

#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config.lock().await.config.clone())
}

#[tauri::command]
pub async fn save_config(
    app: AppHandle,
    state: State<'_, AppState>,
    config: AppConfig,
) -> Result<AppConfig, String> {
    // Ordinary settings must not wait for a long scrape. Only a change of scan
    // exclusions resets scan state, which has to stay out of a running refresh.
    let exclusions_changed = state.config.lock().await.config.scan_excluded_folders != config.scan_excluded_folders;
    let mutation_guard = if exclusions_changed { Some(state.tasks.locks().lock(&LockScope::Global).await?) } else { None };
    let tray_enabled = config.tray_enabled;
    let mut store = state.config.lock().await;
    let old = store.config.clone();
    let exclusions_changed = old.scan_excluded_folders != config.scan_excluded_folders;
    if exclusions_changed && mutation_guard.is_none() {
        return Err("settings changed concurrently; try again".into());
    }
    store.config = config;
    if let Err(error) = store.save() {
        store.config = old;
        return Err(error.to_string());
    }
    if exclusions_changed {
        for library in state.db.list_libraries().map_err(err_string)? {
            state.db.clear_scan_states(&library.id).map_err(err_string)?;
        }
    }
    let saved = store.config.clone();
    drop(store);
    state
        .keep_running_on_close
        .store(saved.keep_running_on_close, Ordering::Relaxed);
    crate::tray::set_enabled(&app, tray_enabled);
    crate::tray::set_locale(&app, &saved.ui_locale);
    #[cfg(target_os = "macos")]
    crate::app_menu::set_locale(&app, &saved.ui_locale);
    // Other windows (main ↔ settings) keep their config in sync from this event.
    let _ = app.emit("config-changed", &saved);
    Ok(saved)
}

#[tauri::command]
pub async fn list_libraries(state: State<'_, AppState>) -> Result<Vec<Library>, String> {
    state.db.list_libraries().map_err(err_string)
}

#[tauri::command]
pub async fn add_library(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    root_path: String,
    media_type: MediaType,
) -> Result<Library, String> {
    // Only inserts a row and queues its first refresh; no files change here.
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("library name is empty".into());
    }
    if root_path.trim().is_empty() {
        return Err("library path is empty".into());
    }
    let library = Library::new(name, root_path, media_type);
    state.db.insert_library(&library).map_err(err_string)?;
    let _ = enqueue_refresh_inner(&app, &state, library.id.clone()).await?;
    // Every window keeps its own library list (settings edits them too).
    UiEvents(app.clone()).library_updated();
    Ok(library)
}

#[tauri::command]
pub async fn rename_library(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<Library, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("library name is empty".into());
    }
    let mut library = state
        .db
        .get_library(&id)
        .map_err(err_string)?
        .ok_or_else(|| format!("library not found: {id}"))?;
    library.name = name;
    state.db.update_library(&library).map_err(err_string)?;
    UiEvents(app.clone()).library_updated();
    Ok(library)
}

#[tauri::command]
pub async fn delete_library(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let _mutation_guard = state.tasks.locks().lock(&LockScope::library(&id)).await?;
    state.db.delete_library(&id).map_err(err_string)?;
    UiEvents(app.clone()).library_updated();
    Ok(())
}

#[tauri::command]
pub async fn path_is_dir(path: String) -> Result<bool, String> {
    sula_core::files::path_is_dir(path).await
}

/// LIB-08: rebind library root when the previous path is stale / moved.
#[tauri::command]
pub async fn rebind_library(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    root_path: String,
) -> Result<Library, String> {
    // A new root can overlap other libraries, so rebinding excludes all of them.
    let _mutation_guard = state.tasks.locks().lock(&LockScope::Global).await?;
    let root_path = root_path.trim().to_string();
    if root_path.is_empty() {
        return Err("library path is empty".into());
    }
    if !std::path::Path::new(&root_path).is_dir() {
        return Err("selected path is not a directory".into());
    }
    let mut library = state
        .db
        .get_library(&id)
        .map_err(err_string)?
        .ok_or_else(|| format!("library not found: {id}"))?;
    library.root_path = root_path;
    library.bookmark_data = None;
    state.db.update_library(&library).map_err(err_string)?;
    // Path changed → wipe scan state so next refresh re-bootstraps.
    if let Err(error) = state.db.clear_scan_states(&library.id) {
        tracing::warn!(library_id = %library.id, %error, "scan state not cleared after rebind");
    }
    let _ = enqueue_refresh_inner(&app, &state, library.id.clone()).await?;
    UiEvents(app.clone()).library_updated();
    Ok(library)
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
pub async fn list_media_items(state: State<'_, AppState>, library_id: String) -> Result<Vec<MediaItem>, String> {
    state.media_items(library_id).await
}

#[tauri::command]
pub async fn list_media_page(
    state: State<'_, AppState>, library_id: String, offset: Option<u32>, limit: Option<u32>,
) -> Result<MediaListPayload, String> {
    state.media_page(library_id, offset, limit).await
}

/// Read-only preview of duplicate-show merges for a library or selected items, so
/// the user sees exactly which folders would be absorbed before anything moves.
#[tauri::command]
pub async fn plan_show_merges(
    state: State<'_, AppState>,
    library_id: Option<String>,
    item_ids: Option<Vec<String>>,
) -> Result<Vec<ShowMergePlanDto>, String> {
    let db = Arc::clone(&state.db);
    blocking(move || crate::app::organize::plan_show_merges(&db, library_id, item_ids)).await
}

/// Execute merges the user confirmed from `plan_show_merges`. Each pair is checked
/// again against the current index and skipped if the match changed meanwhile.
#[tauri::command]
pub async fn merge_planned_shows(
    app: AppHandle,
    state: State<'_, AppState>,
    pairs: Vec<ShowMergePair>,
) -> Result<u32, String> {
    let db = Arc::clone(&state.db);
    // Every library of every pair, locked in one sorted acquisition.
    let scope = items_scope(&db, &crate::app::organize::merge_item_ids(&pairs)).await?;
    let mutation_guard = state.tasks.locks().lock(&scope).await?;
    let templates = state.config.lock().await.config.rename_templates();
    blocking(move || {
        let _mutation_guard = mutation_guard;
        crate::app::organize::merge_planned_shows(&db, &pairs, &templates, &UiEvents(app.clone()))
    })
    .await
}

#[tauri::command]
pub async fn get_media_detail(state: State<'_, AppState>, id: String) -> Result<MediaDetailDto, String> {
    state.media_detail(id).await
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
pub async fn refresh_library(
    app: AppHandle,
    state: State<'_, AppState>,
    library_id: String,
) -> Result<TaskSnapshot, String> {
    enqueue_refresh_inner(&app, &state, library_id).await
}

#[tauri::command]
pub async fn refresh_media_items(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<TaskSnapshot, String> {
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }
    let locale = ui_locale(&state).await;
    let title = if item_ids.len() == 1 {
        crate::ui_i18n::t(&locale, "task.refreshItems")
    } else {
        crate::ui_i18n::tf(&locale, "task.refreshItemsN", &[("n", &item_ids.len().to_string())])
    };
    let config_store = Arc::clone(&state.config);
    let db = Arc::clone(&state.db);
    let lock = items_scope(&db, &item_ids).await?;
    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Refresh, item_ids.first().cloned(), task_scope("items", &item_ids), Some(lock), move |handle| {
            Box::pin(async move {
                let excluded_folders = config_store.lock().await.config.scan_excluded_folders.clone();
                let service = RefreshService { db, excluded_folders, templates: Default::default(), locale };
                service.refresh_items(item_ids, &handle).await
            })
        })
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskSnapshot>, String> {
    Ok(state.tasks.list().await)
}

#[tauri::command]
pub async fn enqueue_smoke_task(
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<TaskSnapshot, String> {
    let snapshot = state
        .tasks
        .enqueue_smoke(title.unwrap_or_else(|| "M0 smoke task".into()))
        .await;
    Ok(snapshot)
}

#[tauri::command]
pub async fn cancel_task(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    Ok(state.tasks.cancel(&id).await)
}

async fn enqueue_refresh_inner(
    app: &AppHandle,
    state: &State<'_, AppState>,
    library_id: String,
) -> Result<TaskSnapshot, String> {
    if let Some(existing) = state
        .tasks
        .find_active(TaskKind::Refresh, &library_id)
        .await
    {
        let _ = app.emit("task-updated", &existing);
        return Ok(existing);
    }

    let library = state
        .db
        .get_library(&library_id)
        .map_err(err_string)?
        .ok_or_else(|| format!("library not found: {library_id}"))?;
    let locale = ui_locale(state).await;
    let config_store = Arc::clone(&state.config);
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(&locale, "task.refreshLib", &[("name", &library.name)]);
    let target_id = Some(library_id.clone());
    let lock = Some(LockScope::library(&library_id));

    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Refresh, target_id, task_scope("library", std::slice::from_ref(&library_id)), lock, move |handle| {
            Box::pin(async move {
                // Settings are read when the refresh starts, not when it was queued.
                let config = config_store.lock().await.config.clone();
                let service = RefreshService {
                    db,
                    excluded_folders: config.scan_excluded_folders.clone(),
                    templates: config.rename_templates(),
                    locale,
                };
                service.refresh_library(&library_id, &handle).await
            })
        })
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn scrape_library(
    app: AppHandle,
    state: State<'_, AppState>,
    library_id: String,
) -> Result<TaskSnapshot, String> {
    let library = state
        .db
        .get_library(&library_id)
        .map_err(err_string)?
        .ok_or_else(|| format!("library not found: {library_id}"))?;
    let config = state.config.lock().await.config.clone();
    let title = crate::ui_i18n::tf(&config.ui_locale, "task.scrapeAll", &[("name", &library.name)]);
    let target_id = Some(library_id.clone());

    if let Some(existing) = state
        .tasks
        .find_active(TaskKind::BatchScrape, &library_id)
        .await
    {
        let _ = app.emit("task-updated", &existing);
        return Ok(existing);
    }

    // No held lock: the service fetches unlocked and locks the library only to write.
    let service = scrape_service(&state, &config);
    let snapshot = state
        .tasks
        .enqueue(title, TaskKind::BatchScrape, target_id, None, move |handle| {
            Box::pin(async move { service.scrape_library(&library_id, &handle).await })
        })
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn scrape_items(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<TaskSnapshot, String> {
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }
    let config = state.config.lock().await.config.clone();
    let title = crate::ui_i18n::tf(&config.ui_locale, "task.scrapeN", &[("n", &item_ids.len().to_string())]);
    let service = scrape_service(&state, &config);
    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Scrape, item_ids.first().cloned(), task_scope("items", &item_ids), None, move |handle| {
            Box::pin(async move { service.scrape_items(item_ids, &handle).await })
        })
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn rescrape_items(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<TaskSnapshot, String> {
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }
    let mut scraped_ids = Vec::new();
    for id in &item_ids {
        let item = state
            .db
            .get_media_item(id)
            .map_err(err_string)?
            .ok_or_else(|| format!("media item not found: {id}"))?;
        if item.status == ScrapedStatus::Scraped {
            scraped_ids.push(id.clone());
        }
    }
    if scraped_ids.is_empty() {
        return Err("no scraped items selected".into());
    }

    let config = state.config.lock().await.config.clone();
    let title = crate::ui_i18n::tf(&config.ui_locale, "task.rescrapeN", &[("n", &scraped_ids.len().to_string())]);
    let service = scrape_service(&state, &config);
    let snapshot = state
        .tasks
        .enqueue_scoped(
            title,
            TaskKind::Rescrape,
            scraped_ids.first().cloned(),
            task_scope("items", &scraped_ids),
            None,
            move |handle| Box::pin(async move { service.scrape_items(scraped_ids, &handle).await }),
        )
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn scrape_season(
    state: State<'_, AppState>,
    media_item_id: String,
    season_number: i32,
) -> Result<TaskSnapshot, String> {
    let config = state.config.lock().await.config.clone();
    let service = scrape_service(&state, &config);
    let snapshot = state.tasks.enqueue_scoped(format!("Season {season_number}"), TaskKind::Scrape,
        Some(media_item_id.clone()), task_scope("season", &[media_item_id.clone(), season_number.to_string()]), None,
        move |handle| Box::pin(async move { service.scrape_season(&media_item_id, season_number, &handle).await })).await;
    Ok(snapshot)
}

#[tauri::command]
pub async fn apply_rename_templates(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<TaskSnapshot, String> {
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }
    let config = state.config.lock().await.config.clone();
    let title = crate::ui_i18n::tf(&config.ui_locale, "task.renameN", &[("n", &item_ids.len().to_string())]);
    let lock = items_scope(&state.db, &item_ids).await?;
    let service = organize_service(&state, &config);
    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Rename, item_ids.first().cloned(), task_scope("items", &item_ids), Some(lock), move |handle| {
            Box::pin(async move { service.apply_rename_templates(item_ids, &handle).await })
        })
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn organize_season_folders(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<TaskSnapshot, String> {
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }
    let mut targets = Vec::new();
    for id in &item_ids {
        let item = state
            .db
            .get_media_item(id)
            .map_err(err_string)?
            .ok_or_else(|| format!("media item not found: {id}"))?;
        if item.status == ScrapedStatus::Scraped
            && matches!(item.media_type, MediaType::TvShow | MediaType::Anime)
        {
            targets.push(id.clone());
        }
    }
    if targets.is_empty() {
        return Err("no scraped tv/anime items selected".into());
    }

    let config = state.config.lock().await.config.clone();
    let title = crate::ui_i18n::tf(&config.ui_locale, "task.organizeN", &[("n", &targets.len().to_string())]);
    let lock = items_scope(&state.db, &targets).await?;
    let service = organize_service(&state, &config);
    let snapshot = state
        .tasks
        .enqueue_scoped(
            title,
            TaskKind::Organize,
            targets.first().cloned(),
            task_scope("items", &targets),
            Some(lock),
            move |handle| Box::pin(async move { service.organize_season_folders(targets, &handle).await }),
        )
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn scan_media_residuals(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<Vec<media_core::ResidualCandidate>, String> {
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }
    let db = Arc::clone(&state.db);
    tokio::task::spawn_blocking(move || media_core::find_residuals(&db, &item_ids))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err_string)
}

#[tauri::command]
pub async fn cleanup_media_residuals(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<TaskSnapshot, String> {
    if paths.is_empty() {
        return Err("no residual files selected".into());
    }
    let locale = ui_locale(&state).await;
    let title = crate::ui_i18n::tf(&locale, "task.cleanupN", &[("n", &paths.len().to_string())]);
    let db = Arc::clone(&state.db);
    // Candidates are revalidated across every library, so the job excludes all of them.
    let snapshot = state
        .tasks
        .enqueue(title, TaskKind::Cleanup, None, Some(LockScope::Global), move |handle| {
            Box::pin(async move { crate::app::cleanup::cleanup_residuals(db, paths, &locale, Arc::new(SystemTrash), &handle).await })
        })
        .await;

    Ok(snapshot)
}

#[tauri::command]
pub async fn delete_media_items(
    app: AppHandle,
    state: State<'_, AppState>,
    item_ids: Vec<String>,
    also_trash: bool,
) -> Result<usize, String> {
    let scope = items_scope(&state.db, &item_ids).await?;
    let mutation_guard = state.tasks.locks().lock(&scope).await?;
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }

    let db = Arc::clone(&state.db);
    blocking(move || {
        let _mutation_guard = mutation_guard;
        crate::app::cleanup::delete_media_items(&db, &item_ids, also_trash, &SystemTrash, &UiEvents(app.clone()))
    })
    .await
}

#[tauri::command]
pub async fn search_match_candidates(
    state: State<'_, AppState>,
    query: String,
    media_type: MediaType,
) -> Result<Vec<scraper_kit::SearchResult>, String> {
    let config = state.config.lock().await.config.clone();
    let locale = config.ui_locale.clone();
    let coordinator = scraper_kit::ScraperCoordinator::new(scraper_keys(&config));
    coordinator
        .search_manual(&query, media_type, &config.metadata_language)
        .await
        .map_err(|e| localized_error(&locale, e))
}

#[tauri::command]
pub async fn apply_manual_match(
    state: State<'_, AppState>,
    item_id: String,
    source_id: String,
) -> Result<TaskSnapshot, String> {
    let config = state.config.lock().await.config.clone();
    let item = state
        .db
        .get_media_item(&item_id)
        .map_err(err_string)?
        .ok_or_else(|| format!("media item not found: {item_id}"))?;
    let title = crate::ui_i18n::tf(
        &config.ui_locale,
        "task.manualMatch",
        &[("title", &item.title)],
    );
    let target_id = Some(item_id.clone());
    let service = scrape_service(&state, &config);

    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::ManualMatch, target_id, task_scope("match", &[item_id.clone(), source_id.clone()]), None, move |handle| {
            Box::pin(async move { service.apply_manual_match(&item, &source_id, &handle).await })
        })
        .await;

    Ok(snapshot)
}

fn scrape_options_from_config(config: &AppConfig) -> scraper_kit::ScrapeOptions {
    scraper_kit::ScrapeOptions {
        language: config.metadata_language.clone(),
        concurrency: config.scrape_concurrency.max(1) as usize,
        keys: scraper_keys(config),
    }
}

fn scraper_keys(config: &AppConfig) -> scraper_kit::ScraperKeys {
    scraper_kit::ScraperKeys {
        tmdb: config.api_keys.tmdb.clone(),
        bangumi: config.api_keys.bangumi.clone(),
        omdb: config.api_keys.omdb.clone(),
        tvdb: config.api_keys.tvdb.clone(),
    }
}

fn scrape_service(state: &State<'_, AppState>, config: &AppConfig) -> ScrapeService<scraper_kit::ScrapeClient> {
    let options = scrape_options_from_config(config);
    ScrapeService {
        db: Arc::clone(&state.db),
        locks: Arc::clone(state.tasks.locks()),
        source: Arc::new(scraper_kit::ScrapeClient::new(&options)),
        settings: ScrapeSettings {
            concurrency: options.concurrency,
            nfo_format: config.nfo_format.clone(),
            locale: config.ui_locale.clone(),
            templates: config.rename_templates(),
            auto_rename: config.rename_auto_after_scrape,
            create_season_folders: config.rename_create_season_folders,
        },
    }
}

fn organize_service(state: &State<'_, AppState>, config: &AppConfig) -> OrganizeService {
    OrganizeService {
        db: Arc::clone(&state.db),
        templates: config.rename_templates(),
        create_season_folders: config.rename_create_season_folders,
        locale: config.ui_locale.clone(),
    }
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
    blocking(move || renamer_collect_files_sync(paths)).await
}

fn renamer_collect_files_sync(paths: Vec<String>) -> Result<Vec<renamer::FileEntry>, String> {
    let mut out = Vec::new();
    for raw in paths {
        let path = std::path::PathBuf::from(&raw);
        collect_paths_into(&path, &mut out).map_err(err_string)?;
        if out.len() > MAX_RENAMER_FILES {
            return Err(format!("too many files (max {MAX_RENAMER_FILES})"));
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn renamer_preview(
    files: Vec<renamer::FileEntry>,
    pipeline: renamer::RulePipeline,
) -> Result<Vec<renamer::PreviewResult>, String> {
    Ok(renamer::preview(&files, &pipeline))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamerOutcome {
    pub renames: Vec<renamer::CompletedRename>,
    /// Set when the batch stopped early; `renames` still lists what already moved.
    pub error: Option<String>,
    /// Library index rows that could not follow a moved file; the next refresh
    /// would otherwise treat those files as deleted.
    pub index_sync_failures: usize,
    /// Undo only: entries whose renamed file no longer exists and was left alone.
    pub skipped: usize,
}

/// The index stores canonical paths. After a rename only the parent directory of
/// either side still resolves, so canonicalize that and re-attach the file name.
fn canonical_entry_path(raw: &str) -> String {
    let path = std::path::Path::new(raw);
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            std::path::Path::new(&media_core::scanner::canonicalize_lossy(parent))
                .join(name)
                .to_string_lossy()
                .into_owned()
        }
        _ => raw.to_string(),
    }
}

fn sync_renamed_entry(db: &media_core::AppDatabase, rename: &renamer::CompletedRename, failures: &mut usize) {
    let old = canonical_entry_path(&rename.original_path);
    let new = canonical_entry_path(&rename.new_path);
    if let Err(error) = db.remap_renamed_path(&old, &new) {
        tracing::warn!(%old, %new, %error, "renamer: library index not updated");
        *failures += 1;
    }
}

/// Previews are recomputed here from the same inputs as `renamer_preview`, so a
/// stale or forged preview from the webview can never choose the destination.
#[tauri::command]
pub async fn renamer_execute(
    app: AppHandle,
    state: State<'_, AppState>,
    files: Vec<renamer::FileEntry>,
    pipeline: renamer::RulePipeline,
) -> Result<RenamerOutcome, String> {
    let mutation_guard = state.tasks.locks().lock(&LockScope::Global).await?;
    let (db, undo) = (Arc::clone(&state.db), Arc::clone(&state.rename_undo));
    blocking(move || {
        let _mutation_guard = mutation_guard;
        let previews = renamer::preview(&files, &pipeline);
        let mut outcome = RenamerOutcome { renames: Vec::new(), error: None, index_sync_failures: 0, skipped: 0 };
        let result = renamer::execute(&previews, &undo, |done| {
            outcome.renames.push(done.clone());
            sync_renamed_entry(&db, done, &mut outcome.index_sync_failures);
        });
        finish_renamer_batch(&app, outcome, result.map(|_| ()))
    })
    .await
}

#[tauri::command]
pub async fn renamer_undo_last(app: AppHandle, state: State<'_, AppState>) -> Result<RenamerOutcome, String> {
    let mutation_guard = state.tasks.locks().lock(&LockScope::Global).await?;
    let (db, undo) = (Arc::clone(&state.db), Arc::clone(&state.rename_undo));
    blocking(move || {
        let _mutation_guard = mutation_guard;
        let mut outcome = RenamerOutcome { renames: Vec::new(), error: None, index_sync_failures: 0, skipped: 0 };
        let result = undo.undo_last_report(|done| {
            outcome.renames.push(done.clone());
            sync_renamed_entry(&db, done, &mut outcome.index_sync_failures);
        });
        if let Ok(report) = &result { outcome.skipped = report.skipped; }
        finish_renamer_batch(&app, outcome, result.map(|_| ()))
    })
    .await
}

/// A failure before anything moved is a plain error; after that the caller must
/// still learn which entries moved, so the error travels inside the outcome.
fn finish_renamer_batch(
    app: &AppHandle,
    mut outcome: RenamerOutcome,
    result: Result<(), renamer::ExecuteError>,
) -> Result<RenamerOutcome, String> {
    if !outcome.renames.is_empty() {
        UiEvents(app.clone()).library_updated();
    }
    match result {
        Ok(()) => Ok(outcome),
        Err(error) if outcome.renames.is_empty() => Err(err_string(error)),
        Err(error) => {
            outcome.error = Some(err_string(error));
            Ok(outcome)
        }
    }
}

#[tauri::command]
pub async fn renamer_snapshot_count(state: State<'_, AppState>) -> Result<usize, String> {
    Ok(state.rename_undo.snapshots().map_err(err_string)?.len())
}

#[tauri::command]
pub async fn renamer_list_presets(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    state.rename_presets.list_presets().map_err(err_string)
}

#[tauri::command]
pub async fn renamer_save_preset(
    state: State<'_, AppState>,
    name: String,
    pipeline: renamer::RulePipeline,
) -> Result<(), String> {
    state
        .rename_presets
        .save(&name, &pipeline)
        .map_err(err_string)
}

#[tauri::command]
pub async fn renamer_load_preset(
    state: State<'_, AppState>,
    name: String,
) -> Result<Option<renamer::RulePipeline>, String> {
    state.rename_presets.load(&name).map_err(err_string)
}

#[tauri::command]
pub async fn renamer_delete_preset(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.rename_presets.delete(&name).map_err(err_string)
}

#[tauri::command]
pub async fn renamer_auto_save_pipeline(
    state: State<'_, AppState>,
    pipeline: renamer::RulePipeline,
) -> Result<(), String> {
    state.rename_presets.auto_save(&pipeline).map_err(err_string)
}

#[tauri::command]
pub async fn renamer_auto_load_pipeline(
    state: State<'_, AppState>,
) -> Result<Option<renamer::RulePipeline>, String> {
    state.rename_presets.auto_load().map_err(err_string)
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

#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<DirectoryEntryDto>, String> {
    sula_core::files::list_directory(path).await
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

const MAX_RENAMER_FILES: usize = 5_000;

fn collect_paths_into(
    path: &std::path::Path,
    out: &mut Vec<renamer::FileEntry>,
) -> std::io::Result<()> {
    if path.is_file() {
        out.push(renamer::FileEntry::new(path));
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if child.is_dir() {
            collect_paths_into(&child, out)?;
        } else if child.is_file() {
            out.push(renamer::FileEntry::new(&child));
        }
        if out.len() > MAX_RENAMER_FILES {
            break;
        }
    }
    Ok(())
}


fn task_scope(label: &str, ids: &[String]) -> Option<String> {
    let mut ids = ids.to_vec(); ids.sort(); ids.dedup();
    Some(format!("{label}:{}", serde_json::to_string(&ids).expect("string list")))
}
