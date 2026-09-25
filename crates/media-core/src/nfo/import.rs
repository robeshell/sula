//! Port of Swift `LibraryRefreshService.importNFOForItem` (+ helpers).

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use std::sync::OnceLock;
use walkdir::WalkDir;

use crate::models::{MediaItem, MediaMetadata, MediaType, ScrapedStatus, TvEpisode};
use crate::nfo::reader::{NfoParsedData, NfoReader};
use crate::AppDatabase;
use crate::DatabaseError;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "tbn"];

/// Mirror of Swift `LibraryRefreshService.importNFOForItem`.
/// Returns `Ok(true)` when an NFO was parsed and applied; `Ok(false)` when none found (no-op).
pub fn import_nfo_for_item(db: &AppDatabase, item: &MediaItem) -> Result<bool, DatabaseError> {
    let folder = PathBuf::from(&item.folder_path);
    if !folder.is_dir() {
        return Ok(false);
    }

    // Avoid an ownership/database walk for the common scan case with no NFO.
    let has_nfo = fs::read_dir(&folder)?.any(|entry| entry.ok().is_some_and(|entry| {
        entry.path().extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("nfo"))
    }));
    if !has_nfo { return Ok(false); }
    let exclusive = crate::media_files::owns_folder(db, item)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let Some(parsed) = find_and_parse_nfo(item, &folder, exclusive) else {
        return Ok(false);
    };

    let movie_stem = Path::new(&item.file_path).file_stem().and_then(|v| v.to_str());
    let artwork = |suffix: &str, fallbacks: &[&str]| {
        let specific = movie_stem.filter(|_| item.media_type == MediaType::Movie)
            .and_then(|stem| detect_artwork(&folder, &[&format!("{stem}-{suffix}")]));
        specific.or_else(|| if exclusive { detect_artwork(&folder, fallbacks) } else { None })
    };
    let poster_path = artwork("poster", &["poster", "cover", "folder"]);
    let fanart_path = artwork("fanart", &["fanart", "backdrop", "background"]);

    let mut metadata = MediaMetadata {
        media_item_id: item.id.clone(),
        overview: parsed.overview.clone(),
        outline: parsed.outline.clone(),
        tagline: parsed.tagline.clone(),
        genres: parsed.genres.clone(),
        tags: parsed.tags.clone(),
        rating: parsed.rating,
        rating_votes: parsed.rating_votes,
        content_rating: parsed.content_rating.clone(),
        director: parsed.director.clone(),
        writer: parsed.writer.clone(),
        credits: parsed.credits.clone(),
        studio: parsed.studio.clone(),
        country: parsed.country.clone(),
        language: parsed.language.clone(),
        premiered: parsed.premiered.clone(),
        end_date: parsed.end_date.clone(),
        runtime: parsed.runtime,
        show_status: parsed.show_status.clone(),
        collection_name: parsed.collection_name.clone(),
        collection_id: parsed.collection_id.clone(),
        source_id: parsed.source_id.clone().unwrap_or_default(),
        imdb_id: parsed.imdb_id.clone(),
        tmdb_id: parsed.tmdb_id.clone(),
        tvdb_id: parsed.tvdb_id.clone(),
        bangumi_id: parsed.bangumi_id.clone(),
        poster_path,
        fanart_path,
        banner_path: None,
        logo_path: None,
        thumb_path: None,
        video_codec: parsed.video_codec.clone(),
        video_resolution: parsed.video_resolution.clone(),
        audio_codec: parsed.audio_codec.clone(),
        audio_channels: parsed.audio_channels.clone(),
        trailer: parsed.trailer.clone(),
        scraped_at: Utc::now(),
    };
    // An NFO (including Sula's own, which carries only a subset of fields) adds to
    // what the index already knows; it never blanks values it simply doesn't mention.
    if let Some(existing) = db.fetch_metadata(&item.id)? {
        keep_existing_fields(&mut metadata, existing);
    }

    db.upsert_metadata(&metadata)?;
    db.update_status(&item.id, ScrapedStatus::Scraped, None)?;

    if !parsed.title.is_empty() {
        let original_title = parsed.original_title.as_deref().or(item.original_title.as_deref());
        db.update_title(&item.id, &parsed.title, original_title)?;
    }

    if matches!(item.media_type, MediaType::TvShow | MediaType::Anime) {
        import_episode_nfos(db, item, &folder)?;
    }

    Ok(true)
}

