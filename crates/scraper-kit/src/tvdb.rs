//! TVDB v4 scraper for TV / anime (SCRAPE-07).

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use media_core::MediaType;
use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::http::{humanize_error, reqwest_err, send_with_retry, RATE_LIMITED};
use crate::matching::relevance_score;
use crate::types::{
    parse_source_numeric_id, ArtworkUrls, ScrapedEpisode, ScrapedMetadata, ScrapedSeason,
    SearchResult,
};

const BASE: &str = "https://api4.thetvdb.com/v4";
/// TVDB tokens last ~1 month; refresh a little early (and on any 401).
const TOKEN_TTL: Duration = Duration::from_secs(25 * 24 * 60 * 60);
/// 500 episodes per page; bounds a bad `links.next` loop.
const MAX_EPISODE_PAGES: u32 = 20;

/// Process-wide `(base|api key) -> token`; the async lock also serializes `/login`.
fn token_cache() -> &'static Mutex<HashMap<String, (Instant, String)>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, (Instant, String)>>> = OnceLock::new();
    TOKENS.get_or_init(Default::default)
}

#[derive(Clone)]
pub struct TvdbScraper {
    client: Client,
    api_key: String,
    base: String,
}

impl TvdbScraper {
    pub fn new(client: Client, api_key: impl Into<String>) -> Self {
        Self {
            client,
            api_key: api_key.into(),
            base: BASE.into(),
        }
    }

    #[cfg(test)]
    fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    /// `query_year` is the local item's year; it only biases relevance, never filters.
    pub async fn search(
        &self,
        query: &str,
        query_year: Option<i32>,
        media_type: MediaType,
        language: &str,
    ) -> Result<Vec<SearchResult>, String> {
        if !matches!(media_type, MediaType::TvShow | MediaType::Anime) {
            return Ok(Vec::new());
        }
        if !self.is_configured() {
            return Err("TVDB API key missing".into());
        }
        let url = format!(
            "{}/search?query={}&type=series",
            self.base,
            urlencoding::encode(query)
        );
        let data: ApiEnvelope<Vec<SearchHit>> = self.get_json_auth(&url, language).await?;
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
                    query_year,
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
        let series_url = format!("{}/series/{id}/extended", self.base);
        let series: ApiEnvelope<SeriesExtended> = self.get_json_auth(&series_url, language).await?;
        let series = series
            .data
            .ok_or_else(|| format!("TVDB series not found: {id}"))?;
        let (episodes, issue) = self.fetch_episodes(id, language).await?;
        let seasons = group_episodes(episodes);

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
            // TVDB v4 `score` is a popularity metric, never a 0–10 rating.
            rating: None,
            rating_votes: None,
            content_rating: series
                .content_ratings
                .unwrap_or_default()
                .into_iter()
                .find_map(|r| r.name.or(r.country)),
            director: None,
            writer: None,
            credits: Vec::new(),
            studio: series.companies.into_iter().next(),
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
            issues: issue.into_iter().collect(),
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

    /// All episode pages. Only a rate limit fails; other page errors keep what was
    /// fetched and return a humanized issue so the item is saved as partial.
    async fn fetch_episodes(
        &self,
        id: &str,
        language: &str,
    ) -> Result<(Vec<TvdbEpisode>, Option<String>), String> {
        let mut episodes = Vec::new();
        for page in 0..MAX_EPISODE_PAGES {
            let url = format!("{}/series/{id}/episodes/default?page={page}", self.base);
            let envelope: ApiEnvelope<EpisodesPayload> =
                match self.get_json_auth(&url, language).await {
                    Ok(envelope) => envelope,
                    Err(err) if err == RATE_LIMITED => return Err(err),
                    Err(err) => {
                        let issue = format!("episodes page {}: {}", page + 1, humanize_error(&err));
                        return Ok((episodes, Some(issue)));
                    }
                };
            let batch = envelope.data.and_then(|p| p.episodes).unwrap_or_default();
            let has_next = envelope
                .links
                .as_ref()
                .and_then(|l| l.get("next"))
                .is_some_and(|n| !n.is_null() && n.as_str() != Some(""));
            let last = batch.is_empty() || !has_next;
            episodes.extend(batch);
            if last {
                return Ok((episodes, None));
            }
        }
        let issue = format!("episodes: truncated after {MAX_EPISODE_PAGES} pages");
        Ok((episodes, Some(issue)))
    }

    fn token_key(&self) -> String {
        format!("{}|{}", self.base, self.api_key.trim())
    }

    /// Cached token; `stale` is a token the server just rejected, forcing a fresh login
    /// unless another task already replaced it.
    async fn token(&self, stale: Option<&str>) -> Result<String, String> {
        let key = self.token_key();
        let mut cache = token_cache().lock().await;
        if let Some((at, token)) = cache.get(&key) {
            if at.elapsed() < TOKEN_TTL && stale != Some(token.as_str()) {
                return Ok(token.clone());
            }
        }
        cache.remove(&key);
        let body = serde_json::json!({ "apikey": self.api_key.trim() });
        let resp: LoginResponse =
            send_with_retry(self.client.post(format!("{}/login", self.base)).json(&body))
                .await?
                .error_for_status()
                .map_err(reqwest_err)?
                .json()
                .await
                .map_err(reqwest_err)?;
        let token = resp
            .data
            .and_then(|d| d.token)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| "TVDB login failed".to_string())?;
        cache.insert(key, (Instant::now(), token.clone()));
        Ok(token)
    }

