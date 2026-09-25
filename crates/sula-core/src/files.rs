//! Browsing folders on disk (the in-app folder browser).

use std::path::PathBuf;

use serde::Serialize;

use crate::error::{blocking, failed, CoreError, CoreResult};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntryDto {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub file_size: Option<u64>,
    pub modified_at: Option<String>,
}

pub async fn path_is_dir(path: String) -> CoreResult<bool> {
    blocking(move || Ok(std::path::Path::new(path.trim()).is_dir())).await
}

/// Visible entries of a folder, folders first, then by name (case-insensitive).
pub async fn list_directory(path: String) -> CoreResult<Vec<DirectoryEntryDto>> {
    blocking(move || list_directory_sync(PathBuf::from(path))).await
}

fn list_directory_sync(root: PathBuf) -> CoreResult<Vec<DirectoryEntryDto>> {
    if !root.is_dir() {
        return Err(CoreError::invalid("path is not a directory"));
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let meta = entry.metadata().ok();
        let is_directory = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let file_size = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len());
        let modified_at = meta.and_then(|m| m.modified().ok()).map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            dt.format("%Y-%m-%d").to_string()
        });
        out.push(DirectoryEntryDto {
            name,
            path: entry.path().to_string_lossy().into_owned(),
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

#[cfg(test)]
mod tests {
    #[test]
    fn lists_folders_first_and_hides_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.nfo"), b"x").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("Season 01")).unwrap();
        std::fs::write(dir.path().join("A.mkv"), b"xyz").unwrap();
        let names: Vec<_> = super::list_directory_sync(dir.path().to_path_buf()).unwrap().into_iter().map(|e| (e.name, e.file_size)).collect();
        assert_eq!(names, [("Season 01".into(), None), ("A.mkv".into(), Some(3)), ("b.nfo".into(), Some(1))]);
    }
}
