//! Scrape workflows. The network phase (match, detail fetch, image bytes) runs
//! without any file-mutation lock; only writing results and the follow-up
//! auto-rename take the item's library lock. Cancellation is honoured in both.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use media_core::{AppDatabase, MediaItem, MediaType, ScrapedStatus};
use renamer::RenameTemplates;
use scraper_kit::{MatchOutcome, ScrapedMetadata, ScrapedSeason};

use super::locks::{LockScope, MutationLocks};
use super::organize::{auto_rename_blocking, consolidate_blocking};
use super::scrape_persist::{persist_match, persist_season, ScrapeImages};
use super::{blocking, cancellable, err_string, Progress, TaskProgress};

/// Network side of a scrape: scraper-kit's `ScrapeClient`, or a fake in tests.
pub trait MetadataSource: Send + Sync + 'static {
    fn match_item(&self, item: &MediaItem) -> impl Future<Output = MatchOutcome> + Send;
    fn fetch_by_source(&self, source_id: &str, media_type: MediaType) -> impl Future<Output = Result<ScrapedMetadata, String>> + Send;
    fn fetch_season(&self, tmdb_id: &str, season_number: i32) -> impl Future<Output = Result<ScrapedSeason, String>> + Send;
    fn fetch_image(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
}

impl MetadataSource for scraper_kit::ScrapeClient {
    fn match_item(&self, item: &MediaItem) -> impl Future<Output = MatchOutcome> + Send {
        scraper_kit::ScrapeClient::match_item(self, item)
    }
    fn fetch_by_source(&self, source_id: &str, media_type: MediaType) -> impl Future<Output = Result<ScrapedMetadata, String>> + Send {
        scraper_kit::ScrapeClient::fetch_by_source(self, source_id, media_type)
    }
    fn fetch_season(&self, tmdb_id: &str, season_number: i32) -> impl Future<Output = Result<ScrapedSeason, String>> + Send {
        scraper_kit::ScrapeClient::fetch_season(self, tmdb_id, season_number)
    }
    fn fetch_image(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>> + Send {
        scraper_kit::ScrapeClient::fetch_image(self, url)
    }
}

/// Outcome of scraping one media item (auto-match path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrapeItemOutcome {
    Matched,
    Unmatched,
    Failed,
}

#[derive(Debug, Clone, Default)]
pub struct ScrapeSummary {
    pub success_ids: Vec<String>,
    pub unmatched: u32,
    pub failed: u32,
}

impl ScrapeSummary {
    pub fn format_result(&self) -> String {
        format!("success={} unmatched={} failed={}", self.success_ids.len(), self.unmatched, self.failed)
    }

    pub fn parse_result(s: &str) -> Option<(u32, u32, u32)> {
        let mut success = None;
        let mut unmatched = None;
        let mut failed = None;
        for part in s.split_whitespace() {
            if let Some(v) = part.strip_prefix("success=") {
                success = v.parse().ok();
            } else if let Some(v) = part.strip_prefix("unmatched=") {
                unmatched = v.parse().ok();
            } else if let Some(v) = part.strip_prefix("failed=") {
                failed = v.parse().ok();
            }
        }
        Some((success?, unmatched?, failed?))
    }

    fn record(&mut self, id: String, outcome: ScrapeItemOutcome) {
        match outcome {
            ScrapeItemOutcome::Matched => self.success_ids.push(id),
            ScrapeItemOutcome::Unmatched => self.unmatched += 1,
            ScrapeItemOutcome::Failed => self.failed += 1,
        }
    }

    fn count(&self) -> u32 {
        self.success_ids.len() as u32 + self.unmatched + self.failed
    }
}

pub fn localized_summary(locale: &str, raw: &str) -> String {
    if let Some((s, u, f)) = ScrapeSummary::parse_result(raw) {
        return crate::ui_i18n::tf(locale, "prog.scrapeSummary", &[("success", &s.to_string()), ("unmatched", &u.to_string()), ("failed", &f.to_string())]);
    }
    raw.to_string()
}

pub fn localized_error(locale: &str, err: String) -> String {
    if err.starts_with("err.") { crate::ui_i18n::t(locale, &err) } else { err }
}