fn keep_existing_fields(metadata: &mut MediaMetadata, existing: MediaMetadata) {
    macro_rules! keep {
        ($($field:ident),*) => { $( if metadata.$field.is_none() { metadata.$field = existing.$field; } )* };
    }
    keep!(overview, outline, tagline, rating, rating_votes, content_rating, director, writer, studio,
        country, language, premiered, end_date, runtime, show_status, collection_name, collection_id,
        imdb_id, tmdb_id, tvdb_id, bangumi_id, poster_path, fanart_path, banner_path, logo_path, thumb_path,
        video_codec, video_resolution, audio_codec, audio_channels, trailer);
    if metadata.genres.is_empty() { metadata.genres = existing.genres; }
    if metadata.tags.is_empty() { metadata.tags = existing.tags; }
    if metadata.credits.is_empty() { metadata.credits = existing.credits; }
    if metadata.source_id.is_empty() { metadata.source_id = existing.source_id; }
}

fn find_and_parse_nfo(item: &MediaItem, folder: &Path, exclusive: bool) -> Option<NfoParsedData> {
    let mut candidate_names = Vec::new();
    if !item.file_path.is_empty() {
        let video = Path::new(&item.file_path);
        if let Some(stem) = video.file_stem().and_then(|s| s.to_str()) {
            candidate_names.push(format!("{stem}.nfo"));
        }
    }
    if !exclusive {
        return candidate_names.iter().find_map(|name| try_parse_file(&folder.join(name), item.media_type));
    }
    if let Some(folder_name) = folder.file_name().and_then(|n| n.to_str()) {
        candidate_names.push(format!("{folder_name}.nfo"));
    }
    candidate_names.push(if item.media_type == MediaType::Movie {
        "movie.nfo".into()
    } else {
        "tvshow.nfo".into()
    });

    for name in &candidate_names {
        let nfo_path = folder.join(name);
        if let Some(parsed) = try_parse_file(&nfo_path, item.media_type) {
            return Some(parsed);
        }
    }

    let entries = fs::read_dir(folder).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("nfo"))
        {
            if let Some(parsed) = try_parse_file(&path, item.media_type) {
                return Some(parsed);
            }
        }
    }
    None
}

fn try_parse_file(path: &Path, media_type: MediaType) -> Option<NfoParsedData> {
    let xml = fs::read_to_string(path).ok()?;
    parse_nfo(&xml, media_type).ok()
}

fn parse_nfo(xml: &str, media_type: MediaType) -> Result<NfoParsedData, crate::nfo::reader::NfoError> {
    match media_type {
        MediaType::Movie => NfoReader::parse_movie_nfo(xml),
        MediaType::TvShow | MediaType::Anime => NfoReader::parse_tvshow_nfo(xml),
    }
}

