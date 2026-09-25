//! Residual companion cleanup (MAINT-04/05).
//!
//! Residuals = sidecar files (nfo/srt/images/…) whose stem no longer matches any
//! current media/episode file under a scraped item's folder.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use walkdir::WalkDir;

use crate::filesystem::{FilesystemError, FilesystemService};
use crate::models::{MediaType, ScrapedStatus};
use crate::scanner::{FileNameParser, MEDIA_EXTENSIONS};
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

const KEEP_BASENAMES: &[&str] = &["tvshow.nfo", "movie.nfo", "season.nfo", "season-specials.nfo"];

/// Kodi / Emby / Jellyfin artwork stems kept with any image extension.
const KEEP_ART_STEMS: &[&str] = &[
    "poster", "fanart", "banner", "logo", "clearlogo", "clearart", "landscape", "thumb", "discart",
    "disc", "folder", "cover", "backdrop", "background", "keyart", "characterart", "art",
];

/// Directories that hold artwork/extras/subtitles by convention and are never scanned.
const KEEP_DIRS: &[&str] = &[
    "extrafanart", "extrathumbs", "subs", "subtitles", ".actors", "extras", "featurettes",
    "behind the scenes", "deleted scenes", "interviews", "scenes", "shorts", "trailers", "backdrops",
    "theme-music", "other",
];

const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "ass", "ssa", "sub", "idx", "sup"];

