//! OMDb movie scraper (SCRAPE-08).

use media_core::MediaType;
use reqwest::Client;
use serde::Deserialize;

use crate::http::{reqwest_err, send_with_retry};
use crate::matching::relevance_score;
use crate::types::{parse_source_numeric_id, ArtworkUrls, ScrapedMetadata, SearchResult};

const BASE: &str = "https://www.omdbapi.com/";

#[derive(Clone)]
pub struct OmdbScraper {
    client: Client,
    api_key: String,
}

impl OmdbScraper {
    pub fn new(client: Client, api_key: impl Into<String>) -> Self {
        Self {
            client,
            api_key: api_key.into(),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub async fn search(
        &self,
        query: &str,
        media_type: MediaType,
        _language: &str,
    ) -> Result<Vec<SearchResult>, String> {
        if media_type != MediaType::Movie {
            return Ok(Vec::new());
        }
        if !self.is_configured() {
            return Err("OMDb API key missing".into());
        }
        let url = format!(
            "{BASE}?apikey={}&s={}&type=movie",
            urlencoding::encode(self.api_key.trim()),
            urlencoding::encode(query)
        );
        let data: SearchResponse = send_with_retry(self.client.get(&url))
            .await?
            .error_for_status()
            .map_err(reqwest_err)?
            .json()
            .await
            .map_err(reqwest_err)?;
        if data.response.eq_ignore_ascii_case("false") {
            return Ok(Vec::new());
        }
        Ok(data
            .search
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                let imdb = item.imdb_id?;
                let year = parse_year(item.year.as_deref());
                let title = item.title.unwrap_or_default();
                if title.is_empty() {
                    return None;
                }
                let confidence =
                    relevance_score(query, None, &title, None, year, MediaType::Movie);
                Some(SearchResult {
                    source_id: format!("omdb:{imdb}"),
                    title,
                    original_title: None,
                    year,
                    overview: None,
                    poster_url: normalize_poster(item.poster),
                    confidence,
                    media_type: MediaType::Movie,
                })
            })
            .collect())
    }

    pub async fn fetch_metadata(
        &self,
        source_id: &str,
        media_type: MediaType,
        _language: &str,
    ) -> Result<ScrapedMetadata, String> {
        if media_type != MediaType::Movie {
            return Err("OMDb only supports movies".into());
        }
        if !self.is_configured() {
            return Err("OMDb API key missing".into());
        }
        let imdb = parse_source_numeric_id(source_id)
            .filter(|s| s.starts_with("tt"))
            .or_else(|| source_id.strip_prefix("omdb:"))
            .ok_or_else(|| format!("invalid OMDb source: {source_id}"))?;
        let url = format!(
            "{BASE}?apikey={}&i={}&plot=full",
            urlencoding::encode(self.api_key.trim()),
            urlencoding::encode(imdb)
        );
        let data: DetailResponse = send_with_retry(self.client.get(&url))
            .await?
            .error_for_status()
            .map_err(reqwest_err)?
            .json()
            .await
            .map_err(reqwest_err)?;
        if data.response.as_deref().unwrap_or("").eq_ignore_ascii_case("false") {
            return Err(data
                .error
                .unwrap_or_else(|| "OMDb movie not found".into()));
        }
        let title = data.title.unwrap_or_default();
        if title.is_empty() {
            return Err("OMDb empty title".into());
        }
        let imdb_id = data.imdb_id.or_else(|| Some(imdb.to_string()));
        Ok(ScrapedMetadata {
            source_id: format!("omdb:{}", imdb_id.clone().unwrap_or_else(|| imdb.to_string())),
            title,
            original_title: None,
            year: parse_year(data.year.as_deref()),
            overview: data.plot.filter(|s| s != "N/A"),
            tagline: None,
            genres: split_csv(data.genre.as_deref()),
            tags: Vec::new(),
            rating: parse_rating(data.imdb_rating.as_deref()),
            rating_votes: parse_votes(data.imdb_votes.as_deref()),
            content_rating: data.rated.filter(|s| s != "N/A"),
            director: data.director.filter(|s| s != "N/A"),
            writer: data.writer.filter(|s| s != "N/A"),
            credits: Vec::new(),
            studio: None,
            country: data.country.filter(|s| s != "N/A"),
            language: data.language.filter(|s| s != "N/A"),
            premiered: data.released.filter(|s| s != "N/A"),
            end_date: None,
            runtime: parse_runtime(data.runtime.as_deref()),
            show_status: None,
            collection_name: None,
            collection_id: None,
            poster_url: normalize_poster(data.poster),
            fanart_url: None,
            banner_url: None,
            trailer: None,
            imdb_id,
            tmdb_id: None,
            tvdb_id: None,
            bangumi_id: None,
            seasons: Vec::new(),
        })
    }

    pub async fn fetch_artwork(
        &self,
        source_id: &str,
        media_type: MediaType,
    ) -> Result<ArtworkUrls, String> {
        let meta = self.fetch_metadata(source_id, media_type, "en").await?;
        Ok(ArtworkUrls {
            poster_url: meta.poster_url,
            fanart_url: None,
            banner_url: None,
        })
    }
}

fn normalize_poster(poster: Option<String>) -> Option<String> {
    poster.filter(|p| !p.is_empty() && p != "N/A")
}

fn parse_year(raw: Option<&str>) -> Option<i32> {
    raw.and_then(|s| s.get(0..4)?.parse().ok())
}

fn parse_rating(raw: Option<&str>) -> Option<f64> {
    raw.filter(|s| *s != "N/A").and_then(|s| s.parse().ok())
}

fn parse_votes(raw: Option<&str>) -> Option<i32> {
    raw.filter(|s| *s != "N/A")
        .and_then(|s| s.replace(',', "").parse().ok())
}

fn parse_runtime(raw: Option<&str>) -> Option<i32> {
    raw.and_then(|s| {
        let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    })
}

fn split_csv(raw: Option<&str>) -> Vec<String> {
    raw.filter(|s| *s != "N/A")
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(rename = "Search")]
    search: Option<Vec<SearchItem>>,
    #[serde(rename = "Response")]
    response: String,
}

#[derive(Debug, Deserialize)]
struct SearchItem {
    #[serde(rename = "Title")]
    title: Option<String>,
    #[serde(rename = "Year")]
    year: Option<String>,
    #[serde(rename = "imdbID")]
    imdb_id: Option<String>,
    #[serde(rename = "Poster")]
    poster: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DetailResponse {
    #[serde(rename = "Title")]
    title: Option<String>,
    #[serde(rename = "Year")]
    year: Option<String>,
    #[serde(rename = "Rated")]
    rated: Option<String>,
    #[serde(rename = "Released")]
    released: Option<String>,
    #[serde(rename = "Runtime")]
    runtime: Option<String>,
    #[serde(rename = "Genre")]
    genre: Option<String>,
    #[serde(rename = "Director")]
    director: Option<String>,
    #[serde(rename = "Writer")]
    writer: Option<String>,
    #[serde(rename = "Plot")]
    plot: Option<String>,
    #[serde(rename = "Language")]
    language: Option<String>,
    #[serde(rename = "Country")]
    country: Option<String>,
    #[serde(rename = "Poster")]
    poster: Option<String>,
    #[serde(rename = "imdbRating")]
    imdb_rating: Option<String>,
    #[serde(rename = "imdbVotes")]
    imdb_votes: Option<String>,
    #[serde(rename = "imdbID")]
    imdb_id: Option<String>,
    #[serde(rename = "Response")]
    response: Option<String>,
    #[serde(rename = "Error")]
    error: Option<String>,
}
