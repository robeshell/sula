//! TVDB v4 scraper for TV / anime (SCRAPE-07).

use std::sync::Arc;

use media_core::MediaType;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::http::{reqwest_err, send_with_retry, RATE_LIMITED};
use crate::matching::relevance_score;
use crate::types::{
    parse_source_numeric_id, ArtworkUrls, ScrapedEpisode, ScrapedMetadata, ScrapedSeason,
    SearchResult,
};

const BASE: &str = "https://api4.thetvdb.com/v4";

#[derive(Clone)]
pub struct TvdbScraper {
    client: Client,
    api_key: String,
    token: Arc<Mutex<Option<String>>>,
}

impl TvdbScraper {
    pub fn new(client: Client, api_key: impl Into<String>) -> Self {
        Self {
            client,
            api_key: api_key.into(),
            token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub async fn search(
        &self,
        query: &str,
        media_type: MediaType,
        language: &str,
    ) -> Result<Vec<SearchResult>, String> {
        if !matches!(media_type, MediaType::TvShow | MediaType::Anime) {
            return Ok(Vec::new());
        }
        if !self.is_configured() {
            return Err("TVDB API key missing".into());
        }
        let token = self.ensure_token().await?;
        let url = format!(
            "{BASE}/search?query={}&type=series",
            urlencoding::encode(query)
        );
        let data: ApiEnvelope<Vec<SearchHit>> = self
            .get_json_auth(&url, &token, language)
            .await?;
        let rows = data.data.unwrap_or_default();
        Ok(rows
            .into_iter()
            .filter_map(|hit| {
                let id = hit.tvdb_id.or(hit.id)?;
                let title = hit.name.unwrap_or_default();
                if title.is_empty() {
                    return None;
                }
                let year = hit.year.and_then(|y| y.parse().ok()).or_else(|| {
                    hit.first_air_time
                        .as_deref()
                        .and_then(|d| d.get(0..4)?.parse().ok())
                });
                let confidence = relevance_score(
                    query,
                    None,
                    &title,
                    eng_translation(hit.translations.clone()).as_deref(),
                    year,
                    media_type,
                );
                Some(SearchResult {
                    source_id: format!("tvdb:{id}"),
                    title,
                    original_title: eng_translation(hit.translations),
                    year,
                    overview: hit.overview,
                    poster_url: hit.image_url.or(hit.thumbnail),
                    confidence,
                    media_type,
                })
            })
            .collect())
    }

    pub async fn fetch_metadata(
        &self,
        source_id: &str,
        media_type: MediaType,
        language: &str,
    ) -> Result<ScrapedMetadata, String> {
        if !matches!(media_type, MediaType::TvShow | MediaType::Anime) {
            return Err("TVDB only supports TV/anime".into());
        }
        if !self.is_configured() {
            return Err("TVDB API key missing".into());
        }
        let id = parse_source_numeric_id(source_id)
            .ok_or_else(|| format!("invalid TVDB source: {source_id}"))?;
        let token = self.ensure_token().await?;
        let series_url = format!("{BASE}/series/{id}/extended");
        let series: ApiEnvelope<SeriesExtended> = self
            .get_json_auth(&series_url, &token, language)
            .await?;
        let series = series
            .data
            .ok_or_else(|| format!("TVDB series not found: {id}"))?;

        let episodes_url = format!("{BASE}/series/{id}/episodes/default?page=0");
        // Episodes are best-effort, but a rate limit must fail the fetch (retryable)
        // instead of producing a show with no seasons.
        let episodes_page: ApiEnvelope<EpisodesPayload> = match self
            .get_json_auth(&episodes_url, &token, language)
            .await
        {
            Ok(page) => page,
            Err(err) if err == RATE_LIMITED => return Err(err),
            Err(_) => ApiEnvelope { data: None, status: None },
        };
        let seasons = group_episodes(episodes_page.data.and_then(|p| p.episodes).unwrap_or_default());

        let title = series.name.unwrap_or_else(|| format!("TVDB {id}"));
        let year = series.year.and_then(|y| y.parse().ok()).or_else(|| {
            series
                .first_aired
                .as_deref()
                .and_then(|d| d.get(0..4)?.parse().ok())
        });
        let artwork = pick_artwork(&series.artworks);
        Ok(ScrapedMetadata {
            source_id: format!("tvdb:{id}"),
            title,
            original_title: series.original_name,
            year,
            overview: series.overview,
            tagline: None,
            genres: series
                .genres
                .unwrap_or_default()
                .into_iter()
                .filter_map(|g| g.name)
                .collect(),
            tags: Vec::new(),
            rating: series.score,
            rating_votes: None,
            content_rating: series
                .content_ratings
                .unwrap_or_default()
                .into_iter()
                .find_map(|r| r.name.or(r.country)),
            director: None,
            writer: None,
            credits: Vec::new(),
            studio: series
                .companies
                .as_ref()
                .and_then(|c| c.studio.as_ref())
                .and_then(|list| list.first())
                .and_then(|c| c.name.clone()),
            country: series
                .original_country
                .filter(|s| !s.is_empty()),
            language: series.original_language,
            premiered: series.first_aired,
            end_date: series.last_aired,
            runtime: series.average_runtime.and_then(|r| {
                if r > 0.0 {
                    Some(r.round() as i32)
                } else {
                    None
                }
            }),
            show_status: series.status.and_then(|s| s.name),
            collection_name: None,
            collection_id: None,
            poster_url: artwork.poster_url.or(series.image),
            fanart_url: artwork.fanart_url,
            banner_url: artwork.banner_url,
            trailer: None,
            imdb_id: series
                .remote_ids
                .unwrap_or_default()
                .into_iter()
                .find(|r| {
                    r.source_name
                        .as_deref()
                        .map(|n| n.eq_ignore_ascii_case("IMDB"))
                        .unwrap_or(false)
                })
                .and_then(|r| r.id),
            tmdb_id: None,
            tvdb_id: Some(id.to_string()),
            bangumi_id: None,
            seasons,
        })
    }

    pub async fn fetch_artwork(
        &self,
        source_id: &str,
        media_type: MediaType,
    ) -> Result<ArtworkUrls, String> {
        let meta = self.fetch_metadata(source_id, media_type, "eng").await?;
        Ok(ArtworkUrls {
            poster_url: meta.poster_url,
            fanart_url: meta.fanart_url,
            banner_url: meta.banner_url,
        })
    }

    async fn ensure_token(&self) -> Result<String, String> {
        {
            let guard = self.token.lock().await;
            if let Some(t) = guard.as_ref() {
                if !t.is_empty() {
                    return Ok(t.clone());
                }
            }
        }
        let body = serde_json::json!({ "apikey": self.api_key.trim() });
        let resp: LoginResponse =
            send_with_retry(self.client.post(format!("{BASE}/login")).json(&body))
                .await?
                .error_for_status()
                .map_err(reqwest_err)?
                .json()
                .await
                .map_err(reqwest_err)?;
        let token = resp
            .data
            .and_then(|d| d.token)
            .ok_or_else(|| "TVDB login failed".to_string())?;
        *self.token.lock().await = Some(token.clone());
        Ok(token)
    }

    async fn get_json_auth<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        token: &str,
        language: &str,
    ) -> Result<T, String> {
        let accept_lang = if language.starts_with("zh") {
            "zho"
        } else if language.starts_with("en") {
            "eng"
        } else {
            "eng"
        };
        let request = self
            .client
            .get(url)
            .bearer_auth(token)
            .header("Accept-Language", accept_lang);
        send_with_retry(request)
            .await?
            .error_for_status()
            .map_err(reqwest_err)?
            .json()
            .await
            .map_err(reqwest_err)
    }
}

fn group_episodes(episodes: Vec<TvdbEpisode>) -> Vec<ScrapedSeason> {
    use std::collections::BTreeMap;
    let mut by_season: BTreeMap<i32, Vec<ScrapedEpisode>> = BTreeMap::new();
    for ep in episodes {
        let season = ep.season_number.unwrap_or(1);
        let number = ep.number.unwrap_or(0);
        if number <= 0 {
            continue;
        }
        by_season.entry(season).or_default().push(ScrapedEpisode {
            episode_number: number,
            title: ep.name,
            overview: ep.overview,
            air_date: ep.aired,
            still_url: ep.image,
            runtime: ep.runtime.and_then(|r| {
                if r > 0 {
                    Some(r)
                } else {
                    None
                }
            }),
            rating: None,
            director: None,
            writer: None,
        });
    }
    by_season
        .into_iter()
        .map(|(season_number, mut episodes)| {
            episodes.sort_by_key(|e| e.episode_number);
            let episode_count = Some(episodes.len() as i32);
            ScrapedSeason {
                season_number,
                title: Some(format!("Season {season_number}")),
                overview: None,
                poster_url: None,
                air_date: episodes.first().and_then(|e| e.air_date.clone()),
                episode_count,
                episodes,
            }
        })
        .collect()
}

fn pick_artwork(artworks: &Option<Vec<Artwork>>) -> ArtworkUrls {
    let mut out = ArtworkUrls::default();
    let Some(list) = artworks else {
        return out;
    };
    for art in list {
        let url = art.image.as_ref().or(art.thumbnail.as_ref());
        let Some(url) = url else { continue };
        // TVDB artwork type ids: 2=poster-ish, 3=banner, 7/15 fanart vary; use typeName fallback.
        let ty = art
            .type_name
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        if out.poster_url.is_none() && (ty.contains("poster") || art.r#type == Some(2)) {
            out.poster_url = Some(url.clone());
        } else if out.fanart_url.is_none()
            && (ty.contains("background") || ty.contains("fanart") || art.r#type == Some(3))
        {
            out.fanart_url = Some(url.clone());
        } else if out.banner_url.is_none() && ty.contains("banner") {
            out.banner_url = Some(url.clone());
        }
    }
    out
}

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    data: Option<T>,
    #[allow(dead_code)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginResponse {
    data: Option<LoginData>,
}

#[derive(Debug, Deserialize)]
struct LoginData {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    #[serde(default, deserialize_with = "deserialize_opt_i64")]
    tvdb_id: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_opt_i64")]
    id: Option<i64>,
    name: Option<String>,
    overview: Option<String>,
    year: Option<String>,
    #[serde(alias = "first_air_time")]
    first_air_time: Option<String>,
    image_url: Option<String>,
    thumbnail: Option<String>,
    #[serde(default)]
    translations: Option<serde_json::Value>,
}

fn eng_translation(value: Option<serde_json::Value>) -> Option<String> {
    let value = value?;
    value
        .get("eng")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .as_object()
                .and_then(|m| m.values().find_map(|v| v.as_str().map(str::to_string)))
        })
}