/// Config snapshot taken when the task was enqueued.
#[derive(Debug, Clone)]
pub struct ScrapeSettings {
    pub concurrency: usize,
    pub nfo_format: String,
    pub locale: String,
    pub templates: RenameTemplates,
    pub auto_rename: bool,
    pub create_season_folders: bool,
}

/// Result of the unlocked network phase for one item.
enum Fetched {
    Matched(Box<(ScrapedMetadata, ScrapeImages)>),
    Unmatched,
    Failed(String),
}

/// Why an item was not written.
enum Abort {
    Cancelled,
    /// Ends the whole job (e.g. media recovery pending).
    Job(String),
    Item(String),
}

pub struct ScrapeService<S> {
    pub db: Arc<AppDatabase>,
    pub locks: Arc<MutationLocks>,
    pub source: Arc<S>,
    pub settings: ScrapeSettings,
}

impl<S: MetadataSource> ScrapeService<S> {
    pub async fn scrape_library(&self, library_id: &str, progress: &impl Progress) -> Result<(), String> {
        let locale = self.settings.locale.clone();
        let summary = self.scrape_library_items(library_id, progress).await.map_err(|e| localized_error(&locale, e))?;
        progress.scrape_result(&summary).await;
        if progress.is_cancelled() { return Err("cancelled".into()); }
        let n = summary.count();
        progress.update(TaskProgress::new(n, n, localized_summary(&locale, &summary.format_result()), "saveResults")).await;
        let renamed = summary.success_ids.len() as u32;
        self.after_scrape(&summary, &LockScope::library(library_id), (renamed, n), progress).await
    }