/// Library-level metadata/artwork that belongs to the folder, not to one media file:
/// `poster.jpg`, `clearart.png`, `fanart1.jpg`, `season01-poster.jpg`,
/// `season-specials-landscape.jpg`, `season02.nfo`, `season 02.nfo`, …
fn is_known_folder_artwork(name: &str) -> bool {
    static RE_ART: OnceLock<Regex> = OnceLock::new();
    static RE_SEASON: OnceLock<Regex> = OnceLock::new();
    let lower = name.to_lowercase();
    if KEEP_BASENAMES.contains(&lower.as_str()) {
        return true;
    }
    let (stem, ext) = lower.rsplit_once('.').unwrap_or((lower.as_str(), ""));
    let is_image = matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "tbn" | "gif" | "svg");
    let re_art = RE_ART.get_or_init(|| {
        let stems = KEEP_ART_STEMS.join("|");
        Regex::new(&format!(r"^(?:{stems})\d*$")).unwrap()
    });
    if is_image && re_art.is_match(stem) {
        return true;
    }
    let re_season = RE_SEASON.get_or_init(|| {
        let stems = KEEP_ART_STEMS.join("|");
        Regex::new(&format!(r"^season[ ._-]?(?:\d{{1,3}}|specials|all)(?:-?(?:{stems}))?$")).unwrap()
    });
    re_season.is_match(stem) && (is_image || ext == "nfo")
}

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

        // A show's `file_path` is its folder; its stem (`Show`) would keep every `Show.*` sidecar.
        if !item.file_path.is_empty() && item.media_type == MediaType::Movie {
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

        let folder_name = folder.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let is_show = matches!(item.media_type, MediaType::TvShow | MediaType::Anime);
        let videos = indexed_videos(&keep_paths);
        let walker = WalkDir::new(&folder).follow_links(false).into_iter().filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            let keep_dir = e.file_type().is_dir() && KEEP_DIRS.iter().any(|d| name.eq_ignore_ascii_case(d));
            !name.starts_with('.') && !keep_dir
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
            if is_known_folder_artwork(&name) {
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
            if stem_ok || is_folder_named(&stem, &folder_name) || belongs_to_video(path, &stem, &ext, &videos, is_show) {
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

/// `<folder name>.nfo` / `<folder name>-poster.jpg` / `<folder name>_landscape.jpg`: the
/// folder-level NFO and artwork that `nfo::import` and the scanner accept.
fn is_folder_named(stem: &str, folder_name: &str) -> bool {
    if folder_name.is_empty() {
        return false;
    }
    stem.eq_ignore_ascii_case(folder_name)
        || stem.rsplit_once(['-', '_']).is_some_and(|(head, tail)| {
            head.eq_ignore_ascii_case(folder_name) && KEEP_ART_STEMS.iter().any(|a| tail.eq_ignore_ascii_case(a))
        })
}

/// Indexed video files as (parent dir, stem, season/episode identity).
type IndexedVideo = (PathBuf, String, Option<(Option<i32>, i32)>);

fn indexed_videos(keep_paths: &HashSet<String>) -> Vec<IndexedVideo> {
    keep_paths
        .iter()
        .map(Path::new)
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| MEDIA_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            Some((p.parent()?.to_path_buf(), episode_identity(&stem), stem))
        })
        .map(|(dir, id, stem)| (dir, stem, id))
        .collect()
}

fn episode_identity(stem: &str) -> Option<(Option<i32>, i32)> {
    let parsed = FileNameParser::parse(&format!("{stem}.mkv"));
    parsed.episode.map(|ep| (parsed.season, ep))
}

/// A sidecar whose stem is shorter than the video's (`Show.S01E01.srt` next to
/// `Show.S01E01.1080p.mkv`): after dropping subtitle language/flag suffixes (or an image's
/// `-landscape` / `-clearart` … suffix), its stem is a
/// prefix of a video stem at a `.`/` `/`-`/`_` boundary, or (for shows, same directory)
/// both parse to the same season/episode.
fn belongs_to_video(path: &Path, stem: &str, ext: &str, videos: &[IndexedVideo], is_show: bool) -> bool {
    let base = if SUBTITLE_EXTENSIONS.contains(&ext) {
        strip_language_suffixes(stem)
    } else {
        // `Movie.2020-landscape.jpg` next to `Movie.2020.1080p.mkv`.
        stem.rsplit_once(['-', '_'])
            .filter(|(head, tail)| !head.is_empty() && KEEP_ART_STEMS.iter().any(|a| tail.eq_ignore_ascii_case(a)))
            .map_or(stem, |(head, _)| head)
    };
    if base.is_empty() {
        return false;
    }
    let prefix_of = |video: &str| {
        video.strip_prefix(base).is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', ' ', '-', '_']))
    };
    if videos.iter().any(|(_, v, _)| prefix_of(v)) {
        return true;
    }
    if !is_show {
        return false;
    }
    let Some(identity) = episode_identity(base) else { return false };
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let dir = PathBuf::from(canonicalize_lossy(&dir));
    videos.iter().any(|(vdir, _, vid)| *vid == Some(identity) && *vdir == dir)
}

/// `Show.S01E01.chs.forced` → `Show.S01E01`.
fn strip_language_suffixes(stem: &str) -> &str {
    static RE_LANG: OnceLock<Regex> = OnceLock::new();
    let re = RE_LANG.get_or_init(|| {
        Regex::new(r"(?i)^(?:[a-z]{2,3}(?:[-_][a-z]{2,4})?|forced|default|sdh|cc|hi|简体|繁體|繁体|简中|繁中|中文|双语|雙語|简日|繁日|简英|繁英|中英|中日)$").unwrap()
    });
    let mut base = stem;
    for _ in 0..4 {
        match base.rsplit_once('.') {
            Some((head, tail)) if !head.is_empty() && re.is_match(tail) => base = head,
            _ => break,
        }
    }
    base
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

    fn scanned_show(root: &Path, db: &AppDatabase) -> String {
        let lib = Library::new("TV", root.to_string_lossy(), MediaType::TvShow);
        db.insert_library(&lib).unwrap();
        let result = crate::scanner::scan_shows(&lib, &Default::default(), &Default::default(), |_| {}).unwrap();
        let item = result.new_items[0].clone();
        assert_eq!(item.status, ScrapedStatus::Scraped);
        db.insert_media_items(std::slice::from_ref(&item)).unwrap();
        db.insert_show_episodes(&item.id, &result.episodes[&item.id]).unwrap();
        item.id
    }

    fn residual_names(db: &AppDatabase, id: &str) -> Vec<String> {
        let mut names: Vec<String> = find_residuals(db, &[id.to_string()]).unwrap().into_iter()
            .map(|r| Path::new(&r.path).file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn keeps_kodi_emby_jellyfin_artwork_and_folders() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Show");
        let season = show.join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        std::fs::write(season.join("Show.S01E01.mkv"), b"x").unwrap();
        for name in ["tvshow.nfo", "landscape.jpg", "ClearArt.png", "clearlogo.png", "thumb.jpg", "discart.png",
            "banner.jpg", "fanart1.jpg", "season01-poster.jpg", "Season-Specials-Poster.jpg", "season02-landscape.jpg",
            "season-all-banner.jpg", "season01.nfo", "season 02.nfo", "Old.S01E09.nfo"] {
            std::fs::write(show.join(name), b"x").unwrap();
        }
        std::fs::write(season.join("season.nfo"), b"x").unwrap();
        for (sub, name) in [("extrafanart", "fanart9.jpg"), ("extrathumbs", "thumb1.jpg"), ("Subs", "2_English.srt"),
            ("subs", "x.srt"), (".actors", "Actor.jpg"), ("extras", "junk.nfo"), ("Featurettes", "x.jpg")] {
            let d = season.join(sub);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(name), b"x").unwrap();
        }
        let db = AppDatabase::open_in_memory().unwrap();
        let id = scanned_show(dir.path(), &db);
        assert_eq!(residual_names(&db, &id), vec!["Old.S01E09.nfo"]);
    }

    #[test]
    fn keeps_folder_named_nfo_and_artwork() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Andor (2022)");
        std::fs::create_dir_all(&show).unwrap();
        std::fs::write(show.join("Andor.S01E01.mkv"), b"x").unwrap();
        for name in ["Andor (2022).nfo", "Andor (2022)-poster.jpg", "Andor (2022)-landscape.jpg",
            "Andor (2022).S01E02.nfo", "Andor (2022)-notes.jpg"] {
            std::fs::write(show.join(name), b"x").unwrap();
        }
        let db = AppDatabase::open_in_memory().unwrap();
        let id = scanned_show(dir.path(), &db);
        assert_eq!(residual_names(&db, &id), vec!["Andor (2022)-notes.jpg", "Andor (2022).S01E02.nfo"]);

        let movies = tempdir().unwrap();
        let folder = movies.path().join("Movie (2020)");
        std::fs::create_dir_all(&folder).unwrap();
        let media = folder.join("Movie.2020.1080p.mkv");
        std::fs::write(&media, b"x").unwrap();
        std::fs::write(folder.join("Movie (2020).nfo"), b"x").unwrap();
        let lib = Library::new("Movies", movies.path().to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let item = crate::models::MediaItem::new_movie("Movie", Some(2020), folder.to_string_lossy(),
            media.to_string_lossy(), lib.id.clone(), ScrapedStatus::Scraped);
        db.insert_media_items(std::slice::from_ref(&item)).unwrap();
        assert!(residual_names(&db, &item.id).is_empty());
    }

    #[test]
    fn subtitles_with_shorter_stems_belong_to_their_episode_only() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Show");
        std::fs::create_dir_all(&show).unwrap();
        std::fs::write(show.join("tvshow.nfo"), b"<tvshow/>").unwrap();
        std::fs::write(show.join("Show.S01E01.1080p.WEB-DL.mkv"), b"x").unwrap();
        for name in ["Show.S01E01.srt", "Show.S01E01.zh.srt", "Show.S01E01.chs.default.ass",
            "Show.S01E01.eng.forced.srt", "Show - S01E01.zh-CN.srt", "Show.S01E01-landscape.jpg",
            "Show.S01E02.srt", "Show.S01E02.zh.srt", "Show.S01E0.srt"] {
            std::fs::write(show.join(name), b"x").unwrap();
        }
        let db = AppDatabase::open_in_memory().unwrap();
        let id = scanned_show(dir.path(), &db);
        assert_eq!(residual_names(&db, &id), vec!["Show.S01E0.srt", "Show.S01E02.srt", "Show.S01E02.zh.srt"]);
    }

    #[test]
    fn movie_sidecars_with_release_tags_in_video_name_are_kept() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("Movie (2020)");
        std::fs::create_dir_all(&folder).unwrap();
        let media = folder.join("Movie.2020.1080p.BluRay.mkv");
        std::fs::write(&media, b"x").unwrap();
        for name in ["Movie.2020.srt", "Movie.2020.en.srt", "Movie.2020-landscape.jpg", "Other.2019.srt"] {
            std::fs::write(folder.join(name), b"x").unwrap();
        }
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("Movies", dir.path().to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let item = crate::models::MediaItem::new_movie("Movie", Some(2020), folder.to_string_lossy(),
            media.to_string_lossy(), lib.id.clone(), ScrapedStatus::Scraped);
        db.insert_media_items(std::slice::from_ref(&item)).unwrap();
        assert_eq!(residual_names(&db, &item.id), vec!["Other.2019.srt"]);
    }
}
