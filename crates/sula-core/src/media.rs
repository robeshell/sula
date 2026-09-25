//! Read-only views of libraries and their media for the UI.

use std::sync::Arc;

use media_core::{MediaItem, MediaMetaSummary, MediaMetadata, MediaType, ShowListStats, TvEpisode, TvSeason};
use serde::Serialize;

use crate::app::{blocking, err_string};
use crate::state::{AppState, AppStatusDto, CratesDto};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaListPayload {
    pub next_offset: Option<u32>,
    pub items: Vec<MediaItem>,
    pub metadata: Vec<MediaMetaSummary>,
    pub show_stats: Vec<ShowListStats>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDetailDto {
    pub item: MediaItem,
    pub metadata: Option<MediaMetadata>,
    pub seasons: Vec<TvSeason>,
    pub episodes: Vec<TvEpisode>,
}

impl AppState {
    pub async fn status(&self) -> Result<AppStatusDto, String> {
        let library_count = self.db.library_count().map_err(err_string)?;
        let config = self.config.lock().await.config.clone();
        Ok(AppStatusDto {
            app_name: "Sula".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            data_dir: self.data_dir.display().to_string(),
            database_path: self.db.path().display().to_string(),
            library_count,
            config,
            crates: CratesDto {
                media_core: "media-core".into(),
                scraper_kit: scraper_kit::crate_name().into(),
                renamer: renamer::crate_name().into(),
            },
        })
    }

    pub async fn media_items(&self, library_id: String) -> Result<Vec<MediaItem>, String> {
        let db = Arc::clone(&self.db);
        blocking(move || db.list_media_items(&library_id).map_err(err_string)).await
    }

    /// One page of a library with the metadata and show stats its rows need.
    pub async fn media_page(&self, library_id: String, offset: Option<u32>, limit: Option<u32>) -> Result<MediaListPayload, String> {
        let db = Arc::clone(&self.db);
        blocking(move || {
            let offset = offset.unwrap_or(0);
            let limit = limit.unwrap_or(256).clamp(1, 512);
            let items = db.list_media_items_page(&library_id, offset, limit).map_err(err_string)?;
            let ids = serde_json::to_string(&items.iter().map(|i| &i.id).collect::<Vec<_>>()).map_err(err_string)?;
            let metadata = db.list_metadata_summaries_for_ids(&ids).map_err(err_string)?;
            let show_stats = db.list_show_stats_for_ids(&ids).map_err(err_string)?;
            let next_offset = if items.len() == limit as usize { offset.checked_add(limit) } else { None };
            Ok(MediaListPayload { items, metadata, show_stats, next_offset })
        })
        .await
    }

    /// An item with its metadata and, for shows, every season and episode.
    pub async fn media_detail(&self, id: String) -> Result<MediaDetailDto, String> {
        let db = Arc::clone(&self.db);
        blocking(move || {
            let item = db
                .get_media_item(&id)
                .map_err(err_string)?
                .ok_or_else(|| format!("media item not found: {id}"))?;
            let metadata = db.fetch_metadata(&id).map_err(err_string)?;
            let (seasons, episodes) = if matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
                let seasons = db.fetch_seasons(&id).map_err(err_string)?;
                let mut episodes = Vec::new();
                for season in &seasons {
                    episodes.extend(db.fetch_episodes(&season.id).map_err(err_string)?);
                }
                (seasons, episodes)
            } else {
                (Vec::new(), Vec::new())
            };
            Ok(MediaDetailDto { item, metadata, seasons, episodes })
        })
        .await
    }
}
