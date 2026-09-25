//! Read-only views of libraries and their media for the UI.

use std::sync::Arc;

use media_core::{MediaItem, MediaMetaSummary, MediaMetadata, MediaType, ShowListStats, TvEpisode, TvSeason};
use serde::Serialize;

use crate::error::{blocking, failed, CoreError, CoreResult};
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
    pub async fn status(&self) -> CoreResult<AppStatusDto> {
        let library_count = self.db.library_count().map_err(failed)?;
        let config = self.config().await;
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

    pub async fn media_items(&self, library_id: String) -> CoreResult<Vec<MediaItem>> {
        let db = Arc::clone(&self.db);
        blocking(move || db.list_media_items(&library_id).map_err(failed)).await
    }

    /// One page of a library with the metadata and show stats its rows need.
    pub async fn media_page(&self, library_id: String, offset: Option<u32>, limit: Option<u32>) -> CoreResult<MediaListPayload> {
        let db = Arc::clone(&self.db);
        blocking(move || {
            let offset = offset.unwrap_or(0);
            let limit = limit.unwrap_or(256).clamp(1, 512);
            let items = db.list_media_items_page(&library_id, offset, limit).map_err(failed)?;
            let ids = serde_json::to_string(&items.iter().map(|i| &i.id).collect::<Vec<_>>()).map_err(failed)?;
            let metadata = db.list_metadata_summaries_for_ids(&ids).map_err(failed)?;
            let show_stats = db.list_show_stats_for_ids(&ids).map_err(failed)?;
            let next_offset = if items.len() == limit as usize { offset.checked_add(limit) } else { None };
            Ok(MediaListPayload { items, metadata, show_stats, next_offset })
        })
        .await
    }

    /// An item with its metadata and, for shows, every season and episode.
    pub async fn media_detail(&self, id: String) -> CoreResult<MediaDetailDto> {
        let db = Arc::clone(&self.db);
        blocking(move || {
            let item = db
                .get_media_item(&id)
                .map_err(failed)?
                .ok_or_else(|| CoreError::not_found("media item", &id))?;
            let metadata = db.fetch_metadata(&id).map_err(failed)?;
            let (seasons, episodes) = if matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
                let seasons = db.fetch_seasons(&id).map_err(failed)?;
                let mut episodes = Vec::new();
                for season in &seasons {
                    episodes.extend(db.fetch_episodes(&season.id).map_err(failed)?);
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
