//! Queued work: refreshing, scraping, matching, renaming and organizing. Each
//! call returns the queued task; later changes arrive through `Events`.

use std::sync::Arc;

use media_core::{MediaType, ScrapedStatus};

use crate::app::library::RefreshService;
use crate::app::locks::LockScope;
use crate::app::organize::OrganizeService;
use crate::app::scrape::{localized_error, ScrapeService, ScrapeSettings};
use crate::error::{blocking, failed, CoreError, CoreResult};
use crate::config::AppConfig;
use crate::state::AppState;
use crate::task_queue::{TaskKind, TaskSnapshot};
use crate::ui_i18n;

impl AppState {
    pub async fn tasks(&self) -> Vec<TaskSnapshot> {
        self.tasks.list().await
    }

    pub async fn cancel_task(&self, id: String) -> bool {
        self.tasks.cancel(&id).await
    }

    /// A five-step no-op task for exercising the task UI.
    pub async fn enqueue_smoke_task(&self, title: Option<String>) -> TaskSnapshot {
        self.tasks.enqueue_smoke(title.unwrap_or_else(|| "M0 smoke task".into())).await
    }

    /// Rescans a library; a refresh already queued or running is returned instead.
    pub async fn refresh_library(&self, library_id: String) -> CoreResult<TaskSnapshot> {
        if let Some(existing) = self.tasks.find_active(TaskKind::Refresh, &library_id).await {
            self.events.task_updated(&existing);
            return Ok(existing);
        }

        let library = self
            .db
            .get_library(&library_id)
            .map_err(failed)?
            .ok_or_else(|| CoreError::not_found("library", &library_id))?;
        let locale = self.ui_locale().await;
        let config_store = Arc::clone(&self.config);
        let db = Arc::clone(&self.db);
        let title = ui_i18n::tf(&locale, "task.refreshLib", &[("name", &library.name)]);
        let target_id = Some(library_id.clone());
        let lock = Some(LockScope::library(&library_id));

        let snapshot = self
            .tasks
            .enqueue_scoped(title, TaskKind::Refresh, target_id, task_scope("library", std::slice::from_ref(&library_id)), lock, move |handle| {
                Box::pin(async move {
                    // Settings are read when the refresh starts, not when it was queued.
                    let config = config_store.lock().await.config.clone();
                    let service = RefreshService {
                        db,
                        excluded_folders: config.scan_excluded_folders.clone(),
                        templates: config.rename_templates(),
                        locale,
                    };
                    service.refresh_library(&library_id, &handle).await
                })
            })
            .await;
        Ok(snapshot)
    }

    /// Re-reads the given items from disk.
    pub async fn refresh_items(&self, item_ids: Vec<String>) -> CoreResult<TaskSnapshot> {
        if item_ids.is_empty() {
            return Err(CoreError::invalid("no items selected"));
        }
        let locale = self.ui_locale().await;
        let title = if item_ids.len() == 1 {
            ui_i18n::t(&locale, "task.refreshItems")
        } else {
            ui_i18n::tf(&locale, "task.refreshItemsN", &[("n", &item_ids.len().to_string())])
        };
        let config_store = Arc::clone(&self.config);
        let db = Arc::clone(&self.db);
        let lock = self.items_scope(&item_ids).await?;
        let snapshot = self
            .tasks
            .enqueue_scoped(title, TaskKind::Refresh, item_ids.first().cloned(), task_scope("items", &item_ids), Some(lock), move |handle| {
                Box::pin(async move {
                    let excluded_folders = config_store.lock().await.config.scan_excluded_folders.clone();
                    let service = RefreshService { db, excluded_folders, templates: Default::default(), locale };
                    service.refresh_items(item_ids, &handle).await
                })
            })
            .await;
        Ok(snapshot)
    }

    /// Scrapes every item of a library that still needs metadata.
    pub async fn scrape_library(&self, library_id: String) -> CoreResult<TaskSnapshot> {
        let library = self
            .db
            .get_library(&library_id)
            .map_err(failed)?
            .ok_or_else(|| CoreError::not_found("library", &library_id))?;
        let config = self.config_with_keys().await;
        let title = ui_i18n::tf(&config.ui_locale, "task.scrapeAll", &[("name", &library.name)]);
        let target_id = Some(library_id.clone());

        if let Some(existing) = self.tasks.find_active(TaskKind::BatchScrape, &library_id).await {
            self.events.task_updated(&existing);
            return Ok(existing);
        }

        // No held lock: the service fetches unlocked and locks the library only to write.
        let service = self.scrape_service(&config);
        let snapshot = self
            .tasks
            .enqueue(title, TaskKind::BatchScrape, target_id, None, move |handle| {
                Box::pin(async move { service.scrape_library(&library_id, &handle).await })
            })
            .await;
        Ok(snapshot)
    }

