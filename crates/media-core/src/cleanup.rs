//! Residual companion cleanup (MAINT-04/05).
//!
//! Residuals = sidecar files (nfo/srt/images/…) whose stem no longer matches any
//! current media/episode file under a scraped item's folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::filesystem::{FilesystemError, FilesystemService};
use crate::models::{MediaType, ScrapedStatus};
use crate::scanner::MEDIA_EXTENSIONS;
use crate::AppDatabase;
use crate::DatabaseError;

pub const COMPANION_EXTENSIONS: &[&str] = &[
    "nfo", "srt", "ass", "ssa", "sub", "idx", "sup", "jpg", "jpeg", "png", "webp", "tbn",
];

/// Sidecar name suffix relative to a media stem.
///
/// Accepts exact stem, subtitle style (`stem.zh`), and image style (`stem-thumb` / `stem_thumb`).
pub fn companion_suffix<'a>(file_stem: &'a str, media_stem: &str) -> Option<&'a str> {
    if media_stem.is_empty() {
        return None;
    }
    if file_stem == media_stem {
        return Some("");
    }
    let rest = file_stem.strip_prefix(media_stem)?;
    if rest.starts_with('.') || rest.starts_with('-') || rest.starts_with('_') {
        Some(rest)
    } else {
        None
    }
}

