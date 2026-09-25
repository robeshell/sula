//! Apply media rename templates to scraped items (M3).
//! Movie + TV show folder/file rename after scrape.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use media_core::{
    companion_suffix, AppDatabase, FileNameParser, FilesystemService, MediaItem,
    MediaType, ScrapedStatus, TvEpisode, TvSeason, COMPANION_EXTENSIONS,
};
use thiserror::Error;

use crate::template::TemplateEngine;

#[derive(Debug, Clone)]
pub struct RenameTemplates {
    pub movie_folder: String,
    pub movie_file: String,
    pub tv_show_folder: String,
    pub season_folder: String,
    pub episode_file: String,
}

impl Default for RenameTemplates {
    fn default() -> Self {
        Self {
            movie_folder: TemplateEngine::MOVIE_FOLDER.into(),
            movie_file: TemplateEngine::MOVIE_FILE.into(),
            tv_show_folder: TemplateEngine::TV_SHOW_FOLDER.into(),
            season_folder: TemplateEngine::SEASON_FOLDER.into(),
            episode_file: TemplateEngine::EPISODE_FILE.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum RenameError {
    #[error("item is not scraped")]
    NotScraped,
    #[error("organize season folders is only for tv/anime")]
    NotTvShow,
    #[error("empty rename target")]
    EmptyTarget,
    #[error("destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("path not found: {0}")]
    NotFound(PathBuf),
    #[error("database: {0}")]
    Database(String),
    #[error("filesystem: {0}")]
    Filesystem(String),
}

pub fn rename_after_scrape(
    db: &AppDatabase,
    item: &MediaItem,
    templates: &RenameTemplates,
) -> Result<(), RenameError> {
    rename_after_scrape_with_options(db, item, templates, false)
}

/// Apply templates; when `create_season_folders` is true, ensure TV episodes live under Season XX.
pub fn rename_after_scrape_with_options(
    db: &AppDatabase,
    item: &MediaItem,
    templates: &RenameTemplates,
    create_season_folders: bool,
) -> Result<(), RenameError> {
    if item.status != ScrapedStatus::Scraped {
        return Err(RenameError::NotScraped);
    }
    match item.media_type {
        MediaType::Movie => rename_movie(db, item, templates),
        MediaType::TvShow | MediaType::Anime => {
            rename_tv_show(db, item, templates, create_season_folders)
        }
    }
}

/// Move episode files into `Season XX` folders under the show root (RENAME-T-06).
pub fn organize_season_folders(
    db: &AppDatabase,
    item: &MediaItem,
    templates: &RenameTemplates,
) -> Result<(), RenameError> {
    if item.status != ScrapedStatus::Scraped {
        return Err(RenameError::NotScraped);
    }
    if !matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
        return Err(RenameError::NotTvShow);
    }
    rename_tv_show(db, item, templates, true)
}

/// Merge duplicate TV/anime items that share TMDB/TVDB/Bangumi (or title+year).
/// Returns how many source items were absorbed into a canonical show.
pub fn consolidate_library_duplicate_shows(
    db: &AppDatabase,
    library_id: &str,
    templates: &RenameTemplates,
) -> Result<usize, RenameError> {
    let items = db
        .list_media_items(library_id)
        .map_err(|e| RenameError::Database(e.to_string()))?;
    let mut ranked: Vec<MediaItem> = items
        .into_iter()
        .filter(|i| matches!(i.media_type, MediaType::TvShow | MediaType::Anime))
        .collect();
    // Merge low-score release dumps into high-score canonical roots first.
    ranked.sort_by_key(|a| show_root_score(a));
    let mut merged = 0usize;
    for item in ranked {
        let Some(fresh) = db
            .get_media_item(&item.id)
            .map_err(|e| RenameError::Database(e.to_string()))?
        else {
            continue;
        };
        if consolidate_show_item(db, &fresh, templates)? {
            merged += 1;
        }
    }
    Ok(merged)
}

/// Dry run of the consolidation: which show would be absorbed into which, in the
/// order the merge would run. Nothing on disk or in the index changes.
pub fn plan_duplicate_show_merges(
    db: &AppDatabase,
    candidates: &[MediaItem],
) -> Result<Vec<(MediaItem, MediaItem)>, RenameError> {
    let mut ranked: Vec<&MediaItem> = candidates
        .iter()
        .filter(|i| matches!(i.media_type, MediaType::TvShow | MediaType::Anime))
        .collect();
    ranked.sort_by_key(|a| show_root_score(a));
    let mut absorbed = std::collections::HashSet::new();
    let mut plan = Vec::new();
    for item in ranked {
        if absorbed.contains(&item.id) {
            continue;
        }
        if let Some(target) = find_canonical_show_duplicate_excluding(db, item, &absorbed)? {
            absorbed.insert(item.id.clone());
            plan.push((item.clone(), target));
        }
    }
    Ok(plan)
}

/// Merge one pair the user confirmed from [`plan_duplicate_show_merges`]. Skips
/// (returns `false`) when the current index no longer picks the same target.
pub fn merge_planned_show(
    db: &AppDatabase,
    source_id: &str,
    target_id: &str,
    templates: &RenameTemplates,
) -> Result<bool, RenameError> {
    let Some(source) = db
        .get_media_item(source_id)
        .map_err(|e| RenameError::Database(e.to_string()))?
    else {
        return Ok(false);
    };
    match find_canonical_show_duplicate(db, &source)? {
        Some(target) if target.id == target_id => {
            merge_show_into(db, &source, &target, templates)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// If `item` is a duplicate of a better canonical show, merge it in and delete `item`.
pub fn consolidate_show_item(
    db: &AppDatabase,
    item: &MediaItem,
    templates: &RenameTemplates,
) -> Result<bool, RenameError> {
    let Some(target) = find_canonical_show_duplicate(db, item)? else {
        return Ok(false);
    };
    merge_show_into(db, item, &target, templates)?;
    Ok(true)
}

fn find_canonical_show_duplicate(
    db: &AppDatabase,
    item: &MediaItem,
) -> Result<Option<MediaItem>, RenameError> {
    find_canonical_show_duplicate_excluding(db, item, &std::collections::HashSet::new())
}

fn find_canonical_show_duplicate_excluding(
    db: &AppDatabase,
    item: &MediaItem,
    excluded: &std::collections::HashSet<String>,
) -> Result<Option<MediaItem>, RenameError> {
    if !matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
        return Ok(None);
    }
    let meta = db
        .fetch_metadata(&item.id)
        .map_err(|e| RenameError::Database(e.to_string()))?;
    let tmdb = meta
        .as_ref()
        .and_then(|m| m.tmdb_id.clone())
        .filter(|s| !s.is_empty());
    let bangumi = meta
        .as_ref()
        .and_then(|m| m.bangumi_id.clone())
        .filter(|s| !s.is_empty());
    let tvdb = meta
        .as_ref()
        .and_then(|m| m.tvdb_id.clone())
        .filter(|s| !s.is_empty());
    let title_key = normalize_show_title(&item.title);
    let year = item.year;

    let others = db
        .list_media_items(&item.library_id)
        .map_err(|e| RenameError::Database(e.to_string()))?;
    let mut matches = Vec::new();
    for other in others {
        if other.id == item.id || excluded.contains(&other.id) {
            continue;
        }
        if !matches!(other.media_type, MediaType::TvShow | MediaType::Anime) {
            continue;
        }
        // Never merge a show into one of its own nested subfolders (or vice versa)
        // unless identity matches — nested junk under the show root is a separate issue.
        let om = db
            .fetch_metadata(&other.id)
            .map_err(|e| RenameError::Database(e.to_string()))?;
        let same_provider = tmdb
            .as_ref()
            .zip(om.as_ref().and_then(|m| m.tmdb_id.as_ref()))
            .is_some_and(|(a, b)| a == b)
            || bangumi
                .as_ref()
                .zip(om.as_ref().and_then(|m| m.bangumi_id.as_ref()))
                .is_some_and(|(a, b)| a == b)
            || tvdb
                .as_ref()
                .zip(om.as_ref().and_then(|m| m.tvdb_id.as_ref()))
                .is_some_and(|(a, b)| a == b);
        // Two different provider records are two different shows, whatever the title says.
        let conflicting_provider = [
            (tmdb.as_ref(), om.as_ref().and_then(|m| m.tmdb_id.as_ref())),
            (bangumi.as_ref(), om.as_ref().and_then(|m| m.bangumi_id.as_ref())),
            (tvdb.as_ref(), om.as_ref().and_then(|m| m.tvdb_id.as_ref())),
        ]
        .iter()
        .any(|(a, b)| matches!((a, b), (Some(a), Some(b)) if !b.is_empty() && a != b));
        let same_title_year = !conflicting_provider
            && !title_key.is_empty()
            && title_key == normalize_show_title(&other.title)
            && year.is_some()
            && year == other.year
            && item.status == ScrapedStatus::Scraped
            && other.status == ScrapedStatus::Scraped;
        if same_provider || same_title_year {
            matches.push(other);
        }
    }
    if matches.is_empty() {
        return Ok(None);
    }

    let mut all = matches;
    all.push(item.clone());
    all.sort_by(|a, b| {
        show_root_score(b)
            .cmp(&show_root_score(a))
            .then_with(|| a.added_at.cmp(&b.added_at))
    });
    let best = all.remove(0);
    if best.id == item.id {
        Ok(None)
    } else {
        Ok(Some(best))
    }
}

fn merge_show_into(
    db: &AppDatabase,
    source: &MediaItem,
    target: &MediaItem,
    templates: &RenameTemplates,
) -> Result<(), RenameError> {
    crate::recover_media_operations(db).map_err(RenameError::Filesystem)?;
    if source.id == target.id {
        return Ok(());
    }
    if !media_core::media_files::owns_folder(db, source)
        .map_err(|e| RenameError::Filesystem(e.to_string()))?
    {
        return Err(RenameError::Filesystem(
            "cannot organize a shared show folder".into(),
        ));
    }
    if !media_core::media_files::owns_folder(db, target)
        .map_err(|e| RenameError::Filesystem(e.to_string()))?
    {
        return Err(RenameError::Filesystem(
            "cannot organize a shared show folder".into(),
        ));
    }
    let target_root = PathBuf::from(&target.folder_path);
    if !target_root.is_dir() {
        return Err(RenameError::NotFound(target_root));
    }
    // Reject duplicate episode identities or destination files before moving anything.
    let mut target_keys = std::collections::HashSet::new();
    for season in db
        .fetch_seasons(&target.id)
        .map_err(|e| RenameError::Database(e.to_string()))?
    {
        for episode in db
            .fetch_episodes(&season.id)
            .map_err(|e| RenameError::Database(e.to_string()))?
        {
            target_keys.insert((season.season_number, episode.episode_number));
        }
    }
    for season in db
        .fetch_seasons(&source.id)
        .map_err(|e| RenameError::Database(e.to_string()))?
    {
        let mut values = HashMap::new();
        values.insert("season".into(), season.season_number.to_string());
        let name = TemplateEngine::sanitize_filename(&TemplateEngine::render(
            &templates.season_folder,
            &values,
        ));
        for episode in db
            .fetch_episodes(&season.id)
            .map_err(|e| RenameError::Database(e.to_string()))?
        {
            if episode.file_path.is_empty() {
                continue;
            }
            let source_path = Path::new(&episode.file_path);
            if !source_path.is_file() {
                return Err(RenameError::NotFound(source_path.into()));
            }
            let file_name = source_path
                .file_name()
                .ok_or_else(|| RenameError::NotFound(source_path.into()))?;
            let destination = target_root.join(&name).join(file_name);
            if target_keys.contains(&(season.season_number, episode.episode_number))
                || destination.exists()
            {
                return Err(RenameError::DestinationExists(destination));
            }
        }
    }
    let fs = FilesystemService::new();
    let mut journal = crate::media_journal::MergeJournal::begin(db).map_err(RenameError::Filesystem)?;
    let mut moved_paths = Vec::new();
    let mut moved_files: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| -> Result<(), RenameError> {
        let seasons = db
            .fetch_seasons(&source.id)
            .map_err(|e| RenameError::Database(e.to_string()))?;
        for season in seasons {
            let episodes = db
                .fetch_episodes(&season.id)
                .map_err(|e| RenameError::Database(e.to_string()))?;
            let mut season_values = HashMap::new();
            season_values.insert("season".into(), season.season_number.to_string());
            let season_folder_name = TemplateEngine::sanitize_filename(&TemplateEngine::render(
                &templates.season_folder,
                &season_values,
            ));
            let dest_season = if season_folder_name.is_empty() {
                target_root.clone()
            } else {
                let dir = target_root.join(&season_folder_name);
                if !dir.exists() {
                    fs.create_directory(&dir)
                        .map_err(|e| RenameError::Filesystem(e.to_string()))?;
                }
                dir
            };

            for ep in episodes {
                if ep.file_path.is_empty() {
                    continue;
                }
                let src = PathBuf::from(&ep.file_path);
                if !src.is_file() {
                    return Err(RenameError::NotFound(src));
                }
                let file_name = src
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                if file_name.is_empty() {
                    continue;
                }
                let stem = src
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                let from_dir = src
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| dest_season.clone());
                let dest = dest_season.join(&file_name);
                if src != dest {
                    if dest.exists() {
                        return Err(RenameError::DestinationExists(dest));
                    }
                    journal.move_file(&src, &dest)
                        .map_err(|e| RenameError::Filesystem(e.to_string()))?;
                    moved_files.push((src.clone(), dest.clone()));
                    for entry in std::fs::read_dir(&from_dir)
                        .map_err(|e| RenameError::Filesystem(e.to_string()))?
                    {
                        let path = entry
                            .map_err(|e| RenameError::Filesystem(e.to_string()))?
                            .path();
                        if !path.is_file() {
                            continue;
                        }
                        let ext = path
                            .extension()
                            .and_then(|v| v.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        let file_stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("");
                        if !COMPANION_EXTENSIONS.contains(&ext.as_str())
                            || companion_suffix(file_stem, &stem).is_none()
                        {
                            continue;
                        }
                        let destination = dest_season.join(path.file_name().unwrap());
                        journal.move_file(&path, &destination)
                            .map_err(|e| RenameError::Filesystem(e.to_string()))?;
                        moved_files.push((path, destination));
                    }
                }
                moved_paths.push((ep.id, dest.to_string_lossy().into_owned()));
            }
        }

        db.merge_show_records(&source.id, &target.id, &moved_paths, &moved_files, &journal.id)
            .map_err(|e| RenameError::Database(e.to_string()))?;
        Ok(())
    })();
    if let Err(error) = result {
        return match crate::recover_media_operations(db) {
            Ok(()) => Err(error),
            Err(recovery) => Err(RenameError::Filesystem(format!("{error}; recovery pending: {recovery}"))),
        };
    }
    // A cleanup failure must never roll back an already committed merge.
    crate::recover_media_operations(db).map_err(RenameError::Filesystem)?;
    remove_merged_source_folder(source, target);
    Ok(())
}

/// Remove empty directories only. Unmoved sidecars and unknown files stay on disk.
fn remove_merged_source_folder(source: &MediaItem, target: &MediaItem) {
    let source_root = PathBuf::from(&source.folder_path);
    let target_root = PathBuf::from(&target.folder_path);
    if !source_root.is_dir() {
        return;
    }
    // Never touch the canonical root or nested parent/child paths.
    if source_root == target_root
        || source_root.starts_with(&target_root)
        || target_root.starts_with(&source_root)
    {
        return;
    }
    fn remove_empty(path: &Path) {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) { remove_empty(&entry.path()); }
            }
        }
        let _ = std::fs::remove_dir(path);
    }
    remove_empty(&source_root);
}

