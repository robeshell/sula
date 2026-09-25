use media_core::{MediaItem, MediaType};
use reqwest::Client;

use crate::coordinator::{MatchOutcome, ScraperCoordinator, ScraperKeys};
use crate::tmdb::TmdbScraper;
use crate::types::{ScrapedMetadata, ScrapedSeason};

#[derive(Debug, Clone)]
pub struct ScrapeOptions {
    pub language: String,
    pub concurrency: usize,
    pub keys: ScraperKeys,
}

impl Default for ScrapeOptions {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(),
            concurrency: 4,
            keys: ScraperKeys::default(),
        }
    }
}

/// Network-only scrape client: matching, detail fetches and image bytes. The
/// caller owns persistence (database rows, NFO, artwork files) and decides where
/// fetched bytes land, so this crate never writes into a library folder.
#[derive(Clone)]
pub struct ScrapeClient {
    coordinator: ScraperCoordinator,
    tmdb: TmdbScraper,
    client: Client,
    language: String,
}

impl ScrapeClient {
    pub fn new(options: &ScrapeOptions) -> Self {
        let client = crate::http::build_client();
        Self {
            coordinator: ScraperCoordinator::new(options.keys.clone()),
            tmdb: TmdbScraper::new(client.clone(), options.keys.tmdb.clone()),
            client,
            language: options.language.clone(),
        }
    }

    pub async fn match_item(&self, item: &MediaItem) -> MatchOutcome {
        self.coordinator.match_item(item, &self.language).await
    }

    pub async fn fetch_by_source(&self, source_id: &str, media_type: MediaType) -> Result<ScrapedMetadata, String> {
        self.coordinator
            .fetch_by_source(source_id, media_type, &self.language)
            .await
            .map_err(|e| crate::http::humanize_error(&e))
    }

    /// One TV season from TMDB (title / overview / poster / episodes).
    pub async fn fetch_season(&self, tmdb_id: &str, season_number: i32) -> Result<ScrapedSeason, String> {
        if !self.tmdb.is_configured() {
            return Err("err.apiKey".into());
        }
        self.tmdb
            .fetch_season(tmdb_id, season_number, &self.language)
            .await
            .map_err(|e| crate::http::humanize_error(&e))
    }

    pub async fn fetch_image(&self, url: &str) -> Result<Vec<u8>, String> {
        crate::artwork::fetch_image(&self.client, url).await
    }
}
