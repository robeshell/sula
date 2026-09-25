//! The batch renamer window: collecting files, previewing rule pipelines,
//! renaming with undo, and saved presets.

use std::sync::Arc;

use crate::app::locks::LockScope;
use crate::app::{blocking, err_string};
use crate::state::AppState;

const MAX_RENAMER_FILES: usize = 5_000;

/// Every file under the given files and folders (hidden entries skipped).
pub async fn collect_files(paths: Vec<String>) -> Result<Vec<renamer::FileEntry>, String> {
    blocking(move || {
        let mut out = Vec::new();
        for raw in paths {
            let path = std::path::PathBuf::from(&raw);
            collect_paths_into(&path, &mut out).map_err(err_string)?;
            if out.len() > MAX_RENAMER_FILES {
                return Err(format!("too many files (max {MAX_RENAMER_FILES})"));
            }
        }
        Ok(out)
    })
    .await
}

pub fn preview(files: &[renamer::FileEntry], pipeline: &renamer::RulePipeline) -> Vec<renamer::PreviewResult> {
    renamer::preview(files, pipeline)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamerOutcome {
    pub renames: Vec<renamer::CompletedRename>,
    /// Set when the batch stopped early; `renames` still lists what already moved.
    pub error: Option<String>,
    /// Library index rows that could not follow a moved file; the next refresh
    /// would otherwise treat those files as deleted.
    pub index_sync_failures: usize,
    /// Undo only: entries whose renamed file no longer exists and was left alone.
    pub skipped: usize,
}

impl AppState {
    /// Previews are recomputed here from the same inputs as `preview`, so a stale
    /// or forged preview from the UI can never choose the destination.
    pub async fn rename_files(&self, files: Vec<renamer::FileEntry>, pipeline: renamer::RulePipeline) -> Result<RenamerOutcome, String> {
        let mutation_guard = self.tasks.locks().lock(&LockScope::Global).await?;
        let (db, undo, events) = (Arc::clone(&self.db), Arc::clone(&self.rename_undo), Arc::clone(&self.events));
        blocking(move || {
            let _mutation_guard = mutation_guard;
            let previews = renamer::preview(&files, &pipeline);
            let mut outcome = RenamerOutcome { renames: Vec::new(), error: None, index_sync_failures: 0, skipped: 0 };
            let result = renamer::execute(&previews, &undo, |done| {
                outcome.renames.push(done.clone());
                sync_renamed_entry(&db, done, &mut outcome.index_sync_failures);
            });
            finish_batch(events.as_ref(), outcome, result.map(|_| ()))
        })
        .await
    }

    pub async fn undo_last_rename(&self) -> Result<RenamerOutcome, String> {
        let mutation_guard = self.tasks.locks().lock(&LockScope::Global).await?;
        let (db, undo, events) = (Arc::clone(&self.db), Arc::clone(&self.rename_undo), Arc::clone(&self.events));
        blocking(move || {
            let _mutation_guard = mutation_guard;
            let mut outcome = RenamerOutcome { renames: Vec::new(), error: None, index_sync_failures: 0, skipped: 0 };
            let result = undo.undo_last_report(|done| {
                outcome.renames.push(done.clone());
                sync_renamed_entry(&db, done, &mut outcome.index_sync_failures);
            });
            if let Ok(report) = &result { outcome.skipped = report.skipped; }
            finish_batch(events.as_ref(), outcome, result.map(|_| ()))
        })
        .await
    }

    pub async fn rename_snapshot_count(&self) -> Result<usize, String> {
        Ok(self.rename_undo.snapshots().map_err(err_string)?.len())
    }

    pub async fn rename_presets(&self) -> Result<Vec<String>, String> {
        self.rename_presets.list_presets().map_err(err_string)
    }

    pub async fn save_rename_preset(&self, name: String, pipeline: renamer::RulePipeline) -> Result<(), String> {
        self.rename_presets.save(&name, &pipeline).map_err(err_string)
    }

    pub async fn load_rename_preset(&self, name: String) -> Result<Option<renamer::RulePipeline>, String> {
        self.rename_presets.load(&name).map_err(err_string)
    }

    pub async fn delete_rename_preset(&self, name: String) -> Result<(), String> {
        self.rename_presets.delete(&name).map_err(err_string)
    }

    /// The rules the window had open last time, restored when it reopens.
    pub async fn save_last_rename_pipeline(&self, pipeline: renamer::RulePipeline) -> Result<(), String> {
        self.rename_presets.auto_save(&pipeline).map_err(err_string)
    }

    pub async fn last_rename_pipeline(&self) -> Result<Option<renamer::RulePipeline>, String> {
        self.rename_presets.auto_load().map_err(err_string)
    }
}

/// The index stores canonical paths. After a rename only the parent directory of
/// either side still resolves, so canonicalize that and re-attach the file name.
fn canonical_entry_path(raw: &str) -> String {
    let path = std::path::Path::new(raw);
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            std::path::Path::new(&media_core::scanner::canonicalize_lossy(parent))
                .join(name)
                .to_string_lossy()
                .into_owned()
        }
        _ => raw.to_string(),
    }
}

fn sync_renamed_entry(db: &media_core::AppDatabase, rename: &renamer::CompletedRename, failures: &mut usize) {
    let old = canonical_entry_path(&rename.original_path);
    let new = canonical_entry_path(&rename.new_path);
    if let Err(error) = db.remap_renamed_path(&old, &new) {
        tracing::warn!(%old, %new, %error, "renamer: library index not updated");
        *failures += 1;
    }
}

/// A failure before anything moved is a plain error; after that the caller must
/// still learn which entries moved, so the error travels inside the outcome.
fn finish_batch(
    events: &dyn crate::app::Events,
    mut outcome: RenamerOutcome,
    result: Result<(), renamer::ExecuteError>,
) -> Result<RenamerOutcome, String> {
    if !outcome.renames.is_empty() {
        events.library_updated();
    }
    match result {
        Ok(()) => Ok(outcome),
        Err(error) if outcome.renames.is_empty() => Err(err_string(error)),
        Err(error) => {
            outcome.error = Some(err_string(error));
            Ok(outcome)
        }
    }
}

fn collect_paths_into(
    path: &std::path::Path,
    out: &mut Vec<renamer::FileEntry>,
) -> std::io::Result<()> {
    if path.is_file() {
        out.push(renamer::FileEntry::new(path));
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let child = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if child.is_dir() {
            collect_paths_into(&child, out)?;
        } else if child.is_file() {
            out.push(renamer::FileEntry::new(&child));
        }
        if out.len() > MAX_RENAMER_FILES {
            break;
        }
    }
    Ok(())
}


#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn collects_files_recursively_and_skips_hidden_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Season 01")).unwrap();
        std::fs::create_dir(dir.path().join(".cache")).unwrap();
        std::fs::write(dir.path().join("Season 01/E01.mkv"), b"x").unwrap();
        std::fs::write(dir.path().join(".cache/skip.mkv"), b"x").unwrap();
        std::fs::write(dir.path().join(".hidden.mkv"), b"x").unwrap();
        std::fs::write(dir.path().join("poster.jpg"), b"x").unwrap();
        let mut names: Vec<_> = super::collect_files(vec![dir.path().display().to_string()]).await.unwrap()
            .into_iter().map(|entry| entry.original_name).collect();
        names.sort();
        assert_eq!(names, ["E01.mkv", "poster.jpg"]);
    }
}