    pub async fn scrape_items(&self, item_ids: Vec<String>) -> CoreResult<TaskSnapshot> {
        if item_ids.is_empty() {
            return Err(CoreError::invalid("no items selected"));
        }
        let config = self.config_with_keys().await;
        let title = ui_i18n::tf(&config.ui_locale, "task.scrapeN", &[("n", &item_ids.len().to_string())]);
        let service = self.scrape_service(&config);
        let snapshot = self
            .tasks
            .enqueue_scoped(title, TaskKind::Scrape, item_ids.first().cloned(), task_scope("items", &item_ids), None, move |handle| {
                Box::pin(async move { service.scrape_items(item_ids, &handle).await })
            })
            .await;
        Ok(snapshot)
    }

    /// Scrapes again the selected items that already have metadata.
    pub async fn rescrape_items(&self, item_ids: Vec<String>) -> CoreResult<TaskSnapshot> {
        if item_ids.is_empty() {
            return Err(CoreError::invalid("no items selected"));
        }
        let mut scraped_ids = Vec::new();
        for id in &item_ids {
            let item = self
                .db
                .get_media_item(id)
                .map_err(failed)?
                .ok_or_else(|| CoreError::not_found("media item", id))?;
            if item.status == ScrapedStatus::Scraped {
                scraped_ids.push(id.clone());
            }
        }
        if scraped_ids.is_empty() {
            return Err(CoreError::invalid("no scraped items selected"));
        }

        let config = self.config_with_keys().await;
        let title = ui_i18n::tf(&config.ui_locale, "task.rescrapeN", &[("n", &scraped_ids.len().to_string())]);
        let service = self.scrape_service(&config);
        let snapshot = self
            .tasks
            .enqueue_scoped(
                title,
                TaskKind::Rescrape,
                scraped_ids.first().cloned(),
                task_scope("items", &scraped_ids),
                None,
                move |handle| Box::pin(async move { service.scrape_items(scraped_ids, &handle).await }),
            )
            .await;
        Ok(snapshot)
    }

    pub async fn scrape_season(&self, media_item_id: String, season_number: i32) -> CoreResult<TaskSnapshot> {
        let config = self.config_with_keys().await;
        let service = self.scrape_service(&config);
        let snapshot = self.tasks.enqueue_scoped(format!("Season {season_number}"), TaskKind::Scrape,
            Some(media_item_id.clone()), task_scope("season", &[media_item_id.clone(), season_number.to_string()]), None,
            move |handle| Box::pin(async move { service.scrape_season(&media_item_id, season_number, &handle).await })).await;
        Ok(snapshot)
    }

    /// Searches the configured sources for the manual-match dialog.
    pub async fn search_match_candidates(&self, query: String, media_type: MediaType) -> CoreResult<Vec<scraper_kit::SearchResult>> {
        let config = self.config_with_keys().await;
        let locale = config.ui_locale.clone();
        let coordinator = scraper_kit::ScraperCoordinator::new(scraper_keys(&config));
        coordinator
            .search_manual(&query, media_type, &config.metadata_language)
            .await
            .map_err(|e| CoreError::Failed(localized_error(&locale, e)))
    }

    pub async fn apply_manual_match(&self, item_id: String, source_id: String) -> CoreResult<TaskSnapshot> {
        let config = self.config_with_keys().await;
        let item = self
            .db
            .get_media_item(&item_id)
            .map_err(failed)?
            .ok_or_else(|| CoreError::not_found("media item", &item_id))?;
        let title = ui_i18n::tf(&config.ui_locale, "task.manualMatch", &[("title", &item.title)]);
        let target_id = Some(item_id.clone());
        let service = self.scrape_service(&config);

        let snapshot = self
            .tasks
            .enqueue_scoped(title, TaskKind::ManualMatch, target_id, task_scope("match", &[item_id.clone(), source_id.clone()]), None, move |handle| {
                Box::pin(async move { service.apply_manual_match(&item, &source_id, &handle).await })
            })
            .await;
        Ok(snapshot)
    }

