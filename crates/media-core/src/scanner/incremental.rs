use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::Utc;
use walkdir::WalkDir;

use crate::db::DirectoryScanState;
use crate::models::Library;

/// mtime 比较容差（秒），对齐迁移计划中的 NAS 阈值说明。
pub const MTIME_EPSILON_SECS: f64 = 0.01;

#[derive(Debug, Clone, Default)]
pub struct DirectoryPlan {
    /// Live directories discovered on disk (path → mtime secs).
    pub live: HashMap<String, f64>,
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub unchanged: Vec<String>,
    pub removed: Vec<String>,
    /// True when the library had no prior scan-state rows (first incremental bootstrap).
    pub bootstrap: bool,
}

impl DirectoryPlan {
    pub fn needs_file_scan(&self) -> bool {
        self.bootstrap || !self.added.is_empty() || !self.changed.is_empty()
    }

    pub fn has_removals(&self) -> bool {
        !self.removed.is_empty()
    }

    /// Keep shallowest changed roots so direct files in changed ancestors are scanned.
    pub fn scan_roots(&self) -> Vec<PathBuf> {
        let mut candidates: Vec<String> = self.added.iter().chain(self.changed.iter()).cloned().collect();
        if self.bootstrap {
            // Bootstrap: prefer library-wide single root if present, else all live dirs.
            if let Some((root, _)) = self.live.iter().min_by_key(|(p, _)| p.len()) {
                // Use the shortest path as library root when all dirs are new.
                let root = root.clone();
                if self.live.keys().all(|p| p == &root || path_is_under(p, &root)) {
                    return vec![PathBuf::from(root)];
                }
            }
            candidates = self.live.keys().cloned().collect();
        }
        prune_descendant_paths(&candidates)
            .into_iter()
            .map(PathBuf::from)
            .collect()
    }

    pub fn to_scan_states(&self, library_id: &str) -> Vec<DirectoryScanState> {
        let now = Utc::now();
        self.live
            .iter()
            .map(|(path, mtime)| DirectoryScanState {
                library_id: library_id.to_string(),
                directory_path: path.clone(),
                last_known_modification_time: *mtime,
                last_scanned_at: now,
            })
            .collect()
    }
}

/// Stat every previously recorded directory. If all still exist with the same
/// mtime (within epsilon), a full directory WalkDir can be skipped: creating a
/// new child always bumps the parent directory mtime on POSIX / APFS / NTFS.
#[cfg(test)]
pub fn known_directories_unchanged(
    previous: &[DirectoryScanState],
) -> Result<bool, std::io::Error> {
    known_directories_unchanged_cancellable(previous, &std::sync::atomic::AtomicBool::new(false))
}

pub fn known_directories_unchanged_cancellable(
    previous: &[DirectoryScanState],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<bool, std::io::Error> {
    super::check_cancel(cancel)?;
    if previous.is_empty() {
        return Ok(false);
    }
    for state in previous {
        super::check_cancel(cancel)?;
        let path = Path::new(&state.directory_path);
        if !path.is_dir() {
            return Ok(false);
        }
        let mtime = dir_mtime_secs(path)?;
        if mtime_changed(state.last_known_modification_time, mtime) {
            return Ok(false);
        }
    }
    Ok(true)
}



/// Directory-only walk + diff against stored scan state (SCAN-11).
#[cfg(test)]
pub fn plan_directories(
    library: &Library,
    excluded_folders: &HashSet<String>,
    previous: &[DirectoryScanState],
) -> Result<DirectoryPlan, std::io::Error> {
    plan_directories_cancellable(library, excluded_folders, previous, &std::sync::atomic::AtomicBool::new(false))
}

pub fn plan_directories_cancellable(
    library: &Library,
    excluded_folders: &HashSet<String>,
    previous: &[DirectoryScanState],
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<DirectoryPlan, std::io::Error> {
    super::check_cancel(cancel)?;
    let root = PathBuf::from(&library.root_path);
    if !root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("library root not found: {}", library.root_path),
        ));
    }

    let live = walk_directories(&root, excluded_folders, cancel)?;
    let prev_map: HashMap<String, f64> = previous
        .iter()
        .map(|s| {
            (
                s.directory_path.clone(),
                s.last_known_modification_time,
            )
        })
        .collect();
    let bootstrap = previous.is_empty();

    let mut plan = DirectoryPlan {
        live: live.clone(),
        bootstrap,
        ..DirectoryPlan::default()
    };

    if bootstrap {
        plan.changed = live.keys().cloned().collect();
        plan.changed.sort();
        return Ok(plan);
    }

    for (path, mtime) in &live {
        match prev_map.get(path) {
            None => plan.added.push(path.clone()),
            Some(old) if mtime_changed(*old, *mtime) => plan.changed.push(path.clone()),
            Some(_) => plan.unchanged.push(path.clone()),
        }
    }
    for path in prev_map.keys() {
        if !live.contains_key(path) {
            plan.removed.push(path.clone());
        }
    }
    plan.added.sort();
    plan.changed.sort();
    plan.unchanged.sort();
    plan.removed.sort();
    Ok(plan)
}