fn import_episode_nfos(
    db: &AppDatabase,
    item: &MediaItem,
    folder: &Path,
) -> Result<(), DatabaseError> {
    let seasons = db.fetch_seasons(&item.id)?;
    for season in seasons {
        let episodes = db.fetch_episodes(&season.id)?;
        let mut updated_season = season.clone();

        if let Some(season_nfo) =
            detect_season_nfo_url(season.season_number, &episodes, folder)
        {
            if let Ok(xml) = fs::read_to_string(&season_nfo) {
                if let Ok(parsed) = NfoReader::parse_season_nfo(&xml) {
                    if !parsed.title.is_empty() {
                        updated_season.title = Some(parsed.title);
                    }
                    if let Some(overview) = parsed.overview.filter(|s| !s.is_empty()) {
                        updated_season.overview = Some(overview);
                    }
                    if let Some(air_date) = parsed.premiered.filter(|s| !s.is_empty()) {
                        updated_season.air_date = Some(air_date);
                    }
                }
            }
        }

        if let Some(poster) =
            detect_season_poster_path(season.season_number, &episodes, folder)
        {
            updated_season.poster_path = Some(poster);
        }
        db.upsert_season(&updated_season)?;

        for ep in episodes {
            if ep.file_path.is_empty() {
                continue;
            }
            let mut updated = ep.clone();
            if let Some(still) = detect_episode_still_path(&ep.file_path, folder) {
                updated.still_path.replace(still);
            }

            let nfo_path = Path::new(&ep.file_path).with_extension("nfo");
            if let Ok(xml) = fs::read_to_string(&nfo_path) {
                if let Ok(parsed) = NfoReader::parse_episode_nfo(&xml) {
                    if !parsed.title.is_empty() {
                        updated.title = Some(parsed.title);
                    }
                    if let Some(overview) = parsed.overview.filter(|s| !s.is_empty()) {
                        updated.overview = Some(overview);
                    }
                    if let Some(air_date) = parsed.premiered.filter(|s| !s.is_empty()) {
                        updated.air_date = Some(air_date);
                    }
                    if let Some(rating) = parsed.rating {
                        updated.rating = Some(rating);
                    }
                    if let Some(runtime) = parsed.runtime {
                        updated.runtime = Some(runtime);
                    }
                    if let Some(director) = parsed.director.filter(|s| !s.is_empty()) {
                        updated.director = Some(director);
                    }
                    if let Some(writer) = parsed.writer.filter(|s| !s.is_empty()) {
                        updated.writer = Some(writer);
                    }
                    if !parsed.credits.is_empty() {
                        updated.guest_cast = parsed.credits;
                    }
                }
            }
            db.upsert_episode(&updated)?;
        }
    }
    Ok(())
}

fn detect_artwork(folder: &Path, names: &[&str]) -> Option<String> {
    let entries = fs::read_dir(folder).ok()?;
    let images: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_image(p))
        .collect();

    for image in &images {
        let stem = image
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if names.iter().any(|n| *n == stem) {
            return image
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string);
        }
    }

    static SEASON_PREFIX: OnceLock<Regex> = OnceLock::new();
    let season_prefix = SEASON_PREFIX.get_or_init(|| Regex::new(r"(?i)^season\d+-").unwrap());

    for image in &images {
        let stem = image
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if season_prefix.is_match(&stem) {
            continue;
        }
        for keyword in names {
            if stem.contains(keyword) {
                return image
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string);
            }
        }
    }
    None
}

fn detect_episode_still_path(episode_file_path: &str, show_root: &Path) -> Option<String> {
    let episode_url = Path::new(episode_file_path);
    let episode_dir = episode_url.parent()?;
    let entries = fs::read_dir(episode_dir).ok()?;
    let images: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_image(p))
        .collect();

    let stem = episode_url
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let candidates = [
        format!("{stem}-thumb"),
        format!("{stem}_thumb"),
        stem.clone(),
    ];

    let matched = images.into_iter().find(|image| {
        let image_stem = image
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        candidates.iter().any(|c| c == &image_stem)
    })?;

    relative_to_show(&matched, show_root)
}

fn detect_season_nfo_url(
    season_number: i32,
    episodes: &[TvEpisode],
    show_root: &Path,
) -> Option<PathBuf> {
    if let Some(first) = episodes.iter().find(|e| !e.file_path.is_empty()) {
        let season_dir = Path::new(&first.file_path).parent()?;
        let season_nfo = season_dir.join("season.nfo");
        if season_nfo.is_file() {
            return Some(season_nfo);
        }
    }

    for candidate in [
        show_root.join(format!("season{season_number:02}.nfo")),
        show_root.join(format!("season{season_number}.nfo")),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let tokens = season_match_tokens(season_number);
    for entry in WalkDir::new(show_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("nfo"))
        {
            continue;
        }
        let lower_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let lower_parent = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if lower_name == "season" && tokens.iter().any(|t| lower_parent.contains(t)) {
            return Some(path.to_path_buf());
        }
        if tokens.iter().any(|t| lower_name.contains(t)) {
            return Some(path.to_path_buf());
        }
    }
    None
}