    /// Renames the items' folders and files with the configured templates.
    pub async fn apply_rename_templates(&self, item_ids: Vec<String>) -> CoreResult<TaskSnapshot> {
        if item_ids.is_empty() {
            return Err(CoreError::invalid("no items selected"));
        }
        let config = self.config().await;
        let title = ui_i18n::tf(&config.ui_locale, "task.renameN", &[("n", &item_ids.len().to_string())]);
        let lock = self.items_scope(&item_ids).await?;
        let service = self.organize_service(&config);
        let snapshot = self
            .tasks
            .enqueue_scoped(title, TaskKind::Rename, item_ids.first().cloned(), task_scope("items", &item_ids), Some(lock), move |handle| {
                Box::pin(async move { service.apply_rename_templates(item_ids, &handle).await })
            })
            .await;
        Ok(snapshot)
    }

    /// Moves episodes of the selected scraped shows into season folders.
    pub async fn organize_season_folders(&self, item_ids: Vec<String>) -> CoreResult<TaskSnapshot> {
        if item_ids.is_empty() {
            return Err(CoreError::invalid("no items selected"));
        }
        let mut targets = Vec::new();
        for id in &item_ids {
            let item = self
                .db
                .get_media_item(id)
                .map_err(failed)?
                .ok_or_else(|| CoreError::not_found("media item", id))?;
            if item.status == ScrapedStatus::Scraped && matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
                targets.push(id.clone());
            }
        }
        if targets.is_empty() {
            return Err(CoreError::invalid("no scraped tv/anime items selected"));
        }

        let config = self.config().await;
        let title = ui_i18n::tf(&config.ui_locale, "task.organizeN", &[("n", &targets.len().to_string())]);
        let lock = self.items_scope(&targets).await?;
        let service = self.organize_service(&config);
        let snapshot = self
            .tasks
            .enqueue_scoped(
                title,
                TaskKind::Organize,
                targets.first().cloned(),
                task_scope("items", &targets),
                Some(lock),
                move |handle| Box::pin(async move { service.organize_season_folders(targets, &handle).await }),
            )
            .await;
        Ok(snapshot)
    }

    /// The lock scope covering the libraries the given items belong to.
    pub(crate) async fn items_scope(&self, item_ids: &[String]) -> CoreResult<LockScope> {
        let (db, ids) = (Arc::clone(&self.db), item_ids.to_vec());
        blocking(move || LockScope::for_items(&db, &ids).map_err(CoreError::from)).await
    }

    fn scrape_service(&self, config: &AppConfig) -> ScrapeService<scraper_kit::ScrapeClient> {
        let options = scrape_options_from_config(config);
        ScrapeService {
            db: Arc::clone(&self.db),
            locks: Arc::clone(self.tasks.locks()),
            source: Arc::new(scraper_kit::ScrapeClient::new(&options)),
            settings: ScrapeSettings {
                concurrency: options.concurrency,
                nfo_format: config.nfo_format.clone(),
                locale: config.ui_locale.clone(),
                templates: config.rename_templates(),
                auto_rename: config.rename_auto_after_scrape,
                create_season_folders: config.rename_create_season_folders,
            },
        }
    }

    fn organize_service(&self, config: &AppConfig) -> OrganizeService {
        OrganizeService {
            db: Arc::clone(&self.db),
            templates: config.rename_templates(),
            create_season_folders: config.rename_create_season_folders,
            locale: config.ui_locale.clone(),
        }
    }
}

/// Identifies equivalent requests so a double click returns the queued task.
pub(crate) fn task_scope(label: &str, ids: &[String]) -> Option<String> {
    let mut ids = ids.to_vec();
    ids.sort();
    ids.dedup();
    Some(format!("{label}:{}", serde_json::to_string(&ids).expect("string list")))
}

fn scrape_options_from_config(config: &AppConfig) -> scraper_kit::ScrapeOptions {
    scraper_kit::ScrapeOptions {
        language: config.metadata_language.clone(),
        concurrency: config.scrape_concurrency.max(1) as usize,
        keys: scraper_keys(config),
    }
}

fn scraper_keys(config: &AppConfig) -> scraper_kit::ScraperKeys {
    scraper_kit::ScraperKeys {
        tmdb: config.api_keys.tmdb.clone(),
        bangumi: config.api_keys.bangumi.clone(),
        omdb: config.api_keys.omdb.clone(),
        tvdb: config.api_keys.tvdb.clone(),
    }
}