    /// Auto-match every unscraped item with bounded fetch concurrency.
    async fn scrape_library_items(&self, library_id: &str, progress: &impl Progress) -> Result<ScrapeSummary, String> {
        let cancel = progress.cancel_flag();
        if cancel.load(Ordering::SeqCst) { return Err("cancelled".into()); }
        let (db, id) = (Arc::clone(&self.db), library_id.to_string());
        let items = blocking(move || db.list_media_items(&id).map_err(err_string)).await?
            .into_iter().filter(|i| i.status == ScrapedStatus::Unscraped).collect::<Vec<_>>();
        let total = items.len() as u32;
        let scope = LockScope::library(library_id);
        let mut items = items.into_iter();
        let mut done = 0;
        let mut summary = ScrapeSummary::default();
        let mut jobs = tokio::task::JoinSet::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                // Fetched but unwritten results are dropped: cancel stops all writes.
                jobs.abort_all();
                while jobs.join_next().await.is_some() {}
                return Ok(summary);
            }
            while jobs.len() < self.settings.concurrency.clamp(1, 8) {
                let Some(item) = items.next() else { break; };
                let (db, source) = (Arc::clone(&self.db), Arc::clone(&self.source));
                jobs.spawn(async move {
                    let fetched = fetch_item(&*source, &db, &item).await;
                    (item.id, item.title, fetched)
                });
            }
            if jobs.is_empty() { break; }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
                result = jobs.join_next() => {
                    // A panicked fetch fails only its own item (it stays Unscraped for the next run).
                    let (id, title, fetched) = match result.expect("nonempty job set") {
                        Ok(joined) => joined,
                        Err(error) => {
                            tracing::warn!(%error, "scrape task failed");
                            (String::new(), String::new(), Fetched::Failed(error.to_string()))
                        }
                    };
                    if cancel.load(Ordering::SeqCst) { continue; }
                    done += 1;
                    let outcome = if id.is_empty() { ScrapeItemOutcome::Failed } else {
                        match self.store(&scope, id.clone(), fetched, &cancel).await {
                            Ok(outcome) => outcome,
                            Err(Abort::Cancelled) => continue,
                            Err(Abort::Job(error)) => return Err(error),
                            Err(Abort::Item(_)) => ScrapeItemOutcome::Failed,
                        }
                    };
                    summary.record(id, outcome);
                    progress.update(TaskProgress::new(done, total, title, "matching")).await;
                }
            }
        }
        Ok(summary)
    }

    /// Selected items, one at a time (scrape and rescrape).
    pub async fn scrape_items(&self, item_ids: Vec<String>, progress: &impl Progress) -> Result<(), String> {
        let cancel = progress.cancel_flag();
        let mut summary = ScrapeSummary::default();
        let total = item_ids.len() as u32;
        let mut libraries = Vec::new();
        for (idx, id) in item_ids.into_iter().enumerate() {
            if cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            let (db, item_id) = (Arc::clone(&self.db), id.clone());
            let item = blocking(move || db.get_media_item(&item_id).map_err(err_string)?.ok_or_else(|| format!("media item not found: {item_id}"))).await?;
            progress.update(TaskProgress::new(idx as u32, total, item.title.clone(), "matching")).await;
            libraries.push(item.library_id.clone());
            let result = match cancellable(&cancel, async { Ok(fetch_item(&*self.source, &self.db, &item).await) }).await {
                Ok(fetched) => self.store(&LockScope::library(&item.library_id), id.clone(), fetched, &cancel).await,
                Err(_) => Err(Abort::Cancelled),
            };
            match result {
                Ok(outcome) => summary.record(id, outcome),
                Err(Abort::Cancelled) => return Err("cancelled".into()),
                Err(Abort::Job(error)) => return Err(error),
                Err(Abort::Item(error)) => {
                    if cancel.load(Ordering::SeqCst) { return Err(error); }
                    summary.failed += 1;
                    self.db.update_status(&id, ScrapedStatus::Partial, Some(&error)).map_err(err_string)?;
                }
            }
            progress.scrape_result(&summary).await;
        }
        let locale = &self.settings.locale;
        progress.update(TaskProgress::new(total, total, localized_summary(locale, &summary.format_result()), "saveResults")).await;
        libraries.sort();
        libraries.dedup();
        self.after_scrape(&summary, &LockScope::Libraries(libraries), (total, total), progress).await
    }

    /// Refresh one season's metadata / episode info from TMDB (show must already be scraped).
    pub async fn scrape_season(&self, item_id: &str, season_number: i32, progress: &impl Progress) -> Result<(), String> {
        let cancel = progress.cancel_flag();
        let (db, id) = (Arc::clone(&self.db), item_id.to_string());
        let (item, meta) = blocking(move || {
            let item = db.get_media_item(&id).map_err(err_string)?.ok_or_else(|| "media item not found".to_string())?;
            let meta = db.fetch_metadata(&id).map_err(err_string)?;
            Ok((item, meta))
        }).await?;
        if !matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
            return Err("err.notTvShow".into());
        }
        let meta = meta.ok_or_else(|| "err.notScraped".to_string())?;
        let tmdb_id = meta.tmdb_id.as_deref().filter(|s| !s.is_empty()).map(str::to_string)
            .or_else(|| meta.source_id.strip_prefix("tmdb:").filter(|s| !s.is_empty()).map(str::to_string))
            .ok_or_else(|| "err.noTmdbId".to_string())?;
        let (season, images) = cancellable(&cancel, async {
            let season = self.source.fetch_season(&tmdb_id, season_number).await?;
            let mut images = ScrapeImages::default();
            fetch_season_images(&*self.source, &self.db, &item, std::slice::from_ref(&season), &mut images).await;
            Ok((season, images))
        }).await?;
        let guard = self.locks.lock(&LockScope::library(&item.library_id)).await?;
        if cancel.load(Ordering::SeqCst) { return Err("cancelled".into()); }
        let db = Arc::clone(&self.db);
        blocking(move || {
            let _guard = guard;
            let item = db.get_media_item(&item.id).map_err(err_string)?.ok_or_else(|| "media item not found".to_string())?;
            persist_season(&db, &item, &season, &images)
        }).await
    }

    /// Apply a user-chosen match, then rename or consolidate like an auto-match.
    pub async fn apply_manual_match(&self, item: &MediaItem, source_id: &str, progress: &impl Progress) -> Result<(), String> {
        let cancel = progress.cancel_flag();
        let locale = self.settings.locale.clone();
        progress.update(TaskProgress::new(0, 1, item.title.clone(), "matching")).await;
        let fetched = cancellable(&cancel, async {
            let meta = self.source.fetch_by_source(source_id, item.media_type).await?;
            let images = fetch_images(&*self.source, &self.db, item, &meta).await;
            Ok((meta, images))
        }).await.map_err(|e| localized_error(&locale, e))?;
        let scope = LockScope::library(&item.library_id);
        let guard = self.locks.lock(&scope).await?;
        if cancel.load(Ordering::SeqCst) { return Err("cancelled".into()); }
        let (db, id, nfo_format) = (Arc::clone(&self.db), item.id.clone(), self.settings.nfo_format.clone());
        blocking(move || {
            let _guard = guard;
            let (meta, images) = fetched;
            let item = db.get_media_item(&id).map_err(err_string)?.ok_or_else(|| "media item removed during scrape".to_string())?;
            persist_match(&db, &item, &meta, &images, &nfo_format)
        }).await.map_err(|e| localized_error(&locale, e))?;
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let ids = [item.id.clone()];
        if self.settings.auto_rename {
            progress.update(TaskProgress::new(0, 1, crate::ui_i18n::t(&locale, "prog.autoRename"), "rename")).await;
            self.auto_rename(&scope, &ids, &cancel).await?;
        } else {
            self.consolidate(&scope, &ids, &cancel).await?;
        }
        progress.update(TaskProgress::new(1, 1, item.title.clone(), "saveResults")).await;
        Ok(())
    }

    /// Write one fetched result under the library lock, unless cancelled meanwhile.
    async fn store(&self, scope: &LockScope, id: String, fetched: Fetched, cancel: &AtomicBool) -> Result<ScrapeItemOutcome, Abort> {
        if cancel.load(Ordering::SeqCst) { return Err(Abort::Cancelled); }
        let guard = self.locks.lock(scope).await.map_err(Abort::Job)?;
        // The lock may have been contended for a while.
        if cancel.load(Ordering::SeqCst) { return Err(Abort::Cancelled); }
        let (db, nfo_format) = (Arc::clone(&self.db), self.settings.nfo_format.clone());
        blocking(move || {
            let _guard = guard;
            store_item(&db, &id, fetched, &nfo_format)
        }).await.map_err(Abort::Item)
    }

    /// Rename (or only consolidate) the successes, then report the final summary.
    async fn after_scrape(&self, summary: &ScrapeSummary, scope: &LockScope, (rename_n, final_n): (u32, u32), progress: &impl Progress) -> Result<(), String> {
        if summary.success_ids.is_empty() {
            return Ok(());
        }
        let cancel = progress.cancel_flag();
        let locale = &self.settings.locale;
        if !self.settings.auto_rename {
            return self.consolidate(scope, &summary.success_ids, &cancel).await;
        }
        progress.update(TaskProgress::new(rename_n, rename_n, crate::ui_i18n::t(locale, "prog.autoRename"), "rename")).await;
        let (renamed, rename_failed) = self.auto_rename(scope, &summary.success_ids, &cancel).await?;
        let mut text = localized_summary(locale, &summary.format_result());
        if rename_failed > 0 {
            let result = crate::ui_i18n::tf(locale, "prog.autoRenameResult", &[("ok", &renamed.to_string()), ("failed", &rename_failed.to_string())]);
            text = format!("{text} · {result}");
        }
        progress.update(TaskProgress::new(final_n, final_n, text, "saveResults")).await;
        Ok(())
    }

    async fn auto_rename(&self, scope: &LockScope, ids: &[String], cancel: &Arc<AtomicBool>) -> Result<(u32, u32), String> {
        let guard = self.locks.lock(scope).await?;
        let (db, ids, templates, cancel) = (Arc::clone(&self.db), ids.to_vec(), self.settings.templates.clone(), Arc::clone(cancel));
        let (create_season_folders, total) = (self.settings.create_season_folders, ids.len() as u32);
        Ok(tokio::task::spawn_blocking(move || {
            let _guard = guard;
            auto_rename_blocking(&db, &ids, &templates, create_season_folders, &cancel)
        }).await.unwrap_or((0, total)))
    }

    async fn consolidate(&self, scope: &LockScope, ids: &[String], cancel: &Arc<AtomicBool>) -> Result<(), String> {
        let guard = self.locks.lock(scope).await?;
        let (db, ids, templates, cancel) = (Arc::clone(&self.db), ids.to_vec(), self.settings.templates.clone(), Arc::clone(cancel));
        let _ = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            consolidate_blocking(&db, &ids, &templates, &cancel)
        }).await;
        Ok(())
    }
}

