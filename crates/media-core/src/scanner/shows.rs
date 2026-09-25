use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;
use std::sync::OnceLock;
use walkdir::WalkDir;

use crate::models::{Library, MediaItem, ScrapedStatus};

use super::filename::{FileNameParser, ParsedFileName};
use super::incremental::canonicalize_lossy;
use super::movies::{ScanProgress, MEDIA_EXTENSIONS};

#[derive(Debug, Clone)]
pub struct ScannedEpisode {
    pub season: i32,
    pub episode: i32,
    pub file_path: String,
    pub title: String,
}

#[derive(Debug, Clone, Default)]
pub struct ShowScanResult {
    pub new_items: Vec<MediaItem>,
    pub episodes: HashMap<String, Vec<ScannedEpisode>>,
}

struct ShowFile {
    path: PathBuf,
    parsed: ParsedFileName,
    season_from_dir: Option<i32>,
}

pub fn scan_shows(
    library: &Library,
    existing_show_paths: &HashSet<String>,
    excluded_folders: &HashSet<String>,
    on_progress: impl FnMut(ScanProgress),
) -> Result<ShowScanResult, std::io::Error> {
    let root = PathBuf::from(&library.root_path);
    if !root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("library root not found: {}", library.root_path),
        ));
    }
    scan_shows_under(
        library,
        &[root],
        existing_show_paths,
        excluded_folders,
        on_progress,
    )
}

/// Scan show media under one or more directory roots (SCAN-12).
pub fn scan_shows_under(
    library: &Library,
    roots: &[PathBuf],
    existing_show_paths: &HashSet<String>,
    excluded_folders: &HashSet<String>,
    on_progress: impl FnMut(ScanProgress),
) -> Result<ShowScanResult, std::io::Error> {
    scan_shows_under_cancellable(library, roots, existing_show_paths, excluded_folders, on_progress, &std::sync::atomic::AtomicBool::new(false))
}

pub fn scan_shows_under_cancellable(
    library: &Library,
    roots: &[PathBuf],
    existing_show_paths: &HashSet<String>,
    excluded_folders: &HashSet<String>,
    mut on_progress: impl FnMut(ScanProgress),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<ShowScanResult, std::io::Error> {
    super::check_cancel(cancel)?;
    let library_root = PathBuf::from(&library.root_path);
    let root_canon = canonicalize_lossy(&library_root);

    let mut show_groups: HashMap<String, Vec<ShowFile>> = HashMap::new();
    let mut discovered = 0u32;
    let mut seen_files: HashSet<String> = HashSet::new();

    for scan_root in roots {
        if !scan_root.is_dir() {
            continue;
        }
        let walker = WalkDir::new(scan_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                if entry.depth() == 0 {
                    return true;
                }
                let name = entry.file_name().to_string_lossy();
                if name.starts_with('.') {
                    return false;
                }
                if entry.file_type().is_dir() {
                    !excluded_folders.contains(&name.to_ascii_lowercase())
                } else {
                    true
                }
            });

        for entry in walker {
            super::check_cancel(cancel)?;
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !MEDIA_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }

            let absolute = canonicalize_lossy(path);
            if !seen_files.insert(absolute) {
                continue;
            }

            let parent = path.parent().unwrap_or(path);
            let (show_root, season_from_dir) = resolve_show_root(parent, Path::new(&root_canon));
            let show_key = canonicalize_lossy(&show_root);

            // Existing shows are skipped later; don't spam progress while walking them.
            if existing_show_paths.contains(&show_key) {
                continue;
            }

            discovered += 1;
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            on_progress(ScanProgress {
                discovered_count: discovered,
                current_path: path.display().to_string(),
                current_name: file_name.clone(),
            });

            let parsed = FileNameParser::parse(&file_name);
            show_groups.entry(show_key).or_default().push(ShowFile {
                path: path.to_path_buf(),
                parsed,
                season_from_dir,
            });
        }
    }

    super::check_cancel(cancel)?;
    let title_overrides = merge_flat_season_groups(&mut show_groups, Path::new(&root_canon));

    let mut new_items = Vec::new();
    let mut episodes_map = HashMap::new();

    for (show_path, files) in show_groups {
        if existing_show_paths.contains(&show_path) {
            continue;
        }
        if files.is_empty() {
            continue;
        }

        let show_name = Path::new(&show_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        let (title, year) = if let Some(override_title) = title_overrides.get(&show_path) {
            (override_title.clone(), None)
        } else {
            let dir_parsed = FileNameParser::parse(&format!("{show_name}.mkv"));
            let title = if dir_parsed.title.is_empty() {
                show_name.clone()
            } else {
                dir_parsed.title
            };
            (title, dir_parsed.year)
        };

        let has_nfo = Path::new(&show_path).join("tvshow.nfo").is_file()
            || Path::new(&show_path)
                .join(format!("{show_name}.nfo"))
                .is_file();
        let status = if has_nfo {
            ScrapedStatus::Scraped
        } else {
            ScrapedStatus::Unscraped
        };

        let item = MediaItem::new_show(
            library.media_type,
            title,
            year,
            show_path.clone(),
            library.id.clone(),
            status,
        );

        let mut episodes = Vec::new();
        for (idx, file) in files.into_iter().enumerate() {
            let season = file.season_from_dir.or(file.parsed.season).unwrap_or(1);
            let episode = file.parsed.episode.unwrap_or((idx + 1) as i32);
            episodes.push(ScannedEpisode {
                season,
                episode,
                file_path: canonicalize_lossy(&file.path),
                title: file.parsed.title,
            });
        }

        episodes_map.insert(item.id.clone(), episodes);
        new_items.push(item);
    }

    Ok(ShowScanResult {
        new_items,
        episodes: episodes_map,
    })
}

