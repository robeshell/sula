use rusqlite::{params, OptionalExtension};

use crate::models::{CastMember, TvEpisode, TvSeason};
use crate::scanner::ScannedEpisode;
use crate::DatabaseError;

use super::AppDatabase;

impl AppDatabase {
    pub fn list_episode_file_paths_for_library(
        &self,
        library_id: &str,
    ) -> Result<Vec<String>, DatabaseError> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT e.filePath
                 FROM tv_episodes e
                 JOIN tv_seasons s ON s.id = e.seasonId
                 JOIN media_items m ON m.id = s.mediaItemId
                 WHERE m.libraryId = ?1",
            )?;
            let rows = stmt.query_map([library_id], |row| row.get(0))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    /// Insert seasons/episodes for newly scanned shows in one transaction.
    pub fn insert_show_episodes(
        &self,
        media_item_id: &str,
        episodes: &[ScannedEpisode],
    ) -> Result<(), DatabaseError> {
        if episodes.is_empty() {
            return Ok(());
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            insert_scanned_episodes(&tx, media_item_id, episodes)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Commit one completed filesystem observation without resetting scraped metadata.
    pub fn reconcile_show_episodes(
        &self,
        media_item_id: &str,
        removed: &[String],
        updated: &[(String, String)],
        added: &[ScannedEpisode],
    ) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            for id in removed {
                tx.execute("DELETE FROM tv_episodes WHERE id=?1 AND seasonId IN (SELECT id FROM tv_seasons WHERE mediaItemId=?2)", params![id, media_item_id])?;
            }
            for (id, path) in updated {
                tx.execute("UPDATE tv_episodes SET filePath=?1 WHERE id=?2 AND seasonId IN (SELECT id FROM tv_seasons WHERE mediaItemId=?3)", params![path, id, media_item_id])?;
            }
            insert_scanned_episodes(&tx, media_item_id, added)?;
            tx.execute("UPDATE tv_seasons SET episodeCount=(SELECT COUNT(*) FROM tv_episodes WHERE seasonId=tv_seasons.id) WHERE mediaItemId=?1", [media_item_id])?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Transfer a show's episode records and remove its source in one commit.
    /// Episode IDs and all scraped columns remain intact.
    pub fn merge_show_records(
        &self,
        source: &str,
        target: &str,
        moved: &[(String, String)],
        files: &[(std::path::PathBuf, std::path::PathBuf)],
        operation_id: &str,
    ) -> Result<(), DatabaseError> {
        if source == target {
            return Err(DatabaseError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "cannot merge a show into itself",
            )));
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let same_library: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM media_items a JOIN media_items b ON a.libraryId=b.libraryId WHERE a.id=?1 AND b.id=?2)", params![source, target], |r| r.get(0))?;
            if !same_library { return Err(DatabaseError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "merge requires two existing items in the same library"))); }
            let source_root: String = tx.query_row("SELECT folderPath FROM media_items WHERE id=?1", [source], |r| r.get(0))?;
            let remap = |path: String| {
                if path.is_empty() { return path; }
                let original = std::path::Path::new(&source_root).join(&path);
                files.iter().find(|(from, _)| *from == original).map(|(_, to)| to.clone()).unwrap_or(original).to_string_lossy().into_owned()
            };
            let mut assets = tx.prepare("SELECT e.id, e.stillPath FROM tv_episodes e JOIN tv_seasons s ON s.id=e.seasonId WHERE s.mediaItemId=?1 AND e.stillPath IS NOT NULL")?;
            let stills = assets.query_map([source], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            drop(assets);
            for (id, path) in stills { tx.execute("UPDATE tv_episodes SET stillPath=?1 WHERE id=?2", params![remap(path), id])?; }
            let mut stmt = tx.prepare("SELECT id, seasonNumber FROM tv_seasons WHERE mediaItemId=?1")?;
            let seasons = stmt.query_map([source], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i32>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            for (id, path) in moved {
                tx.execute("UPDATE tv_episodes SET filePath=?1 WHERE id=?2 AND seasonId IN (SELECT id FROM tv_seasons WHERE mediaItemId=?3)", params![path, id, source])?;
            }
            for (source_season, number) in seasons {
                let target_season = tx.query_row("SELECT id FROM tv_seasons WHERE mediaItemId=?1 AND seasonNumber=?2", params![target, number], |r| r.get::<_, String>(0)).optional()?;
                let target_season = if let Some(id) = target_season { id } else {
                    let id = format!("{target}_S{number}");
                    let poster: Option<String> = tx.query_row("SELECT posterPath FROM tv_seasons WHERE id=?1", [&source_season], |r| r.get(0))?;
                    let poster = poster.map(&remap);
                    tx.execute("INSERT INTO tv_seasons (id, mediaItemId, seasonNumber, title, overview, posterPath, airDate, episodeCount) SELECT ?1, ?2, seasonNumber, title, overview, ?4, airDate, episodeCount FROM tv_seasons WHERE id=?3", params![id, target, source_season, poster])?;
                    id
                };
                let conflict: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM tv_episodes a JOIN tv_episodes b ON a.episodeNumber=b.episodeNumber WHERE a.seasonId=?1 AND b.seasonId=?2)", params![source_season, target_season], |r| r.get(0))?;
                if conflict { return Err(DatabaseError::Io(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "duplicate episode during merge"))); }
                tx.execute("UPDATE tv_episodes SET seasonId=?1 WHERE seasonId=?2", params![target_season, source_season])?;
                tx.execute("UPDATE tv_seasons SET episodeCount=(SELECT COUNT(*) FROM tv_episodes WHERE seasonId=?1) WHERE id=?1", [&target_season])?;
            }
            tx.execute("DELETE FROM media_items WHERE id=?1", [source])?;
            if tx.execute("UPDATE media_operation_journal SET committed=1 WHERE id=?1 AND committed=0", [operation_id])? != 1 {
                return Err(DatabaseError::Io(std::io::Error::other("merge journal missing")));
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn fetch_seasons(&self, media_item_id: &str) -> Result<Vec<TvSeason>, DatabaseError> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, mediaItemId, seasonNumber, title, overview, posterPath, airDate, episodeCount
                 FROM tv_seasons
                 WHERE mediaItemId = ?1
                 ORDER BY seasonNumber ASC",
            )?;
            let rows = stmt.query_map([media_item_id], map_season)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn fetch_episodes(&self, season_id: &str) -> Result<Vec<TvEpisode>, DatabaseError> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, seasonId, episodeNumber, title, overview, airDate, stillPath, stillURL,
                        filePath, runtime, rating, director, writer, guestCast, absoluteNumber,
                        finaleType, videoCodec, videoResolution, audioCodec
                 FROM tv_episodes
                 WHERE seasonId = ?1
                 ORDER BY episodeNumber ASC",
            )?;
            let rows = stmt.query_map([season_id], map_episode)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn update_episode_file_path(
        &self,
        episode_id: &str,
        file_path: &str,
    ) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE tv_episodes SET filePath = ?2 WHERE id = ?1",
                params![episode_id, file_path],
            )?;
            Ok(())
        })
    }

    pub fn update_episode_still_path(
        &self,
        episode_id: &str,
        still_path: &str,
    ) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE tv_episodes SET stillPath = ?2 WHERE id = ?1",
                params![episode_id, still_path],
            )?;
            Ok(())
        })
    }

    pub fn upsert_season(&self, season: &TvSeason) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tv_seasons (
                    id, mediaItemId, seasonNumber, title, overview, posterPath, airDate, episodeCount
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    mediaItemId=excluded.mediaItemId,
                    seasonNumber=excluded.seasonNumber,
                    title=excluded.title,
                    overview=excluded.overview,
                    posterPath=excluded.posterPath,
                    airDate=excluded.airDate,
                    episodeCount=excluded.episodeCount",
                params![
                    season.id,
                    season.media_item_id,
                    season.season_number,
                    season.title,
                    season.overview,
                    season.poster_path,
                    season.air_date,
                    season.episode_count,
                ],
            )?;
            Ok(())
        })
    }

    pub fn upsert_episode(&self, episode: &TvEpisode) -> Result<(), DatabaseError> {
        let guest_cast =
            serde_json::to_string(&episode.guest_cast).unwrap_or_else(|_| "[]".into());
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO tv_episodes (
                    id, seasonId, episodeNumber, title, overview, airDate, stillPath, stillURL,
                    filePath, runtime, rating, director, writer, guestCast, absoluteNumber,
                    finaleType, videoCodec, videoResolution, audioCodec
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                    ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18, ?19
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    seasonId=excluded.seasonId,
                    episodeNumber=excluded.episodeNumber,
                    title=excluded.title,
                    overview=excluded.overview,
                    airDate=excluded.airDate,
                    stillPath=excluded.stillPath,
                    stillURL=excluded.stillURL,
                    filePath=excluded.filePath,
                    runtime=excluded.runtime,
                    rating=excluded.rating,
                    director=excluded.director,
                    writer=excluded.writer,
                    guestCast=excluded.guestCast,
                    absoluteNumber=excluded.absoluteNumber,
                    finaleType=excluded.finaleType,
                    videoCodec=excluded.videoCodec,
                    videoResolution=excluded.videoResolution,
                    audioCodec=excluded.audioCodec",
                params![
                    episode.id,
                    episode.season_id,
                    episode.episode_number,
                    episode.title,
                    episode.overview,
                    episode.air_date,
                    episode.still_path,
                    episode.still_url,
                    episode.file_path,
                    episode.runtime,
                    episode.rating,
                    episode.director,
                    episode.writer,
                    guest_cast,
                    episode.absolute_number,
                    episode.finale_type,
                    episode.video_codec,
                    episode.video_resolution,
                    episode.audio_codec,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_episode(&self, episode_id: &str) -> Result<bool, DatabaseError> {
        self.with_conn(|conn| {
            let n = conn.execute("DELETE FROM tv_episodes WHERE id = ?1", params![episode_id])?;
            Ok(n > 0)
        })
    }

    #[allow(dead_code)]
    pub fn get_season(&self, id: &str) -> Result<Option<TvSeason>, DatabaseError> {
        self.with_read_conn(|conn| {
            conn.query_row(
                "SELECT id, mediaItemId, seasonNumber, title, overview, posterPath, airDate, episodeCount
                 FROM tv_seasons WHERE id = ?1",
                [id],
                map_season,
            )
            .optional()
            .map_err(DatabaseError::from)
        })
    }
}

fn map_season(row: &rusqlite::Row<'_>) -> rusqlite::Result<TvSeason> {
    Ok(TvSeason {
        id: row.get(0)?,
        media_item_id: row.get(1)?,
        season_number: row.get(2)?,
        title: row.get(3)?,
        overview: row.get(4)?,
        poster_path: row.get(5)?,
        air_date: row.get(6)?,
        episode_count: row.get(7)?,
    })
}

fn map_episode(row: &rusqlite::Row<'_>) -> rusqlite::Result<TvEpisode> {
    let guest_cast_json: String = row.get(13)?;
    let guest_cast: Vec<CastMember> = serde_json::from_str(&guest_cast_json).unwrap_or_default();
    Ok(TvEpisode {
        id: row.get(0)?,
        season_id: row.get(1)?,
        episode_number: row.get(2)?,
        title: row.get(3)?,
        overview: row.get(4)?,
        air_date: row.get(5)?,
        still_path: row.get(6)?,
        still_url: row.get(7)?,
        file_path: row.get(8)?,
        runtime: row.get(9)?,
        rating: row.get(10)?,
        director: row.get(11)?,
        writer: row.get(12)?,
        guest_cast,
        absolute_number: row.get(14)?,
        finale_type: row.get(15)?,
        video_codec: row.get(16)?,
        video_resolution: row.get(17)?,
        audio_codec: row.get(18)?,
    })
}

fn insert_scanned_episodes(
    conn: &rusqlite::Connection,
    media_item_id: &str,
    episodes: &[ScannedEpisode],
) -> rusqlite::Result<()> {
    {
        let mut by_season: std::collections::HashMap<i32, Vec<&ScannedEpisode>> =
            std::collections::HashMap::new();
        for ep in episodes {
            by_season.entry(ep.season).or_default().push(ep);
        }

        let mut season_stmt = conn.prepare(
                    "INSERT INTO tv_seasons (
                        id, mediaItemId, seasonNumber, title, overview, posterPath, airDate, episodeCount
                     ) VALUES (?1, ?2, ?3, NULL, NULL, NULL, NULL, ?4)
                     ON CONFLICT(id) DO NOTHING",
                )?;
        let mut episode_stmt = conn.prepare(
            "INSERT INTO tv_episodes (
                        id, seasonId, episodeNumber, title, overview, airDate, stillPath, stillURL,
                        filePath, runtime, rating, director, writer, guestCast, absoluteNumber,
                        finaleType, videoCodec, videoResolution, audioCodec
                     ) VALUES (
                        ?1, ?2, ?3, ?4, NULL, NULL, NULL, NULL,
                        ?5, NULL, NULL, NULL, NULL, '[]', NULL,
                        NULL, NULL, NULL, NULL
                     ) ON CONFLICT(id) DO UPDATE SET filePath=excluded.filePath",
        )?;
        let mut episode_count_stmt = conn.prepare(
            "UPDATE tv_seasons
                     SET episodeCount = (
                         SELECT COUNT(*) FROM tv_episodes WHERE seasonId = ?1
                     )
                     WHERE id = ?1",
        )?;

        for (season_num, eps) in by_season {
            let season_id: String = conn
                .query_row(
                    "SELECT id FROM tv_seasons WHERE mediaItemId=?1 AND seasonNumber=?2",
                    params![media_item_id, season_num],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or_else(|| format!("{media_item_id}_S{season_num}"));
            season_stmt.execute(params![
                season_id,
                media_item_id,
                season_num,
                eps.len() as i64
            ])?;
            for ep in eps {
                let episode_id: String = conn
                    .query_row(
                        "SELECT id FROM tv_episodes WHERE seasonId=?1 AND episodeNumber=?2",
                        params![season_id, ep.episode],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or_else(|| format!("{season_id}_E{}", ep.episode));
                let title = if ep.title.is_empty() {
                    None
                } else {
                    Some(ep.title.as_str())
                };
                episode_stmt.execute(params![
                    episode_id,
                    season_id,
                    ep.episode,
                    title,
                    ep.file_path,
                ])?;
            }
            episode_count_stmt.execute([&season_id])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;
    use crate::{Library, MediaItem, MediaType, ScrapedStatus};

    fn fixture() -> (AppDatabase, String, String) {
        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("TV", "/library", MediaType::TvShow);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_show(MediaType::TvShow, "Show", None, "/library/show", library.id, ScrapedStatus::Scraped);
        db.insert_media_items(&[item.clone()]).unwrap();
        db.insert_show_episodes(&item.id, &[episode(1, "/old")]).unwrap();
        let season = db.fetch_seasons(&item.id).unwrap().remove(0);
        (db, item.id, season.id)
    }
    fn episode(number: i32, path: &str) -> ScannedEpisode {
        ScannedEpisode { season: 1, episode: number, file_path: path.into(), title: "Scanned".into() }
    }
    #[test]
    fn scan_upsert_keeps_scraped_metadata_and_custom_ids() {
        let (db, item, season) = fixture();
        let mut existing = db.fetch_episodes(&season).unwrap().remove(0);
        db.delete_episode(&existing.id).unwrap();
        existing.id = "provider-episode".into();
        existing.overview = Some("Keep overview".into());
        existing.title = Some("Keep title".into());
        db.upsert_episode(&existing).unwrap();
        db.insert_show_episodes(&item, &[episode(1, "/new")]).unwrap();
        let eps = db.fetch_episodes(&season).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].id, existing.id);
        assert_eq!(eps[0].overview, existing.overview);
        assert_eq!(eps[0].title, existing.title);
        assert_eq!(eps[0].file_path, "/new");
    }
    #[test]
    fn reconciliation_failure_rolls_back_deletes_and_path_updates() {
        let (db, item, season) = fixture();
        db.insert_show_episodes(&item, &[episode(2, "/two")]).unwrap();
        let eps = db.fetch_episodes(&season).unwrap();
        db.with_conn(|conn| { conn.execute_batch("CREATE TRIGGER fail_episode_insert BEFORE INSERT ON tv_episodes BEGIN SELECT RAISE(ABORT, 'injected'); END;")?; Ok(()) }).unwrap();
        assert!(db.reconcile_show_episodes(&item, &[eps[0].id.clone()], &[(eps[1].id.clone(), "/changed".into())], &[episode(3, "/three")]).is_err());
        let after = db.fetch_episodes(&season).unwrap();
        assert_eq!(after.len(), 2);
        assert_eq!(after[0].file_path, "/old");
        assert_eq!(after[1].file_path, "/two");
    }
}