fn walk_directories(
    root: &Path,
    excluded_folders: &HashSet<String>,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<HashMap<String, f64>, std::io::Error> {
    let mut live = HashMap::new();
    let root_canon = canonicalize_lossy(root);
    live.insert(root_canon.clone(), dir_mtime_secs(root)?);

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
            // Still enter files' parents; filter_entry for files returns true so walk continues,
            // but we only record directories below.
            true
        }
    });

    for entry in walker {
        super::check_cancel(cancel)?;
        let entry = entry?;
        if !entry.file_type().is_dir() {
            continue;
        }
        if entry.depth() == 0 {
            continue;
        }
        let path = entry.path();
        let key = canonicalize_lossy(path);
        live.insert(key, dir_mtime_secs(path)?);
    }
    Ok(live)
}

pub fn dir_mtime_secs(path: &Path) -> Result<f64, std::io::Error> {
    let modified = std::fs::metadata(path)?.modified()?;
    Ok(system_time_to_secs(modified))
}

fn system_time_to_secs(t: SystemTime) -> f64 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -(e.duration().as_secs_f64()),
    }
}

pub fn mtime_changed(old: f64, new: f64) -> bool {
    (new - old).abs() > MTIME_EPSILON_SECS
}

pub fn canonicalize_lossy(path: &Path) -> String {
    let buf = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    // Windows `canonicalize` yields `\\?\C:\...` / `\\?\UNC\...`, which break
    // Explorer reveal APIs and confuse users in the UI.
    strip_windows_verbatim_prefix(&buf)
        .to_string_lossy()
        .into_owned()
}

/// Drop Windows verbatim (`\\?\`) prefixes when safe for shell/UI use.
pub fn strip_windows_verbatim_prefix(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

fn path_is_under(path: &str, root: &str) -> bool {
    crate::db::path_rooted_under(path, root)
}

fn prune_descendant_paths(paths: &[String]) -> Vec<PathBuf> {
    let mut sorted: Vec<String> = paths.to_vec();
    sorted.sort_by_key(|p| p.len());
    let mut kept: Vec<String> = Vec::new();
    for path in sorted {
        if kept.iter().any(|root| path == *root || path_is_under(&path, root)) {
            continue;
        }
        kept.push(path);
    }
    kept.sort();
    kept.into_iter().map(PathBuf::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MediaType;
    use tempfile::tempdir;

    #[test]
    fn bootstrap_marks_all_changed() {
        let dir = tempdir().unwrap();
        let movie = dir.path().join("Film");
        std::fs::create_dir_all(&movie).unwrap();
        let library = Library::new("L", dir.path().display().to_string(), MediaType::Movie);
        let plan = plan_directories(&library, &HashSet::new(), &[]).unwrap();
        assert!(plan.bootstrap);
        assert!(plan.needs_file_scan());
        assert!(plan.removed.is_empty());
        assert!(!plan.live.is_empty());
    }

    #[test]
    fn unchanged_plan_needs_no_file_scan() {
        let dir = tempdir().unwrap();
        let movie = dir.path().join("Film");
        std::fs::create_dir_all(&movie).unwrap();
        let library = Library::new("L", dir.path().display().to_string(), MediaType::Movie);
        let first = plan_directories(&library, &HashSet::new(), &[]).unwrap();
        let states = first.to_scan_states(&library.id);
        let second = plan_directories(&library, &HashSet::new(), &states).unwrap();
        assert!(!second.bootstrap);
        assert!(!second.needs_file_scan());
        assert!(!second.has_removals());
    }

    #[test]
    fn prune_keeps_ancestor_coverage() {
        let paths = vec![
            "/lib".into(),
            "/lib/A".into(),
            "/lib/A/B".into(),
            "/lib/C".into(),
        ];
        let kept = prune_descendant_paths(&paths);
        let kept: Vec<String> = kept
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(kept, vec!["/lib"]);
    }

    #[test]
    fn known_dirs_unchanged_skips_when_stable() {
        let dir = tempdir().unwrap();
        let movie = dir.path().join("Film");
        std::fs::create_dir_all(&movie).unwrap();
        let library = Library::new("L", dir.path().display().to_string(), MediaType::Movie);
        let first = plan_directories(&library, &HashSet::new(), &[]).unwrap();
        let states = first.to_scan_states(&library.id);
        assert!(known_directories_unchanged(&states).unwrap());
    }

    #[test]
    fn known_dirs_unchanged_false_when_child_added() {
        let dir = tempdir().unwrap();
        let movie = dir.path().join("Film");
        std::fs::create_dir_all(&movie).unwrap();
        let library = Library::new("L", dir.path().display().to_string(), MediaType::Movie);
        let first = plan_directories(&library, &HashSet::new(), &[]).unwrap();
        let states = first.to_scan_states(&library.id);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::create_dir_all(dir.path().join("Other")).unwrap();
        assert!(!known_directories_unchanged(&states).unwrap());
    }
}
