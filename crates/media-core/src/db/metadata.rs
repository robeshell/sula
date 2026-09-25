use rusqlite::params;

use crate::models::MediaMetadata;
use crate::DatabaseError;

use super::AppDatabase;

impl AppDatabase {
    pub fn upsert_metadata(&self, metadata: &MediaMetadata) -> Result<(), DatabaseError> {
        let genres = serde_json::to_string(&metadata.genres).unwrap_or_else(|_| "[]".into());
        let tags = serde_json::to_string(&metadata.tags).unwrap_or_else(|_| "[]".into());
        let credits = serde_json::to_string(&metadata.credits).unwrap_or_else(|_| "[]".into());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO media_metadata (
                    mediaItemId, overview, outline, tagline, genres, tags, rating, ratingVotes,
                    contentRating, director, writer, credits, studio, country, language,
                    premiered, endDate, runtime, showStatus, collectionName, collectionId,
                    sourceId, imdbId, tmdbId, tvdbId, bangumiId,
                    posterPath, fanartPath, bannerPath, logoPath, thumbPath,
                    videoCodec, videoResolution, audioCodec, audioChannels, trailer, scrapedAt
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18, ?19, ?20, ?21,
                    ?22, ?23, ?24, ?25, ?26,
                    ?27, ?28, ?29, ?30, ?31,
                    ?32, ?33, ?34, ?35, ?36, ?37
                 )
                 ON CONFLICT(mediaItemId) DO UPDATE SET
                    overview=excluded.overview, outline=excluded.outline, tagline=excluded.tagline,
                    genres=excluded.genres, tags=excluded.tags, rating=excluded.rating,
                    ratingVotes=excluded.ratingVotes, contentRating=excluded.contentRating,
                    director=excluded.director, writer=excluded.writer, credits=excluded.credits,
                    studio=excluded.studio, country=excluded.country, language=excluded.language,
                    premiered=excluded.premiered, endDate=excluded.endDate, runtime=excluded.runtime,
                    showStatus=excluded.showStatus, collectionName=excluded.collectionName,
                    collectionId=excluded.collectionId, sourceId=excluded.sourceId,
                    imdbId=excluded.imdbId, tmdbId=excluded.tmdbId, tvdbId=excluded.tvdbId,
                    bangumiId=excluded.bangumiId, posterPath=excluded.posterPath,
                    fanartPath=excluded.fanartPath, bannerPath=excluded.bannerPath,
                    logoPath=excluded.logoPath, thumbPath=excluded.thumbPath,
                    videoCodec=excluded.videoCodec, videoResolution=excluded.videoResolution,
                    audioCodec=excluded.audioCodec, audioChannels=excluded.audioChannels,
                    trailer=excluded.trailer, scrapedAt=excluded.scrapedAt",
                params![
                    metadata.media_item_id,
                    metadata.overview,
                    metadata.outline,
                    metadata.tagline,
                    genres,
                    tags,
                    metadata.rating,
                    metadata.rating_votes,
                    metadata.content_rating,
                    metadata.director,
                    metadata.writer,
                    credits,
                    metadata.studio,
                    metadata.country,
                    metadata.language,
                    metadata.premiered,
                    metadata.end_date,
                    metadata.runtime,
                    metadata.show_status,
                    metadata.collection_name,
                    metadata.collection_id,
                    metadata.source_id,
                    metadata.imdb_id,
                    metadata.tmdb_id,
                    metadata.tvdb_id,
                    metadata.bangumi_id,
                    metadata.poster_path,
                    metadata.fanart_path,
                    metadata.banner_path,
                    metadata.logo_path,
                    metadata.thumb_path,
                    metadata.video_codec,
                    metadata.video_resolution,
                    metadata.audio_codec,
                    metadata.audio_channels,
                    metadata.trailer,
                    metadata.scraped_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn fetch_metadata(
        &self,
        media_item_id: &str,
    ) -> Result<Option<MediaMetadata>, DatabaseError> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT mediaItemId, overview, outline, tagline, genres, tags, rating, ratingVotes,
                        contentRating, director, writer, credits, studio, country, language,
                        premiered, endDate, runtime, showStatus, collectionName, collectionId,
                        sourceId, imdbId, tmdbId, tvdbId, bangumiId,
                        posterPath, fanartPath, bannerPath, logoPath, thumbPath,
                        videoCodec, videoResolution, audioCodec, audioChannels, trailer, scrapedAt
                 FROM media_metadata WHERE mediaItemId = ?1",
            )?;
            let mut rows = stmt.query([media_item_id])?;
            if let Some(row) = rows.next()? {
                Ok(Some(map_metadata(row)?))
            } else {
                Ok(None)
            }
        })
    }
}

fn map_metadata(row: &rusqlite::Row<'_>) -> Result<MediaMetadata, rusqlite::Error> {
    let genres_json: String = row.get(4)?;
    let tags_json: String = row.get(5)?;
    let credits_json: String = row.get(11)?;
    let scraped_at: String = row.get(36)?;
    Ok(MediaMetadata {
        media_item_id: row.get(0)?,
        overview: row.get(1)?,
        outline: row.get(2)?,
        tagline: row.get(3)?,
        genres: serde_json::from_str(&genres_json).unwrap_or_default(),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        rating: row.get(6)?,
        rating_votes: row.get(7)?,
        content_rating: row.get(8)?,
        director: row.get(9)?,
        writer: row.get(10)?,
        credits: serde_json::from_str(&credits_json).unwrap_or_default(),
        studio: row.get(12)?,
        country: row.get(13)?,
        language: row.get(14)?,
        premiered: row.get(15)?,
        end_date: row.get(16)?,
        runtime: row.get(17)?,
        show_status: row.get(18)?,
        collection_name: row.get(19)?,
        collection_id: row.get(20)?,
        source_id: row.get(21)?,
        imdb_id: row.get(22)?,
        tmdb_id: row.get(23)?,
        tvdb_id: row.get(24)?,
        bangumi_id: row.get(25)?,
        poster_path: row.get(26)?,
        fanart_path: row.get(27)?,
        banner_path: row.get(28)?,
        logo_path: row.get(29)?,
        thumb_path: row.get(30)?,
        video_codec: row.get(31)?,
        video_resolution: row.get(32)?,
        audio_codec: row.get(33)?,
        audio_channels: row.get(34)?,
        trailer: row.get(35)?,
        scraped_at: chrono::DateTime::parse_from_rfc3339(&scraped_at)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
    })
}
