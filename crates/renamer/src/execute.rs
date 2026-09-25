//! Rename execute + undo (RENAME-R-10/11).

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use media_core::{CollisionPolicy, FilesystemService};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::preview::PreviewResult;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompletedRename {
    pub original_path: String,
    pub new_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RenameSnapshot {
    pub id: String,
    pub date: DateTime<Utc>,
    pub renames: Vec<CompletedRename>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<PendingRename>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PendingRename {
    rename: CompletedRename,
    undo: bool,
    identity: FileIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct FileIdentity {
    length: u64,
    modified: Option<std::time::SystemTime>,
    directory: bool,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

pub(crate) fn file_identity(path: &Path) -> std::io::Result<FileIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        length: metadata.len(), modified: metadata.modified().ok(), directory: metadata.is_dir(),
        #[cfg(unix)] device: metadata.dev(),
        #[cfg(unix)] inode: metadata.ino(),
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    #[error(transparent)]
    Filesystem(#[from] media_core::FilesystemError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("no undo snapshots")]
    NoSnapshots,
    #[error("rename recovery needs inspection: {0}")]
    RecoveryConflict(String),
    #[error("rename manager lock poisoned")]
    Poisoned,
}

/// Execute renames from preview. Skips conflict / invalid / unchanged.
/// Saves an undo snapshot when anything moved.
pub fn execute(
    previews: &[PreviewResult],
    undo: &RenameUndoManager,
) -> Result<Vec<CompletedRename>, ExecuteError> {
    execute_with(previews, undo, |src, dst| {
        FilesystemService::new().move_item(src, dst, CollisionPolicy::Fail).map(|_| ())
    })
}

fn execute_with(
    previews: &[PreviewResult],
    undo: &RenameUndoManager,
    mut move_item: impl FnMut(&Path, &Path) -> Result<(), media_core::FilesystemError>,
) -> Result<Vec<CompletedRename>, ExecuteError> {
    let _guard = undo.mutation.lock().map_err(|_| ExecuteError::Poisoned)?;
    undo.recover_pending()?;
    let mut snapshot = RenameSnapshot {
        id: Uuid::new_v4().to_string(), date: Utc::now(), renames: Vec::new(), pending: None,
    };
    // Check that recovery storage is writable before the first mutation.
    undo.write_snapshot(&snapshot)?;
    for preview in previews.iter().filter(|p| p.is_executable()) {
        let src = PathBuf::from(&preview.path);
        let dst = preview.destination_path();
        if !src.is_file() && !src.is_dir() { continue; }
        if dst.exists() { continue; }
        let rename = CompletedRename {
            original_path: src.to_string_lossy().into_owned(),
            new_path: dst.to_string_lossy().into_owned(),
        };
        snapshot.pending = Some(PendingRename { rename, undo: false, identity: file_identity(&src)? });
        // Write intent before changing the filesystem. A crash between move and
        // acknowledgement is resolved from both paths plus the file identity.
        undo.write_snapshot(&snapshot)?;
        if let Err(error) = move_item(&src, &dst) {
            if src.exists() && !dst.exists() {
                snapshot.pending = None;
                undo.write_snapshot(&snapshot)?;
            }
            return Err(error.into());
        }
        snapshot.renames.push(snapshot.pending.take().unwrap().rename);
        undo.write_snapshot(&snapshot)?;

    }
    if snapshot.renames.is_empty() {
        fs::remove_file(undo.storage_dir.join(format!("{}.json", snapshot.id)))?;
    }
    undo.trim()?;
    Ok(snapshot.renames)
}

pub struct RenameUndoManager {
    storage_dir: PathBuf,
    max_snapshots: usize,
    mutation: std::sync::Mutex<()>,
}

impl RenameUndoManager {
    pub fn open(storage_dir: impl Into<PathBuf>) -> Result<Self, ExecuteError> {
        let storage_dir = storage_dir.into();
        fs::create_dir_all(&storage_dir)?;
        Ok(Self {
            storage_dir,
            max_snapshots: 10,
            mutation: std::sync::Mutex::new(()),
        })
    }

    pub fn open_default() -> Result<Self, ExecuteError> {
        let base = dirs::data_dir().ok_or_else(|| {
            ExecuteError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no data directory",
            ))
        })?;
        Self::open(base.join("sula").join("rename_snapshots"))
    }

    pub fn save_snapshot(&self, renames: &[CompletedRename]) -> Result<(), ExecuteError> {
        let _guard = self.mutation.lock().map_err(|_| ExecuteError::Poisoned)?;
        self.recover_pending()?;
        let snapshot = RenameSnapshot {
            id: Uuid::new_v4().to_string(),
            date: Utc::now(),
            renames: renames.to_vec(),
            pending: None,
        };
        self.write_snapshot(&snapshot)?;
        self.trim()?;
        Ok(())
    }

    fn write_snapshot(&self, snapshot: &RenameSnapshot) -> Result<(), ExecuteError> {
        let path = self.storage_dir.join(format!("{}.json", snapshot.id));
        let data = serde_json::to_vec_pretty(snapshot)?;
        FilesystemService::new().write_file(&data, path, media_core::WriteOptions {
            collision_policy: CollisionPolicy::Replace,
            ..Default::default()
        })?;
        Ok(())
    }

    pub fn snapshots(&self) -> Result<Vec<RenameSnapshot>, ExecuteError> {
        let _guard = self.mutation.lock().map_err(|_| ExecuteError::Poisoned)?;
        let mut out = self.load_all()?;
        out.sort_by(|a, b| b.date.cmp(&a.date));
        Ok(out)
    }

    fn recover_pending(&self) -> Result<(), ExecuteError> {
        for mut snapshot in self.load_all()? {
            let Some(pending) = snapshot.pending.as_ref() else { continue; };
            let from = Path::new(&pending.rename.original_path);
            let to = Path::new(&pending.rename.new_path);
            let moved = match (from.try_exists()?, to.try_exists()?) {
                (false, true) if file_identity(to)? == pending.identity => true,
                (true, false) if file_identity(from)? == pending.identity => false,
                _ => return Err(ExecuteError::RecoveryConflict(format!("{} -> {}", from.display(), to.display()))),
            };
            let pending = snapshot.pending.take().unwrap();
            if moved {
                if pending.undo { snapshot.renames.pop(); }
                else { snapshot.renames.push(pending.rename); }
            }
            self.write_snapshot(&snapshot)?;
        }
        Ok(())
    }

    pub fn undo_last(&self) -> Result<usize, ExecuteError> {
        let _guard = self.mutation.lock().map_err(|_| ExecuteError::Poisoned)?;
        self.recover_pending()?;
        let mut all = self.load_all()?;
        all.sort_by(|a, b| b.date.cmp(&a.date));
        let Some(mut latest) = all.into_iter().next() else { return Err(ExecuteError::NoSnapshots); };
        let fs = FilesystemService::new();
        let mut n = 0;
        while let Some(rename) = latest.renames.last().cloned() {
            let new_path = Path::new(&rename.new_path);
            let original = Path::new(&rename.original_path);
            if !new_path.try_exists()? {
                return Err(ExecuteError::RecoveryConflict(format!("missing undo source: {}", new_path.display())));
            }
            if original.try_exists()? {
                return Err(ExecuteError::RecoveryConflict(format!("undo destination exists: {}", original.display())));
            }
            latest.pending = Some(PendingRename {
                rename: CompletedRename { original_path: rename.new_path.clone(), new_path: rename.original_path.clone() },
                undo: true, identity: file_identity(new_path)?,
            });
            self.write_snapshot(&latest)?;
            fs.move_item(new_path, original, CollisionPolicy::Fail)?;
            latest.renames.pop();
            latest.pending = None;
            self.write_snapshot(&latest)?;
            n += 1;
        }
        std::fs::remove_file(self.storage_dir.join(format!("{}.json", latest.id)))?;
        Ok(n)
    }

    fn load_all(&self) -> Result<Vec<RenameSnapshot>, ExecuteError> {
        if !self.storage_dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.storage_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = fs::read(&path)?;
            let snap = serde_json::from_slice::<RenameSnapshot>(&data)?;
            if !snap.renames.is_empty() || snap.pending.is_some() { out.push(snap); }
        }
        Ok(out)
    }

    fn trim(&self) -> Result<(), ExecuteError> {
        let mut all = self.load_all()?;
        if all.len() <= self.max_snapshots {
            return Ok(());
        }
        all.sort_by(|a, b| a.date.cmp(&b.date));
        let excess = all.len() - self.max_snapshots;
        for old in all.into_iter().take(excess) {
            let path = self.storage_dir.join(format!("{}.json", old.id));
            let _ = fs::remove_file(path);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::{preview, FileEntry};
    use crate::rules::{AnyRenameRule, RulePipeline, TextReplace};
    use tempfile::tempdir;

    #[test]
    fn interrupted_move_is_recovered_after_reopening() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("old.mkv");
        let dst = dir.path().join("new.mkv");
        fs::write(&src, b"video").unwrap();
        let storage = dir.path().join("snapshots");
        let manager = RenameUndoManager::open(&storage).unwrap();
        let snapshot = RenameSnapshot {
            id: "interrupted".into(), date: Utc::now(), renames: vec![],
            pending: Some(PendingRename {
                rename: CompletedRename { original_path: src.to_string_lossy().into_owned(), new_path: dst.to_string_lossy().into_owned() },
                undo: false, identity: file_identity(&src).unwrap(),
            }),
        };
        manager.write_snapshot(&snapshot).unwrap();
        fs::rename(&src, &dst).unwrap(); // simulate exit before acknowledgement
        drop(manager);
        let reopened = RenameUndoManager::open(storage).unwrap();
        assert_eq!(reopened.undo_last().unwrap(), 1);
        assert_eq!(fs::read(src).unwrap(), b"video");
        assert!(!dst.exists());
    }

    #[test]
    fn ambiguous_interrupted_move_preserves_both_files() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("old.mkv");
        let dst = dir.path().join("new.mkv");
        fs::write(&src, b"original").unwrap();
        let manager = RenameUndoManager::open(dir.path().join("snapshots")).unwrap();
        manager.write_snapshot(&RenameSnapshot {
            id: "ambiguous".into(), date: Utc::now(), renames: vec![],
            pending: Some(PendingRename {
                rename: CompletedRename { original_path: src.to_string_lossy().into_owned(), new_path: dst.to_string_lossy().into_owned() },
                undo: false, identity: file_identity(&src).unwrap(),
            }),
        }).unwrap();
        fs::write(&dst, b"unrelated").unwrap();
        assert!(matches!(manager.undo_last(), Err(ExecuteError::RecoveryConflict(_))));
        assert_eq!(fs::read(src).unwrap(), b"original");
        assert_eq!(fs::read(dst).unwrap(), b"unrelated");
        assert!(manager.storage_dir.join("ambiguous.json").exists());
    }

    #[test]
    fn interrupted_undo_does_not_repeat_completed_reverse_move() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("old.mkv");
        let dst = dir.path().join("new.mkv");
        fs::write(&dst, b"video").unwrap();
        let manager = RenameUndoManager::open(dir.path().join("snapshots")).unwrap();
        manager.write_snapshot(&RenameSnapshot {
            id: "undo-interrupted".into(), date: Utc::now(),
            renames: vec![CompletedRename { original_path: src.to_string_lossy().into_owned(), new_path: dst.to_string_lossy().into_owned() }],
            pending: Some(PendingRename {
                rename: CompletedRename { original_path: dst.to_string_lossy().into_owned(), new_path: src.to_string_lossy().into_owned() },
                undo: true, identity: file_identity(&dst).unwrap(),
            }),
        }).unwrap();
        fs::rename(&dst, &src).unwrap();
        assert!(matches!(manager.undo_last(), Err(ExecuteError::NoSnapshots)));
        assert!(manager.snapshots().unwrap().is_empty());
        assert_eq!(fs::read(src).unwrap(), b"video");
    }

    #[test]
    fn partial_failure_keeps_completed_moves_undoable() {
        let dir = tempdir().unwrap();
        let files: Vec<_> = ["old_a.mkv", "old_b.mkv"].iter().map(|name| {
            let path = dir.path().join(name);
            fs::write(&path, b"video").unwrap();
            FileEntry::new(path)
        }).collect();
        let previews = preview(&files, &RulePipeline::new(vec![AnyRenameRule::TextReplace(TextReplace::new("old", "new"))]));
        let undo = RenameUndoManager::open(dir.path().join("snaps")).unwrap();
        let mut calls = 0;
        let result = execute_with(&previews, &undo, |src, dst| {
            calls += 1;
            if calls == 2 { return Err(media_core::FilesystemError::TrashUnavailable); }
            FilesystemService::new().move_item(src, dst, CollisionPolicy::Fail).map(|_| ())
        });
        assert!(result.is_err());
        assert_eq!(undo.snapshots().unwrap()[0].renames.len(), 1);
        assert_eq!(undo.undo_last().unwrap(), 1);
        assert!(dir.path().join("old_a.mkv").is_file());
        assert!(dir.path().join("old_b.mkv").is_file());
        assert!(!dir.path().join("new_a.mkv").exists());
    }

    #[test]
    fn execute_and_undo() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("old_name.mkv");
        fs::write(&src, b"x").unwrap();

        let pipeline = RulePipeline::new(vec![AnyRenameRule::TextReplace(TextReplace::new(
            "old_name", "new_name",
        ))]);
        let files = [FileEntry::new(&src)];
        let previews = preview(&files, &pipeline);
        assert!(previews[0].is_executable());

        let undo = RenameUndoManager::open(dir.path().join("snaps")).unwrap();
        let done = execute(&previews, &undo).unwrap();
        assert_eq!(done.len(), 1);
        assert!(!src.exists());
        assert!(dir.path().join("new_name.mkv").is_file());

        let n = undo.undo_last().unwrap();
        assert_eq!(n, 1);
        assert!(src.is_file());
        assert!(!dir.path().join("new_name.mkv").exists());
    }
}
