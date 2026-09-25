//! Rename / organize / merge workflows over indexed items. Callers hold the lock
//! scope of the items' libraries while these run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use media_core::{AppDatabase, MediaItem};
use renamer::RenameTemplates;
use serde::{Deserialize, Serialize};

use super::{blocking, err_string, Events, Progress, TaskProgress};

/// Rename freshly scraped items; season packs are absorbed into an existing show first.
pub fn auto_rename_blocking(
    db: &AppDatabase,
    ids: &[String],
    templates: &RenameTemplates,
    create_season_folders: bool,
    cancel: &AtomicBool,
) -> (u32, u32) {
    let mut ok = 0u32;
    let mut failed = 0u32;
    for id in ids {
        if cancel.load(Ordering::SeqCst) { break; }
        let Some(item) = db.get_media_item(id).ok().flatten() else {
            // May already have been merged into a canonical show.
            continue;
        };
        if let Ok(true) = renamer::consolidate_show_item(db, &item, templates) {
            ok += 1;
            continue;
        }
        let Some(item) = db.get_media_item(id).ok().flatten() else {
            continue;
        };
        match renamer::rename_after_scrape_with_options(db, &item, templates, create_season_folders) {
            Ok(()) => ok += 1,
            Err(err) => {
                failed += 1;
                tracing::warn!(item_id = %id, title = %item.title, error = %err, "auto-rename after scrape failed");
            }
        }
    }
    (ok, failed)
}

pub fn consolidate_blocking(db: &AppDatabase, ids: &[String], templates: &RenameTemplates, cancel: &AtomicBool) {
    for id in ids {
        if cancel.load(Ordering::SeqCst) { break; }
        let Some(item) = db.get_media_item(id).ok().flatten() else {
            continue;
        };
        if let Err(err) = renamer::consolidate_show_item(db, &item, templates) {
            tracing::warn!(item_id = %id, title = %item.title, error = %err, "consolidate duplicate show failed");
        }
    }
}

pub struct OrganizeService {
    pub db: Arc<AppDatabase>,
    pub templates: RenameTemplates,
    pub create_season_folders: bool,
    pub locale: String,
}

impl OrganizeService {
    async fn item(&self, id: &str) -> Result<MediaItem, String> {
        let (db, id) = (Arc::clone(&self.db), id.to_string());
        blocking(move || db.get_media_item(&id).map_err(err_string)?.ok_or_else(|| format!("media item not found: {id}"))).await
    }

    pub async fn apply_rename_templates(&self, item_ids: Vec<String>, progress: &impl Progress) -> Result<(), String> {
        let total = item_ids.len() as u32;
        for (idx, id) in item_ids.into_iter().enumerate() {
            if progress.is_cancelled() {
                return Err("cancelled".into());
            }
            let item = self.item(&id).await?;
            progress.update(TaskProgress::new(idx as u32, total, item.title.clone(), "rename")).await;
            let (db, templates, create_season_folders) = (Arc::clone(&self.db), self.templates.clone(), self.create_season_folders);
            blocking(move || {
                // Season packs that share TMDB with an existing show are absorbed first.
                if let Err(error) = renamer::consolidate_show_item(&db, &item, &templates) {
                    tracing::warn!(item_id = %id, %error, "consolidate before rename failed");
                }
                let Some(item) = db.get_media_item(&id).map_err(err_string)? else { return Ok(()) };
                renamer::rename_after_scrape_with_options(&db, &item, &templates, create_season_folders).map_err(err_string)
            }).await?;
        }
        let done = crate::ui_i18n::tf(&self.locale, "prog.renamed", &[("n", &total.to_string())]);
        progress.update(TaskProgress::new(total, total, done, "rename")).await;
        Ok(())
    }

