//! Writes fetched scrape results: database rows, NFO and artwork files. Runs on a
//! blocking thread under the item's library lock; the item passed in is re-read
//! after the lock was taken, so its folder reflects any rename during the fetch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use media_core::{AppDatabase, MediaItem, MediaMetadata, MediaType, ScrapedStatus, TvEpisode, TvSeason};
use scraper_kit::{ScrapedMetadata, ScrapedSeason};

pub(crate) type Image = Result<Vec<u8>, String>;

/// Image bytes fetched before the lock. Paths are chosen here, at write time.
#[derive(Debug, Default)]
pub struct ScrapeImages {
    /// `poster` / `fanart` / `banner`.
    pub artwork: Vec<(&'static str, Image)>,
    pub season_posters: HashMap<i32, Image>,
    /// Keyed by (season, episode); the URL guards against a changed still.
    pub stills: HashMap<(i32, i32), (String, Image)>,
}

pub fn season_poster_name(season: i32) -> String {
    format!("season{season}-poster.jpg")
}

fn write_image(folder: &Path, file_name: &str, image: &Image) -> Result<PathBuf, String> {
    let bytes = image.as_ref().map_err(Clone::clone)?;
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let path = folder.join(file_name);
    media_core::FilesystemService::new().write_file(bytes, &path, media_core::WriteOptions {
        collision_policy: media_core::CollisionPolicy::Replace,
        ..Default::default()
    }).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn persist_match(
    db: &AppDatabase,
    item: &MediaItem,
    scraped: &ScrapedMetadata,
    images: &ScrapeImages,
    nfo_format: &str,
) -> Result<(), String> {
    let folder = Path::new(&item.folder_path);
    let movie_stem = if item.media_type == MediaType::Movie {
        Some(Path::new(&item.file_path).file_stem().and_then(|v| v.to_str())
            .ok_or_else(|| "movie file path is missing".to_string())?)
    } else {
        if !media_core::media_files::owns_folder(db, item).map_err(|e| e.to_string())? {
            return Err("cannot write metadata into a shared show folder".into());
        }
        None
    };
    let mut issues = Vec::new();
    let (mut poster_path, mut fanart_path, mut banner_path) = (None, None, None);
    for (role, image) in &images.artwork {
        let name = movie_stem.map(|stem| format!("{stem}-{role}.jpg")).unwrap_or_else(|| format!("{role}.jpg"));
        match write_image(folder, &name, image) {
            Ok(_) => match *role { "poster" => poster_path = Some(name), "fanart" => fanart_path = Some(name), _ => banner_path = Some(name) },
            Err(error) => issues.push(format!("{role}: {error}")),
        }
    }
    issues.extend(scraped.issues.iter().cloned());

    let metadata = MediaMetadata {
        media_item_id: item.id.clone(),
        overview: scraped.overview.clone(),
        outline: None,
        tagline: scraped.tagline.clone(),
        genres: scraped.genres.clone(),
        tags: scraped.tags.clone(),
        rating: scraped.rating,
        rating_votes: scraped.rating_votes,
        content_rating: scraped.content_rating.clone(),
        director: scraped.director.clone(),
        writer: scraped.writer.clone(),
        credits: scraped.credits.clone(),
        studio: scraped.studio.clone(),
        country: scraped.country.clone(),
        language: scraped.language.clone(),
        premiered: scraped.premiered.clone(),
        end_date: scraped.end_date.clone(),
        runtime: scraped.runtime,
        show_status: scraped.show_status.clone(),
        collection_name: scraped.collection_name.clone(),
        collection_id: scraped.collection_id.clone(),
        source_id: scraped.source_id.clone(),
        imdb_id: scraped.imdb_id.clone(),
        tmdb_id: scraped.tmdb_id.clone(),
        tvdb_id: scraped.tvdb_id.clone(),
        bangumi_id: scraped.bangumi_id.clone(),
        poster_path,
        fanart_path,
        banner_path,
        logo_path: None,
        thumb_path: None,
        video_codec: None,
        video_resolution: None,
        audio_codec: None,
        audio_channels: None,
        trailer: scraped.trailer.clone(),
        scraped_at: Utc::now(),
    };
    db.upsert_metadata(&metadata).map_err(|e| e.to_string())?;
    db.update_title(&item.id, &scraped.title, scraped.original_title.as_deref()).map_err(|e| e.to_string())?;
    if let Some(year) = scraped.year {
        db.update_year(&item.id, Some(year)).map_err(|e| e.to_string())?;
    }
    if matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
        if let Err(error) = merge_seasons(db, item, &scraped.seasons, images) { issues.push(error); }
    }
    let updated = db.get_media_item(&item.id).map_err(|e| e.to_string())?
        .ok_or_else(|| "media item removed during scrape".to_string())?;
    if let Err(error) = media_core::nfo::write_nfo(&updated, &metadata, nfo_format) {
        issues.push(format!("NFO: {error}"));
    }
    if !issues.is_empty() {
        let issue = scraper_kit::redact_secrets(&issues.join("; "));
        db.update_status(&item.id, ScrapedStatus::Partial, Some(&issue)).map_err(|e| e.to_string())?;
        return Err(issue);
    }
    db.update_status(&item.id, ScrapedStatus::Scraped, None).map_err(|e| e.to_string())?;
    Ok(())
}

/// Season refresh of an already scraped show; problems leave it `Partial`.
pub fn persist_season(db: &AppDatabase, item: &MediaItem, season: &ScrapedSeason, images: &ScrapeImages) -> Result<(), String> {
    if !media_core::media_files::owns_folder(db, item).map_err(|e| e.to_string())? {
        return Err("cannot write metadata into a shared show folder".into());
    }
    let result = merge_seasons(db, item, std::slice::from_ref(season), images).map_err(|e| scraper_kit::redact_secrets(&e));
    if let Err(ref issue) = result { db.update_status(&item.id, ScrapedStatus::Partial, Some(issue)).map_err(|e| e.to_string())?; }
    result
}

fn merge_seasons(
    db: &AppDatabase,
    item: &MediaItem,
    scraped_seasons: &[ScrapedSeason],
    images: &ScrapeImages,
) -> Result<(), String> {
    let folder = Path::new(&item.folder_path);
    let mut issues = Vec::new();
    let existing_seasons = db.fetch_seasons(&item.id).map_err(|e| e.to_string())?;
    for scraped_season in scraped_seasons {
        let number = scraped_season.season_number;
        let season_id = existing_seasons
            .iter()
            .find(|s| s.season_number == number)
            .map(|s| s.id.clone())
            .unwrap_or_else(|| format!("{}_S{}", item.id, number));
        let mut poster = existing_seasons.iter().find(|s| s.season_number == number).and_then(|s| s.poster_path.clone());
        if let (Some(_), Some(image)) = (&scraped_season.poster_url, images.season_posters.get(&number)) {
            let name = season_poster_name(number);
            match write_image(folder, &name, image) {
                Ok(_) => poster = Some(name),
                Err(error) => issues.push(format!("season {number} poster: {error}")),
            }
        }
        let season = TvSeason {
            id: season_id.clone(),
            media_item_id: item.id.clone(),
            season_number: number,
            title: scraped_season.title.clone(),
            overview: scraped_season.overview.clone(),
            poster_path: poster,
            air_date: scraped_season.air_date.clone(),
            episode_count: scraped_season.episode_count,
        };
        db.upsert_season(&season).map_err(|e| e.to_string())?;
        let existing_eps = db.fetch_episodes(&season_id).map_err(|e| e.to_string())?;
        for scraped_ep in &scraped_season.episodes {
            let Some(existing) = existing_eps.iter().find(|e| e.episode_number == scraped_ep.episode_number) else { continue };
            let mut still_path = existing.still_path.clone();
            let mut still_url = scraped_ep.still_url.clone().or_else(|| existing.still_url.clone());
            // SCRAPE-17: episode still next to the media file.
            let fetched = images.stills.get(&(number, existing.episode_number)).filter(|(url, _)| scraped_ep.still_url.as_ref() == Some(url));
            if let (Some((url, image)), false) = (fetched, existing.file_path.is_empty()) {
                let ep_path = Path::new(&existing.file_path);
                if let Some(stem) = ep_path.file_stem().and_then(|s| s.to_str()) {
                    let dest_dir = ep_path.parent().unwrap_or(folder);
                    match write_image(dest_dir, &format!("{stem}-thumb.jpg"), image) {
                        Ok(abs) => { still_path = Some(relative_to_show(&abs, folder)); still_url = Some(url.clone()); },
                        Err(error) => issues.push(format!("episode {} still: {error}", existing.episode_number)),
                    }
                }
            }
            let updated = TvEpisode {
                title: scraped_ep.title.clone().or_else(|| existing.title.clone()),
                overview: scraped_ep.overview.clone().or_else(|| existing.overview.clone()),
                air_date: scraped_ep.air_date.clone().or_else(|| existing.air_date.clone()),
                runtime: scraped_ep.runtime.or(existing.runtime),
                rating: scraped_ep.rating.or(existing.rating),
                director: scraped_ep.director.clone().or_else(|| existing.director.clone()),
                writer: scraped_ep.writer.clone().or_else(|| existing.writer.clone()),
                still_path,
                still_url,
                ..existing.clone()
            };
            db.upsert_episode(&updated).map_err(|e| e.to_string())?;
        }
    }
    if issues.is_empty() { Ok(()) } else { Err(issues.join("; ")) }
}

fn relative_to_show(path: &Path, show_root: &Path) -> String {
    path.strip_prefix(show_root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::{metadata, movie_library};

    #[test]
    fn nfo_uses_matched_title_and_write_failure_is_partial() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        persist_match(&db, &item, &metadata(), &ScrapeImages::default(), "kodi").unwrap();
        let path = Path::new(&item.file_path).with_extension("nfo");
        let xml = std::fs::read_to_string(&path).unwrap();
        assert!(xml.contains("<title>Matched title</title>"));
        assert!(xml.contains("<year>2024</year>"));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(persist_match(&db, &item, &metadata(), &ScrapeImages::default(), "kodi").is_err());
        let updated = db.get_media_item(&item.id).unwrap().unwrap();
        assert_eq!(updated.status, ScrapedStatus::Partial);
        assert!(updated.scrape_issue.unwrap().contains("NFO"));
    }

    #[test]
    fn provider_issues_save_as_partial_with_data() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        let mut meta = metadata();
        meta.issues.push("seasons 2: err.connect".into());
        let err = persist_match(&db, &item, &meta, &ScrapeImages::default(), "kodi").unwrap_err();
        assert!(err.contains("seasons 2: err.connect"));
        let updated = db.get_media_item(&item.id).unwrap().unwrap();
        assert_eq!(updated.status, ScrapedStatus::Partial);
        assert_eq!(updated.title, "Matched title");
        assert!(Path::new(&item.file_path).with_extension("nfo").exists());
    }

    #[test]
    fn fetched_artwork_is_written_and_failed_download_is_partial() {
        let (_dir, db, _lib, item) = movie_library(ScrapedStatus::Unscraped);
        let images = ScrapeImages {
            artwork: vec![("poster", Ok(b"img".to_vec())), ("fanart", Err("err.http".into()))],
            ..Default::default()
        };
        let err = persist_match(&db, &item, &metadata(), &images, "kodi").unwrap_err();
        assert_eq!(err, "fanart: err.http");
        let folder = Path::new(&item.folder_path);
        assert_eq!(std::fs::read(folder.join("Original-poster.jpg")).unwrap(), b"img");
        assert!(!folder.join("Original-fanart.jpg").exists());
        let meta = db.fetch_metadata(&item.id).unwrap().unwrap();
        assert_eq!(meta.poster_path.as_deref(), Some("Original-poster.jpg"));
        assert!(meta.fanart_path.is_none());
        assert_eq!(db.get_media_item(&item.id).unwrap().unwrap().status, ScrapedStatus::Partial);
    }
}
