//! Port of Swift `MediaCore/NFO/NFOReader.swift`.
//! Behavior must stay aligned with Swift + `NFOReaderTests`.

use roxmltree::{Document, Node};

use crate::models::CastMember;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NfoParsedData {
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub outline: Option<String>,
    pub tagline: Option<String>,
    pub rating: Option<f64>,
    pub rating_votes: Option<i32>,
    pub content_rating: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub director: Option<String>,
    pub writer: Option<String>,
    pub credits: Vec<CastMember>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub language: Option<String>,
    pub premiered: Option<String>,
    pub end_date: Option<String>,
    pub runtime: Option<i32>,
    pub show_status: Option<String>,
    pub collection_name: Option<String>,
    pub collection_id: Option<String>,
    pub source_id: Option<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub bangumi_id: Option<String>,
    pub trailer: Option<String>,
    pub video_codec: Option<String>,
    pub video_resolution: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<String>,
}

impl NfoParsedData {
    /// Actor names only (Swift `cast` convenience).
    pub fn cast(&self) -> Vec<String> {
        self.credits
            .iter()
            .filter(|c| c.r#type.as_deref().is_none_or(|t| t == "Actor"))
            .map(|c| c.name.clone())
            .collect()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NfoError {
    #[error("invalid encoding")]
    InvalidEncoding,
    #[error("xml parse error: {0}")]
    Xml(String),
}

pub struct NfoReader;

impl NfoReader {
    pub fn parse_movie_nfo(xml: &str) -> Result<NfoParsedData, NfoError> {
        with_root(xml, parse_common)
    }

    pub fn parse_tvshow_nfo(xml: &str) -> Result<NfoParsedData, NfoError> {
        with_root(xml, |root| {
            let mut parsed = parse_common(root);
            parsed.show_status = child_text(root, "status");
            parsed.end_date = child_text(root, "enddate");
            parsed
        })
    }

    pub fn parse_season_nfo(xml: &str) -> Result<NfoParsedData, NfoError> {
        with_root(xml, |root| {
            let mut parsed = parse_common(root);
            if parsed.title.is_empty() {
                if let Some(show) = child_text(root, "showtitle") {
                    parsed.title = show;
                }
            }
            parsed
        })
    }

    pub fn parse_episode_nfo(xml: &str) -> Result<NfoParsedData, NfoError> {
        with_root(xml, |root| {
            let mut parsed = parse_common(root);
            if parsed.premiered.is_none() {
                parsed.premiered = child_text(root, "aired");
            }
            parsed
        })
    }
}

fn with_root<T>(xml: &str, f: impl FnOnce(Node<'_, '_>) -> T) -> Result<T, NfoError> {
    let doc = Document::parse(xml).map_err(|e| NfoError::Xml(e.to_string()))?;
    Ok(f(doc.root_element()))
}

fn parse_common(root: Node<'_, '_>) -> NfoParsedData {
    let ids = parse_all_source_ids(root);
    NfoParsedData {
        title: child_text(root, "title").unwrap_or_default(),
        original_title: child_text(root, "originaltitle"),
        year: parse_year(root),
        overview: child_text(root, "plot"),
        outline: child_text(root, "outline"),
        tagline: child_text(root, "tagline"),
        rating: parse_rating(root),
        rating_votes: parse_votes(root),
        content_rating: child_text(root, "mpaa"),
        genres: children_texts(root, "genre"),
        tags: children_texts(root, "tag"),
        director: child_text(root, "director"),
        writer: child_text(root, "writer").or_else(|| child_text(root, "credits")),
        credits: parse_credits(root),
        studio: child_text(root, "studio"),
        country: child_text(root, "country"),
        language: child_text(root, "languages").or_else(|| child_text(root, "language")),
        premiered: child_text(root, "premiered"),
        end_date: child_text(root, "enddate"),
        runtime: child_text(root, "runtime").and_then(|s| s.parse().ok()),
        show_status: None,
        collection_name: parse_collection_name(root),
        collection_id: child_text(root, "collectionnumber"),
        source_id: ids.primary,
        imdb_id: ids.imdb,
        tmdb_id: ids.tmdb,
        tvdb_id: ids.tvdb,
        bangumi_id: ids.bangumi,
        trailer: child_text(root, "trailer"),
        video_codec: nested_text(root, &["fileinfo", "streamdetails", "video", "codec"]),
        video_resolution: parse_video_resolution(root),
        audio_codec: nested_text(root, &["fileinfo", "streamdetails", "audio", "codec"]),
        audio_channels: nested_text(root, &["fileinfo", "streamdetails", "audio", "channels"]),
    }
}

fn parse_rating(root: Node<'_, '_>) -> Option<f64> {
    if let Some(simple) = child_text(root, "rating").and_then(|s| s.parse().ok()) {
        return Some(simple);
    }
    let ratings = child(root, "ratings")?;
    let rating_els: Vec<_> = children_named(ratings, "rating").collect();
    let chosen = rating_els
        .iter()
        .find(|el| attribute(el, "default") == Some("true"))
        .or(rating_els.first())?;
    child_text(*chosen, "value").and_then(|s| s.parse().ok())
}

fn parse_votes(root: Node<'_, '_>) -> Option<i32> {
    if let Some(v) = child_text(root, "votes").and_then(|s| s.parse().ok()) {
        return Some(v);
    }
    let ratings = child(root, "ratings")?;
    let rating_els: Vec<_> = children_named(ratings, "rating").collect();
    let chosen = rating_els
        .iter()
        .find(|el| attribute(el, "default") == Some("true"))
        .or(rating_els.first())?;
    child_text(*chosen, "votes").and_then(|s| s.parse().ok())
}

fn parse_year(root: Node<'_, '_>) -> Option<i32> {
    if let Some(y) = child_text(root, "year").and_then(|s| s.parse().ok()) {
        return Some(y);
    }
    let premiered = child_text(root, "premiered")?;
    // Byte slicing would panic on non-ASCII dates such as `二〇二三年`.
    if let Some(year) = premiered.get(..4) {
        return year.parse().ok();
    }
    None
}

fn parse_credits(root: Node<'_, '_>) -> Vec<CastMember> {
    let mut result = Vec::new();
    let mut order = 0i32;
    for actor in children_named(root, "actor") {
        let Some(name) = child_text(actor, "name").filter(|n| !n.is_empty()) else {
            continue;
        };
        let role = child_text(actor, "role");
        let type_ = child_text(actor, "type").unwrap_or_else(|| "Actor".into());
        let thumb = child_text(actor, "thumb");
        let sort_order = child_text(actor, "order")
            .and_then(|s| s.parse().ok())
            .unwrap_or(order);
        result.push(CastMember {
            name,
            role,
            r#type: Some(type_),
            thumb_url: thumb,
            order: Some(sort_order),
        });
        order += 1;
    }
    result
}

#[derive(Default)]
struct SourceIds {
    primary: Option<String>,
    imdb: Option<String>,
    tmdb: Option<String>,
    tvdb: Option<String>,
    bangumi: Option<String>,
}

fn parse_all_source_ids(root: Node<'_, '_>) -> SourceIds {
    let elements: Vec<_> = children_named(root, "uniqueid").collect();
    let mut ids = SourceIds::default();

    for el in &elements {
        let type_ = attribute(el, "type").unwrap_or_default();
        let Some(value) = node_text(*el).filter(|v| !v.is_empty()) else {
            continue;
        };
        let is_default = attribute(el, "default") == Some("true");

        match type_.to_ascii_lowercase().as_str() {
            "imdb" => ids.imdb = Some(value.clone()),
            "tmdb" => ids.tmdb = Some(value.clone()),
            "tvdb" => ids.tvdb = Some(value.clone()),
            "bangumi" => ids.bangumi = Some(value.clone()),
            _ => {}
        }

        if is_default {
            ids.primary = Some(format!("{type_}:{value}"));
        }
    }

    if ids.primary.is_none() {
        if let Some(tmdb) = &ids.tmdb {
            ids.primary = Some(format!("tmdb:{tmdb}"));
        } else if let Some(imdb) = &ids.imdb {
            ids.primary = Some(format!("imdb:{imdb}"));
        } else if let Some(tvdb) = &ids.tvdb {
            ids.primary = Some(format!("tvdb:{tvdb}"));
        } else if let Some(el) = elements.first() {
            if let (Some(type_), Some(value)) = (attribute(el, "type"), node_text(*el)) {
                if !value.is_empty() {
                    ids.primary = Some(format!("{type_}:{value}"));
                }
            }
        }
    }

    if ids.imdb.is_none() {
        ids.imdb = child_text(root, "imdb_id").or_else(|| child_text(root, "imdbid"));
    }
    if ids.tmdb.is_none() {
        ids.tmdb = child_text(root, "tmdbid");
    }
    if ids.tvdb.is_none() {
        ids.tvdb = child_text(root, "tvdbid");
    }
    if ids.bangumi.is_none() {
        ids.bangumi = child_text(root, "bangumiid");
    }

    ids
}

fn parse_collection_name(root: Node<'_, '_>) -> Option<String> {
    let set_el = child(root, "set")?;
    if let Some(name) = child_text(set_el, "name") {
        return Some(name);
    }
    node_text(set_el).filter(|t| !t.is_empty())
}

fn parse_video_resolution(root: Node<'_, '_>) -> Option<String> {
    let video = nested(root, &["fileinfo", "streamdetails", "video"])?;
    let w = child_text(video, "width")?;
    let h = child_text(video, "height")?;
    Some(format!("{w}x{h}"))
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
}

fn children_named<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    let name = name.to_string();
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(&name))
}

fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name).and_then(node_text)
}