fn detect_season_poster_path(
    season_number: i32,
    episodes: &[TvEpisode],
    show_root: &Path,
) -> Option<String> {
    let tokens = season_match_tokens(season_number);
    let poster_tokens = [
        format!("season{season_number:02}-poster"),
        format!("season{season_number}-poster"),
        format!("s{season_number:02}-poster"),
        format!("season{season_number:02}poster"),
        format!("season{season_number}poster"),
        format!("s{season_number:02}poster"),
    ];

    if let Some(first) = episodes.iter().find(|e| !e.file_path.is_empty()) {
        if let Some(season_dir) = Path::new(&first.file_path).parent() {
            if let Some(m) =
                detect_season_poster_in_directory(season_dir, show_root, &tokens, &poster_tokens)
            {
                return Some(m);
            }
        }
    }

    if let Some(m) =
        detect_season_poster_in_directory(show_root, show_root, &tokens, &poster_tokens)
    {
        return Some(m);
    }

    for entry in WalkDir::new(show_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() || !is_image(entry.path()) {
            continue;
        }
        let path = entry.path();
        let lower_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let lower_parent = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let matches = poster_tokens.iter().any(|t| lower_stem.contains(t))
            || (["poster", "folder", "season", "cover"].contains(&lower_stem.as_str())
                && tokens.iter().any(|t| lower_parent.contains(t)));
        if matches {
            return relative_to_show(path, show_root);
        }
    }
    None
}

fn detect_season_poster_in_directory(
    directory: &Path,
    show_root: &Path,
    tokens: &[String],
    poster_tokens: &[String],
) -> Option<String> {
    let entries = fs::read_dir(directory).ok()?;
    let lower_dir = directory
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    for entry in entries.flatten() {
        let path = entry.path();
        if !is_image(&path) {
            continue;
        }
        let lower_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let matches = poster_tokens.iter().any(|t| lower_stem.contains(t))
            || (["poster", "folder", "season", "cover"].contains(&lower_stem.as_str())
                && tokens.iter().any(|t| lower_dir.contains(t)));
        if matches {
            return relative_to_show(&path, show_root);
        }
    }
    None
}

fn season_match_tokens(season_number: i32) -> Vec<String> {
    let padded = format!("{season_number:02}");
    vec![
        format!("season {season_number}"),
        format!("season {padded}"),
        format!("season{season_number}"),
        format!("season{padded}"),
        format!("s{season_number}"),
        format!("s{padded}"),
        format!("第{season_number}季"),
        format!("第 {season_number} 季"),
        format!("第{padded}季"),
        format!("第 {padded} 季"),
    ]
}

