use std::collections::HashSet;
use std::path::PathBuf;

use walkdir::WalkDir;

use crate::models::{Library, MediaItem, ScrapedStatus};

use super::filename::FileNameParser;
use super::incremental::canonicalize_lossy;

pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "mov", "wmv", "flv", "ts", "m2ts", "iso",
];

#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub discovered_count: u32,
    pub current_path: String,
    pub current_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct MovieScanResult {
    pub new_items: Vec<MediaItem>,
}

pub fn scan_movies(
    library: &Library,
    existing_file_paths: &HashSet<String>,
    excluded_folders: &HashSet<String>,
    on_progress: impl FnMut(ScanProgress),
) -> Result<MovieScanResult, std::io::Error> {
    let root = PathBuf::from(&library.root_path);
    scan_movies_under(
        library,
        &[root],
        existing_file_paths,
        excluded_folders,
        on_progress,
    )
}

/// Scan media files under one or more directory roots (SCAN-12).
pub fn scan_movies_under(
    library: &Library,
    roots: &[PathBuf],
    existing_file_paths: &HashSet<String>,
    excluded_folders: &HashSet<String>,
    on_progress: impl FnMut(ScanProgress),
) -> Result<MovieScanResult, std::io::Error> {
    scan_movies_under_cancellable(library, roots, existing_file_paths, excluded_folders, on_progress, &std::sync::atomic::AtomicBool::new(false))
}

pub fn scan_movies_under_cancellable(
    library: &Library,
    roots: &[PathBuf],
    existing_file_paths: &HashSet<String>,
    excluded_folders: &HashSet<String>,
    mut on_progress: impl FnMut(ScanProgress),
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<MovieScanResult, std::io::Error> {
    super::check_cancel(cancel)?;
    let mut items = Vec::new();
    let mut discovered = 0u32;
    let mut seen_files: HashSet<String> = HashSet::new();

    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let walker = WalkDir::new(root).follow_links(false).into_iter().filter_entry(|entry| {
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
            if !seen_files.insert(absolute.clone()) {
                continue;
            }

            // Already indexed: do not inflate progress (looks like a full rescan).
            if existing_file_paths.contains(&absolute) {
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

            let folder = path
                .parent()
                .map(canonicalize_lossy)
                .unwrap_or_else(|| absolute.clone());
            let parsed = FileNameParser::parse(&file_name);
            let nfo_path = path.with_extension("nfo");
            let status = if nfo_path.is_file() {
                ScrapedStatus::Scraped
            } else {
                ScrapedStatus::Unscraped
            };

            items.push(MediaItem::new_movie(
                parsed.title,
                parsed.year,
                folder,
                absolute,
                library.id.clone(),
                status,
            ));
        }
    }

    super::check_cancel(cancel)?;
    Ok(MovieScanResult { new_items: items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaType;
    use tempfile::tempdir;

    #[test]
    fn scans_new_movie_files() {
        let dir = tempdir().unwrap();
        let movie_dir = dir.path().join("Oppenheimer (2023)");
        std::fs::create_dir_all(&movie_dir).unwrap();
        std::fs::write(movie_dir.join("Oppenheimer.2023.mkv"), b"x").unwrap();
        std::fs::write(movie_dir.join("ignore.txt"), b"x").unwrap();

        let library = Library::new("Movies", dir.path().display().to_string(), MediaType::Movie);
        let result = scan_movies(&library, &HashSet::new(), &HashSet::new(), |_| {}).unwrap();
        assert_eq!(result.new_items.len(), 1);
        assert_eq!(result.new_items[0].title, "Oppenheimer");
        assert_eq!(result.new_items[0].year, Some(2023));
    }

    #[test]
    fn scan_under_skips_unrelated_roots() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("A");
        let b = dir.path().join("B");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("A.2020.mkv"), b"x").unwrap();
        std::fs::write(b.join("B.2021.mkv"), b"x").unwrap();

        let library = Library::new("Movies", dir.path().display().to_string(), MediaType::Movie);
        let result =
            scan_movies_under(&library, &[a], &HashSet::new(), &HashSet::new(), |_| {}).unwrap();
        assert_eq!(result.new_items.len(), 1);
        assert_eq!(result.new_items[0].title, "A");
    }
}