fn children_texts(node: Node<'_, '_>, name: &str) -> Vec<String> {
    children_named(node, name)
        .filter_map(node_text)
        .collect()
}

fn nested<'a, 'input>(node: Node<'a, 'input>, path: &[&str]) -> Option<Node<'a, 'input>> {
    let mut cur = node;
    for name in path {
        cur = child(cur, name)?;
    }
    Some(cur)
}

fn nested_text(node: Node<'_, '_>, path: &[&str]) -> Option<String> {
    nested(node, path).and_then(node_text)
}

fn attribute<'a>(node: &Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

fn node_text(node: Node<'_, '_>) -> Option<String> {
    let text = node.text()?.trim();
    if text.is_empty() {
        // Element may wrap text in nested nodes; collect direct text children.
        let joined: String = node
            .children()
            .filter_map(|c| c.text())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("");
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    } else {
        Some(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_media_manager_movie_nfo() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
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
                <rating name="imdb" max="10">
                    <value>8.4</value>
                    <votes>654321</votes>
                </rating>
            </ratings>
            <genre>传记</genre>
            <genre>剧情</genre>
            <genre>历史</genre>
            <director>Christopher Nolan</director>
            <actor>
                <name>Cillian Murphy</name>
                <role>J. Robert Oppenheimer</role>
            </actor>
            <actor>
                <name>Emily Blunt</name>
                <role>Kitty Oppenheimer</role>
            </actor>
            <uniqueid type="imdb">tt15398776</uniqueid>
            <uniqueid type="tmdb" default="true">872585</uniqueid>
        </movie>"#;
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.title, "奥本海默");
        assert_eq!(parsed.original_title.as_deref(), Some("Oppenheimer"));
        assert_eq!(parsed.year, Some(2023));
        assert_eq!(
            parsed.overview.as_deref(),
            Some("这部电影讲述了理论物理学家罗伯特·奥本海默的故事。")
        );
        assert_eq!(parsed.rating, Some(8.1));
        assert_eq!(parsed.genres, vec!["传记", "剧情", "历史"]);
        assert_eq!(parsed.director.as_deref(), Some("Christopher Nolan"));
        assert_eq!(
            parsed.cast(),
            vec!["Cillian Murphy".to_string(), "Emily Blunt".to_string()]
        );
        assert_eq!(parsed.source_id.as_deref(), Some("tmdb:872585"));
    }

    #[test]
    fn non_ascii_premiered_does_not_panic() {
        let parsed = NfoReader::parse_movie_nfo("<movie><title>X</title><premiered>二〇二三年</premiered></movie>").unwrap();
        assert_eq!(parsed.year, None);
        let parsed = NfoReader::parse_movie_nfo("<movie><title>X</title><premiered>2023-07-21</premiered></movie>").unwrap();
        assert_eq!(parsed.year, Some(2023));
    }

    #[test]
    fn simple_rating_format() {
        let xml = r#"<movie>
            <title>Test Movie</title>
            <rating>7.5</rating>
            <uniqueid type="tmdb">12345</uniqueid>
        </movie>"#;
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.rating, Some(7.5));
        assert_eq!(parsed.source_id.as_deref(), Some("tmdb:12345"));
    }

    #[test]
    fn source_id_prefers_default() {
        let xml = r#"<movie>
            <title>Test</title>
            <uniqueid type="imdb">tt1234567</uniqueid>
            <uniqueid type="tmdb" default="true">99999</uniqueid>
        </movie>"#;
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.source_id.as_deref(), Some("tmdb:99999"));
    }

    #[test]
    fn source_id_falls_back_to_tmdb() {
        let xml = r#"<movie>
            <title>Test</title>
            <uniqueid type="imdb">tt1234567</uniqueid>
            <uniqueid type="tmdb">88888</uniqueid>
        </movie>"#;
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.source_id.as_deref(), Some("tmdb:88888"));
    }

    #[test]
    fn source_id_falls_back_to_imdb() {
        let xml = r#"<movie>
            <title>Test</title>
            <uniqueid type="imdb">tt1234567</uniqueid>
        </movie>"#;
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.source_id.as_deref(), Some("imdb:tt1234567"));
    }

    #[test]
    fn year_from_premiered() {
        let xml = r#"<movie>
            <title>Test</title>
            <premiered>2023-07-21</premiered>
        </movie>"#;
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.year, Some(2023));
    }

    #[test]
    fn year_tag_takes_precedence_over_premiered() {
        let xml = r#"<movie>
            <title>Test</title>
            <year>2023</year>
            <premiered>2022-12-01</premiered>
        </movie>"#;
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.year, Some(2023));
    }

    #[test]
    fn empty_nfo() {
        let xml = "<movie><title></title></movie>";
        let parsed = NfoReader::parse_movie_nfo(xml).unwrap();
        assert_eq!(parsed.title, "");
        assert_eq!(parsed.year, None);
        assert_eq!(parsed.overview, None);
        assert_eq!(parsed.rating, None);
        assert!(parsed.genres.is_empty());
        assert!(parsed.cast().is_empty());
        assert_eq!(parsed.source_id, None);
    }

    #[test]
    fn episode_aired_maps_to_premiered() {
        let xml = r#"<episodedetails>
            <title>第 1 集</title>
            <plot>episode overview</plot>
            <aired>2024-09-01</aired>
        </episodedetails>"#;
        let parsed = NfoReader::parse_episode_nfo(xml).unwrap();
        assert_eq!(parsed.title, "第 1 集");
        assert_eq!(parsed.premiered.as_deref(), Some("2024-09-01"));
    }
}