/// Fold newly scanned flat `Title 第N季` / `Title Season N` folders into an existing
/// show that already shares the same base title (cross-refresh merge).
///
/// Same-scan merging is handled by [`merge_flat_season_groups`]; this covers the case
/// where season 1 was imported earlier and season 2 appears on a later refresh.
pub fn absorb_flat_seasons_into_existing(
    result: ShowScanResult,
    existing: &[MediaItem],
) -> (ShowScanResult, Vec<(String, Vec<ScannedEpisode>)>) {
    if result.new_items.is_empty() || existing.is_empty() {
        return (result, Vec::new());
    }

    let mut index: HashMap<String, String> = HashMap::new();
    for item in existing {
        for key in existing_show_merge_keys(item) {
            index.entry(key).or_insert_with(|| item.id.clone());
        }
    }

    let mut kept_items = Vec::new();
    let mut kept_episodes = HashMap::new();
    let mut absorbed = Vec::new();

    for item in result.new_items {
        let episodes = result
            .episodes
            .get(&item.id)
            .cloned()
            .unwrap_or_default();
        if let Some(key) = flat_season_merge_key(&item) {
            if let Some(existing_id) = index.get(&key) {
                if !episodes.is_empty() {
                    absorbed.push((existing_id.clone(), episodes));
                }
                continue;
            }
        }
        kept_episodes.insert(item.id.clone(), episodes);
        kept_items.push(item);
    }

    (
        ShowScanResult {
            new_items: kept_items,
            episodes: kept_episodes,
        },
        absorbed,
    )
}

fn normalize_show_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn flat_season_merge_key(item: &MediaItem) -> Option<String> {
    let name = Path::new(&item.folder_path)
        .file_name()
        .and_then(|n| n.to_str())?;
    let (base, _) = FileNameParser::extract_season_suffix(name)?;
    Some(normalize_show_title(&base))
}