async fn fetch_item<S: MetadataSource>(source: &S, db: &Arc<AppDatabase>, item: &MediaItem) -> Fetched {
    match source.match_item(item).await {
        MatchOutcome::Matched(meta) => {
            let images = fetch_images(source, db, item, &meta).await;
            Fetched::Matched(Box::new((meta, images)))
        }
        MatchOutcome::Unmatched { .. } => Fetched::Unmatched,
        MatchOutcome::Failed(err) => Fetched::Failed(err),
    }
}

async fn fetch_images<S: MetadataSource>(source: &S, db: &Arc<AppDatabase>, item: &MediaItem, meta: &ScrapedMetadata) -> ScrapeImages {
    let mut images = ScrapeImages::default();
    for (role, url) in [("poster", &meta.poster_url), ("fanart", &meta.fanart_url), ("banner", &meta.banner_url)] {
        if let Some(url) = url {
            images.artwork.push((role, source.fetch_image(url).await));
        }
    }
    if matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
        fetch_season_images(source, db, item, &meta.seasons, &mut images).await;
    }
    images
}

/// Season posters, plus stills only for episodes that have a local file to sit next to.
async fn fetch_season_images<S: MetadataSource>(source: &S, db: &Arc<AppDatabase>, item: &MediaItem, seasons: &[ScrapedSeason], images: &mut ScrapeImages) {
    let (db, id) = (Arc::clone(db), item.id.clone());
    let local = blocking(move || {
        let mut local = std::collections::HashSet::new();
        for season in db.fetch_seasons(&id).map_err(err_string)? {
            for episode in db.fetch_episodes(&season.id).map_err(err_string)? {
                if !episode.file_path.is_empty() { local.insert((season.season_number, episode.episode_number)); }
            }
        }
        Ok(local)
    }).await.unwrap_or_default();
    for season in seasons {
        if let Some(url) = &season.poster_url {
            images.season_posters.insert(season.season_number, source.fetch_image(url).await);
        }
        for episode in &season.episodes {
            let key = (season.season_number, episode.episode_number);
            if let (Some(url), true) = (&episode.still_url, local.contains(&key)) {
                images.stills.insert(key, (url.clone(), source.fetch_image(url).await));
            }
        }
    }
}

