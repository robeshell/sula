//! Library refresh workflows. The caller holds the library lock scope.

use std::sync::Arc;

use media_core::{AppDatabase, MediaType, ScanProgress};
use renamer::RenameTemplates;

use super::{err_string, Progress, TaskProgress};

fn loc_progress(locale: &str, name: &str) -> String {
    match name {
        "scan.checking" => crate::ui_i18n::t(locale, "prog.checking"),
        "scan.unchanged" => crate::ui_i18n::t(locale, "prog.unchanged"),
        _ => name.to_string(),
    }
}

fn is_directory_check(p: &ScanProgress) -> bool {
    p.discovered_count == 0
        && (p.current_name == "scan.checking"
            || p.current_name == "scan.unchanged"
            || p.current_name.starts_with("检查")
            || p.current_name.starts_with("目录")
            || p.current_name.starts_with("scan."))
}

pub struct RefreshService {
    pub db: Arc<AppDatabase>,
    pub excluded_folders: Vec<String>,
    pub templates: RenameTemplates,
    pub locale: String,
}

impl RefreshService {
    /// Scan the library, then merge season packs that resolve to one show.
    pub async fn refresh_library(&self, library_id: &str, progress: &impl Progress) -> Result<(), String> {
        let locale = &self.locale;
        let library = self.db.get_library(library_id).map_err(err_string)?
            .ok_or_else(|| "library removed before refresh".to_string())?;
        let media_type = library.media_type;
        let (db, excluded) = (Arc::clone(&self.db), self.excluded_folders.clone());
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel::<ScanProgress>();
        let cancel = progress.cancel_flag();
        let scan = tokio::task::spawn_blocking(move || {
            let mut last_emit = 0u32;
            let mut saw_check = false;
            media_core::refresh_library_cancellable(&db, &library, &excluded, |p| {
                if is_directory_check(&p) {
                    if !saw_check || p.current_name == "scan.unchanged" || p.current_name.starts_with("目录无变更") {
                        saw_check = true;
                        let _ = progress_tx.send(p);
                    }
                    return;
                }
                if p.discovered_count == 1 || p.discovered_count.saturating_sub(last_emit) >= 25 {
                    last_emit = p.discovered_count;
                    let _ = progress_tx.send(p);
                }
            }, &cancel)
        });

        while let Some(p) = progress_rx.recv().await {
            if progress.is_cancelled() {
                break;
            }
            let stage_key = if is_directory_check(&p) { "checkDirectories" } else { "scanFiles" };
            progress.update(TaskProgress::new(p.discovered_count, 0, loc_progress(locale, &p.current_name), stage_key)).await;
        }

        let report = scan.await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        if progress.is_cancelled() {
            return Err("cancelled".into());
        }

        // Same TMDB season packs → merge into the canonical show (also on early-exit).
        let mut merged_n = 0usize;
        if matches!(media_type, MediaType::TvShow | MediaType::Anime) {
            let (db, templates, lib_id) = (Arc::clone(&self.db), self.templates.clone(), library_id.to_string());
            merged_n = tokio::task::spawn_blocking(move || renamer::consolidate_library_duplicate_shows(&db, &lib_id, &templates))
                .await
                .map_err(|e| e.to_string())?
                .unwrap_or(0);
        }

        let update = if merged_n > 0 {
            TaskProgress::new(merged_n as u32, merged_n as u32, crate::ui_i18n::tf(locale, "prog.mergedShows", &[("n", &merged_n.to_string())]), "saveResults")
        } else if report.early_exit {
            TaskProgress::new(0, 0, crate::ui_i18n::t(locale, "prog.unchanged"), "checkDirectories")
        } else {
            let n = report.new_item_count as u32;
            TaskProgress::new(n, n, crate::ui_i18n::tf(locale, "prog.added", &[("n", &n.to_string())]), "saveResults")
        };
        progress.update(update).await;
        Ok(())
    }

    /// SCAN-15: refresh selected items from disk.
    pub async fn refresh_items(&self, item_ids: Vec<String>, progress: &impl Progress) -> Result<(), String> {
        let locale = &self.locale;
        let total = item_ids.len() as u32;
        progress.update(TaskProgress::new(0, total, crate::ui_i18n::t(locale, "prog.refreshing"), "refreshItems")).await;
        let (db, excluded, cancel) = (Arc::clone(&self.db), self.excluded_folders.clone(), progress.cancel_flag());
        let report = tokio::task::spawn_blocking(move || media_core::refresh_items_cancellable(&db, &item_ids, &excluded, &cancel))
            .await
            .map_err(|e| e.to_string())?
            .map_err(err_string)?;
        if progress.is_cancelled() {
            return Err("cancelled".into());
        }
        let done = crate::ui_i18n::tf(locale, "prog.refreshDone", &[("ok", &report.refreshed.to_string()), ("removed", &report.removed.to_string())]);
        progress.update(TaskProgress::new(total, total, done, "refreshItems")).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{movie_library, FakeProgress};
    use media_core::ScrapedStatus;

    fn service(db: &Arc<AppDatabase>) -> RefreshService {
        RefreshService { db: Arc::clone(db), excluded_folders: Vec::new(), templates: RenameTemplates::default(), locale: "en".into() }
    }

    #[tokio::test]
    async fn refresh_items_drops_missing_files_and_reports_counts() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        std::fs::remove_file(&item.file_path).unwrap();
        let progress = FakeProgress::default();
        service(&db).refresh_items(vec![item.id.clone()], &progress).await.unwrap();
        assert!(db.get_media_item(&item.id).unwrap().is_none());
        assert_eq!(progress.stages(), ["refreshItems", "refreshItems"]);
        assert_eq!(progress.last().completed, 1);
    }

    #[tokio::test]
    async fn refresh_library_indexes_new_movies() {
        let (dir, db, lib, _item) = movie_library(ScrapedStatus::Unscraped);
        let folder = dir.path().join("Another (2011)");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("Another.2011.mkv"), b"video").unwrap();
        let progress = FakeProgress::default();
        service(&db).refresh_library(&lib.id, &progress).await.unwrap();
        assert!(db.list_media_items(&lib.id).unwrap().iter().any(|i| i.file_path.ends_with("Another.2011.mkv")));
        assert_eq!(progress.last().stage_key.as_deref(), Some("saveResults"));
    }

    #[tokio::test]
    async fn cancelled_refresh_fails_as_cancelled() {
        let (_dir, db, lib, _item) = movie_library(ScrapedStatus::Unscraped);
        let progress = FakeProgress::default();
        progress.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(service(&db).refresh_library(&lib.id, &progress).await.is_err());
    }
}