fn existing_show_merge_keys(item: &MediaItem) -> Vec<String> {
    let mut keys = Vec::new();
    let title = item.title.trim();
    if !title.is_empty() {
        keys.push(normalize_show_title(title));
    }
    if let Some(name) = Path::new(&item.folder_path)
        .file_name()
        .and_then(|n| n.to_str())
    {
        if let Some((base, _)) = FileNameParser::extract_season_suffix(name) {
            keys.push(normalize_show_title(&base));
        } else if !name.is_empty() {
            keys.push(normalize_show_title(name));
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

/// Whether an existing show folder may have gained/lost episodes under `scan_roots`.
pub fn existing_show_touched_by_roots(folder_path: &str, scan_roots: &[PathBuf]) -> bool {
    if folder_path.is_empty() || scan_roots.is_empty() {
        return false;
    }
    let show = PathBuf::from(canonicalize_lossy(Path::new(folder_path)));
    for root in scan_roots {
        let root = PathBuf::from(canonicalize_lossy(root));
        if show == root || show.starts_with(&root) || root.starts_with(&show) {
            return true;
        }
    }
    false
}

/// Discover episode media files under an existing show folder (SCAN-15).
pub fn discover_episodes_in_show_cancellable(
    show_folder: &Path,
    excluded_folders: &HashSet<String>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Vec<ScannedEpisode>, std::io::Error> {
    super::check_cancel(cancel)?;
    std::fs::read_dir(show_folder)?;
    let show_canon = canonicalize_lossy(show_folder);
    let mut files = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let walker = WalkDir::new(show_folder)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            if name.starts_with('.') {
                return false;
            }
            if entry.file_type().is_dir() {
                !excluded_folders.contains(&name.to_ascii_lowercase())
            } else {
                true
            }
        });

    for entry in walker {
            super::check_cancel(cancel)?;
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !MEDIA_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let absolute = canonicalize_lossy(path);
        if !seen.insert(absolute.clone()) {
            continue;
        }
        let parent = path.parent().unwrap_or(path);
        let (_root, season_from_dir) = resolve_show_root(parent, Path::new(&show_canon));
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let parsed = FileNameParser::parse(file_name);
        let season = season_from_dir.or(parsed.season).unwrap_or(1);
        let episode = parsed.episode.unwrap_or((files.len() + 1) as i32);
        files.push(ScannedEpisode {
            season,
            episode,
            file_path: absolute,
            title: parsed.title,
        });
    }
    super::check_cancel(cancel)?;
    Ok(files)
}

fn resolve_show_root(file_parent: &Path, library_root: &Path) -> (PathBuf, Option<i32>) {
    let parent = canonicalize_lossy(file_parent);
    let root = canonicalize_lossy(library_root);
    if parent == root {
        return (PathBuf::from(root), None);
    }

    let dir_name = Path::new(&parent)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    if let Some(season) = match_season_dir(dir_name) {
        let show_root = Path::new(&parent)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(&parent));
        if canonicalize_lossy(&show_root) == root {
            return (PathBuf::from(parent), Some(season));
        }
        return (show_root, Some(season));
    }

    if is_specials_dir(dir_name) {
        let show_root = Path::new(&parent)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(&parent));
        if canonicalize_lossy(&show_root) == root {
            return (PathBuf::from(parent), Some(0));
        }
        return (show_root, Some(0));
    }

    if let Some((_, season)) = FileNameParser::extract_season_suffix(dir_name) {
        let grand = Path::new(&parent)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(&parent));
        if canonicalize_lossy(&grand) == root {
            return (PathBuf::from(parent), Some(season));
        }
        return (grand, Some(season));
    }

    (PathBuf::from(parent), None)
}

fn merge_flat_season_groups(
    show_groups: &mut HashMap<String, Vec<ShowFile>>,
    library_root: &Path,
) -> HashMap<String, String> {
    let root = canonicalize_lossy(library_root);
    let keys: Vec<String> = show_groups.keys().cloned().collect();

    struct Candidate {
        path: String,
        base_name: String,
        season: i32,
    }

    let mut candidates = Vec::new();
    for key in keys {
        let url = Path::new(&key);
        let parent = url
            .parent()
            .map(canonicalize_lossy)
            .unwrap_or_default();
        if parent != root {
            continue;
        }
        let name = url.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if let Some((base, season)) = FileNameParser::extract_season_suffix(name) {
            candidates.push(Candidate {
                path: key,
                base_name: base,
                season,
            });
        }
    }

    let mut by_base: HashMap<String, Vec<Candidate>> = HashMap::new();
    for c in candidates {
        by_base.entry(c.base_name.clone()).or_default().push(c);
    }

    let mut title_overrides = HashMap::new();
    for (base_name, mut members) in by_base {
        members.sort_by_key(|m| m.season);
        let canonical = members[0].path.clone();
        title_overrides.insert(canonical.clone(), base_name);
        for member in members.into_iter().skip(1) {
            if let Some(mut files) = show_groups.remove(&member.path) {
                for f in &mut files {
                    f.season_from_dir = Some(member.season);
                }
                show_groups.entry(canonical.clone()).or_default().append(&mut files);
            }
        }
    }

    title_overrides
}