fn store_item(db: &AppDatabase, id: &str, fetched: Fetched, nfo_format: &str) -> Result<ScrapeItemOutcome, String> {
    match fetched {
        Fetched::Matched(matched) => {
            let (meta, images) = *matched;
            // Re-read under the lock: a rename may have moved the folder meanwhile.
            let Some(item) = db.get_media_item(id).map_err(err_string)? else {
                return Ok(ScrapeItemOutcome::Failed);
            };
            match persist_match(db, &item, &meta, &images, nfo_format) {
                Ok(()) => Ok(ScrapeItemOutcome::Matched),
                Err(error) => {
                    db.update_status(id, ScrapedStatus::Partial, Some(&error)).map_err(err_string)?;
                    Ok(ScrapeItemOutcome::Failed)
                }
            }
        }
        Fetched::Unmatched => {
            db.update_status(id, ScrapedStatus::Unmatched, None).map_err(err_string)?;
            Ok(ScrapeItemOutcome::Unmatched)
        }
        Fetched::Failed(err) if err == "noResults" => {
            db.update_status(id, ScrapedStatus::Unmatched, Some(&err)).map_err(err_string)?;
            Ok(ScrapeItemOutcome::Unmatched)
        }
        Fetched::Failed(err) => {
            db.update_status(id, ScrapedStatus::Partial, Some(&err)).map_err(err_string)?;
            Ok(ScrapeItemOutcome::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{metadata, movie_library, FakeProgress};
    use std::path::Path;
    use std::sync::atomic::AtomicU32;

    /// Answers by item title; counts network calls.
    #[derive(Default)]
    struct FakeSource {
        matches: AtomicU32,
        fetched: tokio::sync::Notify,
        /// Runs during the network phase, e.g. a concurrent rename.
        during_fetch: Option<Box<dyn Fn() + Send + Sync>>,
    }

    impl MetadataSource for FakeSource {
        async fn match_item(&self, item: &MediaItem) -> MatchOutcome {
            self.matches.fetch_add(1, Ordering::SeqCst);
            if let Some(hook) = &self.during_fetch { hook(); }
            self.fetched.notify_one();
            match item.title.as_str() {
                "Original" => {
                    let mut meta = metadata();
                    meta.poster_url = Some("poster".into());
                    MatchOutcome::Matched(meta)
                }
                "Nobody" => MatchOutcome::Unmatched { candidates: Vec::new() },
                "Empty" => MatchOutcome::Failed("noResults".into()),
                _ => MatchOutcome::Failed("err.connect".into()),
            }
        }
        async fn fetch_by_source(&self, _source_id: &str, _media_type: MediaType) -> Result<ScrapedMetadata, String> {
            Ok(metadata())
        }
        async fn fetch_season(&self, _tmdb_id: &str, _season_number: i32) -> Result<ScrapedSeason, String> {
            Err("unused".into())
        }
        async fn fetch_image(&self, url: &str) -> Result<Vec<u8>, String> {
            Ok(format!("image:{url}").into_bytes())
        }
    }

    fn service(db: &Arc<AppDatabase>, locks: Arc<MutationLocks>, auto_rename: bool) -> ScrapeService<FakeSource> {
        ScrapeService {
            db: Arc::clone(db),
            locks,
            source: Arc::new(FakeSource::default()),
            settings: ScrapeSettings {
                concurrency: 2,
                nfo_format: "kodi".into(),
                locale: "en".into(),
                templates: RenameTemplates::default(),
                auto_rename,
                create_season_folders: false,
            },
        }
    }

    #[tokio::test]
    async fn scrape_items_persists_match_then_auto_renames() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        let progress = FakeProgress::default();
        service(&db, Arc::new(MutationLocks::unchecked()), true).scrape_items(vec![item.id.clone()], &progress).await.unwrap();
        let scraped = db.get_media_item(&item.id).unwrap().unwrap();
        assert_eq!(scraped.status, ScrapedStatus::Scraped);
        assert_eq!(scraped.title, "Matched title");
        // Auto-rename moved the video and its fetched poster together.
        assert_ne!(scraped.file_path, item.file_path);
        assert!(scraped.file_path.contains("Matched title"), "{}", scraped.file_path);
        assert!(!Path::new(&item.file_path).exists());
        let meta = db.fetch_metadata(&item.id).unwrap().unwrap();
        let poster = Path::new(&scraped.folder_path).join(meta.poster_path.unwrap());
        assert_eq!(std::fs::read(poster).unwrap(), b"image:poster");
        assert_eq!(progress.stages(), ["matching", "saveResults", "rename", "saveResults"]);
        assert_eq!(*progress.results.lock().unwrap(), [(1, 0, 0)]);
    }

    #[tokio::test]
    async fn scrape_library_classifies_outcomes_without_renaming() {
        let (_dir, db, lib, item) = movie_library(ScrapedStatus::Unscraped);
        let mut ids = vec![item.id.clone()];
        for title in ["Nobody", "Empty", "Broken"] {
            let other = MediaItem::new_movie(title, None, &lib.root_path, format!("{}/{title}.mkv", lib.root_path), lib.id.clone(), ScrapedStatus::Unscraped);
            ids.push(other.id.clone());
            db.insert_media_items(&[other]).unwrap();
        }
        let progress = FakeProgress::default();
        service(&db, Arc::new(MutationLocks::unchecked()), false).scrape_library(&lib.id, &progress).await.unwrap();
        let status = |id: &str| { let i = db.get_media_item(id).unwrap().unwrap(); (i.status, i.scrape_issue) };
        assert_eq!(status(&ids[0]).0, ScrapedStatus::Scraped);
        assert_eq!(status(&ids[1]), (ScrapedStatus::Unmatched, None));
        assert_eq!(status(&ids[2]), (ScrapedStatus::Unmatched, Some("noResults".into())));
        assert_eq!(status(&ids[3]), (ScrapedStatus::Partial, Some("err.connect".into())));
        assert_eq!(db.get_media_item(&ids[0]).unwrap().unwrap().file_path, item.file_path);
        assert_eq!(*progress.results.lock().unwrap(), [(1, 2, 1)]);
        let last = progress.last();
        assert_eq!((last.completed, last.total, last.stage_key.as_deref()), (4, 4, Some("saveResults")));
    }

    #[tokio::test]
    async fn network_phase_runs_while_the_library_is_locked() {
        let (_dir, db, lib, item) = movie_library(ScrapedStatus::Unscraped);
        let locks = Arc::new(MutationLocks::unchecked());
        let service = Arc::new(service(&db, Arc::clone(&locks), false));
        let guard = locks.lock(&LockScope::library(&lib.id)).await.unwrap();
        let job = {
            let (service, id) = (Arc::clone(&service), item.id.clone());
            tokio::spawn(async move { service.scrape_items(vec![id], &FakeProgress::default()).await })
        };
        service.source.fetched.notified().await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(service.source.matches.load(Ordering::SeqCst), 1);
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().status, ScrapedStatus::Unscraped);
        assert!(!job.is_finished());
        drop(guard);
        job.await.unwrap().unwrap();
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().status, ScrapedStatus::Scraped);
    }

    #[tokio::test]
    async fn write_phase_uses_the_folder_renamed_during_fetch() {
        let (dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        let moved = dir.path().canonicalize().unwrap().join("Moved");
        let mut service = service(&db, Arc::new(MutationLocks::unchecked()), false);
        let (hook_db, id, old_folder, target) = (Arc::clone(&db), item.id.clone(), item.folder_path.clone(), moved.clone());
        service.source = Arc::new(FakeSource {
            during_fetch: Some(Box::new(move || {
                std::fs::rename(&old_folder, &target).unwrap();
                let file = target.join("Original.mkv");
                hook_db.update_media_paths(&id, &target.to_string_lossy(), &file.to_string_lossy()).unwrap();
            })),
            ..Default::default()
        });
        service.scrape_items(vec![item.id.clone()], &FakeProgress::default()).await.unwrap();
        assert!(moved.join("Original.nfo").is_file());
        assert!(moved.join("Original-poster.jpg").is_file());
        // The old folder is not recreated by artwork or NFO writes.
        assert!(!Path::new(&item.folder_path).exists());
    }

    #[tokio::test]
    async fn result_fetched_before_cancel_is_not_written() {
        let (_dir, db, lib, item) = movie_library(ScrapedStatus::Unscraped);
        let locks = Arc::new(MutationLocks::unchecked());
        let service = Arc::new(service(&db, Arc::clone(&locks), false));
        let guard = locks.lock(&LockScope::library(&lib.id)).await.unwrap();
        let progress = Arc::new(FakeProgress::default());
        let job = {
            let (service, id, progress) = (Arc::clone(&service), item.id.clone(), Arc::clone(&progress));
            tokio::spawn(async move { service.scrape_items(vec![id], &*progress).await })
        };
        service.source.fetched.notified().await;
        progress.cancel.store(true, Ordering::SeqCst);
        drop(guard);
        assert_eq!(job.await.unwrap().unwrap_err(), "cancelled");
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().status, ScrapedStatus::Unscraped);
        assert!(!Path::new(&item.folder_path).join("Original.nfo").exists());
    }

    #[tokio::test]
    async fn cancelled_library_does_not_start_work() {
        let (_dir, db, lib, _item) = movie_library(ScrapedStatus::Unscraped);
        let service = service(&db, Arc::new(MutationLocks::unchecked()), false);
        let progress = FakeProgress::default();
        progress.cancel.store(true, Ordering::SeqCst);
        assert_eq!(service.scrape_library(&lib.id, &progress).await.unwrap_err(), "cancelled");
        assert!(progress.updates.lock().unwrap().is_empty(), "cancelled task must not publish progress");
        assert_eq!(service.source.matches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn pending_recovery_stops_the_write_phase() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        let locks = Arc::new(MutationLocks::new(|| Err("recovery pending".into()), |ids| ids.to_vec()));
        let err = service(&db, locks, false).scrape_items(vec![item.id.clone()], &FakeProgress::default()).await.unwrap_err();
        assert_eq!(err, "recovery pending");
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().status, ScrapedStatus::Unscraped);
    }

    #[tokio::test]
    async fn manual_match_persists_and_reports_like_before() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unmatched);
        let progress = FakeProgress::default();
        service(&db, Arc::new(MutationLocks::unchecked()), false).apply_manual_match(&item, "tmdb:1", &progress).await.unwrap();
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().status, ScrapedStatus::Scraped);
        assert_eq!(progress.stages(), ["matching", "saveResults"]);
    }

    #[test]
    fn summary_text_round_trips() {
        let summary = ScrapeSummary { success_ids: vec!["a".into()], unmatched: 2, failed: 3 };
        assert_eq!(ScrapeSummary::parse_result(&summary.format_result()), Some((1, 2, 3)));
    }
}
