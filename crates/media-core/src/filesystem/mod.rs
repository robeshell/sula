use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemChangeSet {
    pub created_paths: Vec<String>,
    pub removed_paths: Vec<String>,
    pub moved_paths: Vec<FilesystemMoveRecord>,
    pub modified_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FilesystemMoveRecord {
    pub from_path: String,
    pub to_path: String,
}

impl FilesystemChangeSet {
    pub fn is_empty(&self) -> bool {
        self.created_paths.is_empty()
            && self.removed_paths.is_empty()
            && self.moved_paths.is_empty()
            && self.modified_paths.is_empty()
    }

    pub fn merge(&mut self, other: FilesystemChangeSet) {
        self.created_paths.extend(other.created_paths);
        self.removed_paths.extend(other.removed_paths);
        self.moved_paths.extend(other.moved_paths);
        self.modified_paths.extend(other.modified_paths);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionPolicy {
    Fail,
    Skip,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalStrategy {
    TrashOnly,
    DeleteOnly,
    TrashThenDelete,
}

#[derive(Debug, Clone)]
pub struct WriteOptions {
    pub collision_policy: CollisionPolicy,
    pub create_intermediate_directories: bool,
    pub atomic: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            collision_policy: CollisionPolicy::Fail,
            create_intermediate_directories: true,
            atomic: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum FilesystemError {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("destination already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("path not found: {0}")]
    NotFound(PathBuf),
    #[error("trash is not available on this platform yet")]
    TrashUnavailable,
}

pub struct FilesystemService;

impl FilesystemService {
    pub fn new() -> Self {
        Self
    }

    pub fn create_directory(&self, path: impl AsRef<Path>) -> Result<FilesystemChangeSet, FilesystemError> {
        let path = path.as_ref();
        fs::create_dir_all(path).map_err(|source| FilesystemError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(FilesystemChangeSet {
            created_paths: vec![path_string(path)],
            ..Default::default()
        })
    }

    pub fn move_item(
        &self,
        source: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        collision: CollisionPolicy,
    ) -> Result<FilesystemChangeSet, FilesystemError> {
        let source = source.as_ref();
        let destination = destination.as_ref();

        if !source.exists() {
            return Err(FilesystemError::NotFound(source.to_path_buf()));
        }

        if destination.exists() {
            match collision {
                CollisionPolicy::Fail => {
                    return Err(FilesystemError::AlreadyExists(destination.to_path_buf()));
                }
                CollisionPolicy::Skip => return Ok(FilesystemChangeSet::default()),
                CollisionPolicy::Replace => {
                    self.remove_item(destination)?;
                }
            }
        }

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| FilesystemError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        fs::rename(source, destination).map_err(|err| FilesystemError::Io {
            path: source.to_path_buf(),
            source: err,
        })?;

        Ok(FilesystemChangeSet {
            moved_paths: vec![FilesystemMoveRecord {
                from_path: path_string(source),
                to_path: path_string(destination),
            }],
            ..Default::default()
        })
    }

    pub fn write_file(
        &self,
        data: &[u8],
        destination: impl AsRef<Path>,
        options: WriteOptions,
    ) -> Result<FilesystemChangeSet, FilesystemError> {
        let destination = destination.as_ref();

        if destination.exists() {
            match options.collision_policy {
                CollisionPolicy::Fail => {
                    return Err(FilesystemError::AlreadyExists(destination.to_path_buf()));
                }
                CollisionPolicy::Skip => return Ok(FilesystemChangeSet::default()),
                CollisionPolicy::Replace => {}
            }
        }

        if options.create_intermediate_directories {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|source| FilesystemError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
        }

        if options.atomic {
            let tmp = destination.with_extension("tmp-sula");
            fs::write(&tmp, data).map_err(|source| FilesystemError::Io {
                path: tmp.clone(),
                source,
            })?;
            fs::rename(&tmp, destination).map_err(|source| FilesystemError::Io {
                path: destination.to_path_buf(),
                source,
            })?;
        } else {
            fs::write(destination, data).map_err(|source| FilesystemError::Io {
                path: destination.to_path_buf(),
                source,
            })?;
        }

        let key = path_string(destination);
        Ok(FilesystemChangeSet {
            created_paths: if destination.exists() {
                vec![key.clone()]
            } else {
                vec![]
            },
            modified_paths: vec![key],
            ..Default::default()
        })
    }

    pub fn remove_item(&self, path: impl AsRef<Path>) -> Result<FilesystemChangeSet, FilesystemError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(FilesystemError::NotFound(path.to_path_buf()));
        }
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|source| FilesystemError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        } else {
            fs::remove_file(path).map_err(|source| FilesystemError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        }
        Ok(FilesystemChangeSet {
            removed_paths: vec![path_string(path)],
            ..Default::default()
        })
    }

    /// Move path to the platform trash / recycle bin when available.
    /// Unavailable recycle services fail closed; callers must not permanently delete.
    pub fn trash_item(&self, path: impl AsRef<Path>) -> Result<FilesystemChangeSet, FilesystemError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(FilesystemError::NotFound(path.to_path_buf()));
        }

        #[cfg(target_os = "macos")]
        {
            trash_macos(path)?;
            return Ok(FilesystemChangeSet {
                removed_paths: vec![path_string(path)],
                ..Default::default()
            });
        }

        #[cfg(target_os = "linux")]
        {
            if trash_linux(path).is_ok() {
                return Ok(FilesystemChangeSet {
                    removed_paths: vec![path_string(path)],
                    ..Default::default()
                });
            }
            return Err(FilesystemError::TrashUnavailable);
        }

        #[cfg(target_os = "windows")]
        {
            trash_windows(path)?;
            return Ok(FilesystemChangeSet { removed_paths: vec![path_string(path)], ..Default::default() });
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            let _ = path;
            Err(FilesystemError::TrashUnavailable)
        }
    }
}

#[cfg(target_os = "macos")]
fn trash_macos(path: &Path) -> Result<(), FilesystemError> {
    use std::process::Command;

    let posix = path.to_string_lossy();
    // Escape for AppleScript string literal.
    let escaped = posix.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "tell application \"Finder\" to delete (POSIX file \"{escaped}\" as alias)"
    );
    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|source| FilesystemError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(FilesystemError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other(format!("Finder trash failed: {stderr}")),
        })
    }
}

