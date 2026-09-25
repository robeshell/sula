use std::collections::HashSet;

use crate::models::{Library, MediaType};
use crate::nfo::import_nfo_for_item;
use crate::AppDatabase;
use crate::DatabaseError;

use super::incremental::{
    canonicalize_lossy, known_directories_unchanged_cancellable, offline_mount_points,
    plan_directories_cancellable,
};
use super::movies::{scan_movies_under_cancellable, ScanProgress, MEDIA_EXTENSIONS};
use super::shows::scan_shows_under_cancellable;

/// Sub-mounts below a library root that are currently disconnected. Anything that
/// used to live under them only looks deleted and must be left untouched.
struct OfflineMounts(Vec<String>);

impl OfflineMounts {
    fn check(recorded: &[String]) -> Result<Self, RefreshError> {
        let offline = offline_mount_points(recorded)?;
        for mount in &offline {
            tracing::warn!(%mount, "sub-mount offline; keeping entries below it");
        }
        Ok(Self(offline))
    }

    fn covers(&self, path: &str) -> bool {
        self.0.iter().any(|mount| crate::db::path_rooted_under(path, mount))
    }

    fn error(&self, path: &str) -> RefreshError {
        let mount = self.0.iter().find(|mount| crate::db::path_rooted_under(path, mount)).cloned().unwrap_or_default();
        std::io::Error::other(format!("a mounted folder inside the library is offline: {mount}; reconnect it before refreshing")).into()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RefreshReport {
    pub new_item_count: usize,
    pub discovered_media_count: u32,
    pub imported_nfo_count: usize,
    pub removed_item_count: usize,
    /// SCAN-14: no directory changes; skipped media file enumeration.
    pub early_exit: bool,
}

/// Incremental refresh (SCAN-11…14 / 13).
/// Flow: directory mtime plan → prune removed → scan changed roots → persist → upsert scan state.
pub fn refresh_library(
    db: &AppDatabase,
    library: &Library,
    excluded_folders: &[String],
    on_progress: impl FnMut(ScanProgress),
) -> Result<RefreshReport, RefreshError> {
    refresh_library_cancellable(db, library, excluded_folders, on_progress, &std::sync::atomic::AtomicBool::new(false))
}

pub fn refresh_library_cancellable(
    db: &AppDatabase,
    library: &Library,
    excluded_folders: &[String],
    mut on_progress: impl FnMut(ScanProgress),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<RefreshReport, RefreshError> {
    super::check_cancel(cancel)?;
    let excluded: HashSet<String> = excluded_folders
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    on_progress(ScanProgress {
        discovered_count: 0,
        current_path: library.root_path.clone(),
        current_name: "scan.checking".into(),
    });

    super::check_cancel(cancel)?;
    db.verify_library_root(library)?;
    let previous = db.list_scan_states(&library.id)?;
    let deep_scan_due = previous.iter().any(|s| chrono::Utc::now().signed_duration_since(s.last_scanned_at).num_hours() >= 24);
    let recorded_mounts = db.library_mount_points(&library.id)?;
    let offline = OfflineMounts::check(recorded_mounts.as_deref().unwrap_or_default())?;
    let mut report = RefreshReport::default();
    // Directory mtimes are not sufficient on every filesystem/NAS. Check
    // direct root files before taking the fast exit.
    let has_unindexed_root_media = has_unindexed_media_in_library_root(db, library, cancel)?;

    // Fast path: every known dir still present with unchanged mtime → skip WalkDir.
    // Libraries never walked with mount detection take one full walk to record it.
    if !deep_scan_due && !has_unindexed_root_media && recorded_mounts.is_some()
        && known_directories_unchanged_cancellable(&previous, cancel)?
    {
        tracing::info!(
            library_id = %library.id,
            dirs = previous.len(),
            "refresh fast early-exit (known dirs unchanged)"
        );
        report.early_exit = true;
        on_progress(ScanProgress {
            discovered_count: 0,
            current_path: library.root_path.clone(),
            current_name: "scan.unchanged".into(),
        });
        return Ok(report);
    }

    let mut plan = plan_directories_cancellable(library, &excluded, &previous, cancel)?;
    if deep_scan_due { plan.bootstrap = true; }
    // Keep disconnected mounts recorded until they return or their directory is removed.
    for mount in &offline.0 {
        if !plan.mount_points.contains(mount) { plan.mount_points.push(mount.clone()); }
    }
    db.verify_library_root(library)?;

    tracing::info!(
        library_id = %library.id,
        bootstrap = plan.bootstrap,
        live = plan.live.len(),
        added = plan.added.len(),
        changed = plan.changed.len(),
        removed = plan.removed.len(),
        "refresh directory plan"
    );

    // SCAN-13: directories gone from disk → remove rooted media + scan state.
    super::check_cancel(cancel)?;
    if plan.has_removals() {
        let mut missing = Vec::new();
        for path in &plan.removed {
            if offline.covers(path) { continue; }
            match std::fs::metadata(path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing.push(path.clone())
                }
                Err(error) => return Err(error.into()),
                Ok(_) => {} // Excluded directories still exist; do not erase their metadata.
            }
        }
        for item in db.list_media_items(&library.id)? {
            if item.media_type != MediaType::Movie && missing.iter().any(|root| std::path::Path::new(&item.folder_path).starts_with(root)) {
                reanchor_show(db, &item, cancel)?;
            }
        }
        db.verify_library_root(library)?;
        report.removed_item_count = db.delete_media_items_rooted_under(&library.id, &missing)?;
        let _ = db.delete_scan_states(&library.id, &plan.removed)?;
    }

    // SCAN-14: nothing added/changed (and bootstrap false) → persist plan, no file walk.
    if !plan.needs_file_scan() && !has_unindexed_root_media {
        persist_scan_plan(db, library, &plan)?;
        report.early_exit = true;
        on_progress(ScanProgress {
            discovered_count: 0,
            current_path: library.root_path.clone(),
            current_name: "scan.unchanged".into(),
        });
        return Ok(report);
    }

    // SCAN-14: empty scan_roots after prune → persist without file walk.
    let mut scan_roots = plan.scan_roots();
    if has_unindexed_root_media {
        let root =
            std::path::PathBuf::from(canonicalize_lossy(std::path::Path::new(&library.root_path)));
        if !scan_roots.iter().any(|candidate| candidate == &root) {
            scan_roots.push(root);
        }
    }
    if scan_roots.is_empty() {
        persist_scan_plan(db, library, &plan)?;
        report.early_exit = true;
        on_progress(ScanProgress {
            discovered_count: 0,
            current_path: library.root_path.clone(),
            current_name: "scan.unchanged".into(),
        });
        return Ok(report);
    }

    match library.media_type {
        MediaType::Movie => {
            let existing: HashSet<String> =
                db.list_media_file_paths(&library.id)?.into_iter().collect();
            let mut discovered = 0u32;
            let result = scan_movies_under_cancellable(
                library,
                &scan_roots,
                &existing,
                &excluded,
                |p| {
                    discovered = p.discovered_count;
                    on_progress(p);
                },
                cancel,
            )?;
            // Only reconcile after a successful walk. Permission/I/O errors must
            // not be interpreted as evidence that files were deleted.
            let mut missing = Vec::new();
            for item in db.list_media_items(&library.id)? {
                let path = std::path::Path::new(&item.file_path);
                if !scan_roots.iter().any(|root| path.starts_with(root)) || offline.covers(&item.file_path) {
                    continue;
                }
                match std::fs::metadata(path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        missing.push(item.id)
                    }
                    Err(error) => return Err(error.into()),
                    Ok(_) => {}
                }
            }
            db.verify_library_root(library)?;
            report.removed_item_count += db.delete_media_items(&missing)?;
            super::check_cancel(cancel)?;
            db.insert_media_items(&result.new_items)?;
            report.new_item_count = result.new_items.len();
            report.discovered_media_count = discovered;

            // Persist incremental state before NFO so a later NFO failure
            // does not force bootstrap full-scan on the next refresh.
            super::check_cancel(cancel)?;
            persist_scan_plan(db, library, &plan)?;

            super::check_cancel(cancel)?;
            for item in &result.new_items {
                super::check_cancel(cancel)?;
                if import_nfo_for_item(db, item)? {
                    report.imported_nfo_count += 1;
                }
            }
        }
        MediaType::TvShow | MediaType::Anime => {
            let existing_items = db.list_media_items(&library.id)?;
            let existing: HashSet<String> = existing_items
                .iter()
                .map(|i| i.folder_path.clone())
                .collect();
            let mut discovered = 0u32;
            let result = scan_shows_under_cancellable(
                library,
                &scan_roots,
                &existing,
                &excluded,
                |p| {
                    discovered = p.discovered_count;
                    on_progress(p);
                },
                cancel,
            )?;

            let (result, absorbed) =
                super::shows::absorb_flat_seasons_into_existing(result, &existing_items);

            super::check_cancel(cancel)?;
            db.insert_media_items(&result.new_items)?;
            super::check_cancel(cancel)?;
            for item in &result.new_items {
                super::check_cancel(cancel)?;
                if let Some(episodes) = result.episodes.get(&item.id) {
                    db.insert_show_episodes(&item.id, episodes)?;
                }
            }
            for (existing_id, episodes) in &absorbed {
                super::check_cancel(cancel)?;
                db.insert_show_episodes(existing_id, episodes)?;
            }

            // Nested `Show/Season XX`: files under an existing show root are skipped by
            // scan_shows_under — resync touched shows so new seasons land on the same item.
            super::check_cancel(cancel)?;
            for item in &existing_items {
                super::check_cancel(cancel)?;
                if super::shows::existing_show_touched_by_roots(&item.folder_path, &scan_roots)
                    && !offline.covers(&item.folder_path)
                {
                    resync_show_episodes(db, item, &excluded, cancel)?;
                }
            }

            report.new_item_count = result.new_items.len();
            report.discovered_media_count = discovered;

            super::check_cancel(cancel)?;
            persist_scan_plan(db, library, &plan)?;

            super::check_cancel(cancel)?;
            for item in &result.new_items {
                super::check_cancel(cancel)?;
                if import_nfo_for_item(db, item)? {
                    report.imported_nfo_count += 1;
                }
            }
        }
    }

    Ok(report)
}

fn has_unindexed_media_in_library_root(
    db: &AppDatabase,
    library: &Library,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<bool, RefreshError> {
    let mut root_media = Vec::new();
    for entry in std::fs::read_dir(&library.root_path)? {
        super::check_cancel(cancel)?;
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !MEDIA_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }
        root_media.push(canonicalize_lossy(&path));
    }
    if root_media.is_empty() {
        return Ok(false);
    }

    let indexed: HashSet<String> = match library.media_type {
        MediaType::Movie => db.list_media_file_paths(&library.id)?,
        MediaType::TvShow | MediaType::Anime => {
            db.list_episode_file_paths_for_library(&library.id)?
        }
    }
    .into_iter()
    .filter(|path| !path.is_empty())
    .collect();

    Ok(root_media.iter().any(|path| !indexed.contains(path)))
}

#[derive(Debug, Clone, Default)]
pub struct ItemRefreshReport {
    pub refreshed: usize,
    pub removed: usize,
    pub imported_nfo: usize,
}

/// SCAN-15: refresh selected items from disk.
/// Missing primary path → delete DB row (no synthetic "missing" status).
pub fn refresh_items(
    db: &AppDatabase,
    item_ids: &[String],
    excluded_folders: &[String],
) -> Result<ItemRefreshReport, RefreshError> {
    refresh_items_cancellable(db, item_ids, excluded_folders, &std::sync::atomic::AtomicBool::new(false))
}

pub fn refresh_items_cancellable(
    db: &AppDatabase,
    item_ids: &[String],
    excluded_folders: &[String],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<ItemRefreshReport, RefreshError> {
    super::check_cancel(cancel)?;
    let excluded: HashSet<String> = excluded_folders
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let mut report = ItemRefreshReport::default();

    let mut checked: std::collections::HashMap<String, OfflineMounts> = std::collections::HashMap::new();
    super::check_cancel(cancel)?;
    for id in item_ids {
        super::check_cancel(cancel)?;
        if let Some(item) = db.get_media_item(id)? {
            if !checked.contains_key(&item.library_id) {
                let library = db.get_library(&item.library_id)?.ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "library not found")
                })?;
                db.verify_library_root(&library)?;
                for entry in std::fs::read_dir(&library.root_path)? {
                    super::check_cancel(cancel)?;
                    entry?;
                }
                let recorded = db.library_mount_points(&library.id)?.unwrap_or_default();
                checked.insert(library.id.clone(), OfflineMounts::check(&recorded)?);
            }
            let offline = &checked[&item.library_id];
            let primary = if item.media_type == MediaType::Movie { &item.file_path } else { &item.folder_path };
            if offline.covers(primary) {
                return Err(offline.error(primary));
            }
        }
    }
    super::check_cancel(cancel)?;
    for id in item_ids {
        super::check_cancel(cancel)?;
        let Some(mut item) = db.get_media_item(id)? else {
            continue;
        };
        let primary = match item.media_type {
            MediaType::Movie => &item.file_path,
            MediaType::TvShow | MediaType::Anime => &item.folder_path,
        };
        if primary.is_empty() {
            continue;
        }
        let primary_ok = match std::fs::metadata(primary) {
            Ok(metadata) => match item.media_type {
                MediaType::Movie => metadata.is_file(),
                _ => metadata.is_dir(),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        if !primary_ok {
            if let Some(relocated) = reanchor_show(db, &item, cancel)? { item = relocated; }
            else {
                db.delete_media_item(&item.id)?;
                report.removed += 1;
                continue;
            }
        }

        if matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
            resync_show_episodes(db, &item, &excluded, cancel)?;
        }

        if import_nfo_for_item(db, &item)? {
            report.imported_nfo += 1;
        }
        report.refreshed += 1;
    }

    Ok(report)
}