const KEEP_BASENAMES: &[&str] = &[
    "tvshow.nfo",
    "movie.nfo",
    "poster.jpg",
    "poster.png",
    "poster.jpeg",
    "poster.webp",
    "fanart.jpg",
    "fanart.png",
    "banner.jpg",
    "banner.png",
    "logo.png",
    "logo.svg",
    "clearlogo.png",
    "folder.jpg",
    "cover.jpg",
    "backdrop.jpg",
    "background.jpg",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidualCandidate {
    pub path: String,
    pub item_id: String,
    pub item_title: String,
    pub reason: String,
    pub size: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error(transparent)]
    Filesystem(#[from] FilesystemError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Walk(#[from] walkdir::Error),
}

/// Dry-run: list orphan companion files under scraped items' folders.
pub fn find_residuals(
    db: &AppDatabase,
    item_ids: &[String],
) -> Result<Vec<ResidualCandidate>, CleanupError> {
    let mut out = Vec::new();
    for id in item_ids {
        let Some(item) = db.get_media_item(id)? else {
            continue;
        };
        if item.status != ScrapedStatus::Scraped {
            continue;
        }
        if item.folder_path.is_empty() {
            continue;
        }
        let folder = PathBuf::from(&item.folder_path);
        if !folder.is_dir() {
            continue;
        }
        // A shared directory has no unambiguous owner for orphan files.
        if !crate::media_files::owns_folder(db, &item)
            .map_err(|e| std::io::Error::other(e.to_string()))? {
            continue;
        }

        let mut keep_paths: HashSet<String> = HashSet::new();
        let mut keep_stems: HashSet<String> = HashSet::new();

        if !item.file_path.is_empty() {
            remember_file(&item.file_path, &mut keep_paths, &mut keep_stems);
        }
        if matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
            for season in db.fetch_seasons(&item.id)? {
                for ep in db.fetch_episodes(&season.id)? {
                    if !ep.file_path.is_empty() {
                        remember_file(&ep.file_path, &mut keep_paths, &mut keep_stems);
                    }
                    if let Some(still) = &ep.still_path {
                        remember_relative(&folder, still, &mut keep_paths, &mut keep_stems);
                    }
                }
                if let Some(poster) = &season.poster_path {
                    remember_relative(&folder, poster, &mut keep_paths, &mut keep_stems);
                }
            }
        }
        if let Ok(Some(meta)) = db.fetch_metadata(&item.id) {
            for rel in [
                meta.poster_path,
                meta.fanart_path,
                meta.banner_path,
                meta.logo_path,
                meta.thumb_path,
            ]
            .into_iter()
            .flatten()
            {
                remember_relative(&folder, &rel, &mut keep_paths, &mut keep_stems);
            }
        }

        let walker = WalkDir::new(&folder).follow_links(false).into_iter().filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.')
        });

        for entry in walker {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let abs = canonicalize_lossy(path);
            if keep_paths.contains(&abs) {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if KEEP_BASENAMES
                .iter()
                .any(|b| name.eq_ignore_ascii_case(b))
            {
                continue;
            }

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if MEDIA_EXTENSIONS.contains(&ext.as_str()) {
                // Leave unknown video files alone (extras / samples).
                continue;
            }
            if !COMPANION_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }

            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            // Match exact stem, `stem.lang` subtitles, or `stem-thumb` / `stem_thumb` stills.
            let stem_ok = keep_stems
                .iter()
                .any(|k| companion_suffix(&stem, k).is_some());
            if stem_ok {
                continue;
            }

            let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            out.push(ResidualCandidate {
                path: abs,
                item_id: item.id.clone(),
                item_title: item.title.clone(),
                reason: "orphanCompanion".into(),
                size,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    Ok(out)
}

/// Move residual files to trash. Never silently fall back to permanent deletion.
pub fn perform_cleanup(paths: &[String]) -> Result<usize, CleanupError> {
    let fs = FilesystemService::new();
    let mut n = 0usize;
    for path in paths {
        let p = PathBuf::from(path);
        if !p.is_file() {
            continue;
        }
        match fs.trash_item(&p) {
            Ok(_) => n += 1,
            Err(FilesystemError::NotFound(_)) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(n)
}

fn remember_file(path: &str, keep_paths: &mut HashSet<String>, keep_stems: &mut HashSet<String>) {
    let p = PathBuf::from(path);
    keep_paths.insert(canonicalize_lossy(&p));
    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
        keep_stems.insert(stem.to_string());
    }
}

fn remember_relative(
    folder: &Path,
    rel: &str,
    keep_paths: &mut HashSet<String>,
    keep_stems: &mut HashSet<String>,
) {
    let p = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        folder.join(rel)
    };
    keep_paths.insert(canonicalize_lossy(&p));
    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
        keep_stems.insert(stem.to_string());
    }
}

fn canonicalize_lossy(path: &Path) -> String {
    crate::scanner::canonicalize_lossy(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Library, MediaType, ScrapedStatus};
    use crate::AppDatabase;
    use tempfile::tempdir;

    #[test]
    fn shared_movie_directory_never_proposes_other_movies_subtitles() {
        let dir = tempdir().unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("Movies", dir.path().to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let mut ids = Vec::new();
        for name in ["A", "B"] {
            let video = dir.path().join(format!("{name}.mkv"));
            std::fs::write(&video, b"video").unwrap();
            std::fs::write(dir.path().join(format!("{name}.srt")), b"subtitle").unwrap();
            let item = crate::MediaItem::new_movie(name, None, dir.path().to_string_lossy(), video.to_string_lossy(), lib.id.clone(), ScrapedStatus::Scraped);
            ids.push(item.id.clone());
            db.insert_media_items(&[item]).unwrap();
        }
        assert!(find_residuals(&db, &ids[..1]).unwrap().is_empty());
        assert!(find_residuals(&db, &ids).unwrap().is_empty());
    }

    #[test]
    fn companion_suffix_accepts_dot_dash_underscore() {
        assert_eq!(companion_suffix("Show.S01E01", "Show.S01E01"), Some(""));
        assert_eq!(companion_suffix("Show.S01E01.zh", "Show.S01E01"), Some(".zh"));
        assert_eq!(companion_suffix("Show.S01E01-thumb", "Show.S01E01"), Some("-thumb"));
        assert_eq!(companion_suffix("Show.S01E01_thumb", "Show.S01E01"), Some("_thumb"));
        assert_eq!(companion_suffix("Show.S01E02", "Show.S01E01"), None);
        assert_eq!(companion_suffix("Show.S01E01extra", "Show.S01E01"), None);
    }

    #[test]
    fn finds_orphan_nfo_after_rename() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("Movie (2020)");
        std::fs::create_dir_all(&folder).unwrap();
        let media = folder.join("Movie (2020).mkv");
        std::fs::write(&media, b"x").unwrap();
        let orphan = folder.join("Old.Name.nfo");
        std::fs::write(&orphan, b"nfo").unwrap();
        let keep_nfo = folder.join("Movie (2020).nfo");
        std::fs::write(&keep_nfo, b"nfo").unwrap();
        let poster = folder.join("poster.jpg");
        std::fs::write(&poster, b"img").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new(
            "Movies",
            dir.path().to_string_lossy(),
            MediaType::Movie,
        );
        db.insert_library(&lib).unwrap();
        let item = crate::models::MediaItem::new_movie(
            "Movie",
            Some(2020),
            folder.to_string_lossy(),
            media.to_string_lossy(),
            lib.id.clone(),
            ScrapedStatus::Scraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();

        let found = find_residuals(&db, &[item.id.clone()]).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].path.ends_with("Old.Name.nfo"));
    }
}