    async fn send_auth(&self, url: &str, token: &str, language: &str) -> Result<Response, String> {
        let accept_lang = if language.starts_with("zh") { "zho" } else { "eng" };
        let request = self
            .client
            .get(url)
            .bearer_auth(token)
            .header("Accept-Language", accept_lang);
        send_with_retry(request).await
    }

    async fn get_json_auth<T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        language: &str,
    ) -> Result<T, String> {
        let token = self.token(None).await?;
        let mut response = self.send_auth(url, &token, language).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            let token = self.token(Some(&token)).await?;
            response = self.send_auth(url, &token, language).await?;
        }
        response
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
    #[serde(default)]
    links: Option<Value>,
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
    /// Studio names; series send `[Company]`, movies `{studio: [...]}`.
    #[serde(default, deserialize_with = "deserialize_studios")]
    companies: Vec<String>,
}

fn deserialize_studios<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(studio_names(&Option::<Value>::deserialize(deserializer)?.unwrap_or_default()))
}

/// Tolerant of both company shapes; anything unexpected yields no studios.
fn studio_names(value: &Value) -> Vec<String> {
    let name = |c: &Value| c.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    match value {
        Value::Array(list) => list
            .iter()
            .filter(|c| {
                c.pointer("/companyType/companyTypeName")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.eq_ignore_ascii_case("studio"))
            })
            .filter_map(name)
            .collect(),
        Value::Object(map) => map
            .get("studio")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(name).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn series(companies: &str) -> SeriesExtended {
        let json = format!(r#"{{"status":"success","data":{{"name":"Show","companies":{companies}}}}}"#);
        serde_json::from_str::<ApiEnvelope<SeriesExtended>>(&json).unwrap().data.unwrap()
    }

    #[test]
    fn companies_parse_in_any_shape() {
        let array = r#"[{"name":"Fuji TV","companyType":{"companyTypeName":"Network"}},
            {"name":"TMS","primaryCompanyType":2,"companyType":{"companyTypeId":2,"companyTypeName":"Studio"}}]"#;
        assert_eq!(series(array).companies, vec!["TMS"]);
        assert_eq!(series(r#"{"studio":[{"name":"Madhouse"}],"network":[]}"#).companies, vec!["Madhouse"]);
        for odd in [r#""TMS""#, "42", "null", "[1,\"x\",{}]", r#"{"studio":"TMS"}"#] {
            let parsed = series(odd);
            assert_eq!(parsed.name.as_deref(), Some("Show"));
            assert!(parsed.companies.is_empty(), "{odd}");
        }
        let missing: ApiEnvelope<SeriesExtended> =
            serde_json::from_str(r#"{"data":{"name":"Show"}}"#).unwrap();
        assert!(missing.data.unwrap().companies.is_empty());
    }

    fn episodes_page(range: std::ops::Range<i32>, next: bool) -> String {
        let eps: Vec<_> = range
            .map(|n| serde_json::json!({"number": n, "seasonNumber": 1, "name": format!("E{n}")}))
            .collect();
        let next = if next { serde_json::json!("https://x/next") } else { Value::Null };
        serde_json::json!({"status":"success","data":{"episodes":eps},"links":{"next":next}}).to_string()
    }

    fn client() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn episodes_follow_pages_and_share_one_login() {
        let (base, server) = crate::http::serve_routes(4, |line| {
            if line.starts_with("POST /login") {
                r#"{"data":{"token":"tok"}}"#.into()
            } else if line.contains("page=0") {
                episodes_page(1..501, true)
            } else if line.contains("page=1") {
                episodes_page(501..1001, true)
            } else {
                episodes_page(1001..1101, false)
            }
        });
        let tvdb = TvdbScraper::new(client(), "key-pages").with_base(&base);
        let (episodes, issue) = tvdb.fetch_episodes("1", "eng").await.unwrap();
        assert_eq!(episodes.len(), 1100);
        assert!(issue.is_none());
        let seen = server.join().unwrap();
        assert_eq!(seen.iter().filter(|l| l.contains("/login")).count(), 1, "{seen:?}");
        assert_eq!(group_episodes(episodes)[0].episodes.len(), 1100);
    }

    #[tokio::test]
    async fn episode_page_failure_keeps_fetched_and_reports_issue() {
        let (base, server) = crate::http::serve_routes(3, |line| {
            if line.starts_with("POST /login") {
                r#"{"data":{"token":"tok"}}"#.into()
            } else if line.contains("page=0") {
                episodes_page(1..501, true)
            } else {
                "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
            }
        });
        let tvdb = TvdbScraper::new(client(), "key-partial").with_base(&base);
        let (episodes, issue) = tvdb.fetch_episodes("1", "eng").await.unwrap();
        server.join().unwrap();
        assert_eq!(episodes.len(), 500);
        assert!(issue.unwrap().starts_with("episodes page 2:"));
    }

    #[tokio::test]
    async fn bad_next_link_is_bounded() {
        let (base, server) =
            crate::http::serve_routes(1 + MAX_EPISODE_PAGES as usize, |line| {
                if line.starts_with("POST /login") {
                    r#"{"data":{"token":"tok"}}"#.into()
                } else {
                    episodes_page(1..2, true)
                }
            });
        let tvdb = TvdbScraper::new(client(), "key-loop").with_base(&base);
        let (episodes, issue) = tvdb.fetch_episodes("1", "eng").await.unwrap();
        server.join().unwrap();
        assert_eq!(episodes.len(), MAX_EPISODE_PAGES as usize);
        assert!(issue.unwrap().contains("truncated"));
    }

    #[tokio::test]
    async fn expired_token_relogs_once_on_401() {
        let (base, server) = crate::http::serve_routes(5, {
            let logins = std::sync::atomic::AtomicUsize::new(0);
            let gets = std::sync::atomic::AtomicUsize::new(0);
            move |line| {
                use std::sync::atomic::Ordering::SeqCst;
                if line.starts_with("POST /login") {
                    let n = logins.fetch_add(1, SeqCst);
                    format!(r#"{{"data":{{"token":"tok{n}"}}}}"#)
                } else if gets.fetch_add(1, SeqCst) == 1 {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                } else {
                    episodes_page(1..2, false)
                }
            }
        });
        let tvdb = TvdbScraper::new(client(), "key-401").with_base(&base);
        let first = tvdb.clone();
        assert_eq!(first.fetch_episodes("1", "eng").await.unwrap().0.len(), 1);
        // Fresh scraper instance reuses the process-wide token, gets 401, re-logs in once.
        let second = TvdbScraper::new(client(), "key-401").with_base(&base);
        let (episodes, issue) = second.fetch_episodes("1", "eng").await.unwrap();
        assert_eq!((episodes.len(), issue), (1, None));
        let seen = server.join().unwrap();
        assert_eq!(seen.iter().filter(|l| l.contains("/login")).count(), 2, "{seen:?}");
    }
}
