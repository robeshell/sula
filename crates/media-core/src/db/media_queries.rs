use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use crate::models::{MediaItem, MediaType, ScrapedStatus};
use crate::DatabaseError;

use super::AppDatabase;

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetaSummary {
    pub media_item_id: String,
    pub poster_path: Option<String>,
    pub fanart_path: Option<String>,
    pub overview: Option<String>,
    pub rating: Option<f64>,
    pub genres: Vec<String>,
    /// Changes on every (re)scrape, even when an artwork file is overwritten in place.
    pub scraped_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowListStats {
    pub media_item_id: String,
    pub season_count: u32,
    pub episode_count: u32,
    pub local_episode_count: u32,
}

impl AppDatabase {
    pub fn list_metadata_summaries(
        &self,
        library_id: &str,
    ) -> Result<Vec<MediaMetaSummary>, DatabaseError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.mediaItemId, m.posterPath, m.fanartPath, m.overview, m.rating, m.genres, m.scrapedAt
                 FROM media_metadata m
                 INNER JOIN media_items i ON i.id = m.mediaItemId
                 WHERE i.libraryId = ?1",
            )?;
            let rows = stmt.query_map(params![library_id], |row| {
                let genres_json: String = row.get(5)?;
                Ok(MediaMetaSummary {
                    media_item_id: row.get(0)?,
                    poster_path: row.get(1)?,
                    fanart_path: row.get(2)?,
                    overview: row.get(3)?,
                    rating: row.get(4)?,
                    genres: serde_json::from_str(&genres_json).unwrap_or_default(),
                    scraped_at: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Season/episode counts for TV & anime rows in a library (one grouped query).
    pub fn list_show_stats(&self, library_id: &str) -> Result<Vec<ShowListStats>, DatabaseError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.mediaItemId,
                        COUNT(DISTINCT s.id),
                        COUNT(e.id),
                        SUM(CASE WHEN e.filePath IS NOT NULL AND e.filePath != '' THEN 1 ELSE 0 END)
                 FROM tv_seasons s
                 INNER JOIN media_items i ON i.id = s.mediaItemId
                 LEFT JOIN tv_episodes e ON e.seasonId = s.id
                 WHERE i.libraryId = ?1
                 GROUP BY s.mediaItemId",
            )?;
            let rows = stmt.query_map(params![library_id], |row| {
                let local: i64 = row.get::<_, Option<i64>>(3)?.unwrap_or(0);
                Ok(ShowListStats {
                    media_item_id: row.get(0)?,
                    season_count: row.get::<_, i64>(1)? as u32,
                    episode_count: row.get::<_, i64>(2)? as u32,
                    local_episode_count: local.max(0) as u32,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn list_metadata_summaries_for_ids(
        &self,
        ids_json: &str,
    ) -> Result<Vec<MediaMetaSummary>, DatabaseError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.mediaItemId, m.posterPath, m.fanartPath, m.overview, m.rating, m.genres, m.scrapedAt
                 FROM media_metadata m
                 INNER JOIN media_items i ON i.id = m.mediaItemId
                 WHERE i.id IN (SELECT value FROM json_each(?1))",
            )?;
            let rows = stmt.query_map(params![ids_json], |row| {
                let genres_json: String = row.get(5)?;
                Ok(MediaMetaSummary {
                    media_item_id: row.get(0)?,
                    poster_path: row.get(1)?,
                    fanart_path: row.get(2)?,
                    overview: row.get(3)?,
                    rating: row.get(4)?,
                    genres: serde_json::from_str(&genres_json).unwrap_or_default(),
                    scraped_at: row.get(6)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Season/episode counts for TV & anime rows in a library (one grouped query).
    pub fn list_show_stats_for_ids(&self, ids_json: &str) -> Result<Vec<ShowListStats>, DatabaseError> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT s.mediaItemId,
                        COUNT(DISTINCT s.id),
                        COUNT(e.id),
                        SUM(CASE WHEN e.filePath IS NOT NULL AND e.filePath != '' THEN 1 ELSE 0 END)
                 FROM tv_seasons s
                 INNER JOIN media_items i ON i.id = s.mediaItemId
                 LEFT JOIN tv_episodes e ON e.seasonId = s.id
                 WHERE i.id IN (SELECT value FROM json_each(?1))
                 GROUP BY s.mediaItemId",
            )?;
            let rows = stmt.query_map(params![ids_json], |row| {
                let local: i64 = row.get::<_, Option<i64>>(3)?.unwrap_or(0);
                Ok(ShowListStats {
                    media_item_id: row.get(0)?,
                    season_count: row.get::<_, i64>(1)? as u32,
                    episode_count: row.get::<_, i64>(2)? as u32,
                    local_episode_count: local.max(0) as u32,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn get_media_item(&self, id: &str) -> Result<Option<MediaItem>, DatabaseError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, type, title, originalTitle, year, folderPath, filePath,
                        bookmarkData, status, scrapeIssue, libraryId, addedAt
                 FROM media_items WHERE id = ?1",
                [id],
                |row| {
                    let media_type: String = row.get(1)?;
                    let status: String = row.get(8)?;
                    let added_at: String = row.get(11)?;
                    Ok(MediaItem {
                        id: row.get(0)?,
                        media_type: MediaType::parse(&media_type).unwrap_or(MediaType::Movie),
                        title: row.get(2)?,
                        original_title: row.get(3)?,
                        year: row.get(4)?,
                        folder_path: row.get(5)?,
                        file_path: row.get(6)?,
                        bookmark_data: row.get(7)?,
                        status: ScrapedStatus::parse(&status).unwrap_or(ScrapedStatus::Unscraped),
                        scrape_issue: row.get(9)?,
                        library_id: row.get(10)?,
                        added_at: DateTime::parse_from_rfc3339(&added_at)
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                    })
                },
            )
            .optional()
            .map_err(DatabaseError::from)
        })
    }
}