#[cfg(target_os = "windows")]
fn trash_windows(path: &Path) -> Result<(), FilesystemError> {
    // The path is data in an environment variable, never PowerShell source.
    let script = r#"$ErrorActionPreference = 'Stop'; Add-Type -AssemblyName Microsoft.VisualBasic;
$p = $env:SULA_TRASH_PATH;
if ([System.IO.Directory]::Exists($p)) {
 [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin', 'ThrowException')
} else {
 [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin', 'ThrowException')
}"#;
    let output = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("SULA_TRASH_PATH", path).output().map_err(|source| FilesystemError::Io { path: path.into(), source })?;
    if output.status.success() { Ok(()) } else { Err(FilesystemError::Io { path: path.into(), source: std::io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()) }) }
}

#[cfg(target_os = "linux")]
fn trash_linux(path: &Path) -> Result<(), FilesystemError> {
    use std::process::Command;

    // Prefer gio (GLib), then trash-put (trash-cli).
    for (bin, args_prefix) in [("gio", &["trash"][..]), ("trash-put", &[][..])] {
        let mut cmd = Command::new(bin);
        for a in args_prefix {
            cmd.arg(a);
        }
        let output = cmd.arg(path).output();
        if let Ok(output) = output {
            if output.status.success() {
                return Ok(());
            }
        }
    }
    Err(FilesystemError::TrashUnavailable)
}

impl Default for FilesystemService {
    fn default() -> Self {
        Self::new()
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_and_move_file() {
        let dir = tempdir().unwrap();
        let fs = FilesystemService::new();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");

        let created = fs
            .write_file(b"hello", &src, WriteOptions::default())
            .unwrap();
        assert!(!created.modified_paths.is_empty());

        let moved = fs
            .move_item(&src, &dst, CollisionPolicy::Fail)
            .unwrap();
        assert_eq!(moved.moved_paths.len(), 1);
        assert!(dst.exists());
        assert!(!src.exists());
    }
}