fn deserialize_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Number(n)) => n.as_i64(),
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        Some(_) => None,
    })
}

#[derive(Debug, Deserialize)]
struct SeriesExtended {
    name: Option<String>,
    #[serde(alias = "originalName")]
    original_name: Option<String>,
    overview: Option<String>,
    year: Option<String>,
    #[serde(alias = "firstAired")]
    first_aired: Option<String>,
    #[serde(alias = "lastAired")]
    last_aired: Option<String>,
    score: Option<f64>,
    image: Option<String>,
    #[serde(alias = "averageRuntime")]
    average_runtime: Option<f64>,
    #[serde(alias = "originalCountry")]
    original_country: Option<String>,
    #[serde(alias = "originalLanguage")]
    original_language: Option<String>,
    genres: Option<Vec<Named>>,
    status: Option<Named>,
    artworks: Option<Vec<Artwork>>,
    #[serde(alias = "contentRatings")]
    content_ratings: Option<Vec<Named>>,
    #[serde(alias = "remoteIds")]
    remote_ids: Option<Vec<RemoteId>>,
    companies: Option<Companies>,
}

#[derive(Debug, Deserialize)]
struct Named {
    name: Option<String>,
    #[allow(dead_code)]
    country: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Artwork {
    image: Option<String>,
    thumbnail: Option<String>,
    #[serde(rename = "type")]
    r#type: Option<i32>,
    #[serde(alias = "typeName")]
    type_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteId {
    id: Option<String>,
    #[serde(alias = "sourceName")]
    source_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Companies {
    studio: Option<Vec<Named>>,
}

#[derive(Debug, Deserialize)]
struct EpisodesPayload {
    episodes: Option<Vec<TvdbEpisode>>,
}

#[derive(Debug, Deserialize)]
struct TvdbEpisode {
    name: Option<String>,
    overview: Option<String>,
    number: Option<i32>,
    #[serde(alias = "seasonNumber")]
    season_number: Option<i32>,
    aired: Option<String>,
    image: Option<String>,
    runtime: Option<i32>,
}