fn match_season_dir(name: &str) -> Option<i32> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)^(?:S(?:eason\s*)?|第\s*)(\d{1,2})(?:\s*季)?$").unwrap()
    });
    re.captures(name)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

fn is_specials_dir(name: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)^(?:Specials?|SP|OVA|OAD|Extras?)$").unwrap());
    re.is_match(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaType;
    use tempfile::tempdir;

    #[test]
    fn scans_show_with_season_folder() {
        let dir = tempdir().unwrap();
        let show = dir.path().join("Andor");
        let season = show.join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        std::fs::write(season.join("Andor.S01E01.mkv"), b"x").unwrap();
        std::fs::write(season.join("Andor.S01E02.mkv"), b"x").unwrap();

        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        let result = scan_shows(&library, &HashSet::new(), &HashSet::new(), |_| {}).unwrap();
        assert_eq!(result.new_items.len(), 1);
        assert_eq!(result.new_items[0].title, "Andor");
        let eps = result.episodes.get(&result.new_items[0].id).unwrap();
        assert_eq!(eps.len(), 2);
        assert!(eps.iter().all(|e| e.season == 1));
    }

    #[test]
    fn merges_flat_cn_season_dirs() {
        let dir = tempdir().unwrap();
        let s1 = dir.path().join("IT狂人 第 1 季");
        let s2 = dir.path().join("IT狂人 第 2 季");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s1.join("ep1.mkv"), b"x").unwrap();
        std::fs::write(s2.join("ep1.mkv"), b"x").unwrap();

        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        let result = scan_shows(&library, &HashSet::new(), &HashSet::new(), |_| {}).unwrap();
        assert_eq!(result.new_items.len(), 1);
        assert_eq!(result.new_items[0].title, "IT狂人");
        let eps = result.episodes.get(&result.new_items[0].id).unwrap();
        assert_eq!(eps.len(), 2);
        let seasons: HashSet<_> = eps.iter().map(|e| e.season).collect();
        assert_eq!(seasons, HashSet::from([1, 2]));
    }

    #[test]
    fn absorbs_flat_season_into_existing_show_across_scans() {
        let dir = tempdir().unwrap();
        let s1 = dir.path().join("后室 第 1 季");
        std::fs::create_dir_all(&s1).unwrap();
        std::fs::write(s1.join("S01E01.mkv"), b"x").unwrap();

        let library = Library::new("TV", dir.path().display().to_string(), MediaType::TvShow);
        let first = scan_shows(&library, &HashSet::new(), &HashSet::new(), |_| {}).unwrap();
        assert_eq!(first.new_items.len(), 1);
        assert_eq!(first.new_items[0].title, "后室");

        let s2 = dir.path().join("后室 第 2 季");
        std::fs::create_dir_all(&s2).unwrap();
        std::fs::write(s2.join("S02E01.mkv"), b"x").unwrap();

        let existing_paths: HashSet<String> = first
            .new_items
            .iter()
            .map(|i| i.folder_path.clone())
            .collect();
        let second = scan_shows(&library, &existing_paths, &HashSet::new(), |_| {}).unwrap();
        assert_eq!(second.new_items.len(), 1);

        let (merged, absorbed) =
            absorb_flat_seasons_into_existing(second, &first.new_items);
        assert!(merged.new_items.is_empty());
        assert_eq!(absorbed.len(), 1);
        assert_eq!(absorbed[0].0, first.new_items[0].id);
        assert_eq!(absorbed[0].1.len(), 1);
        assert_eq!(absorbed[0].1[0].season, 2);
    }
}
