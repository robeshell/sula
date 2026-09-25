use std::sync::atomic::Ordering;
use std::sync::Arc;

use media_core::{
    Library, MediaItem, MediaMetaSummary, MediaMetadata, MediaType, ScrapedStatus, ShowListStats,
    TvEpisode, TvSeason,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::AppConfig;
use crate::state::{AppState, AppStatusDto, CratesDto};
use crate::task_queue::{TaskKind, TaskProgress, TaskSnapshot, TaskStatus};

async fn ui_locale(state: &State<'_, AppState>) -> String {
    state.config.lock().await.config.ui_locale.clone()
}

fn loc_progress(locale: &str, name: &str) -> String {
    match name {
        "scan.checking" => crate::ui_i18n::t(locale, "prog.checking"),
        "scan.unchanged" => crate::ui_i18n::t(locale, "prog.unchanged"),
        _ => name.to_string(),
    }
}

fn loc_scrape_summary(locale: &str, raw: &str) -> String {
    if let Some((s, u, f)) = scraper_kit::ScrapeSummary::parse_result(raw) {
        return crate::ui_i18n::tf(
            locale,
            "prog.scrapeSummary",
            &[
                ("success", &s.to_string()),
                ("unmatched", &u.to_string()),
                ("failed", &f.to_string()),
            ],
        );
    }
    raw.to_string()
}

fn loc_err(locale: &str, err: String) -> String {
    if err.starts_with("err.") {
        crate::ui_i18n::t(locale, &err)
    } else {
        err
    }
}

/// File moves can be slow on network shares: keep them off the async workers.
async fn auto_rename_after_scrape(
    db: &Arc<media_core::AppDatabase>,
    ids: &[String],
    templates: &renamer::RenameTemplates,
    create_season_folders: bool,
    handle: &crate::task_queue::TaskHandle,
) -> (u32, u32) {
    let (db, ids, templates, handle) = (Arc::clone(db), ids.to_vec(), templates.clone(), handle.clone());
    let total = ids.len() as u32;
    tokio::task::spawn_blocking(move || auto_rename_blocking(&db, &ids, &templates, create_season_folders, &handle))
        .await
        .unwrap_or((0, total))
}

async fn consolidate_after_scrape(
    db: &Arc<media_core::AppDatabase>,
    ids: &[String],
    templates: &renamer::RenameTemplates,
    handle: &crate::task_queue::TaskHandle,
) {
    let (db, ids, templates, handle) = (Arc::clone(db), ids.to_vec(), templates.clone(), handle.clone());
    let _ = tokio::task::spawn_blocking(move || consolidate_blocking(&db, &ids, &templates, &handle)).await;
}

fn auto_rename_blocking(
    db: &media_core::AppDatabase,
    ids: &[String],
    templates: &renamer::RenameTemplates,
    create_season_folders: bool,
    handle: &crate::task_queue::TaskHandle,
) -> (u32, u32) {
    let mut ok = 0u32;
    let mut failed = 0u32;
    for id in ids {
        if handle.is_cancelled() { break; }
        let Some(item) = db.get_media_item(id).ok().flatten() else {
            // May already have been merged into a canonical show.
            continue;
        };
        // Absorb season packs / release folders into the existing series first.
        if let Ok(true) = renamer::consolidate_show_item(db, &item, templates) {
            ok += 1;
            continue;
        }
        let Some(item) = db.get_media_item(id).ok().flatten() else {
            continue;
        };
        match renamer::rename_after_scrape_with_options(
            db,
            &item,
            templates,
            create_season_folders,
        ) {
            Ok(()) => ok += 1,
            Err(err) => {
                failed += 1;
                tracing::warn!(
                    item_id = %id,
                    title = %item.title,
                    error = %err,
                    "auto-rename after scrape failed"
                );
            }
        }
    }
    (ok, failed)
}

fn consolidate_blocking(
    db: &media_core::AppDatabase,
    ids: &[String],
    templates: &renamer::RenameTemplates,
    handle: &crate::task_queue::TaskHandle,
) {
    for id in ids {
        if handle.is_cancelled() { break; }
        let Some(item) = db.get_media_item(id).ok().flatten() else {
            continue;
        };
        if let Err(err) = renamer::consolidate_show_item(db, &item, templates) {
            tracing::warn!(
                item_id = %id,
                title = %item.title,
                error = %err,
                "consolidate duplicate show failed"
            );
        }
    }
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
    let mutation_guard = if exclusions_changed { Some(state.tasks.lock_mutations().await?) } else { None };
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
    Ok(library)
}

#[tauri::command]
pub async fn rename_library(
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
    Ok(library)
}

#[tauri::command]
pub async fn delete_library(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _mutation_guard = state.tasks.lock_mutations().await?;
    state.db.delete_library(&id).map_err(err_string)
}

#[tauri::command]
pub async fn path_is_dir(path: String) -> Result<bool, String> {
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
    let _mutation_guard = state.tasks.lock_mutations().await?;
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
    let _ = app.emit("library-updated", ());
    Ok(library)
}

#[tauri::command]
pub async fn clear_thumbnail_cache(state: State<'_, AppState>) -> Result<usize, String> {
    let thumbs = state.thumbs.clear_all().map_err(err_string)?;
    let avatars = state.avatars.clear().map_err(err_string)?;
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
    state.db.list_media_items(&library_id).map_err(err_string)
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
    let offset = offset.unwrap_or(0);
    let limit = limit.unwrap_or(256).clamp(1, 512);
    let items = state.db.list_media_items_page(&library_id, offset, limit).map_err(err_string)?;
    let ids = serde_json::to_string(&items.iter().map(|i| &i.id).collect::<Vec<_>>()).map_err(err_string)?;
    let metadata = state.db.list_metadata_summaries_for_ids(&ids).map_err(err_string)?;
    let show_stats = state.db.list_show_stats_for_ids(&ids).map_err(err_string)?;
    let next_offset = if items.len() == limit as usize { offset.checked_add(limit) } else { None };
    Ok(MediaListPayload { items, metadata, show_stats, next_offset })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowMergePlanDto {
    pub source_id: String,
    pub source_title: String,
    pub source_folder: String,
    pub target_id: String,
    pub target_title: String,
    pub target_folder: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowMergePair {
    pub source_id: String,
    pub target_id: String,
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
    tokio::task::spawn_blocking(move || {
        let candidates = match (library_id, item_ids) {
            (_, Some(ids)) => ids
                .iter()
                .filter_map(|id| db.get_media_item(id).transpose())
                .collect::<Result<Vec<_>, _>>()
                .map_err(err_string)?,
            (Some(library_id), None) => db.list_media_items(&library_id).map_err(err_string)?,
            (None, None) => Vec::new(),
        };
        let plan = renamer::plan_duplicate_show_merges(&db, &candidates).map_err(err_string)?;
        Ok(plan
            .into_iter()
            .map(|(source, target)| ShowMergePlanDto {
                source_id: source.id,
                source_title: source.title,
                source_folder: source.folder_path,
                target_id: target.id,
                target_title: target.title,
                target_folder: target.folder_path,
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Execute merges the user confirmed from `plan_show_merges`. Each pair is checked
/// again against the current index and skipped if the match changed meanwhile.
#[tauri::command]
pub async fn merge_planned_shows(
    app: AppHandle,
    state: State<'_, AppState>,
    pairs: Vec<ShowMergePair>,
) -> Result<u32, String> {
    let mutation_guard = state.tasks.lock_mutations().await?;
    let templates = state.config.lock().await.config.rename_templates();
    let db = Arc::clone(&state.db);
    let merged = tokio::task::spawn_blocking(move || {
        let _mutation_guard = mutation_guard;
        let mut merged = 0u32;
        for pair in pairs {
            if renamer::merge_planned_show(&db, &pair.source_id, &pair.target_id, &templates).map_err(err_string)? {
                merged += 1;
            }
        }
        Ok::<_, String>(merged)
    })
    .await
    .map_err(|e| e.to_string())??;
    if merged > 0 {
        let _ = app.emit("library-updated", ());
    }
    Ok(merged)
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
    let item = state
        .db
        .get_media_item(&id)
        .map_err(err_string)?
        .ok_or_else(|| format!("media item not found: {id}"))?;
    let metadata = state.db.fetch_metadata(&id).map_err(err_string)?;
    let (seasons, episodes) = if matches!(
        item.media_type,
        MediaType::TvShow | MediaType::Anime
    ) {
        let seasons = state.db.fetch_seasons(&id).map_err(err_string)?;
        let mut episodes = Vec::new();
        for season in &seasons {
            episodes.extend(state.db.fetch_episodes(&season.id).map_err(err_string)?);
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
    let Some(source) = media_core::ThumbnailCache::resolve_poster_source_with_fallbacks(
        &folder_path,
        &poster_path,
        allow_fallbacks,
    ) else {
        return Ok(None);
    };
    // Thumbnails land in a webview-readable cache: only images inside a library.
    let canonical = media_core::scanner::canonicalize_lossy(std::path::Path::new(&source));
    let inside_library = state.db.list_libraries().map_err(err_string)?.iter().any(|library| {
        let root = media_core::scanner::canonicalize_lossy(std::path::Path::new(&library.root_path));
        media_core::db::path_rooted_under(&canonical, &root)
    });
    if !inside_library {
        return Ok(None);
    }
    let thumbs = Arc::clone(&state.thumbs);
    let result = tokio::task::spawn_blocking(move || thumbs.ensure(&source, width, height))
        .await
        .map_err(|e| e.to_string())?;
    match result {
        // Return cache file path; frontend uses convertFileSrc (faster than base64 IPC).
        Ok(path) => Ok(Some(path.display().to_string())),
        Err(media_core::ThumbnailError::Missing(_)) => Ok(None),
        Err(err) => Err(err.to_string()),
    }
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
    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Refresh, item_ids.first().cloned(), task_scope("items", &item_ids), move |handle| {
            let locale = locale.clone();
            Box::pin(async move {
                let excluded = config_store.lock().await.config.scan_excluded_folders.clone();
                let total = item_ids.len() as u32;
                handle
                    .update_progress(TaskProgress {
                        completed: 0,
                        total,
                        current: crate::ui_i18n::t(&locale, "prog.refreshing"),
                        stage_key: Some("refreshItems".into()),
                    })
                    .await;
                let cancel = handle.cancellation_flag();
                let report = tokio::task::spawn_blocking(move || {
                    media_core::refresh_items_cancellable(&db, &item_ids, &excluded, &cancel)
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(err_string)?;
                if handle.is_cancelled() {
                    return Err("cancelled".into());
                }
                handle
                    .update_progress(TaskProgress {
                        completed: total,
                        total,
                        current: crate::ui_i18n::tf(
                            &locale,
                            "prog.refreshDone",
                            &[
                                ("ok", &report.refreshed.to_string()),
                                ("removed", &report.removed.to_string()),
                            ],
                        ),
                        stage_key: Some("refreshItems".into()),
                    })
                    .await;
                Ok(())
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
    let locale = {
        let cfg = state.config.lock().await;
        cfg.config.ui_locale.clone()
    };
    let config_store = Arc::clone(&state.config);
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(&locale, "task.refreshLib", &[("name", &library.name)]);
    let target_id = Some(library_id.clone());
    let library_id_for_merge = library_id.clone();

    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Refresh, target_id, task_scope("library", &[library_id.clone()]), move |handle| {
            let locale = locale.clone();
            Box::pin(async move {
                let library_for_scan = db.get_library(&library.id).map_err(err_string)?
                    .ok_or_else(|| "library removed before refresh".to_string())?;
                let config = config_store.lock().await.config.clone();
                let excluded_folders = config.scan_excluded_folders.clone();
                let rename_templates = config.rename_templates();
                let media_type = library_for_scan.media_type;
                let db_scan = Arc::clone(&db);
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::unbounded_channel::<media_core::ScanProgress>();
                let cancel = handle.cancellation_flag();
                let scan = tokio::task::spawn_blocking(move || {
                    let mut last_emit = 0u32;
                    let mut saw_check = false;
                    media_core::refresh_library_cancellable(&db_scan, &library_for_scan, &excluded_folders, |p| {
                        let is_check = p.discovered_count == 0
                            && (p.current_name == "scan.checking"
                                || p.current_name == "scan.unchanged"
                                || p.current_name.starts_with("检查")
                                || p.current_name.starts_with("目录")
                                || p.current_name.starts_with("scan."));
                        if is_check {
                            if !saw_check || p.current_name == "scan.unchanged" || p.current_name.starts_with("目录无变更") {
                                saw_check = true;
                                let _ = progress_tx.send(p);
                            }
                            return;
                        }
                        if p.discovered_count == 1
                            || p.discovered_count.saturating_sub(last_emit) >= 25
                        {
                            last_emit = p.discovered_count;
                            let _ = progress_tx.send(p);
                        }
                    }, &cancel)
                });

                while let Some(p) = progress_rx.recv().await {
                    if handle.is_cancelled() {
                        break;
                    }
                    let stage_key = if p.discovered_count == 0
                        && (p.current_name == "scan.checking"
                            || p.current_name == "scan.unchanged"
                            || p.current_name.starts_with("检查")
                            || p.current_name.starts_with("目录")
                            || p.current_name.starts_with("scan."))
                    {
                        "checkDirectories"
                    } else {
                        "scanFiles"
                    };
                    handle
                        .update_progress(TaskProgress {
                            completed: p.discovered_count,
                            total: 0,
                            current: loc_progress(&locale, &p.current_name),
                            stage_key: Some(stage_key.into()),
                        })
                        .await;
                }

                let report = scan
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;

                if handle.is_cancelled() {
                    return Err("cancelled".into());
                }

                // Same TMDB season packs → merge into the canonical show (also on early-exit).
                let mut merged_n = 0usize;
                if matches!(
                    media_type,
                    media_core::MediaType::TvShow | media_core::MediaType::Anime
                ) {
                    let db_merge = Arc::clone(&db);
                    let templates = rename_templates.clone();
                    let lib_id = library_id_for_merge.clone();
                    merged_n = tokio::task::spawn_blocking(move || {
                        renamer::consolidate_library_duplicate_shows(&db_merge, &lib_id, &templates)
                    })
                    .await
                    .map_err(|e| e.to_string())?
                    .unwrap_or(0);
                }

                if merged_n > 0 {
                    handle
                        .update_progress(TaskProgress {
                            completed: merged_n as u32,
                            total: merged_n as u32,
                            current: crate::ui_i18n::tf(
                                &locale,
                                "prog.mergedShows",
                                &[("n", &merged_n.to_string())],
                            ),
                            stage_key: Some("saveResults".into()),
                        })
                        .await;
                } else if report.early_exit {
                    handle
                        .update_progress(TaskProgress {
                            completed: 0,
                            total: 0,
                            current: crate::ui_i18n::t(&locale, "prog.unchanged"),
                            stage_key: Some("checkDirectories".into()),
                        })
                        .await;
                } else {
                    handle
                        .update_progress(TaskProgress {
                            completed: report.new_item_count as u32,
                            total: report.new_item_count as u32,
                            current: crate::ui_i18n::tf(&locale, "prog.added", &[("n", &report.new_item_count.to_string())]),
                            stage_key: Some("saveResults".into()),
                        })
                        .await;
                }
                Ok(())
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
    let locale = config.ui_locale.clone();
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(&locale, "task.scrapeAll", &[("name", &library.name)]);
    let options = scrape_options_from_config(&config);
    let target_id = Some(library_id.clone());

    if let Some(existing) = state
        .tasks
        .find_active(TaskKind::BatchScrape, &library_id)
        .await
    {
        let _ = app.emit("task-updated", &existing);
        return Ok(existing);
    }

    let snapshot = state
        .tasks
        .enqueue(title, TaskKind::BatchScrape, target_id, move |handle| {
            Box::pin(async move {
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::unbounded_channel::<scraper_kit::ScrapeProgress>();
                let db_job = Arc::clone(&db);
                let cancel = handle.cancellation_flag();
                let job = tokio::spawn(async move {
                    scraper_kit::scrape_library_cancellable(db_job, &library_id, options, cancel, |p| {
                        let _ = progress_tx.send(p);
                    })
                    .await
                });
                while let Some(p) = progress_rx.recv().await {
                    if handle.is_cancelled() {
                        break;
                    }
                    handle
                        .update_progress(TaskProgress {
                            completed: p.completed,
                            total: p.total,
                            current: p.current,
                            stage_key: Some(p.stage_key),
                        })
                        .await;
                }
                let summary = job
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| loc_err(&locale, e))?;
                handle.record_scrape_result(&summary).await;
                if handle.is_cancelled() { return Err("cancelled".into()); }
                let success_ids = summary.success_ids.clone();
                handle
                    .update_progress(TaskProgress {
                        completed: summary.success_ids.len() as u32
                            + summary.unmatched
                            + summary.failed,
                        total: summary.success_ids.len() as u32
                            + summary.unmatched
                            + summary.failed,
                        current: loc_scrape_summary(&locale, &summary.format_result()),
                        stage_key: Some("saveResults".into()),
                    })
                    .await;
                if !success_ids.is_empty() {
                    let templates = config.rename_templates();
                    if config.rename_auto_after_scrape {
                        handle
                            .update_progress(TaskProgress {
                                completed: success_ids.len() as u32,
                                total: success_ids.len() as u32,
                                current: crate::ui_i18n::t(&locale, "prog.autoRename"),
                                stage_key: Some("rename".into()),
                            })
                            .await;
                        let (renamed, rename_failed) = auto_rename_after_scrape(
                            &db,
                            &success_ids,
                            &templates,
                            config.rename_create_season_folders,
                            &handle,
                        ).await;
                        let mut summary_text = loc_scrape_summary(&locale, &summary.format_result());
                        if rename_failed > 0 {
                            summary_text = format!(
                                "{summary_text} · {}",
                                crate::ui_i18n::tf(
                                    &locale,
                                    "prog.autoRenameResult",
                                    &[
                                        ("ok", &renamed.to_string()),
                                        ("failed", &rename_failed.to_string()),
                                    ],
                                )
                            );
                        }
                        handle
                            .update_progress(TaskProgress {
                                completed: success_ids.len() as u32
                                    + summary.unmatched
                                    + summary.failed,
                                total: success_ids.len() as u32
                                    + summary.unmatched
                                    + summary.failed,
                                current: summary_text,
                                stage_key: Some("saveResults".into()),
                            })
                            .await;
                    } else {
                        consolidate_after_scrape(&db, &success_ids, &templates, &handle).await;
                    }
                }
                Ok(())
            })
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
    let locale = config.ui_locale.clone();
    let options = scrape_options_from_config(&config);
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(&locale, "task.scrapeN", &[("n", &item_ids.len().to_string())]);

    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Scrape, item_ids.first().cloned(), task_scope("items", &item_ids), move |handle| {
            Box::pin(async move {
                let mut summary = scraper_kit::ScrapeSummary::default();
                let total = item_ids.len() as u32;
                for (idx, id) in item_ids.into_iter().enumerate() {
                    if handle.is_cancelled() {
                        return Err("cancelled".into());
                    }
                    let item = db
                        .get_media_item(&id)
                        .map_err(err_string)?
                        .ok_or_else(|| format!("media item not found: {id}"))?;
                    handle
                        .update_progress(TaskProgress {
                            completed: idx as u32,
                            total,
                            current: item.title.clone(),
                            stage_key: Some("matching".into()),
                        })
                        .await;
                    match handle.run_cancellable(scraper_kit::scrape_item(&db, &item, &options))
                        .await
                    {
                        Ok(scraper_kit::ScrapeItemOutcome::Matched) => {
                            summary.success_ids.push(id);
                        }
                        Ok(scraper_kit::ScrapeItemOutcome::Unmatched) => {
                            summary.unmatched += 1;
                        }
                        Ok(scraper_kit::ScrapeItemOutcome::Failed) => {
                            summary.failed += 1;
                        }
                        Err(error) => {
                            if handle.is_cancelled() { return Err(error); }
                            summary.failed += 1;
                            db.update_status(&id, media_core::ScrapedStatus::Partial, Some(&error)).map_err(err_string)?;
                        }
                    }
                    handle.record_scrape_result(&summary).await;
                }
                handle
                    .update_progress(TaskProgress {
                        completed: total,
                        total,
                        current: loc_scrape_summary(&locale, &summary.format_result()),
                        stage_key: Some("saveResults".into()),
                    })
                    .await;
                if !summary.success_ids.is_empty() {
                    let templates = config.rename_templates();
                    if config.rename_auto_after_scrape {
                        handle
                            .update_progress(TaskProgress {
                                completed: total,
                                total,
                                current: crate::ui_i18n::t(&locale, "prog.autoRename"),
                                stage_key: Some("rename".into()),
                            })
                            .await;
                        let (renamed, rename_failed) = auto_rename_after_scrape(
                            &db,
                            &summary.success_ids,
                            &templates,
                            config.rename_create_season_folders,
                            &handle,
                        ).await;
                        let mut summary_text = loc_scrape_summary(&locale, &summary.format_result());
                        if rename_failed > 0 {
                            summary_text = format!(
                                "{summary_text} · {}",
                                crate::ui_i18n::tf(
                                    &locale,
                                    "prog.autoRenameResult",
                                    &[
                                        ("ok", &renamed.to_string()),
                                        ("failed", &rename_failed.to_string()),
                                    ],
                                )
                            );
                        }
                        handle
                            .update_progress(TaskProgress {
                                completed: total,
                                total,
                                current: summary_text,
                                stage_key: Some("saveResults".into()),
                            })
                            .await;
                    } else {
                        consolidate_after_scrape(&db, &summary.success_ids, &templates, &handle).await;
                    }
                }
                Ok(())
            })
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
    let locale = config.ui_locale.clone();
    let options = scrape_options_from_config(&config);
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(&locale, "task.rescrapeN", &[("n", &scraped_ids.len().to_string())]);

    let snapshot = state
        .tasks
        .enqueue_scoped(
            title,
            TaskKind::Rescrape,
            scraped_ids.first().cloned(),
            task_scope("items", &scraped_ids),
            move |handle| {
                Box::pin(async move {
                    let mut summary = scraper_kit::ScrapeSummary::default();
                    let total = scraped_ids.len() as u32;
                    for (idx, id) in scraped_ids.into_iter().enumerate() {
                        if handle.is_cancelled() {
                            return Err("cancelled".into());
                        }
                        let item = db
                            .get_media_item(&id)
                            .map_err(err_string)?
                            .ok_or_else(|| format!("media item not found: {id}"))?;
                        handle
                            .update_progress(TaskProgress {
                                completed: idx as u32,
                                total,
                                current: item.title.clone(),
                                stage_key: Some("matching".into()),
                            })
                            .await;
                        match handle.run_cancellable(scraper_kit::scrape_item(&db, &item, &options))
                            .await
                        {
                            Ok(scraper_kit::ScrapeItemOutcome::Matched) => {
                                summary.success_ids.push(id);
                            }
                            Ok(scraper_kit::ScrapeItemOutcome::Unmatched) => {
                                summary.unmatched += 1;
                            }
                            Ok(scraper_kit::ScrapeItemOutcome::Failed) => {
                                summary.failed += 1;
                            }
                        Err(error) => {
                            if handle.is_cancelled() { return Err(error); }
                            summary.failed += 1;
                            db.update_status(&id, media_core::ScrapedStatus::Partial, Some(&error)).map_err(err_string)?;
                        }
                    }
                    handle.record_scrape_result(&summary).await;
                    }
                    handle
                        .update_progress(TaskProgress {
                            completed: total,
                            total,
                            current: loc_scrape_summary(&locale, &summary.format_result()),
                            stage_key: Some("saveResults".into()),
                        })
                        .await;
                    if !summary.success_ids.is_empty() {
                        let templates = config.rename_templates();
                        if config.rename_auto_after_scrape {
                            handle
                                .update_progress(TaskProgress {
                                    completed: total,
                                    total,
                                    current: crate::ui_i18n::t(&locale, "prog.autoRename"),
                                    stage_key: Some("rename".into()),
                                })
                                .await;
                            let (renamed, rename_failed) = auto_rename_after_scrape(
                                &db,
                                &summary.success_ids,
                                &templates,
                                config.rename_create_season_folders,
                                &handle,
                            ).await;
                            let mut summary_text =
                                loc_scrape_summary(&locale, &summary.format_result());
                            if rename_failed > 0 {
                                summary_text = format!(
                                    "{summary_text} · {}",
                                    crate::ui_i18n::tf(
                                        &locale,
                                        "prog.autoRenameResult",
                                        &[
                                            ("ok", &renamed.to_string()),
                                            ("failed", &rename_failed.to_string()),
                                        ],
                                    )
                                );
                            }
                            handle
                                .update_progress(TaskProgress {
                                    completed: total,
                                    total,
                                    current: summary_text,
                                    stage_key: Some("saveResults".into()),
                                })
                                .await;
                        } else {
                            consolidate_after_scrape(&db, &summary.success_ids, &templates, &handle).await;
                        }
                    }
                    Ok(())
                })
            },
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
    let options = scrape_options_from_config(&config);
    let db = Arc::clone(&state.db);
    let snapshot = state.tasks.enqueue_scoped(format!("Season {season_number}"), TaskKind::Scrape,
        Some(media_item_id.clone()), task_scope("season", &[media_item_id.clone(), season_number.to_string()]), move |handle| Box::pin(async move {
            let item = db.get_media_item(&media_item_id).map_err(err_string)?.ok_or_else(|| "media item not found".to_string())?;
            handle.run_cancellable(scraper_kit::scrape_season(&db, &item, season_number, &options)).await
        })).await;
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
    let locale = config.ui_locale.clone();
    let templates = config.rename_templates();
    let create_season_folders = config.rename_create_season_folders;
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(&locale, "task.renameN", &[("n", &item_ids.len().to_string())]);

    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::Rename, item_ids.first().cloned(), task_scope("items", &item_ids), move |handle| {
            Box::pin(async move {
                let total = item_ids.len() as u32;
                for (idx, id) in item_ids.into_iter().enumerate() {
                    if handle.is_cancelled() {
                        return Err("cancelled".into());
                    }
                    let item = db
                        .get_media_item(&id)
                        .map_err(err_string)?
                        .ok_or_else(|| format!("media item not found: {id}"))?;
                    handle
                        .update_progress(TaskProgress {
                            completed: idx as u32,
                            total,
                            current: item.title.clone(),
                            stage_key: Some("rename".into()),
                        })
                        .await;
                    // Season packs that share TMDB with an existing show are absorbed first.
                    if let Err(error) = renamer::consolidate_show_item(&db, &item, &templates) {
                        tracing::warn!(item_id = %id, %error, "consolidate before rename failed");
                    }
                    let Some(item) = db
                        .get_media_item(&id)
                        .map_err(err_string)?
                    else {
                        continue;
                    };
                    renamer::rename_after_scrape_with_options(
                        &db,
                        &item,
                        &templates,
                        create_season_folders,
                    )
                    .map_err(|e| e.to_string())?;
                }
                handle
                    .update_progress(TaskProgress {
                        completed: total,
                        total,
                        current: crate::ui_i18n::tf(&locale, "prog.renamed", &[("n", &total.to_string())]),
                        stage_key: Some("rename".into()),
                    })
                    .await;
                Ok(())
            })
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

    let (templates, locale) = {
        let cfg = state.config.lock().await;
        (cfg.config.rename_templates(), cfg.config.ui_locale.clone())
    };
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(&locale, "task.organizeN", &[("n", &targets.len().to_string())]);

    let snapshot = state
        .tasks
        .enqueue_scoped(
            title,
            TaskKind::Organize,
            targets.first().cloned(),
            task_scope("items", &targets),
            move |handle| {
                Box::pin(async move {
                    let total = targets.len() as u32;
                    for (idx, id) in targets.into_iter().enumerate() {
                        if handle.is_cancelled() {
                            return Err("cancelled".into());
                        }
                        let item = db
                            .get_media_item(&id)
                            .map_err(err_string)?
                            .ok_or_else(|| format!("media item not found: {id}"))?;
                        handle
                            .update_progress(TaskProgress {
                                completed: idx as u32,
                                total,
                                current: item.title.clone(),
                                stage_key: Some("organize".into()),
                            })
                            .await;
                        renamer::organize_season_folders(&db, &item, &templates)
                            .map_err(|e| e.to_string())?;
                    }
                    handle
                        .update_progress(TaskProgress {
                            completed: total,
                            total,
                            current: crate::ui_i18n::tf(&locale, "prog.organized", &[("n", &total.to_string())]),
                            stage_key: Some("organize".into()),
                        })
                        .await;
                    Ok(())
                })
            },
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
    let snapshot = state
        .tasks
        .enqueue(title, TaskKind::Cleanup, None, move |handle| {
            let locale = locale.clone();
            Box::pin(async move {
                let total = paths.len() as u32;
                handle
                    .update_progress(TaskProgress {
                        completed: 0,
                        total,
                        current: crate::ui_i18n::t(&locale, "prog.cleaning"),
                        stage_key: Some("cleanup".into()),
                    })
                    .await;
                let cancel = handle.cancellation_flag();
                let removed = tokio::task::spawn_blocking(move || -> Result<usize, String> {
                    let mut ids = Vec::new();
                    for library in db.list_libraries().map_err(err_string)? {
                        ids.extend(db.list_media_items(&library.id).map_err(err_string)?.into_iter().map(|i| i.id));
                    }
                    let allowed: std::collections::HashSet<_> = media_core::find_residuals(&db, &ids)
                        .map_err(err_string)?.into_iter().map(|c| c.path).collect();
                    if paths.iter().any(|p| !allowed.contains(p)) {
                        return Err("cleanup candidates changed; scan again".into());
                    }
                    let mut removed = 0;
                    for path in paths {
                        if cancel.load(std::sync::atomic::Ordering::SeqCst) { break; }
                        removed += media_core::perform_cleanup(&[path]).map_err(err_string)?;
                    }
                    Ok(removed)
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(err_string)?;
                if handle.is_cancelled() {
                    return Err("cancelled".into());
                }
                handle
                    .update_progress(TaskProgress {
                        completed: removed as u32,
                        total,
                        current: crate::ui_i18n::tf(&locale, "prog.cleaned", &[("n", &removed.to_string())]),
                        stage_key: Some("cleanup".into()),
                    })
                    .await;
                Ok(())
            })
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
    let _mutation_guard = state.tasks.lock_mutations().await?;
    if item_ids.is_empty() {
        return Err("no items selected".into());
    }

    // Preflight the entire request before touching either files or records.
    let mut targets = Vec::new();
    if also_trash {
        for id in &item_ids {
            if let Some(item) = state.db.get_media_item(id).map_err(err_string)? {
                let target = media_core::media_files::deletion_target(&state.db, &item)
                    .map_err(err_string)?;
                targets.push((id.clone(), target));
            }
        }
    }
    let deleted = if also_trash {
        let fs = media_core::FilesystemService::new();
        let mut deleted = 0;
        for (id, path) in targets {
            // Keep the record if trash fails; do not turn a trash request into
            // permanent deletion on platforms without recycle-bin support.
            fs.trash_item(&path).map_err(err_string)?;
            deleted += state.db.delete_media_items(&[id]).map_err(err_string)?;
        }
        deleted
    } else {
        state.db.delete_media_items(&item_ids).map_err(err_string)?
    };

    let _ = app.emit("library-updated", ());
    Ok(deleted)
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
        .map_err(|e| loc_err(&locale, e))
}

#[tauri::command]
pub async fn apply_manual_match(
    app: AppHandle,
    state: State<'_, AppState>,
    item_id: String,
    source_id: String,
) -> Result<TaskSnapshot, String> {
    let config = state.config.lock().await.config.clone();
    let locale = config.ui_locale.clone();
    let options = scrape_options_from_config(&config);
    let item = state
        .db
        .get_media_item(&item_id)
        .map_err(err_string)?
        .ok_or_else(|| format!("media item not found: {item_id}"))?;
    let db = Arc::clone(&state.db);
    let title = crate::ui_i18n::tf(
        &locale,
        "task.manualMatch",
        &[("title", &item.title)],
    );
    let target_id = Some(item_id.clone());

    let snapshot = state
        .tasks
        .enqueue_scoped(title, TaskKind::ManualMatch, target_id, task_scope("match", &[item_id.clone(), source_id.clone()]), move |handle| {
            let locale = locale.clone();
            Box::pin(async move {
                handle
                    .update_progress(TaskProgress {
                        completed: 0,
                        total: 1,
                        current: item.title.clone(),
                        stage_key: Some("matching".into()),
                    })
                    .await;
                handle.run_cancellable(scraper_kit::apply_manual_match(&db, &item, &source_id, &options))
                    .await
                    .map_err(|e| loc_err(&locale, e))?;
                if handle.is_cancelled() {
                    return Err("cancelled".into());
                }
                let templates = config.rename_templates();
                if config.rename_auto_after_scrape {
                    handle
                        .update_progress(TaskProgress {
                            completed: 0,
                            total: 1,
                            current: crate::ui_i18n::t(&locale, "prog.autoRename"),
                            stage_key: Some("rename".into()),
                        })
                        .await;
                    let _ = auto_rename_after_scrape(
                        &db,
                        &[item.id.clone()],
                        &templates,
                        config.rename_create_season_folders,
                        &handle,
                    ).await;
                } else {
                    consolidate_after_scrape(&db, &[item.id.clone()], &templates, &handle).await;
                }
                handle
                    .update_progress(TaskProgress {
                        completed: 1,
                        total: 1,
                        current: item.title.clone(),
                        stage_key: Some("saveResults".into()),
                    })
                    .await;
                Ok(())
            })
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
        nfo_format: config.nfo_format.clone(),
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

    // Match main-window immersive chrome on Windows (macOS keeps system decorations).
    #[cfg(target_os = "windows")]
    let builder = builder.decorations(false);

    builder.build().map_err(err_string)?;
    Ok(())
}

#[tauri::command]
pub async fn renamer_collect_files(paths: Vec<String>) -> Result<Vec<renamer::FileEntry>, String> {
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
    let _mutation_guard = state.tasks.lock_mutations().await?;
    let previews = renamer::preview(&files, &pipeline);
    let mut outcome = RenamerOutcome { renames: Vec::new(), error: None, index_sync_failures: 0 };
    let result = renamer::execute(&previews, &state.rename_undo, |done| {
        outcome.renames.push(done.clone());
        sync_renamed_entry(&state.db, done, &mut outcome.index_sync_failures);
    });
    finish_renamer_batch(&app, outcome, result.map(|_| ()))
}

#[tauri::command]
pub async fn renamer_undo_last(app: AppHandle, state: State<'_, AppState>) -> Result<RenamerOutcome, String> {
    let _mutation_guard = state.tasks.lock_mutations().await?;
    let mut outcome = RenamerOutcome { renames: Vec::new(), error: None, index_sync_failures: 0 };
    let result = state.rename_undo.undo_last_with(|done| {
        outcome.renames.push(done.clone());
        sync_renamed_entry(&state.db, done, &mut outcome.index_sync_failures);
    });
    finish_renamer_batch(&app, outcome, result.map(|_| ()))
}

/// A failure before anything moved is a plain error; after that the caller must
/// still learn which entries moved, so the error travels inside the outcome.
fn finish_renamer_batch(
    app: &AppHandle,
    mut outcome: RenamerOutcome,
    result: Result<(), renamer::ExecuteError>,
) -> Result<RenamerOutcome, String> {
    if !outcome.renames.is_empty() {
        let _ = app.emit("library-updated", ());
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

fn err_string(err: impl ToString) -> String {
    err.to_string()
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