fn relative_to_show(path: &Path, show_root: &Path) -> Option<String> {
    let root = show_root.canonicalize().ok()?;
    let image = path.canonicalize().ok()?;
    image
        .strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Library, MediaType};
    use crate::AppDatabase;
    use tempfile::tempdir;

    #[test]
    fn import_movie_nfo_writes_metadata_and_status() {
        let dir = tempdir().unwrap();
        let movie_dir = dir.path().join("Oppenheimer (2023)");
        std::fs::create_dir_all(&movie_dir).unwrap();
        let video = movie_dir.join("Oppenheimer.2023.mkv");
        std::fs::write(&video, b"x").unwrap();
        std::fs::write(
            movie_dir.join("Oppenheimer.2023.nfo"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <movie>
                <title>奥本海默</title>
                <originaltitle>Oppenheimer</originaltitle>
                <year>2023</year>
                <plot>这部电影讲述了理论物理学家罗伯特·奥本海默的故事。</plot>
                <ratings>
                    <rating name="themoviedb" max="10" default="true">
                        <value>8.1</value>
                        <votes>7890</votes>
                    </rating>
                </ratings>
                <uniqueid type="tmdb" default="true">872585</uniqueid>
            </movie>"#,
        )
        .unwrap();
        std::fs::write(movie_dir.join("poster.jpg"), b"img").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("Movies", dir.path().display().to_string(), MediaType::Movie);
        db.insert_library(&library).unwrap();

        let item = MediaItem::new_movie(
            "Oppenheimer",
            Some(2023),
            movie_dir.display().to_string(),
            video.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Unscraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();

        assert!(import_nfo_for_item(&db, &item).unwrap());

        let meta = db.fetch_metadata(&item.id).unwrap().unwrap();
        assert_eq!(meta.source_id, "tmdb:872585");
        assert_eq!(meta.rating, Some(8.1));
        assert_eq!(meta.overview.as_deref(), Some("这部电影讲述了理论物理学家罗伯特·奥本海默的故事。"));
        assert_eq!(meta.poster_path.as_deref(), Some("poster.jpg"));

        let items = db.list_media_items(&library.id).unwrap();
        assert_eq!(items[0].status, ScrapedStatus::Scraped);
        assert_eq!(items[0].title, "奥本海默");
        assert_eq!(items[0].original_title.as_deref(), Some("Oppenheimer"));
    }

    #[test]
    fn reimporting_a_sparse_nfo_keeps_richer_indexed_metadata() {
        let dir = tempdir().unwrap();
        let movie_dir = dir.path().join("Dune (2021)");
        std::fs::create_dir_all(&movie_dir).unwrap();
        let video = movie_dir.join("Dune.mkv");
        std::fs::write(&video, b"x").unwrap();
        let nfo = movie_dir.join("Dune.nfo");
        std::fs::write(&nfo, r#"<movie><title>Dune</title><originaltitle>Dune: Part One</originaltitle>
            <director>Denis Villeneuve</director><studio>Legendary</studio><runtime>155</runtime>
            <uniqueid type="tmdb" default="true">438631</uniqueid></movie>"#).unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("Movies", dir.path().display().to_string(), MediaType::Movie);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_movie("Dune", Some(2021), movie_dir.display().to_string(),
            video.display().to_string(), library.id.clone(), ScrapedStatus::Unscraped);
        db.insert_media_items(&[item.clone()]).unwrap();
        assert!(import_nfo_for_item(&db, &item).unwrap());

        // A later, sparser NFO (like the compact Kodi one Sula writes) only adds.
        std::fs::write(&nfo, r#"<movie><title>Dune</title><plot>Paul Atreides...</plot>
            <uniqueid type="tmdb" default="true">438631</uniqueid></movie>"#).unwrap();
        let item = db.get_media_item(&item.id).unwrap().unwrap();
        assert!(import_nfo_for_item(&db, &item).unwrap());
        let meta = db.fetch_metadata(&item.id).unwrap().unwrap();
        assert_eq!(meta.director.as_deref(), Some("Denis Villeneuve"));
        assert_eq!(meta.studio.as_deref(), Some("Legendary"));
        assert_eq!(meta.runtime, Some(155));
        assert_eq!(meta.overview.as_deref(), Some("Paul Atreides..."));
        let item = db.get_media_item(&item.id).unwrap().unwrap();
        assert_eq!(item.original_title.as_deref(), Some("Dune: Part One"));
    }

    #[test]
    fn import_without_nfo_is_noop() {
        let dir = tempdir().unwrap();
        let movie_dir = dir.path().join("Bare");
        std::fs::create_dir_all(&movie_dir).unwrap();
        let video = movie_dir.join("Bare.mkv");
        std::fs::write(&video, b"x").unwrap();

        let db = AppDatabase::open_in_memory().unwrap();
        let library = Library::new("Movies", dir.path().display().to_string(), MediaType::Movie);
        db.insert_library(&library).unwrap();
        let item = MediaItem::new_movie(
            "Bare",
            None,
            movie_dir.display().to_string(),
            video.display().to_string(),
            library.id.clone(),
            ScrapedStatus::Unscraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();
        assert!(!import_nfo_for_item(&db, &item).unwrap());
        assert!(db.fetch_metadata(&item.id).unwrap().is_none());
    }
}