    pub async fn organize_season_folders(&self, item_ids: Vec<String>, progress: &impl Progress) -> Result<(), String> {
        let total = item_ids.len() as u32;
        for (idx, id) in item_ids.into_iter().enumerate() {
            if progress.is_cancelled() {
                return Err("cancelled".into());
            }
            let item = self.item(&id).await?;
            progress.update(TaskProgress::new(idx as u32, total, item.title.clone(), "organize")).await;
            let (db, templates) = (Arc::clone(&self.db), self.templates.clone());
            blocking(move || renamer::organize_season_folders(&db, &item, &templates).map_err(err_string)).await?;
        }
        let done = crate::ui_i18n::tf(&self.locale, "prog.organized", &[("n", &total.to_string())]);
        progress.update(TaskProgress::new(total, total, done, "organize")).await;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowMergePlanDto {
    pub source_id: String,
    pub source_title: String,
    pub source_folder: String,
    pub target_id: String,
    pub target_title: String,
    pub target_folder: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowMergePair {
    pub source_id: String,
    pub target_id: String,
}

/// Read-only preview of duplicate-show merges for a library or selected items.
pub fn plan_show_merges(db: &AppDatabase, library_id: Option<String>, item_ids: Option<Vec<String>>) -> Result<Vec<ShowMergePlanDto>, String> {
    let candidates = match (library_id, item_ids) {
        (_, Some(ids)) => ids
            .iter()
            .filter_map(|id| db.get_media_item(id).transpose())
            .collect::<Result<Vec<_>, _>>()
            .map_err(err_string)?,
        (Some(library_id), None) => db.list_media_items(&library_id).map_err(err_string)?,
        (None, None) => Vec::new(),
    };
    let plan = renamer::plan_duplicate_show_merges(db, &candidates).map_err(err_string)?;
    Ok(plan
        .into_iter()
        .map(|(source, target)| ShowMergePlanDto {
            source_id: source.id,
            source_title: source.title,
            source_folder: source.folder_path,
            target_id: target.id,
            target_title: target.title,
            target_folder: target.folder_path,
        })
        .collect())
}

/// Execute confirmed merges; each pair is checked again against the current index.
pub fn merge_planned_shows(db: &AppDatabase, pairs: &[ShowMergePair], templates: &RenameTemplates, events: &dyn Events) -> Result<u32, String> {
    let mut merged = 0u32;
    for pair in pairs {
        if renamer::merge_planned_show(db, &pair.source_id, &pair.target_id, templates).map_err(err_string)? {
            merged += 1;
        }
    }
    if merged > 0 {
        events.library_updated();
    }
    Ok(merged)
}

/// Every item a merge request touches, sources and targets, for its lock scope.
pub fn merge_item_ids(pairs: &[ShowMergePair]) -> Vec<String> {
    pairs.iter().flat_map(|p| [p.source_id.clone(), p.target_id.clone()]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{movie_library, FakeProgress};
    use media_core::ScrapedStatus;
    use std::path::Path;

    fn service(db: &Arc<AppDatabase>) -> OrganizeService {
        OrganizeService { db: Arc::clone(db), templates: RenameTemplates::default(), create_season_folders: false, locale: "en".into() }
    }

    #[tokio::test]
    async fn rename_templates_move_the_movie_and_report_progress() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Scraped);
        let progress = FakeProgress::default();
        service(&db).apply_rename_templates(vec![item.id.clone()], &progress).await.unwrap();
        let renamed = db.get_media_item(&item.id).unwrap().unwrap();
        assert_ne!(renamed.file_path, item.file_path);
        assert!(Path::new(&renamed.file_path).is_file());
        assert!(!Path::new(&item.file_path).exists());
        assert_eq!(progress.stages(), ["rename", "rename"]);
        assert_eq!(progress.last().completed, 1);
    }

    #[tokio::test]
    async fn rename_of_unscraped_item_fails_and_keeps_files() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        let err = service(&db).apply_rename_templates(vec![item.id.clone()], &FakeProgress::default()).await.unwrap_err();
        assert!(err.contains("not scraped"), "{err}");
        assert!(Path::new(&item.file_path).is_file());
    }

    #[tokio::test]
    async fn cancelled_rename_touches_nothing() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Scraped);
        let progress = FakeProgress::default();
        progress.cancel.store(true, Ordering::SeqCst);
        assert_eq!(service(&db).apply_rename_templates(vec![item.id.clone()], &progress).await.unwrap_err(), "cancelled");
        assert!(Path::new(&item.file_path).is_file());
    }

    #[tokio::test]
    async fn organize_rejects_movies_without_moving() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Scraped);
        assert!(service(&db).organize_season_folders(vec![item.id.clone()], &FakeProgress::default()).await.is_err());
        assert!(Path::new(&item.file_path).is_file());
    }

    #[test]
    fn auto_rename_counts_successes_and_failures() {
        let (_dir, db, lib, item) = movie_library(ScrapedStatus::Scraped);
        let other = MediaItem::new_movie("Loose", None, lib.root_path.clone(), format!("{}/missing.mkv", lib.root_path), lib.id.clone(), ScrapedStatus::Scraped);
        db.insert_media_items(std::slice::from_ref(&other)).unwrap();
        let ids = vec![item.id.clone(), other.id.clone(), "vanished".into()];
        let (ok, failed) = auto_rename_blocking(&db, &ids, &RenameTemplates::default(), false, &AtomicBool::new(false));
        assert_eq!((ok, failed), (1, 1));
        assert_eq!(auto_rename_blocking(&db, &ids, &RenameTemplates::default(), false, &AtomicBool::new(true)), (0, 0));
    }
}
