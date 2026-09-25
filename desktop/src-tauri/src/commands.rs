use std::sync::atomic::Ordering;
use std::sync::Arc;

use media_core::{
    AppDatabase, Library, MediaItem, MediaMetaSummary, MediaMetadata, MediaType, ScrapedStatus, ShowListStats,
    TvEpisode, TvSeason,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app::cleanup::SystemTrash;
use crate::app::library::RefreshService;
use crate::app::locks::LockScope;
use crate::app::organize::{OrganizeService, ShowMergePair, ShowMergePlanDto};
use crate::app::scrape::{localized_error, ScrapeService, ScrapeSettings};
use crate::app::{blocking, err_string, Events};
use crate::config::AppConfig;
use crate::state::{AppState, AppStatusDto, CratesDto};
use crate::task_queue::{TaskKind, TaskSnapshot, TaskStatus};

async fn ui_locale(state: &State<'_, AppState>) -> String {
    state.config.lock().await.config.ui_locale.clone()
}

impl Events for AppHandle {
    fn library_updated(&self) {
        let _ = self.emit("library-updated", ());
    }
}

/// Lock scope of the libraries the given items belong to.
async fn items_scope(db: &Arc<AppDatabase>, item_ids: &[String]) -> Result<LockScope, String> {
    let (db, ids) = (Arc::clone(db), item_ids.to_vec());
    blocking(move || LockScope::for_items(&db, &ids)).await
}

#[tauri::command]
pub async fn app_status(state: State<'_, AppState>) -> Result<AppStatusDto, String> {
    let library_count = state.db.library_count().map_err(err_string)?;
    let config = state.config.lock().await.config.clone();
    Ok(AppStatusDto {
        app_name: "Sula".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: state.data_dir.display().to_string(),
        database_path: state.db.path().display().to_string(),
        library_count,
        config,
        crates: CratesDto {
            media_core: "media-core".into(),
            scraper_kit: scraper_kit::crate_name().into(),
            renamer: renamer::crate_name().into(),
        },
    })
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
    app.library_updated();
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
    app.library_updated();
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
    app.library_updated();
    Ok(())
}

#[tauri::command]
pub async fn path_is_dir(path: String) -> Result<bool, String> {
    blocking(move || path_is_dir_sync(path)).await
}

fn path_is_dir_sync(path: String) -> Result<bool, String> {
    Ok(std::path::Path::new(path.trim()).is_dir())
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
    app.library_updated();
    Ok(library)
}

#[tauri::command]
pub async fn clear_thumbnail_cache(state: State<'_, AppState>) -> Result<usize, String> {
    let (thumbs, avatars) = (Arc::clone(&state.thumbs), Arc::clone(&state.avatars));
    blocking(move || clear_thumbnail_cache_sync(&thumbs, &avatars)).await
}

fn clear_thumbnail_cache_sync(thumbs: &media_core::ThumbnailCache, avatars: &media_core::AvatarCache) -> Result<usize, String> {
    let thumbs = thumbs.clear_all().map_err(err_string)?;
    let avatars = avatars.clear().map_err(err_string)?;
    Ok(thumbs + avatars)
}

const MAX_AVATAR_BYTES: usize = 10 * 1024 * 1024;

fn is_public_http_url(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_ascii_lowercase();
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast())
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            let unique_local = (ip.segments()[0] & 0xfe00) == 0xfc00;
            let link_local = (ip.segments()[0] & 0xffc0) == 0xfe80;
            !(ip.is_loopback() || ip.is_unspecified() || unique_local || link_local)
                && ip.to_ipv4_mapped().is_none_or(|v4| !(v4.is_loopback() || v4.is_private()))
        }
        Err(_) => host != "localhost" && !host.ends_with(".localhost") && !host.ends_with(".local"),
    }
}

#[tauri::command]
pub async fn resolve_actor_avatar(
    state: State<'_, AppState>,
    url: String,
) -> Result<Option<String>, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Ok(None);
    }
    if let Some(cached) = state.avatars.cached_path(&url) {
        return Ok(Some(cached.display().to_string()));
    }
    // The URL comes from scraped metadata: only fetch public http(s) hosts, also
    // after redirects, and cap the body so a hostile source can't fill the disk.
    let parsed = reqwest::Url::parse(&url).map_err(err_string)?;
    if !is_public_http_url(&parsed) {
        return Ok(None);
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !is_public_http_url(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(err_string)?;
    let mut response = client.get(parsed).send().await.map_err(|e| e.without_url().to_string())?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url().to_string())? {
        if bytes.len() + chunk.len() > MAX_AVATAR_BYTES {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk);
    }
    let avatars = Arc::clone(&state.avatars);
    let stored = tokio::task::spawn_blocking(move || avatars.store(&url, &bytes))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err_string)?;
    Ok(Some(stored.display().to_string()))
}

