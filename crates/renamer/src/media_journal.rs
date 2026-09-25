//! Write-ahead merge moves. The database commit marker shares the merge transaction.
use crate::execute::{file_identity, FileIdentity};
use media_core::{entry_name_exists, is_case_only_rename, AppDatabase, CollisionPolicy, FilesystemService};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Journals whose operation is still running in this process. Libraries mutate in
/// parallel, so recovery started for one library must not roll back another
/// library's merge mid-flight; only journals of finished or crashed operations
/// are recovered.
fn live_journals() -> &'static Mutex<HashSet<String>> {
    static LIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    LIVE.get_or_init(Default::default)
}

/// One recovery pass at a time, so two lock holders never replay one journal.
static RECOVERY: Mutex<()> = Mutex::new(());

#[derive(Default, Serialize, Deserialize)]
struct Journal {
    version: u32,
    moves: Vec<Move>,
    #[serde(default)]
    created_directories: Vec<PathBuf>,
}
#[derive(Serialize, Deserialize)]
struct Move {
    from: PathBuf,
    to: PathBuf,
    identity: FileIdentity,
}

pub(crate) struct MergeJournal<'a> {
    db: &'a AppDatabase,
    pub id: String,
    journal: Journal,
}
impl<'a> MergeJournal<'a> {
    /// The journal is live until dropped; drop it before recovering its own failure.
    pub fn begin(db: &'a AppDatabase) -> Result<Self, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let journal = Journal {
            version: 1,
            moves: Vec::new(),
            created_directories: Vec::new(),
        };
        // Registered before the row exists, so no recovery pass sees it unowned.
        live_journals().lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone());
        let this = Self { db, id, journal };
        this.db.create_media_operation(
            &this.id,
            &serde_json::to_string(&this.journal).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(this)
    }
    pub fn move_file(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        if from == to {
            return Ok(());
        }
        if to.try_exists().map_err(|e| e.to_string())? && !is_case_only_rename(from, to).map_err(|e| e.to_string())? {
            return Err(format!("destination exists: {}", to.display()));
        }
        let mut parent = to.parent();
        let mut missing = Vec::new();
        while let Some(path) = parent {
            if path.try_exists().map_err(|e| e.to_string())? { break; }
            missing.push(path.to_path_buf());
            parent = path.parent();
        }
        for path in missing.into_iter().rev() {
            if !self.journal.created_directories.contains(&path) { self.journal.created_directories.push(path); }
        }
        let identity = file_identity(from).map_err(|e| e.to_string())?;
        self.journal.moves.push(Move {
            from: from.into(),
            to: to.into(),
            identity,
        });
        // Failure here leaves the filesystem untouched; recover only the persisted prefix.
        self.db
            .update_media_operation(
                &self.id,
                &serde_json::to_string(&self.journal).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        FilesystemService::new()
            .move_item(from, to, CollisionPolicy::Fail)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl Drop for MergeJournal<'_> {
    fn drop(&mut self) {
        live_journals().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

fn was_moved(step: &Move) -> Result<bool, String> {
    let state = (
        step.from.try_exists().map_err(|e| e.to_string())?,
        step.to.try_exists().map_err(|e| e.to_string())?,
    );
    let (path, moved) = match state {
        (true, false) => (&step.from, false),
        (false, true) => (&step.to, true),
        // Case-only rename: both spellings resolve to one entry; the listing decides.
        (true, true) if is_case_only_rename(&step.from, &step.to).map_err(|e| e.to_string())?
            || is_case_only_rename(&step.to, &step.from).map_err(|e| e.to_string())? => {
            (&step.to, entry_name_exists(&step.to).map_err(|e| e.to_string())?)
        }
        _ => {
            return Err(format!(
                "ambiguous media recovery: {} -> {}",
                step.from.display(),
                step.to.display()
            ))
        }
    };
    if file_identity(path).map_err(|e| e.to_string())? != step.identity {
        return Err(format!("media recovery file changed: {}", path.display()));
    }
    Ok(moved)
}

/// Called before mutations and on startup. Pending merges roll back; committed
/// merges keep their files. Ambiguous paths retain the journal and block mutations.
/// Journals of operations still running in this process are left to their owner.
pub fn recover_media_operations(db: &AppDatabase) -> Result<(), String> {
    let _recovery = RECOVERY.lock().unwrap_or_else(|e| e.into_inner());
    for (id, payload, committed) in db.media_operations().map_err(|e| e.to_string())? {
        if live_journals().lock().unwrap_or_else(|e| e.into_inner()).contains(&id) {
            continue;
        }
        if !committed {
            let journal: Journal = serde_json::from_str(&payload)
                .map_err(|e| format!("invalid media journal {id}: {e}"))?;
            if journal.version != 1 {
                return Err(format!(
                    "unsupported media journal version: {}",
                    journal.version
                ));
            }
            // Inspect the entire operation before making any recovery moves.
            for step in &journal.moves {
                was_moved(step)?;
            }
            for step in journal.moves.iter().rev() {
                if was_moved(step)? {
                    FilesystemService::new()
                        .move_item(&step.to, &step.from, CollisionPolicy::Fail)
                        .map_err(|e| format!("media recovery {id}: {e}"))?;
                }
            }
            for directory in journal.created_directories.iter().rev() {
                match std::fs::remove_dir(directory) {
                    Ok(()) => {},
                    Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty) => {},
                    Err(error) => return Err(format!("recovery directory {}: {error}", directory.display())),
                }
            }
        }
        db.remove_media_operation(&id).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use media_core::{Library, MediaItem, MediaType, ScannedEpisode, ScrapedStatus};
    use tempfile::tempdir;

    #[test]
    fn restart_rolls_back_moves_and_handles_interrupted_rollback() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let a = dir.path().join("a.mkv");
        let b = dir.path().join("b.srt");
        let to_a = dir.path().join("target/a.mkv");
        let to_b = dir.path().join("target/b.srt");
        std::fs::write(&a, b"video").unwrap();
        std::fs::write(&b, b"subtitle").unwrap();
        {
            let db = AppDatabase::open(&db_path).unwrap();
            let mut journal = MergeJournal::begin(&db).unwrap();
            journal.move_file(&a, &to_a).unwrap();
            journal.move_file(&b, &to_b).unwrap();
        }
        // A recovery process exited after reversing the last move.
        std::fs::rename(&to_b, &b).unwrap();
        let db = AppDatabase::open(&db_path).unwrap();
        recover_media_operations(&db).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), b"video");
        assert_eq!(std::fs::read(&b).unwrap(), b"subtitle");
        assert!(!to_a.exists());
        assert!(db.media_operations().unwrap().is_empty());
        recover_media_operations(&db).unwrap();
    }

    #[test]
    fn intent_without_move_leaves_original_untouched() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("source");
        let to = dir.path().join("dest");
        std::fs::write(&from, "original").unwrap();
        let db = AppDatabase::open(dir.path().join("db.sqlite")).unwrap();
        let mut journal = MergeJournal::begin(&db).unwrap();
        journal.journal.moves.push(Move {
            from: from.clone(),
            to: to.clone(),
            identity: file_identity(&from).unwrap(),
        });
        db.update_media_operation(
            &journal.id,
            &serde_json::to_string(&journal.journal).unwrap(),
        )
        .unwrap();
        drop(journal); // The process exits.
        recover_media_operations(&db).unwrap();
        assert_eq!(std::fs::read_to_string(from).unwrap(), "original");
        assert!(!to.exists());
    }

    #[test]
    fn running_operation_is_not_recovered_until_it_ends() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("source");
        let to = dir.path().join("dest");
        std::fs::write(&from, "original").unwrap();
        let db = AppDatabase::open(dir.path().join("db.sqlite")).unwrap();
        let mut journal = MergeJournal::begin(&db).unwrap();
        journal.move_file(&from, &to).unwrap();
        // Another library's lock holder runs recovery meanwhile.
        recover_media_operations(&db).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "original");
        assert_eq!(db.media_operations().unwrap().len(), 1);
        drop(journal);
        recover_media_operations(&db).unwrap();
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "original");
        assert!(!to.exists());
        assert!(db.media_operations().unwrap().is_empty());
    }

    #[test]
    fn ambiguous_recovery_preserves_files_and_journal() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("source");
        let to = dir.path().join("dest");
        std::fs::write(&from, "original").unwrap();
        let db = AppDatabase::open(dir.path().join("db.sqlite")).unwrap();
        let mut journal = MergeJournal::begin(&db).unwrap();
        journal.move_file(&from, &to).unwrap();
        drop(journal); // The process exits.
        std::fs::write(&from, "unrelated").unwrap();
        assert!(recover_media_operations(&db).is_err());
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "unrelated");
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "original");
        assert_eq!(db.media_operations().unwrap().len(), 1);
        std::fs::remove_file(&from).unwrap();
        std::fs::write(&to, "externally changed content").unwrap();
        assert!(recover_media_operations(&db).is_err());
        assert!(!from.exists());
    }

    #[test]
    fn committed_merge_survives_restart_before_journal_cleanup() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("db.sqlite");
        let from = dir.path().join("source/video.mkv");
        let to = dir.path().join("target/video.mkv");
        std::fs::create_dir_all(from.parent().unwrap()).unwrap();
        std::fs::write(&from, "video").unwrap();
        let (source_id, target_id);
        {
            let db = AppDatabase::open(&db_path).unwrap();
            let library = Library::new("TV", dir.path().to_string_lossy(), MediaType::TvShow);
            db.insert_library(&library).unwrap();
            let source = MediaItem::new_show(
                MediaType::TvShow,
                "Source",
                None,
                from.parent().unwrap().to_string_lossy(),
                library.id.clone(),
                ScrapedStatus::Scraped,
            );
            let target = MediaItem::new_show(
                MediaType::TvShow,
                "Target",
                None,
                to.parent().unwrap().to_string_lossy(),
                library.id,
                ScrapedStatus::Scraped,
            );
            db.insert_media_items(&[source.clone(), target.clone()])
                .unwrap();
            db.insert_show_episodes(
                &source.id,
                &[ScannedEpisode {
                    season: 1,
                    episode: 1,
                    title: "Episode".into(),
                    file_path: from.to_string_lossy().into_owned(),
                }],
            )
            .unwrap();
            let season = db.fetch_seasons(&source.id).unwrap().remove(0);
            let episode = db.fetch_episodes(&season.id).unwrap().remove(0);
            let mut journal = MergeJournal::begin(&db).unwrap();
            journal.move_file(&from, &to).unwrap();
            db.merge_show_records(
                &source.id,
                &target.id,
                &[(episode.id, to.to_string_lossy().into_owned())],
                &[(from.clone(), to.clone())],
                &journal.id,
            )
            .unwrap();
            assert!(db.media_operations().unwrap()[0].2);
            source_id = source.id;
            target_id = target.id;
        }
        let db = AppDatabase::open(db_path).unwrap();
        recover_media_operations(&db).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "video");
        assert!(db.get_media_item(&source_id).unwrap().is_none());
        let season = db.fetch_seasons(&target_id).unwrap().remove(0);
        assert_eq!(
            db.fetch_episodes(&season.id).unwrap()[0].file_path,
            to.to_string_lossy()
        );
        assert!(db.media_operations().unwrap().is_empty());
    }
}
