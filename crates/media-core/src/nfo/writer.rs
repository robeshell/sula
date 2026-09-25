//! Kodi + Emby/Jellyfin NFO writers (NFO-02…05).

use std::fs;
use std::path::Path;

use crate::models::{MediaItem, MediaMetadata, MediaType};

/// Write NFO using configured format (`kodi` | `emby`).
pub fn write_nfo(item: &MediaItem, metadata: &MediaMetadata, format: &str) -> Result<(), String> {
    match format.trim().to_ascii_lowercase().as_str() {
        "emby" | "jellyfin" => write_emby_nfo(item, metadata),
        _ => write_kodi_nfo(item, metadata),
    }
}

pub fn write_kodi_nfo(item: &MediaItem, metadata: &MediaMetadata) -> Result<(), String> {
    write_xml(item, render_kodi(item, metadata))
}

pub fn write_emby_nfo(item: &MediaItem, metadata: &MediaMetadata) -> Result<(), String> {
    write_xml(item, render_emby(item, metadata))
}

fn write_xml(item: &MediaItem, xml: String) -> Result<(), String> {
    let folder = Path::new(&item.folder_path);
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let path = match item.media_type {
        MediaType::Movie => {
            if item.file_path.is_empty() { return Err("movie file path is missing".into()); }
            Path::new(&item.file_path).with_extension("nfo")
        }
        MediaType::TvShow | MediaType::Anime => folder.join("tvshow.nfo"),
    };
    crate::FilesystemService::new().write_file(xml.as_bytes(), path, crate::WriteOptions {
        collision_policy: crate::CollisionPolicy::Replace,
        ..Default::default()
    }).map(|_| ()).map_err(|e| e.to_string())
}

fn render_kodi(item: &MediaItem, metadata: &MediaMetadata) -> String {
    let root = root_tag(item);
    let mut out = String::new();
    out.push_str(&format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<{root}>\n"
    ));
    out.push_str(&format!("  <title>{}</title>\n", xml_escape(&item.title)));
    if let Some(year) = item.year {
        out.push_str(&format!("  <year>{year}</year>\n"));
    }
    if let Some(plot) = &metadata.overview {
        out.push_str(&format!("  <plot>{}</plot>\n", xml_escape(plot)));
    }
    if let Some(rating) = metadata.rating {
        out.push_str(&format!("  <rating>{rating:.1}</rating>\n"));
    }
    for genre in &metadata.genres {
        out.push_str(&format!("  <genre>{}</genre>\n", xml_escape(genre)));
    }
    if let Some(id) = primary_unique_id(metadata) {
        out.push_str(&format!(
            "  <uniqueid type=\"{}\" default=\"true\">{}</uniqueid>\n",
            id.0,
            xml_escape(&id.1)
        ));
    }
    out.push_str(&format!("</{root}>\n"));
    out
}

fn render_emby(item: &MediaItem, metadata: &MediaMetadata) -> String {
    let root = root_tag(item);
    let mut out = String::new();
    out.push_str(&format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<{root}>\n"
    ));
    out.push_str(&format!("  <title>{}</title>\n", xml_escape(&item.title)));
    if let Some(ot) = &item.original_title {
        if !ot.is_empty() {
            out.push_str(&format!(
                "  <originaltitle>{}</originaltitle>\n",
                xml_escape(ot)
            ));
        }
    }
    if let Some(year) = item.year {
        out.push_str(&format!("  <year>{year}</year>\n"));
    }
    if let Some(plot) = &metadata.overview {
        out.push_str(&format!("  <plot>{}</plot>\n", xml_escape(plot)));
    }
    if let Some(tagline) = &metadata.tagline {
        out.push_str(&format!("  <tagline>{}</tagline>\n", xml_escape(tagline)));
    }
    if metadata.rating.is_some() || metadata.rating_votes.is_some() {
        out.push_str("  <ratings>\n");
        let name = rating_source_name(metadata);
        let default = "true";
        out.push_str(&format!(
            "    <rating name=\"{name}\" max=\"10\" default=\"{default}\">\n"
        ));
        if let Some(rating) = metadata.rating {
            out.push_str(&format!("      <value>{rating:.1}</value>\n"));
        }
        if let Some(votes) = metadata.rating_votes {
            out.push_str(&format!("      <votes>{votes}</votes>\n"));
        }
        out.push_str("    </rating>\n");
        out.push_str("  </ratings>\n");
    }
    for genre in &metadata.genres {
        out.push_str(&format!("  <genre>{}</genre>\n", xml_escape(genre)));
    }
    if let Some(studio) = &metadata.studio {
        out.push_str(&format!("  <studio>{}</studio>\n", xml_escape(studio)));
    }
    let ids = all_unique_ids(metadata);
    let primary = primary_unique_id(metadata).map(|(t, _)| t);
    for (ty, val) in ids {
        let def = if Some(ty) == primary {
            "true"
        } else {
            "false"
        };
        out.push_str(&format!(
            "  <uniqueid type=\"{}\" default=\"{}\">{}</uniqueid>\n",
            ty,
            def,
            xml_escape(&val)
        ));
    }
    out.push_str(&format!("</{root}>\n"));
    out
}