#[tauri::command]
pub async fn list_media_items(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<Vec<MediaItem>, String> {
    let db = Arc::clone(&state.db);
    blocking(move || list_media_items_sync(&db, &library_id)).await
}

fn list_media_items_sync(db: &media_core::AppDatabase, library_id: &str) -> Result<Vec<MediaItem>, String> {
    db.list_media_items(library_id).map_err(err_string)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaListPayload {
    pub next_offset: Option<u32>,
    pub items: Vec<MediaItem>,
    pub metadata: Vec<MediaMetaSummary>,
    pub show_stats: Vec<ShowListStats>,
}

#[tauri::command]
pub async fn list_media_page(
    state: State<'_, AppState>, library_id: String, offset: Option<u32>, limit: Option<u32>,
) -> Result<MediaListPayload, String> {
    let db = Arc::clone(&state.db);
    blocking(move || list_media_page_sync(&db, &library_id, offset, limit)).await
}

fn list_media_page_sync(db: &media_core::AppDatabase, library_id: &str, offset: Option<u32>, limit: Option<u32>) -> Result<MediaListPayload, String> {
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(256).clamp(1, 512);
    let items = db.list_media_items_page(library_id, offset, limit).map_err(err_string)?;
    let ids = serde_json::to_string(&items.iter().map(|i| &i.id).collect::<Vec<_>>()).map_err(err_string)?;
    let metadata = db.list_metadata_summaries_for_ids(&ids).map_err(err_string)?;
    let show_stats = db.list_show_stats_for_ids(&ids).map_err(err_string)?;
    let next_offset = if items.len() == limit as usize { offset.checked_add(limit) } else { None };
    Ok(MediaListPayload { items, metadata, show_stats, next_offset })
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
        crate::app::organize::merge_planned_shows(&db, &pairs, &templates, &app)
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDetailDto {
    pub item: MediaItem,
    pub metadata: Option<MediaMetadata>,
    pub seasons: Vec<TvSeason>,
    pub episodes: Vec<TvEpisode>,
}

#[tauri::command]
pub async fn get_media_detail(
    state: State<'_, AppState>,
    id: String,
) -> Result<MediaDetailDto, String> {
    let db = Arc::clone(&state.db);
    blocking(move || get_media_detail_sync(&db, id)).await
}

fn get_media_detail_sync(db: &media_core::AppDatabase, id: String) -> Result<MediaDetailDto, String> {
    let item = db
        .get_media_item(&id)
        .map_err(err_string)?
        .ok_or_else(|| format!("media item not found: {id}"))?;
    let metadata = db.fetch_metadata(&id).map_err(err_string)?;
    let (seasons, episodes) = if matches!(
        item.media_type,
        MediaType::TvShow | MediaType::Anime
    ) {
        let seasons = db.fetch_seasons(&id).map_err(err_string)?;
        let mut episodes = Vec::new();
        for season in &seasons {
            episodes.extend(db.fetch_episodes(&season.id).map_err(err_string)?);
        }
        (seasons, episodes)
    } else {
        (Vec::new(), Vec::new())
    };
    Ok(MediaDetailDto {
        item,
        metadata,
        seasons,
        episodes,
    })
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
    let width = width.unwrap_or(media_core::POSTER_THUMB_WIDTH);
    let height = height.unwrap_or(media_core::POSTER_THUMB_HEIGHT);
    let allow_fallbacks = allow_fallbacks.unwrap_or(true);
    let (db, thumbs) = (Arc::clone(&state.db), Arc::clone(&state.thumbs));
    blocking(move || {
        let Some(source) = media_core::ThumbnailCache::resolve_poster_source_with_fallbacks(
            &folder_path,
            &poster_path,
            allow_fallbacks,
        ) else {
            return Ok(None);
        };
        // Thumbnails land in a webview-readable cache: only images inside a library.
        let canonical = media_core::scanner::canonicalize_lossy(std::path::Path::new(&source));
        let inside_library = db.list_libraries().map_err(err_string)?.iter().any(|library| {
            let root = media_core::scanner::canonicalize_lossy(std::path::Path::new(&library.root_path));
            media_core::db::path_rooted_under(&canonical, &root)
        });
        if !inside_library {
            return Ok(None);
        }
        match thumbs.ensure(&source, width, height) {
            // Return cache file path; frontend uses convertFileSrc (faster than base64 IPC).
            Ok(path) => Ok(Some(path.display().to_string())),
            Err(media_core::ThumbnailError::Missing(_)) => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    })
    .await
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
    app: AppHandle,
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskSnapshot>, String> {
    Ok(state.tasks.list().await)
}

#[tauri::command]
pub async fn enqueue_smoke_task(
    app: AppHandle,
    state: State<'_, AppState>,
    title: Option<String>,
) -> Result<TaskSnapshot, String> {
    let snapshot = state
        .tasks
        .enqueue_smoke(title.unwrap_or_else(|| "M0 smoke task".into()))
        .await;
    watch_task(app, Arc::clone(&state.tasks), snapshot.id.clone());
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn scrape_items(
    app: AppHandle,
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn rescrape_items(
    app: AppHandle,
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn scrape_season(
    app: AppHandle,
    state: State<'_, AppState>,
    media_item_id: String,
    season_number: i32,
) -> Result<TaskSnapshot, String> {
    let config = state.config.lock().await.config.clone();
    let service = scrape_service(&state, &config);
    let snapshot = state.tasks.enqueue_scoped(format!("Season {season_number}"), TaskKind::Scrape,
        Some(media_item_id.clone()), task_scope("season", &[media_item_id.clone(), season_number.to_string()]), None,
        move |handle| Box::pin(async move { service.scrape_season(&media_item_id, season_number, &handle).await })).await;
    watch_task(app, Arc::clone(&state.tasks), snapshot.id.clone());
    Ok(snapshot)
}

#[tauri::command]
pub async fn apply_rename_templates(
    app: AppHandle,
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub async fn organize_season_folders(
    app: AppHandle,
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
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
    app: AppHandle,
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
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
        crate::app::cleanup::delete_media_items(&db, &item_ids, also_trash, &SystemTrash, &app)
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
    app: AppHandle,
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

    watch_task(app.clone(), Arc::clone(&state.tasks), snapshot.id.clone());
    let _ = app.emit("task-updated", &snapshot);
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

fn watch_task(app: AppHandle, tasks: Arc<crate::task_queue::TaskQueue>, id: String) {
    // A deduplicated enqueue returns an already-watched task; one watcher per task.
    static WATCHED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::OnceLock::new();
    let watched = WATCHED.get_or_init(Default::default);
    if !watched.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone()) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut last_fingerprint = String::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            if let Some(current) = tasks.get(&id).await {
                let fingerprint = task_fingerprint(&current);
                if fingerprint != last_fingerprint {
                    last_fingerprint = fingerprint;
                    let _ = app.emit("task-updated", &current);
                }
                if matches!(
                    current.status,
                    TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
                ) {
                    let _ = app.emit("library-updated", ());
                    if current.status == TaskStatus::Completed {
                        if let Some(state) = app.try_state::<AppState>() {
                            schedule_thumb_warm(
                                Arc::clone(&state.db),
                                Arc::clone(&state.thumbs),
                                current.kind,
                                current.target_id.clone(),
                            );
                        }
                    }
                    break;
                }
            } else {
                break;
            }
        }
        watched.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    });
}

fn schedule_thumb_warm(
    db: Arc<media_core::AppDatabase>,
    thumbs: Arc<media_core::ThumbnailCache>,
    kind: TaskKind,
    target_id: Option<String>,
) {
    let Some(target_id) = target_id else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let warm = tokio::task::spawn_blocking(move || match kind {
            TaskKind::Refresh | TaskKind::BatchScrape => {
                warm_library_posters(&db, &thumbs, &target_id)
            }
            TaskKind::Scrape | TaskKind::ManualMatch => {
                warm_item_poster(&db, &thumbs, &target_id)
            }
            _ => 0,
        })
        .await;
        match warm {
            Ok(n) if n > 0 => tracing::info!(count = n, "poster thumbs warmed"),
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %err, "poster warm task join failed"),
        }
    });
}

fn warm_library_posters(
    db: &media_core::AppDatabase,
    thumbs: &media_core::ThumbnailCache,
    library_id: &str,
) -> usize {
    let Ok(items) = db.list_media_items(library_id) else {
        return 0;
    };
    let Ok(metas) = db.list_metadata_summaries(library_id) else {
        return 0;
    };
    let mut by_id = std::collections::HashMap::new();
    for item in &items {
        by_id.insert(item.id.clone(), item);
    }
    let mut jobs = Vec::new();
    for meta in metas {
        let Some(poster) = meta.poster_path.as_deref().filter(|p| !p.is_empty()) else {
            continue;
        };
        let Some(item) = by_id.get(&meta.media_item_id) else {
            continue;
        };
        let Some(source) =
            media_core::ThumbnailCache::resolve_poster_source(&item.folder_path, poster)
        else {
            continue;
        };
        jobs.push(source);
    }
    if jobs.is_empty() {
        return 0;
    }

    use std::sync::atomic::{AtomicUsize, Ordering};
    let warmed = AtomicUsize::new(0);
    let workers = 4usize.min(jobs.len());
    let chunk = (jobs.len() + workers - 1) / workers;
    std::thread::scope(|scope| {
        for piece in jobs.chunks(chunk.max(1)) {
            let piece = piece.to_vec();
            let warmed = &warmed;
            scope.spawn(move || {
                for source in &piece {
                    if thumbs.ensure_poster_thumb(source).is_ok() {
                        warmed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    warmed.load(Ordering::Relaxed)
}

fn warm_item_poster(
    db: &media_core::AppDatabase,
    thumbs: &media_core::ThumbnailCache,
    item_id: &str,
) -> usize {
    let Ok(Some(item)) = db.get_media_item(item_id) else {
        return 0;
    };
    let Ok(Some(meta)) = db.fetch_metadata(item_id) else {
        return 0;
    };
    let Some(poster) = meta.poster_path.as_deref().filter(|p| !p.is_empty()) else {
        return 0;
    };
    let Some(source) =
        media_core::ThumbnailCache::resolve_poster_source(&item.folder_path, poster)
    else {
        return 0;
    };
    if thumbs.ensure_poster_thumb(&source).is_ok() {
        1
    } else {
        0
    }
}

fn task_fingerprint(task: &TaskSnapshot) -> String {
    serde_json::to_string(task).unwrap_or_default()
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
        app.library_updated();
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntryDto {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub file_size: Option<u64>,
    pub modified_at: Option<String>,
}

#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<DirectoryEntryDto>, String> {
    blocking(move || list_directory_sync(path)).await
}

fn list_directory_sync(path: String) -> Result<Vec<DirectoryEntryDto>, String> {
    let root = std::path::PathBuf::from(&path);
    if !root.is_dir() {
        return Err("path is not a directory".into());
    }
    let mut out = Vec::new();
    let rd = std::fs::read_dir(&root).map_err(err_string)?;
    for entry in rd {
        let entry = entry.map_err(err_string)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let child = entry.path();
        let meta = entry.metadata().ok();
        let is_directory = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let file_size = meta
            .as_ref()
            .filter(|m| m.is_file())
            .map(|m| m.len());
        let modified_at = meta
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Local> = t.into();
                dt.format("%Y-%m-%d").to_string()
            });
        out.push(DirectoryEntryDto {
            name,
            path: child.to_string_lossy().into_owned(),
            is_directory,
            file_size,
            modified_at,
        });
    }
    out.sort_by(|a, b| match (a.is_directory, b.is_directory) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(out)
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

#[cfg(test)]
mod tests {
    use super::is_public_http_url;

    #[test]
    fn avatar_urls_must_be_public_http() {
        let ok = |raw: &str| is_public_http_url(&reqwest::Url::parse(raw).unwrap());
        assert!(ok("https://image.tmdb.org/t/p/w185/a.jpg"));
        assert!(ok("http://lain.bgm.tv/pic/crt/l/a.jpg"));
        for bad in ["file:///etc/passwd", "http://localhost:8080/x", "http://127.0.0.1/x", "http://10.0.0.5/x",
            "http://192.168.1.2/x", "http://169.254.169.254/latest", "http://[::1]/x", "http://[fd00::1]/x",
            "http://nas.local/x", "http://[::ffff:127.0.0.1]/x"] {
            assert!(!ok(bad), "{bad}");
        }
    }
}