fn reanchor_show(db: &AppDatabase, item: &crate::MediaItem, cancel: &std::sync::atomic::AtomicBool) -> Result<Option<crate::MediaItem>, RefreshError> {
    if item.media_type == MediaType::Movie { return Ok(None); }
    let library = db.get_library(&item.library_id)?.ok_or_else(|| std::io::Error::other("library not found"))?;
    let canonical_root = canonicalize_lossy(std::path::Path::new(&library.root_path));
    let root = std::path::Path::new(&canonical_root);
    for season in db.fetch_seasons(&item.id)? {
        for episode in db.fetch_episodes(&season.id)? {
            super::check_cancel(cancel)?;
            if episode.file_path.is_empty() { continue; }
            let path = std::path::Path::new(&episode.file_path);
            if !path.starts_with(root) { continue; }
            match std::fs::metadata(path) {
                Ok(metadata) if metadata.is_file() => {
                    let folder = path.parent().unwrap();
                    db.update_media_paths(&item.id, &folder.to_string_lossy(), &folder.to_string_lossy())?;
                    return Ok(db.get_media_item(&item.id)?);
                }
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(None)
}

fn resync_show_episodes(
    db: &AppDatabase,
    item: &crate::models::MediaItem,
    excluded: &HashSet<String>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(), RefreshError> {
    use super::shows::discover_episodes_in_show_cancellable;
    use std::path::Path;

    let folder = Path::new(&item.folder_path);
    let discovered = discover_episodes_in_show_cancellable(folder, excluded, cancel)?;
    let mut existing = Vec::new();
    for season in db.fetch_seasons(&item.id)? {
        for ep in db.fetch_episodes(&season.id)? {
            existing.push((season.season_number, ep));
        }
    }
    let bound: HashSet<&str> = existing.iter().map(|(_, ep)| ep.file_path.as_str()).collect();
    // Two files claiming one season/episode (e.g. two releases of the same episode)
    // are ambiguous: keep the file already indexed, never abort the whole refresh.
    let mut by_key = std::collections::HashMap::new();
    for episode in &discovered {
        match by_key.entry((episode.season, episode.episode)) {
            std::collections::hash_map::Entry::Vacant(slot) => { slot.insert(episode); }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                tracing::warn!(path = %episode.file_path, other = %slot.get().file_path, season = episode.season,
                    episode = episode.episode, "multiple files have the same season/episode identity");
                if bound.contains(episode.file_path.as_str()) && !bound.contains(slot.get().file_path.as_str()) {
                    slot.insert(episode);
                }
            }
        }
    }
    let mut removed = Vec::new();
    let mut updated = Vec::new();
    for (season_number, ep) in existing {
        {
            super::check_cancel(cancel)?;
            let found = by_key.remove(&(season_number, ep.episode_number));
            // Remote-only metadata has no local file to declare missing.
            if ep.file_path.is_empty() {
                if let Some(found) = found {
                    updated.push((ep.id, found.file_path.clone()));
                }
                continue;
            }
            match std::fs::metadata(&ep.file_path) {
                Ok(_) => {} // Excluded or outside this show's root: preserve existing binding.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if let Some(found) = found {
                        updated.push((ep.id, found.file_path.clone()));
                    } else {
                        removed.push(ep.id);
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    let added: Vec<_> = by_key.into_values().cloned().collect();
    super::check_cancel(cancel)?;
    db.reconcile_show_episodes(&item.id, &removed, &updated, &added)?;
    Ok(())
}

fn persist_scan_plan(
    db: &AppDatabase,
    library: &Library,
    plan: &super::incremental::DirectoryPlan,
) -> Result<(), RefreshError> {
    // Save the state observed BEFORE enumeration, never acknowledge mutations
    // arriving during the scan as if their files had already been inspected.
    db.upsert_scan_states(&plan.to_scan_states(&library.id))?;
    db.set_library_mount_points(&library.id, &plan.mount_points)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::incremental::{known_directories_unchanged, plan_directories};
    use crate::models::MediaType;
    use tempfile::tempdir;

    fn movie_library(root: &std::path::Path) -> Library {
        Library::new("Movies", root.display().to_string(), MediaType::Movie)
    }

    #[test]
    fn offline_sub_mount_keeps_entries_until_its_directory_is_removed() {
        let dir = tempdir().unwrap(); let root = dir.path().join("lib");
        let nas = root.join("NAS");
        std::fs::create_dir_all(nas.join("Film (2020)")).unwrap();
        std::fs::write(nas.join("Film (2020)/Film.mkv"), "video").unwrap();
        std::fs::create_dir_all(root.join("Local")).unwrap();
        std::fs::write(root.join("Local/Local.mkv"), "video").unwrap();
        let db = AppDatabase::open_in_memory().unwrap(); let lib = movie_library(&root);
        db.insert_library(&lib).unwrap(); refresh_library(&db, &lib, &[], |_| {}).unwrap();
        assert_eq!(db.library_mount_points(&lib.id).unwrap(), Some(vec![]));
        // Pretend NAS was a separate mount on the last walk, then it disconnects and
        // leaves the empty mount-point directory behind.
        let nas_key = canonicalize_lossy(&nas);
        db.set_library_mount_points(&lib.id, std::slice::from_ref(&nas_key)).unwrap();
        std::fs::remove_dir_all(&nas).unwrap(); std::fs::create_dir(&nas).unwrap();
        std::fs::write(root.join("Local/New.mkv"), "video").unwrap();

        let report = refresh_library(&db, &lib, &[], |_| {}).unwrap();
        assert_eq!(report.removed_item_count, 0);
        assert_eq!(report.new_item_count, 1, "the rest of the library still refreshes");
        let items = db.list_media_items(&lib.id).unwrap();
        let film = items.iter().find(|i| i.title == "Film").unwrap().id.clone();
        assert_eq!(items.len(), 3);
        assert_eq!(db.library_mount_points(&lib.id).unwrap(), Some(vec![nas_key]));
        assert!(refresh_items(&db, std::slice::from_ref(&film), &[]).is_err());

        std::fs::remove_dir(&nas).unwrap(); // mount retired on purpose
        let report = refresh_library(&db, &lib, &[], |_| {}).unwrap();
        assert_eq!(report.removed_item_count, 1);
        assert_eq!(db.library_mount_points(&lib.id).unwrap(), Some(vec![]));
    }

    #[test]
    fn replaced_mount_root_does_not_erase_index() {
        let dir = tempdir().unwrap(); let root = dir.path().join("mount");
        std::fs::create_dir(&root).unwrap(); std::fs::write(root.join("Film.mkv"), "video").unwrap();
        let db = AppDatabase::open_in_memory().unwrap(); let lib = movie_library(&root);
        db.insert_library(&lib).unwrap(); refresh_library(&db, &lib, &[], |_| {}).unwrap();
        std::fs::rename(&root, dir.path().join("offline")).unwrap(); std::fs::create_dir(&root).unwrap();
        assert!(refresh_library(&db, &lib, &[], |_| {}).is_err());
        assert_eq!(db.list_media_items(&lib.id).unwrap().len(), 1);
    }

    #[test]
    fn stale_directory_timestamps_get_periodic_deep_scan() {
        let dir = tempdir().unwrap(); let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap(); std::fs::write(nested.join("A.mkv"), "a").unwrap();
        let db = AppDatabase::open_in_memory().unwrap(); let lib = movie_library(dir.path());
        db.insert_library(&lib).unwrap(); refresh_library(&db, &lib, &[], |_| {}).unwrap();
        std::fs::write(nested.join("B.mkv"), "b").unwrap();
        let previous = db.list_scan_states(&lib.id).unwrap();
        let mut states = plan_directories(&lib, &HashSet::new(), &previous).unwrap().to_scan_states(&lib.id);
        for state in &mut states { state.last_scanned_at = chrono::Utc::now() - chrono::Duration::hours(25); }
        db.upsert_scan_states(&states).unwrap();
        assert_eq!(refresh_library(&db, &lib, &[], |_| {}).unwrap().new_item_count, 1);
    }

    #[test]
    fn vanished_primary_season_reanchors_existing_show() {
        for individual in [false, true] {
            let dir = tempdir().unwrap();
            let a = dir.path().join("Show 第 1 季");
            let b = dir.path().join("Show 第 2 季");
            std::fs::create_dir(&a).unwrap(); std::fs::create_dir(&b).unwrap();
            std::fs::write(a.join("S01E01.mkv"), "a").unwrap();
            std::fs::write(b.join("S02E01.mkv"), "b").unwrap();
            let db = AppDatabase::open_in_memory().unwrap();
            let lib = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
            db.insert_library(&lib).unwrap();
            refresh_library(&db, &lib, &[], |_| {}).unwrap();
            let item = db.list_media_items(&lib.id).unwrap().remove(0);
            std::fs::remove_dir_all(&item.folder_path).unwrap();
            if individual { refresh_items(&db, &[item.id.clone()], &[]).unwrap(); }
            else { refresh_library(&db, &lib, &[], |_| {}).unwrap(); }
            let relocated = db.get_media_item(&item.id).unwrap().unwrap();
            assert!(std::path::Path::new(&relocated.folder_path).is_dir());
            assert_eq!(db.list_media_items(&lib.id).unwrap().len(), 1);
        }
    }

    #[test]
    fn cancellation_during_enumeration_does_not_commit_partial_scan() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempdir().unwrap();
        for name in ["A.mkv", "B.mkv"] { std::fs::write(dir.path().join(name), b"x").unwrap(); }
        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        let cancel = AtomicBool::new(false);
        let result = refresh_library_cancellable(&db, &library, &[], |p| {
            if p.discovered_count > 0 { cancel.store(true, Ordering::Relaxed); }
        }, &cancel);
        assert!(matches!(result, Err(RefreshError::Io(ref e)) if e.kind() == std::io::ErrorKind::Interrupted));
        assert!(db.list_media_items(&library.id).unwrap().is_empty());
        assert!(db.list_scan_states(&library.id).unwrap().is_empty());
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(refresh_library(&db, &library, &[], |_| {}).unwrap().new_item_count, 2);
    }

    #[test]
    fn cancelled_item_refresh_does_not_delete_missing_item() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("A.mkv"), b"x").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        let item = db.list_media_items(&library.id).unwrap().remove(0);
        std::fs::remove_file(&item.file_path).unwrap();
        assert!(refresh_items_cancellable(&db, &[item.id.clone()], &[], &std::sync::atomic::AtomicBool::new(true)).is_err());
        assert!(db.get_media_item(&item.id).unwrap().is_some());
    }

    #[test]
    fn episode_refresh_preserves_excluded_and_remote_metadata_and_updates_count() {
        let dir = tempdir().unwrap();
        let season = dir.path().join("Show/Season 01");
        std::fs::create_dir_all(&season).unwrap();
        for name in ["S01E01.mkv", "S01E02.mkv"] { std::fs::write(season.join(name), b"x").unwrap(); }
        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        let item = db.list_media_items(&library.id).unwrap().remove(0);
        db.insert_show_episodes(&item.id, &[crate::ScannedEpisode { season: 1, episode: 3, file_path: String::new(), title: "Remote".into() }]).unwrap();
        refresh_items(&db, &[item.id.clone()], &["Season 01".into()]).unwrap();
        let sid = db.fetch_seasons(&item.id).unwrap().remove(0).id;
        assert_eq!(db.fetch_episodes(&sid).unwrap().len(), 3);
        std::fs::remove_file(season.join("S01E02.mkv")).unwrap();
        refresh_items(&db, &[item.id], &[]).unwrap();
        let eps = db.fetch_episodes(&sid).unwrap();
        assert_eq!(eps.iter().map(|e| e.episode_number).collect::<Vec<_>>(), vec![1, 3]);
        assert_eq!(db.with_conn(|conn| Ok(conn.query_row("SELECT episodeCount FROM tv_seasons WHERE id=?1", [&sid], |row| row.get::<_, i64>(0))?)).unwrap(), 2);
    }

    #[test]
    fn ambiguous_episode_files_leave_existing_binding_unchanged() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Show");
        std::fs::create_dir(&show).unwrap();
        std::fs::write(show.join("S01E01.mkv"), b"first").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        let item = db.list_media_items(&library.id).unwrap().remove(0);
        let sid = db.fetch_seasons(&item.id).unwrap().remove(0).id;
        let before = db.fetch_episodes(&sid).unwrap().remove(0).file_path;
        std::fs::write(show.join("S01E01.mp4"), b"alternate").unwrap();
        // Ambiguity is logged, the indexed file wins, and the refresh itself succeeds.
        refresh_items(&db, std::slice::from_ref(&item.id), &[]).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        let eps = db.fetch_episodes(&sid).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].file_path, before);
    }

    #[test]
    fn unnumbered_extras_do_not_take_parsed_episode_slots() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Show");
        std::fs::create_dir(&show).unwrap();
        for name in ["Behind the Scenes.mkv", "Show.S01E01.mkv", "Show.S01E02.mkv"] {
            std::fs::write(show.join(name), b"x").unwrap();
        }
        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        let item = db.list_media_items(&library.id).unwrap().remove(0);
        let sid = db.fetch_seasons(&item.id).unwrap().remove(0).id;
        let mut eps: Vec<_> = db.fetch_episodes(&sid).unwrap().into_iter()
            .map(|e| (e.episode_number, std::path::Path::new(&e.file_path).file_name().unwrap().to_string_lossy().into_owned()))
            .collect();
        eps.sort();
        assert_eq!(eps, vec![
            (1, "Show.S01E01.mkv".to_string()),
            (2, "Show.S01E02.mkv".to_string()),
            (3, "Behind the Scenes.mkv".to_string()),
        ]);
        refresh_items(&db, std::slice::from_ref(&item.id), &[]).unwrap();
        assert_eq!(db.fetch_episodes(&sid).unwrap().len(), 3);
    }

    #[test]
    fn item_refresh_preserves_records_when_library_is_offline() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("mount");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("A.mkv"), b"video").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(&root);
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        let id = db.list_media_items(&library.id).unwrap()[0].id.clone();
        std::fs::rename(&root, dir.path().join("offline")).unwrap();
        assert!(refresh_items(&db, &[id.clone()], &[]).is_err());
        assert!(db.get_media_item(&id).unwrap().is_some());
    }

    #[test]
    fn library_refresh_removes_deleted_movie_with_existing_parent() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("A");
        std::fs::create_dir(&folder).unwrap();
        let video = folder.join("A.mkv");
        std::fs::write(&video, b"video").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::remove_file(&video).unwrap();
        let report = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(report.removed_item_count, 1);
        assert!(folder.exists());
        assert!(db.list_media_items(&library.id).unwrap().is_empty());
    }

    #[test]
    fn changes_during_scan_are_not_acknowledged_as_scanned() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("A.mkv"), b"video").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        let mut changed = false;
        refresh_library(&db, &library, &[], |progress| {
            if progress.discovered_count > 0 && !changed {
                changed = true;
                std::thread::sleep(std::time::Duration::from_millis(20));
                std::fs::create_dir(dir.path().join("Late")).unwrap();
                std::fs::write(dir.path().join("Late/B.mkv"), b"video").unwrap();
            }
        }).unwrap();
        assert!(!known_directories_unchanged(&db.list_scan_states(&library.id).unwrap()).unwrap());
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 2);
    }

    #[test]
    fn changed_parent_keeps_direct_files_when_child_is_added() {
        let dir = tempdir().unwrap();
        let collection = dir.path().join("Collection");
        std::fs::create_dir_all(&collection).unwrap();
        std::fs::write(collection.join("Old.mkv"), b"x").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir(collection.join("Extras")).unwrap();
        std::fs::write(collection.join("New.mkv"), b"x").unwrap();
        let report = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(report.new_item_count, 1);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 2);
        assert!(refresh_library(&db, &library, &[], |_| {}).unwrap().early_exit);
    }

    #[test]
    fn bootstrap_then_early_exit_on_unchanged() {
        let dir = tempdir().unwrap();
        let movie = dir.path().join("Dune (2021)");
        std::fs::create_dir_all(&movie).unwrap();
        std::fs::write(movie.join("Dune.2021.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();

        let first = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(!first.early_exit);
        assert_eq!(first.new_item_count, 1);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 1);

        let second = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(second.early_exit);
        assert_eq!(second.new_item_count, 0);
        assert_eq!(second.discovered_media_count, 0);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 1);
    }

    #[test]
    fn detects_new_movie_folder() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("A (2020)");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("A.2020.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();

        // Ensure mtime can advance on some filesystems.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let b = dir.path().join("B (2021)");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join("B.2021.mkv"), b"x").unwrap();

        let report = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(!report.early_exit);
        assert_eq!(report.new_item_count, 1);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 2);
    }

    #[test]
    fn detects_unindexed_root_movie_even_when_directory_state_is_current() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("Nested (2020)");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("Nested.2020.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();

        let loose = dir.path().join("Loose.Movie.2024.mkv");
        std::fs::write(&loose, b"x").unwrap();

        // Simulate a NAS/filesystem where the directory-only state already looks
        // current even though the new root file has never been indexed.
        let current = plan_directories(&library, &HashSet::new(), &[]).unwrap();
        db.upsert_scan_states(&current.to_scan_states(&library.id))
            .unwrap();
        let states = db.list_scan_states(&library.id).unwrap();
        assert!(known_directories_unchanged(&states).unwrap());

        let report = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(!report.early_exit);
        assert_eq!(report.new_item_count, 1);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 2);
        assert!(db
            .list_media_file_paths(&library.id)
            .unwrap()
            .contains(&canonicalize_lossy(&loose)));
    }

    #[test]
    fn detects_unindexed_root_episode_without_replacing_existing_episode() {
        let dir = tempdir().unwrap();
        let first_episode = dir.path().join("Root.Show.S01E01.mkv");
        std::fs::write(&first_episode, b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();

        let second_episode = dir.path().join("Root.Show.S01E02.mkv");
        std::fs::write(&second_episode, b"x").unwrap();
        let current = plan_directories(&library, &HashSet::new(), &[]).unwrap();
        db.upsert_scan_states(&current.to_scan_states(&library.id))
            .unwrap();

        let report = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(!report.early_exit);
        assert_eq!(report.new_item_count, 0);

        let item = db.list_media_items(&library.id).unwrap().remove(0);
        let season = db.fetch_seasons(&item.id).unwrap().remove(0);
        let episodes = db.fetch_episodes(&season.id).unwrap();
        assert_eq!(episodes.len(), 2);
        assert!(episodes
            .iter()
            .any(|episode| episode.file_path == canonicalize_lossy(&first_episode)));
        assert!(episodes
            .iter()
            .any(|episode| episode.file_path == canonicalize_lossy(&second_episode)));
    }

    #[test]
    fn second_refresh_immediate_should_early_exit() {
        let dir = tempdir().unwrap();
        let movie = dir.path().join("Blade (1998)");
        std::fs::create_dir_all(&movie).unwrap();
        std::fs::write(movie.join("Blade.1998.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();

        let first = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(first.new_item_count, 1);
        let states = db.list_scan_states(&library.id).unwrap();
        assert!(
            !states.is_empty(),
            "scan state should persist after first refresh"
        );

        let second = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(
            second.early_exit,
            "second refresh must early-exit; states={}, discovered={}, new={}, removed={}",
            states.len(),
            second.discovered_media_count,
            second.new_item_count,
            second.removed_item_count
        );
        assert_eq!(second.discovered_media_count, 0);
    }

    #[test]
    fn empty_refresh_many_dirs() {
        // ~300 movie folders: second refresh must early-exit without rediscovering media.
        let dir = tempdir().unwrap();
        for i in 0..300 {
            let movie = dir.path().join(format!("Title{i:03} ({})", 2000 + (i % 20)));
            std::fs::create_dir_all(&movie).unwrap();
            std::fs::write(movie.join(format!("Title{i:03}.mkv")), b"x").unwrap();
        }

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();

        let first = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(!first.early_exit);
        assert_eq!(first.new_item_count, 300);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 300);

        let started = std::time::Instant::now();
        let second = refresh_library(&db, &library, &[], |_| {}).unwrap();
        let elapsed = started.elapsed();
        assert!(
            second.early_exit,
            "unchanged library must early-exit; discovered={}",
            second.discovered_media_count
        );
        assert_eq!(second.discovered_media_count, 0);
        assert_eq!(second.new_item_count, 0);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 300);
        // Soft budget: empty refresh of 300 dirs should stay well under a few seconds locally.
        assert!(
            elapsed.as_secs() < 15,
            "empty refresh too slow: {elapsed:?}"
        );
    }

    #[test]
    fn scan_state_persists_even_if_nfo_step_would_run_after() {
        // Guarantees state exists before NFO loop so next refresh can early-exit.
        let dir = tempdir().unwrap();
        let movie = dir.path().join("X (2000)");
        std::fs::create_dir_all(&movie).unwrap();
        std::fs::write(movie.join("X.2000.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert!(!db.list_scan_states(&library.id).unwrap().is_empty());
    }

    #[test]
    fn removes_media_when_folder_deleted() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("Gone (2019)");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("Gone.2019.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 1);

        std::fs::remove_dir_all(&a).unwrap();
        // Touch library root so parent mtime may change; removal is detected via missing state paths.
        let _ = std::fs::metadata(dir.path());

        let report = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(report.removed_item_count, 1);
        assert!(db.list_media_items(&library.id).unwrap().is_empty());
    }

    #[test]
    fn refresh_items_deletes_when_primary_missing() {
        let dir = tempdir().unwrap();
        let movie = dir.path().join("GoneItem (2019)");
        std::fs::create_dir_all(&movie).unwrap();
        let file = movie.join("GoneItem.2019.mkv");
        std::fs::write(&file, b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = movie_library(dir.path());
        db.insert_library(&library).unwrap();
        refresh_library(&db, &library, &[], |_| {}).unwrap();
        let items = db.list_media_items(&library.id).unwrap();
        assert_eq!(items.len(), 1);
        let id = items[0].id.clone();

        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir_all(&movie).unwrap();

        let report = refresh_items(&db, &[id], &[]).unwrap();
        assert_eq!(report.removed, 1);
        assert_eq!(report.refreshed, 0);
        assert!(db.list_media_items(&library.id).unwrap().is_empty());
    }

    #[test]
    fn flat_season_merges_into_existing_on_second_refresh() {
        let dir = tempdir().unwrap();
        let s1 = dir.path().join("后室 第 1 季");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(s1.join("S01E01.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();

        let first = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(first.new_item_count, 1);
        assert_eq!(db.list_media_items(&library.id).unwrap().len(), 1);

        std::thread::sleep(std::time::Duration::from_millis(20));
        let s2 = dir.path().join("后室 第 2 季");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("S02E01.mkv"), b"x").unwrap();

        let second = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(second.new_item_count, 0, "season 2 must merge, not create a new show");
        let items = db.list_media_items(&library.id).unwrap();
        assert_eq!(items.len(), 1);
        let seasons = db.fetch_seasons(&items[0].id).unwrap();
        let nums: std::collections::HashSet<_> =
            seasons.iter().map(|s| s.season_number).collect();
        assert!(nums.contains(&1));
        assert!(nums.contains(&2));
        refresh_items(&db, &[items[0].id.clone()], &[]).unwrap();
        let season2 = db.fetch_seasons(&items[0].id).unwrap().into_iter().find(|s| s.season_number == 2).unwrap();
        assert_eq!(db.fetch_episodes(&season2.id).unwrap().len(), 1, "an attached season outside the primary folder must survive refresh");
    }

    #[test]
    fn nested_season_resyncs_into_existing_show() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Andor");
        let s1 = show.join("Season 01");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(s1.join("Andor.S01E01.mkv"), b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        assert_eq!(
            refresh_library(&db, &library, &[], |_| {})
                .unwrap()
                .new_item_count,
            1
        );

        std::thread::sleep(std::time::Duration::from_millis(20));
        let s2 = show.join("Season 02");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("Andor.S02E01.mkv"), b"x").unwrap();

        let second = refresh_library(&db, &library, &[], |_| {}).unwrap();
        assert_eq!(second.new_item_count, 0);
        let items = db.list_media_items(&library.id).unwrap();
        assert_eq!(items.len(), 1);
        let seasons = db.fetch_seasons(&items[0].id).unwrap();
        let nums: std::collections::HashSet<_> =
            seasons.iter().map(|s| s.season_number).collect();
        assert!(nums.contains(&1) && nums.contains(&2));
    }
}