fn root_tag(item: &MediaItem) -> &'static str {
    match item.media_type {
        MediaType::Movie => "movie",
        MediaType::TvShow | MediaType::Anime => "tvshow",
    }
}

fn rating_source_name(metadata: &MediaMetadata) -> &'static str {
    if metadata.tmdb_id.is_some() {
        "themoviedb"
    } else if metadata.imdb_id.is_some() {
        "imdb"
    } else if metadata.tvdb_id.is_some() {
        "tvdb"
    } else if metadata.bangumi_id.is_some() {
        "bangumi"
    } else {
        "default"
    }
}

fn primary_unique_id(metadata: &MediaMetadata) -> Option<(&'static str, String)> {
    all_unique_ids(metadata).into_iter().next()
}

fn all_unique_ids(metadata: &MediaMetadata) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(id) = &metadata.tmdb_id {
        out.push(("tmdb", id.clone()));
    }
    if let Some(id) = &metadata.imdb_id {
        out.push(("imdb", id.clone()));
    }
    if let Some(id) = &metadata.tvdb_id {
        out.push(("tvdb", id.clone()));
    }
    if let Some(id) = &metadata.bangumi_id {
        out.push(("bangumi", id.clone()));
    }
    out
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MediaType, ScrapedStatus};
    use chrono::Utc;

    fn sample() -> (MediaItem, MediaMetadata) {
        let item = MediaItem {
            id: "1".into(),
            media_type: MediaType::Movie,
            title: "Test".into(),
            original_title: Some("Original".into()),
            year: Some(2020),
            folder_path: "/tmp".into(),
            file_path: "/tmp/a.mkv".into(),
            bookmark_data: None,
            status: ScrapedStatus::Scraped,
            scrape_issue: None,
            library_id: "lib".into(),
            added_at: Utc::now(),
        };
        let meta = MediaMetadata {
            media_item_id: "1".into(),
            overview: Some("plot".into()),
            outline: None,
            tagline: Some("tag".into()),
            genres: vec!["Action".into()],
            tags: vec![],
            rating: Some(8.5),
            rating_votes: Some(100),
            content_rating: None,
            director: None,
            writer: None,
            credits: vec![],
            studio: Some("Studio".into()),
            country: None,
            language: None,
            premiered: None,
            end_date: None,
            runtime: None,
            show_status: None,
            collection_name: None,
            collection_id: None,
            source_id: "tmdb".into(),
            imdb_id: Some("tt1".into()),
            tmdb_id: Some("42".into()),
            tvdb_id: None,
            bangumi_id: None,
            poster_path: None,
            fanart_path: None,
            banner_path: None,
            logo_path: None,
            thumb_path: None,
            video_codec: None,
            video_resolution: None,
            audio_codec: None,
            audio_channels: None,
            trailer: None,
            scraped_at: Utc::now(),
        };
        (item, meta)
    }

    #[test]
    fn movies_in_shared_directory_write_distinct_nfos() {
        let dir = tempfile::tempdir().unwrap();
        let (mut item, meta) = sample();
        item.folder_path = dir.path().to_string_lossy().into_owned();
        for name in ["A", "B"] {
            item.file_path = dir.path().join(format!("{name}.mkv")).to_string_lossy().into_owned();
            item.title = name.into();
            write_nfo(&item, &meta, "kodi").unwrap();
        }
        for name in ["A", "B"] {
            let xml = std::fs::read_to_string(dir.path().join(format!("{name}.nfo"))).unwrap();
            assert!(xml.contains(&format!("<title>{name}</title>")));
        }
    }

    #[test]
    fn emby_has_nested_ratings_and_multi_uniqueid() {
        let (item, meta) = sample();
        let xml = render_emby(&item, &meta);
        assert!(xml.contains("<ratings>"));
        assert!(xml.contains("<value>8.5</value>"));
        assert!(xml.contains("type=\"tmdb\""));
        assert!(xml.contains("type=\"imdb\""));
        assert!(xml.contains("default=\"true\""));
    }
}
