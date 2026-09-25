use media_core::MediaType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub source_id: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub confidence: f64,
    pub media_type: MediaType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapedMetadata {
    pub source_id: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub rating: Option<f64>,
    pub rating_votes: Option<i32>,
    pub content_rating: Option<String>,
    pub director: Option<String>,
    pub writer: Option<String>,
    pub credits: Vec<media_core::CastMember>,
    pub studio: Option<String>,
    pub country: Option<String>,
    pub language: Option<String>,
    pub premiered: Option<String>,
    pub end_date: Option<String>,
    pub runtime: Option<i32>,
    pub show_status: Option<String>,
    pub collection_name: Option<String>,
    pub collection_id: Option<String>,
    pub poster_url: Option<String>,
    pub fanart_url: Option<String>,
    pub banner_url: Option<String>,
    pub trailer: Option<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    pub tvdb_id: Option<String>,
    pub bangumi_id: Option<String>,
    pub seasons: Vec<ScrapedSeason>,
    /// Humanized partial-fetch problems (e.g. missing seasons); non-empty saves as partial.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScrapedSeason {
    pub season_number: i32,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub air_date: Option<String>,
    pub episode_count: Option<i32>,
    pub episodes: Vec<ScrapedEpisode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScrapedEpisode {
    pub episode_number: i32,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub still_url: Option<String>,
    pub runtime: Option<i32>,
    pub rating: Option<f64>,
    pub director: Option<String>,
    pub writer: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ArtworkUrls {
    pub poster_url: Option<String>,
    pub fanart_url: Option<String>,
    pub banner_url: Option<String>,
}

pub fn parse_source_numeric_id(source_id: &str) -> Option<&str> {
    source_id.split_once(':').map(|(_, id)| id)
}
