//! Residual cleanup and item deletion. Files only ever go to the system trash; the
//! trash step is injected so tests never touch the real recycle bin.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use media_core::{AppDatabase, FilesystemError};

use super::{blocking, err_string, Events, Progress, TaskProgress};

pub trait Trash: Send + Sync + 'static {
    fn trash(&self, path: &Path) -> Result<(), FilesystemError>;
}

pub struct SystemTrash;

impl Trash for SystemTrash {
    fn trash(&self, path: &Path) -> Result<(), FilesystemError> {
        media_core::FilesystemService::new().trash_item(path).map(|_| ())
    }
}

/// Trash residual files the user picked from `scan_media_residuals`. The whole
/// request is rejected if any path is no longer a current residual candidate.
pub async fn cleanup_residuals(
    db: Arc<AppDatabase>,
    paths: Vec<String>,
    locale: &str,
    trash: Arc<dyn Trash>,
    progress: &impl Progress,
) -> Result<(), String> {
    let total = paths.len() as u32;
    progress.update(TaskProgress::new(0, total, crate::ui_i18n::t(locale, "prog.cleaning"), "cleanup")).await;
    let cancel = progress.cancel_flag();
    let removed = blocking(move || {
        let mut ids = Vec::new();
        for library in db.list_libraries().map_err(err_string)? {
            ids.extend(db.list_media_items(&library.id).map_err(err_string)?.into_iter().map(|i| i.id));
        }
        let allowed: std::collections::HashSet<_> = media_core::find_residuals(&db, &ids)
            .map_err(err_string)?.into_iter().map(|c| c.path).collect();
        if paths.iter().any(|p| !allowed.contains(p)) {
            return Err("cleanup candidates changed; scan again".into());
        }
        let mut removed = 0;
        for path in paths {
            if cancel.load(Ordering::SeqCst) { break; }
            removed += media_core::perform_cleanup_with(&[path], |p| trash.trash(p)).map_err(err_string)?;
        }
        Ok(removed)
    })
    .await?;
    if progress.is_cancelled() {
        return Err("cancelled".into());
    }
    progress.update(TaskProgress::new(removed as u32, total, crate::ui_i18n::tf(locale, "prog.cleaned", &[("n", &removed.to_string())]), "cleanup")).await;
    Ok(())
}

/// Remove items from the index, optionally trashing their files first. The whole
/// request is preflighted before any file or record changes.
pub fn delete_media_items(db: &AppDatabase, item_ids: &[String], also_trash: bool, trash: &dyn Trash, events: &dyn Events) -> Result<usize, String> {
    let result = delete_items(db, item_ids, also_trash, trash);
    // Items trashed before a later failure are gone too; refresh either way.
    events.library_updated();
    result
}

fn delete_items(db: &AppDatabase, item_ids: &[String], also_trash: bool, trash: &dyn Trash) -> Result<usize, String> {
    if !also_trash {
        return db.delete_media_items(item_ids).map_err(err_string);
    }
    let mut targets = Vec::new();
    for id in item_ids {
        if let Some(item) = db.get_media_item(id).map_err(err_string)? {
            let target = media_core::media_files::deletion_target(db, &item).map_err(err_string)?;
            targets.push((id.clone(), target));
        }
    }
    let mut deleted = 0;
    for (id, path) in targets {
        // Keep the record if trash fails; do not turn a trash request into
        // permanent deletion on platforms without recycle-bin support.
        trash.trash(&path).map_err(err_string)?;
        deleted += db.delete_media_items(&[id]).map_err(err_string)?;
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{movie_library, FakeEvents, FakeProgress};
    use media_core::ScrapedStatus;
    use std::sync::Mutex;

    /// Records trashed paths and removes them; fails when told to.
    #[derive(Default)]
    struct FakeTrash {
        fail: bool,
        trashed: Mutex<Vec<std::path::PathBuf>>,
    }

    impl Trash for FakeTrash {
        fn trash(&self, path: &Path) -> Result<(), FilesystemError> {
            if self.fail {
                return Err(FilesystemError::TrashUnavailable);
            }
            self.trashed.lock().unwrap().push(path.to_path_buf());
            let removed = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
            removed.map_err(|source| FilesystemError::Io { path: path.to_path_buf(), source })
        }
    }

    #[test]
    fn delete_with_failing_trash_keeps_record_and_files() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Scraped);
        let (trash, events) = (FakeTrash { fail: true, ..Default::default() }, FakeEvents::default());
        assert!(delete_media_items(&db, std::slice::from_ref(&item.id), true, &trash, &events).is_err());
        assert_eq!(*events.0.lock().unwrap(), 1);
        assert!(db.get_media_item(&item.id).unwrap().is_some());
        assert!(Path::new(&item.file_path).is_file());
    }

    #[test]
    fn delete_trashes_owned_folder_then_record() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Scraped);
        let trash = FakeTrash::default();
        assert_eq!(delete_media_items(&db, &[item.id.clone(), "missing".into()], true, &trash, &FakeEvents::default()).unwrap(), 1);
        assert!(db.get_media_item(&item.id).unwrap().is_none());
        assert_eq!(trash.trashed.lock().unwrap().as_slice(), [std::path::PathBuf::from(&item.folder_path)]);
    }

    #[test]
    fn delete_without_trash_only_drops_records() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Scraped);
        let trash = FakeTrash { fail: true, ..Default::default() };
        assert_eq!(delete_media_items(&db, std::slice::from_ref(&item.id), false, &trash, &FakeEvents::default()).unwrap(), 1);
        assert!(Path::new(&item.file_path).is_file());
    }

    #[tokio::test]
    async fn cleanup_trashes_only_current_residuals() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Scraped);
        let folder = Path::new(&item.folder_path);
        let orphan = folder.join("Old.Name.nfo");
        std::fs::write(&orphan, b"nfo").unwrap();
        let candidates = media_core::find_residuals(&db, std::slice::from_ref(&item.id)).unwrap();
        let orphan_path = candidates.iter().find(|c| c.path.ends_with("Old.Name.nfo")).expect("orphan proposed").path.clone();

        let trash = Arc::new(FakeTrash::default());
        let stale = vec![orphan_path.clone(), item.file_path.clone()];
        let err = cleanup_residuals(Arc::clone(&db), stale, "en", trash.clone(), &FakeProgress::default()).await.unwrap_err();
        assert_eq!(err, "cleanup candidates changed; scan again");
        assert!(orphan.exists() && trash.trashed.lock().unwrap().is_empty());

        let progress = FakeProgress::default();
        cleanup_residuals(Arc::clone(&db), vec![orphan_path], "en", trash.clone(), &progress).await.unwrap();
        assert!(!orphan.exists());
        assert!(Path::new(&item.file_path).is_file());
        assert_eq!(progress.last().completed, 1);
    }
}