fn normalize_show_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn show_root_score(item: &MediaItem) -> i32 {
    let name = Path::new(&item.folder_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let lower = name.to_ascii_lowercase();
    let mut score = 0i32;
    if lower.contains("web-dl")
        || lower.contains("webrip")
        || lower.contains("1080p")
        || lower.contains("720p")
        || lower.contains("2160p")
        || lower.contains("hdtv")
        || lower.contains("www.")
        || name.contains('【')
        || (name.contains('[') && name.contains(']'))
    {
        score -= 100;
    }
    if FileNameParser::extract_season_suffix(name).is_some() {
        score -= 20;
    }
    // Prefer classic `Title (Year)` show roots.
    if name.contains('(') && name.ends_with(')') {
        score += 40;
    }
    score
}

fn rename_movie(db: &AppDatabase, item: &MediaItem, templates: &RenameTemplates) -> Result<(), RenameError> {
    rename_planned(db, item, templates, false)
}
fn rename_tv_show(db: &AppDatabase, item: &MediaItem, templates: &RenameTemplates, create_season_folders: bool) -> Result<(), RenameError> {
    rename_planned(db, item, templates, create_season_folders)
}

/// Plan final paths before moving anything; every file is moved at most once.
fn rename_planned(db: &AppDatabase, item: &MediaItem, templates: &RenameTemplates, create_seasons: bool) -> Result<(), RenameError> {
    crate::recover_media_operations(db).map_err(RenameError::Filesystem)?;
    let root = PathBuf::from(&item.folder_path);
    let exclusive = media_core::media_files::owns_folder(db, item).map_err(|e| RenameError::Filesystem(e.to_string()))?;
    let movie = item.media_type == MediaType::Movie;
    if !movie && !exclusive { return Err(RenameError::Filesystem("cannot organize a shared show folder".into())); }
    let values = build_values(db, item).map_err(RenameError::Database)?;
    let render = |template: &str, values: &HashMap<String,String>| -> Result<String, RenameError> {
        let name = TemplateEngine::sanitize_filename(&TemplateEngine::render(template, values));
        if name.is_empty() { Err(RenameError::EmptyTarget) } else { Ok(name) }
    };
    let folder_name = render(if movie { &templates.movie_folder } else { &templates.tv_show_folder }, &values)?;
    let mut new_root = if exclusive { root.parent().ok_or_else(|| RenameError::NotFound(root.clone()))?.join(folder_name) } else { root.clone() };
    let fs_error = |e: std::io::Error| RenameError::Filesystem(e.to_string());
    // `andor (2022)` → `Andor (2022)` on a case-insensitive volume: same directory.
    let case_only_root = new_root != root && media_core::is_case_only_rename(&root, &new_root).map_err(fs_error)?;
    if new_root != root && new_root.exists() && !case_only_root {
        if movie { return Err(RenameError::DestinationExists(new_root)); }
        new_root = root.clone(); // Existing show target: organize within current root.
    }
    let mut all_files = Vec::new();
    collect_files(&root, exclusive, &mut all_files)?;
    let mut destinations: HashMap<PathBuf, PathBuf> = HashMap::new();
    if exclusive {
        for path in &all_files { destinations.insert(path.clone(), new_root.join(path.strip_prefix(&root).unwrap())); }
    }
    let mut videos = Vec::new();
    if movie {
        let video = PathBuf::from(&item.file_path);
        let stem = render(&templates.movie_file, &values)?;
        let ext = video.extension().and_then(|s| s.to_str()).unwrap_or("mkv");
        videos.push((video.clone(), new_root.join(format!("{stem}.{ext}"))));
    } else {
        for season in db.fetch_seasons(&item.id).map_err(|e| RenameError::Database(e.to_string()))? {
            let mut season_values = values.clone();
            season_values.insert("season".into(), season.season_number.to_string());
            season_values.insert("seasonTitle".into(), season.title.clone().unwrap_or_default());
            let name = render(&templates.season_folder, &season_values)?;
            for ep in db.fetch_episodes(&season.id).map_err(|e| RenameError::Database(e.to_string()))? {
                if ep.file_path.is_empty() { continue; }
                let video = PathBuf::from(&ep.file_path);
                let relative = video.strip_prefix(&root).map_err(|_| RenameError::Filesystem("episode outside exclusive show root".into()))?;
                let old_parent = video.parent().unwrap();
                let directory = if create_seasons || old_parent.parent() == Some(root.as_path()) { new_root.join(&name) }
                    else { new_root.join(relative.parent().unwrap_or(Path::new(""))) };
                // Move season artwork and other files with a renamed direct season folder.
                if old_parent != root && old_parent.parent() == Some(root.as_path()) {
                    for path in &all_files {
                        if let Ok(rel) = path.strip_prefix(old_parent) { destinations.insert(path.clone(), directory.join(rel)); }
                    }
                }
                let stem = render(&templates.episode_file, &build_episode_values(&values, &season, &ep))?;
                let ext = video.extension().and_then(|s| s.to_str()).unwrap_or("mkv");
                videos.push((video.clone(), directory.join(format!("{stem}.{ext}"))));
            }
        }
    }
    for (video, destination) in &videos {
        if !video.is_file() { return Err(RenameError::NotFound(video.clone())); }
        destinations.insert(video.clone(), destination.clone());
        let old_stem = video.file_stem().and_then(|v| v.to_str()).unwrap_or("");
        let new_stem = destination.file_stem().and_then(|v| v.to_str()).unwrap_or("");
        for path in &all_files {
            if path.parent() != video.parent() { continue; }
            let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
            if !COMPANION_EXTENSIONS.contains(&ext.as_str()) { continue; }
            let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("");
            // Longest matching video stem owns a sidecar, including loose movies.
            let owner = all_files.iter().filter(|p| p.parent() == video.parent() && p.extension().and_then(|v| v.to_str()).is_some_and(|v| media_core::scanner::MEDIA_EXTENSIONS.contains(&v.to_ascii_lowercase().as_str())))
                .filter_map(|p| p.file_stem().and_then(|v| v.to_str())).filter(|v| companion_suffix(stem, v).is_some()).max_by_key(|v| v.len());
            if owner != Some(old_stem) { continue; }
            if let Some(name) = companion_dest_name(path.file_name().unwrap().to_str().unwrap_or(""), old_stem, new_stem) {
                destinations.insert(path.clone(), destination.parent().unwrap().join(name));
            }
        }
    }
    let mut files: Vec<_> = destinations.into_iter().collect();
    files.sort_by(|a,b| a.0.cmp(&b.0));
    // After a case-only root rename every source lives under the new spelling.
    let source = |from: &Path| -> PathBuf {
        match from.strip_prefix(&root) { Ok(rel) if case_only_root => new_root.join(rel), _ => from.to_path_buf() }
    };
    let mut targets = std::collections::HashSet::new();
    for (from, to) in &files {
        let from = source(from);
        let blocked = from != *to && to.try_exists().map_err(fs_error)? && !media_core::is_case_only_rename(&from, to).map_err(fs_error)?;
        if !targets.insert(to.clone()) || blocked { return Err(RenameError::DestinationExists(to.clone())); }
    }
    let new_file = if movie { videos[0].1.to_string_lossy().into_owned() } else { new_root.to_string_lossy().into_owned() };
    let mut journal = crate::media_journal::MergeJournal::begin(db).map_err(RenameError::Filesystem)?;
    let result = (|| {
        if case_only_root { journal.move_file(&root, &new_root).map_err(RenameError::Filesystem)?; }
        for (from,to) in &files { journal.move_file(&source(from), to).map_err(RenameError::Filesystem)?; }
        db.commit_media_paths(&item.id, &root, &new_root, &new_file, &files, &journal.id).map_err(|e| RenameError::Database(e.to_string()))
    })();
    if let Err(error) = result {
        return match crate::recover_media_operations(db) { Ok(()) => Err(error), Err(recovery) => Err(RenameError::Filesystem(format!("{error}; recovery pending: {recovery}"))) };
    }
    crate::recover_media_operations(db).map_err(RenameError::Filesystem)?;
    if exclusive && root != new_root && !case_only_root { remove_empty_directories(&root); }
    Ok(())
}

fn collect_files(root: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> Result<(), RenameError> {
    for entry in std::fs::read_dir(root).map_err(|e| RenameError::Filesystem(e.to_string()))? {
        let entry = entry.map_err(|e| RenameError::Filesystem(e.to_string()))?;
        let kind = entry.file_type().map_err(|e| RenameError::Filesystem(e.to_string()))?;
        if kind.is_symlink() { return Err(RenameError::Filesystem("symlink in rename plan".into())); }
        if kind.is_dir() && recursive { collect_files(&entry.path(), true, out)?; }
        else if kind.is_file() { out.push(entry.path()); }
    }
    Ok(())
}
fn remove_empty_directories(root: &Path) {
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() { if entry.file_type().is_ok_and(|k| k.is_dir()) { remove_empty_directories(&entry.path()); } }
    }
    let _ = std::fs::remove_dir(root);
}

fn build_values(db: &AppDatabase, item: &MediaItem) -> Result<HashMap<String, String>, String> {
    let mut values = HashMap::new();
    let meta = db.fetch_metadata(&item.id).map_err(|e| e.to_string())?;
    values.insert("title".into(), item.title.clone());
    if let Some(ot) = &item.original_title {
        values.insert("originalTitle".into(), ot.clone());
    }
    if let Some(year) = item.year {
        values.insert("year".into(), year.to_string());
    }
    if let Some(meta) = meta {
        if let Some(g) = meta.genres.first() {
            values.insert("genre".into(), g.clone());
        }
        if let Some(r) = meta.rating {
            values.insert("rating".into(), format!("{r:.1}"));
        }
        if let Some(c) = meta.content_rating {
            values.insert("contentRating".into(), c);
        }
        if let Some(d) = meta.director {
            values.insert("director".into(), d);
        }
        if let Some(s) = meta.studio {
            values.insert("studio".into(), s);
        }
        if let Some(c) = meta.country {
            values.insert("country".into(), c);
        }
        if let Some(cn) = meta.collection_name {
            values.insert("collection".into(), cn);
        }
        if let Some(vc) = meta.video_codec {
            values.insert("videoCodec".into(), vc);
        }
        if let Some(vr) = meta.video_resolution {
            values.insert("videoResolution".into(), vr);
        }
        if let Some(ac) = meta.audio_codec {
            values.insert("audioCodec".into(), ac);
        }
        if let Some(ach) = meta.audio_channels {
            values.insert("audioChannels".into(), ach);
        }
        if let Some(i) = meta.imdb_id {
            values.insert("imdbId".into(), i);
        }
        if let Some(t) = meta.tmdb_id {
            values.insert("tmdbId".into(), t);
        }
    }
    Ok(values)
}

fn build_episode_values(
    show_values: &HashMap<String, String>,
    season: &TvSeason,
    ep: &TvEpisode,
) -> HashMap<String, String> {
    let mut v = show_values.clone();
    v.insert("season".into(), season.season_number.to_string());
    v.insert(
        "seasonTitle".into(),
        season.title.clone().unwrap_or_default(),
    );
    v.insert("episode".into(), ep.episode_number.to_string());
    v.insert(
        "episodeTitle".into(),
        ep.title.clone().unwrap_or_default(),
    );
    if let Some(an) = ep.absolute_number {
        v.insert("absoluteNumber".into(), an.to_string());
    }
    if let Some(ad) = &ep.air_date {
        v.insert("airDate".into(), ad.clone());
    }
    v
}

fn companion_dest_name(file_name: &str, old_stem: &str, new_stem: &str) -> Option<String> {
    let path = Path::new(file_name);
    let stem = path.file_stem()?.to_str()?;
    let ext = path.extension()?.to_str()?;
    let suffix = companion_suffix(stem, old_stem)?;
    Some(format!("{new_stem}{suffix}.{ext}"))
}


#[cfg(test)]
mod tests {
    use super::*;
    use media_core::scanner::ScannedEpisode;
    use media_core::{Library, MediaType, ScrapedStatus};
    use tempfile::tempdir;

    #[test]
    fn template_commit_failure_restores_video_sidecar_and_database() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Old");
        std::fs::create_dir(&root).unwrap();
        let video = root.join("Old.mkv");
        std::fs::write(&video, "video").unwrap();
        std::fs::write(root.join("Old.srt"), "subtitle").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("Movies", dir.path().to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let item = MediaItem::new_movie("New", None, root.to_string_lossy(), video.to_string_lossy(), lib.id, ScrapedStatus::Scraped);
        db.insert_media_items(&[item.clone()]).unwrap();
        db.with_conn(|c| { c.execute_batch("CREATE TRIGGER fail_paths BEFORE UPDATE OF folderPath ON media_items BEGIN SELECT RAISE(ABORT, 'injected'); END;")?; Ok(()) }).unwrap();
        let templates = RenameTemplates { movie_folder: "New".into(), movie_file: "New".into(), ..Default::default() };
        assert!(rename_after_scrape(&db, &item, &templates).is_err());
        assert!(video.is_file());
        assert!(root.join("Old.srt").is_file());
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().file_path, item.file_path);
        assert!(db.media_operations().unwrap().is_empty());
        assert!(!dir.path().join("New").exists());
        db.with_conn(|c| { c.execute_batch("DROP TRIGGER fail_paths")?; Ok(()) }).unwrap();
        rename_after_scrape(&db, &item, &templates).unwrap();
        assert!(dir.path().join("New/New.mkv").is_file());
        assert!(dir.path().join("New/New.srt").is_file());
    }

    #[test]
    fn template_duplicate_destinations_are_rejected_before_any_move() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Old");
        std::fs::create_dir(&root).unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
        db.insert_library(&lib).unwrap();
        let item = MediaItem::new_show(MediaType::TvShow, "New", None, root.to_string_lossy(), lib.id, ScrapedStatus::Scraped);
        db.insert_media_items(&[item.clone()]).unwrap();
        for number in 1..=2 {
            let path = root.join(format!("S01E0{number}.mkv"));
            std::fs::write(&path, "video").unwrap();
            db.insert_show_episodes(&item.id, &[ScannedEpisode { season:1, episode:number, title:String::new(), file_path:path.to_string_lossy().into_owned() }]).unwrap();
        }
        let templates = RenameTemplates { tv_show_folder: "New".into(), episode_file: "same".into(), ..Default::default() };
        assert!(rename_after_scrape(&db, &item, &templates).is_err());
        assert!(root.join("S01E01.mkv").is_file());
        assert!(root.join("S01E02.mkv").is_file());
        assert!(!dir.path().join("New").exists());
    }

    #[test]
    fn merge_database_failure_rolls_back_video_and_sidecars_then_retry_preserves_metadata() {
        let dir = tempdir().unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
        db.insert_library(&lib).unwrap();
        let mut shows = Vec::new();
        for (name, season) in [("Target", 1), ("Source", 2)] {
            let folder = dir.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            let video = folder.join(format!("S0{season}E01.mkv"));
            std::fs::write(&video, name).unwrap();
            let show = MediaItem::new_show(
                MediaType::TvShow,
                "Show",
                None,
                folder.to_string_lossy(),
                lib.id.clone(),
                ScrapedStatus::Scraped,
            );
            db.insert_media_items(&[show.clone()]).unwrap();
            db.insert_show_episodes(
                &show.id,
                &[ScannedEpisode {
                    season,
                    episode: 1,
                    file_path: video.to_string_lossy().into_owned(),
                    title: "Title".into(),
                }],
            )
            .unwrap();
            shows.push(show);
        }
        let target = &shows[0];
        let source = &shows[1];
        let source_season = db.fetch_seasons(&source.id).unwrap().remove(0);
        let mut original = db.fetch_episodes(&source_season.id).unwrap().remove(0);
        original.overview = Some("Keep overview".into());
        original.rating = Some(8.5);
        original.still_path = Some("S02E01-thumb.jpg".into());
        db.upsert_episode(&original).unwrap();
        let subtitle = dir.path().join("Source/S02E01.zh.srt");
        std::fs::write(&subtitle, "subtitle").unwrap();
        std::fs::write(dir.path().join("Source/S02E01-thumb.jpg"), "image").unwrap();
        db.with_conn(|conn| { conn.execute_batch("CREATE TRIGGER fail_merge BEFORE DELETE ON media_items BEGIN SELECT RAISE(ABORT, 'injected'); END;")?; Ok(()) }).unwrap();
        assert!(merge_show_into(&db, source, target, &RenameTemplates::default()).is_err());
        assert!(std::path::Path::new(&original.file_path).exists());
        assert!(subtitle.exists());
        assert!(!dir.path().join("Target/Season 02/S02E01.mkv").exists());
        assert!(db.get_media_item(&source.id).unwrap().is_some());
        assert_eq!(
            db.fetch_episodes(&source_season.id).unwrap()[0].file_path,
            original.file_path
        );
        assert_eq!(db.fetch_seasons(&target.id).unwrap().len(), 1);
        db.with_conn(|conn| {
            conn.execute_batch("DROP TRIGGER fail_merge")?;
            Ok(())
        })
        .unwrap();
        merge_show_into(&db, source, target, &RenameTemplates::default()).unwrap();
        let merged_season = db
            .fetch_seasons(&target.id)
            .unwrap()
            .into_iter()
            .find(|s| s.season_number == 2)
            .unwrap();
        let merged = db.fetch_episodes(&merged_season.id).unwrap().remove(0);
        assert_eq!(merged.id, original.id);
        assert_eq!(merged.overview, original.overview);
        assert_eq!(merged.rating, original.rating);
        assert!(Path::new(merged.still_path.as_ref().unwrap()).is_file());
        assert!(Path::new(&merged.file_path).is_file());
        assert!(db.get_media_item(&source.id).unwrap().is_none());
    }

    #[test]
    fn conflicting_episode_merge_preserves_both_sources_and_records() {
        let dir = tempdir().unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
        db.insert_library(&lib).unwrap();
        let mut shows = Vec::new();
        for name in ["Main", "Duplicate"] {
            let folder = dir.path().join(name);
            std::fs::create_dir(&folder).unwrap();
            let video = folder.join("S01E01.mkv");
            std::fs::write(&video, name).unwrap();
            let item = MediaItem::new_show(MediaType::TvShow, "Show", Some(2020), folder.to_string_lossy(), lib.id.clone(), ScrapedStatus::Scraped);
            db.insert_media_items(&[item.clone()]).unwrap();
            db.insert_show_episodes(&item.id, &[ScannedEpisode {
                season: 1, episode: 1, file_path: video.to_string_lossy().into_owned(), title: "Episode".into(),
            }]).unwrap();
            shows.push(item);
        }
        assert!(merge_show_into(&db, &shows[1], &shows[0], &RenameTemplates::default()).is_err());
        assert_eq!(db.list_media_items(&lib.id).unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(dir.path().join("Main/S01E01.mkv")).unwrap(), "Main");
        assert_eq!(std::fs::read_to_string(dir.path().join("Duplicate/S01E01.mkv")).unwrap(), "Duplicate");
    }

    #[test]
    fn loose_movie_rename_preserves_library_root_and_other_movies() {
        let dir = tempdir().unwrap();
        let video = dir.path().join("Old.mkv");
        std::fs::write(&video, b"a").unwrap();
        std::fs::write(dir.path().join("Other.mkv"), b"b").unwrap();
        std::fs::write(dir.path().join("Old.zh.srt"), b"own subtitle").unwrap();
        std::fs::write(dir.path().join("Old.Extended.mkv"), b"other movie").unwrap();
        std::fs::write(dir.path().join("Old.Extended.srt"), b"other subtitle").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("Movies", dir.path().to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let item = MediaItem::new_movie("New", Some(2020), dir.path().to_string_lossy(), video.to_string_lossy(), lib.id, ScrapedStatus::Scraped);
        db.insert_media_items(&[item.clone()]).unwrap();
        let templates = RenameTemplates { movie_file: "New".into(), movie_folder: "MustNotMoveRoot".into(), ..Default::default() };
        rename_after_scrape(&db, &item, &templates).unwrap();
        assert!(dir.path().join("New.mkv").is_file());
        assert!(dir.path().join("Other.mkv").is_file());
        assert!(dir.path().join("New.zh.srt").is_file());
        assert!(dir.path().join("Old.Extended.srt").is_file());
        assert!(!dir.path().join("New.Extended.srt").exists());
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().folder_path, item.folder_path);
    }

    #[test]
    fn organizes_flat_episodes_into_season_xx() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Andor (2022)");
        std::fs::create_dir_all(&show).unwrap();
        let ep1 = show.join("Andor.S01E01.mkv");
        let ep2 = show.join("Andor.S01E02.mkv");
        std::fs::write(&ep1, b"x").unwrap();
        std::fs::write(&ep2, b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_show(
            MediaType::TvShow,
            "Andor",
            Some(2022),
            show.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Scraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();
        db.insert_show_episodes(
            &item.id,
            &[
                ScannedEpisode {
                    season: 1,
                    episode: 1,
                    file_path: ep1.display().to_string(),
                    title: "Ep1".into(),
                },
                ScannedEpisode {
                    season: 1,
                    episode: 2,
                    file_path: ep2.display().to_string(),
                    title: "Ep2".into(),
                },
            ],
        )
        .unwrap();

        organize_season_folders(&db, &item, &RenameTemplates::default()).unwrap();

        let season_dir = show.join("Season 01");
        assert!(season_dir.is_dir());
        let eps = db.fetch_episodes(&format!("{}_S1", item.id)).unwrap();
        assert_eq!(eps.len(), 2);
        assert!(eps.iter().all(|e| Path::new(&e.file_path).starts_with(&season_dir)));
    }

    #[test]
    fn destination_exists_still_organizes_in_place() {
        let dir = tempdir().unwrap();
        // Collision target already present (e.g. another season of the same series).
        let existing = dir.path().join("Solitary Gourmet (2012)");
        std::fs::create_dir_all(&existing).unwrap();
        let release = dir.path().join("Solitary Gourmet S11 WEB-DL");
        std::fs::create_dir_all(&release).unwrap();
        let ep1 = release.join("S11E01.mkv");
        std::fs::write(&ep1, b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_show(
            MediaType::TvShow,
            "Solitary Gourmet",
            Some(2012),
            release.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Scraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();
        db.insert_show_episodes(
            &item.id,
            &[ScannedEpisode {
                season: 11,
                episode: 1,
                file_path: ep1.display().to_string(),
                title: "Ep1".into(),
            }],
        )
        .unwrap();

        rename_after_scrape_with_options(&db, &item, &RenameTemplates::default(), true).unwrap();

        // Show folder rename skipped due to collision, but Season 11 + template rename still run.
        assert!(release.join("Season 11").is_dir() || existing.join("Season 11").is_dir());
        let eps = db.fetch_episodes(&format!("{}_S11", item.id)).unwrap();
        assert_eq!(eps.len(), 1);
        assert!(
            Path::new(&eps[0].file_path).components().any(|c| {
                c.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("Season 11")
            }),
            "episode should live under Season 11, got {}",
            eps[0].file_path
        );
    }

    #[test]
    fn consolidates_duplicate_tmdb_season_pack_into_canonical() {
        let dir = tempdir().unwrap();
        let main = dir.path().join("孤独的美食家 (2012)");
        let pack = dir.path().join("孤独的美食家.第十一季.WEB-DL.1080p");
        std::fs::create_dir_all(main.join("Season 01")).unwrap();
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(main.join("Season 01/S01E01.mkv"), b"x").unwrap();
        let s11 = pack.join("S11E01.mkv");
        std::fs::write(&s11, b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();

        let canonical = MediaItem::new_show(
            MediaType::TvShow,
            "孤独的美食家",
            Some(2012),
            main.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Scraped,
        );
        let duplicate = MediaItem::new_show(
            MediaType::TvShow,
            "孤独的美食家",
            Some(2012),
            pack.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Scraped,
        );
        db.insert_media_items(&[canonical.clone(), duplicate.clone()])
            .unwrap();
        db.insert_show_episodes(
            &canonical.id,
            &[ScannedEpisode {
                season: 1,
                episode: 1,
                file_path: main.join("Season 01/S01E01.mkv").display().to_string(),
                title: "Ep1".into(),
            }],
        )
        .unwrap();
        db.insert_show_episodes(
            &duplicate.id,
            &[ScannedEpisode {
                season: 11,
                episode: 1,
                file_path: s11.display().to_string(),
                title: "Ep1".into(),
            }],
        )
        .unwrap();

        let n = consolidate_library_duplicate_shows(&db, &library.id, &RenameTemplates::default())
            .unwrap();
        assert_eq!(n, 1);
        assert!(db.get_media_item(&duplicate.id).unwrap().is_none());
        let seasons = db.fetch_seasons(&canonical.id).unwrap();
        let nums: std::collections::HashSet<_> =
            seasons.iter().map(|s| s.season_number).collect();
        assert!(nums.contains(&1) && nums.contains(&11));
        assert!(main.join("Season 11").is_dir());
        assert!(
            !pack.exists(),
            "merged source folder should be removed after consolidate"
        );
    }

    #[test]
    fn merge_plan_is_read_only_and_respects_provider_ids() {
        let dir = tempdir().unwrap();
        let main = dir.path().join("Show (2020)");
        let pack = dir.path().join("Show.S02.1080p");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::create_dir_all(&pack).unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        let canonical = MediaItem::new_show(MediaType::TvShow, "Show", Some(2020), main.display().to_string(), library.id.clone(), ScrapedStatus::Scraped);
        let duplicate = MediaItem::new_show(MediaType::TvShow, "Show", Some(2020), pack.display().to_string(), library.id.clone(), ScrapedStatus::Scraped);
        db.insert_media_items(&[canonical.clone(), duplicate.clone()]).unwrap();
        let set_tmdb = |id: &str, tmdb: &str| db.with_conn(|c| {
            c.execute("INSERT INTO media_metadata (mediaItemId, sourceId, tmdbId, scrapedAt) VALUES (?1, 'tmdb', ?2, '2026-01-01T00:00:00Z')
                ON CONFLICT(mediaItemId) DO UPDATE SET tmdbId=excluded.tmdbId", [id, tmdb])?;
            Ok(())
        }).unwrap();

        let plan = plan_duplicate_show_merges(&db, &db.list_media_items(&library.id).unwrap()).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!((plan[0].0.id.as_str(), plan[0].1.id.as_str()), (duplicate.id.as_str(), canonical.id.as_str()));
        assert!(pack.is_dir() && db.get_media_item(&duplicate.id).unwrap().is_some());

        // Same title and year, different TMDB records: two shows, never merged.
        set_tmdb(&canonical.id, "100");
        set_tmdb(&duplicate.id, "200");
        assert!(plan_duplicate_show_merges(&db, &db.list_media_items(&library.id).unwrap()).unwrap().is_empty());
        assert!(!merge_planned_show(&db, &duplicate.id, &canonical.id, &RenameTemplates::default()).unwrap());
        assert!(db.get_media_item(&duplicate.id).unwrap().is_some());
    }

    #[test]
    fn case_only_movie_rename_renames_folder_and_file_in_place() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("dune (2021)");
        std::fs::create_dir_all(&folder).unwrap();
        let video = folder.join("dune (2021).mkv");
        std::fs::write(&video, b"x").unwrap();
        std::fs::write(folder.join("dune (2021).srt"), b"sub").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("Movies", dir.path().display().to_string(), MediaType::Movie);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_movie("Dune", Some(2021), folder.display().to_string(), video.display().to_string(), library.id.clone(), ScrapedStatus::Scraped);
        db.insert_media_items(&[item.clone()]).unwrap();

        rename_after_scrape(&db, &item, &RenameTemplates::default()).unwrap();

        let new_folder = dir.path().join("Dune (2021)");
        let new_video = new_folder.join("Dune (2021).mkv");
        assert!(media_core::entry_name_exists(&new_folder).unwrap());
        assert!(media_core::entry_name_exists(&new_video).unwrap());
        assert!(media_core::entry_name_exists(&new_folder.join("Dune (2021).srt")).unwrap());
        let stored = db.get_media_item(&item.id).unwrap().unwrap();
        assert_eq!(stored.folder_path, new_folder.display().to_string());
        assert_eq!(stored.file_path, new_video.display().to_string());
        assert!(db.media_operations().unwrap().is_empty());
    }

    #[test]
    fn organizes_moves_thumb_and_subtitle_companions() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Andor (2022)");
        std::fs::create_dir_all(&show).unwrap();
        let ep1 = show.join("Andor.S01E01.mkv");
        std::fs::write(&ep1, b"x").unwrap();
        std::fs::write(show.join("Andor.S01E01-thumb.jpg"), b"img").unwrap();
        std::fs::write(show.join("Andor.S01E01.zh.srt"), b"sub").unwrap();
        std::fs::write(show.join("poster.jpg"), b"poster").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_show(
            MediaType::TvShow,
            "Andor",
            Some(2022),
            show.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Scraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();
        db.insert_show_episodes(
            &item.id,
            &[ScannedEpisode {
                season: 1,
                episode: 1,
                file_path: ep1.display().to_string(),
                title: "Ep1".into(),
            }],
        )
        .unwrap();
        let season_id = format!("{}_S1", item.id);
        let mut eps = db.fetch_episodes(&season_id).unwrap();
        eps[0].still_path = Some("Andor.S01E01-thumb.jpg".into());
        db.upsert_episode(&eps[0]).unwrap();

        organize_season_folders(&db, &item, &RenameTemplates::default()).unwrap();

        let season_dir = show.join("Season 01");
        assert!(season_dir.is_dir());
        assert!(show.join("poster.jpg").is_file());
        assert!(!show.join("Andor.S01E01-thumb.jpg").exists());
        assert!(!show.join("Andor.S01E01.zh.srt").exists());

        let eps = db.fetch_episodes(&season_id).unwrap();
        let new_path = PathBuf::from(&eps[0].file_path);
        assert!(new_path.starts_with(&season_dir));
        assert!(new_path.is_file());
        let new_stem = new_path.file_stem().unwrap().to_str().unwrap();
        assert!(season_dir.join(format!("{new_stem}-thumb.jpg")).is_file());
        assert!(season_dir.join(format!("{new_stem}.zh.srt")).is_file());
        assert_eq!(
            eps[0].still_path.as_deref(),
            Some(format!("Season 01/{new_stem}-thumb.jpg").as_str())
        );
    }

    #[test]
    fn rename_updates_thumb_companion_stem() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Andor (2022)");
        let season = show.join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        let ep1 = season.join("Andor.S01E01.mkv");
        std::fs::write(&ep1, b"x").unwrap();
        std::fs::write(season.join("Andor.S01E01-thumb.jpg"), b"img").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_show(
            MediaType::TvShow,
            "Andor",
            Some(2022),
            show.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Scraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();
        db.insert_show_episodes(
            &item.id,
            &[ScannedEpisode {
                season: 1,
                episode: 1,
                file_path: ep1.display().to_string(),
                title: "Kassa".into(),
            }],
        )
        .unwrap();
        let season_id = format!("{}_S1", item.id);
        let mut eps = db.fetch_episodes(&season_id).unwrap();
        eps[0].still_path = Some("Season 01/Andor.S01E01-thumb.jpg".into());
        db.upsert_episode(&eps[0]).unwrap();

        rename_after_scrape_with_options(&db, &item, &RenameTemplates::default(), false).unwrap();

        let eps = db.fetch_episodes(&season_id).unwrap();
        let new_path = PathBuf::from(&eps[0].file_path);
        assert!(new_path.exists());
        let new_stem = new_path.file_stem().unwrap().to_str().unwrap();
        assert_ne!(new_stem, "Andor.S01E01");
        assert!(season.join(format!("{new_stem}-thumb.jpg")).is_file());
        assert!(!season.join("Andor.S01E01-thumb.jpg").exists());
        assert_eq!(
            eps[0].still_path.as_deref(),
            Some(format!("Season 01/{new_stem}-thumb.jpg").as_str())
        );
    }
}
