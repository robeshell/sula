use media_core::MediaType;
use reqwest::Client;
use serde::Deserialize;

use crate::http::{humanize_error, reqwest_err, send_with_retry, RATE_LIMITED};
use crate::matching::relevance_score;
use crate::types::{
    parse_source_numeric_id, ArtworkUrls, ScrapedEpisode, ScrapedMetadata, ScrapedSeason,
    SearchResult,
};

const BASE: &str = "https://api.themoviedb.org/3";
const IMAGE_BASE: &str = "https://image.tmdb.org/t/p/original";

#[derive(Clone)]
pub struct TmdbScraper {
    client: Client,
    api_key: String,
}

impl TmdbScraper {
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
        language: &str,
    ) -> Result<Vec<SearchResult>, String> {
        if !self.is_configured() {
            return Err("TMDB API key missing".into());
        }
        let uses_tv = matches!(media_type, MediaType::TvShow | MediaType::Anime);
        let path = if uses_tv { "/search/tv" } else { "/search/movie" };
        let url = format!(
            "{BASE}{path}?query={}&language={}",
            urlencoding::encode(query),
            urlencoding::encode(language)
        );
        let data = self.get_json(&url).await?;
        if uses_tv {
            let response: TvSearchResponse =
                serde_json::from_value(data).map_err(|e| e.to_string())?;
            Ok(response
                .results
                .into_iter()
                .map(|item| {
                    let year = extract_year(item.first_air_date.as_deref());
                    let confidence = relevance_score(
                        query,
                        None,
                        &item.name,
                        item.original_name.as_deref(),
                        year,
                        media_type,
                    );
                    SearchResult {
                        source_id: format!("tmdb:{}", item.id),
                        title: item.name,
                        original_title: item.original_name,
                        year,
                        overview: item.overview,
                        poster_url: item.poster_path.map(|p| format!("{IMAGE_BASE}{p}")),
                        confidence,
                        media_type,
                    }
                })
                .collect())
        } else {
            let response: MovieSearchResponse =
                serde_json::from_value(data).map_err(|e| e.to_string())?;
            Ok(response
                .results
                .into_iter()
                .map(|item| {
                    let year = extract_year(item.release_date.as_deref());
                    let confidence = relevance_score(
                        query,
                        None,
                        &item.title,
                        item.original_title.as_deref(),
                        year,
                        media_type,
                    );
                    SearchResult {
                        source_id: format!("tmdb:{}", item.id),
                        title: item.title,
                        original_title: item.original_title,
                        year,
                        overview: item.overview,
                        poster_url: item.poster_path.map(|p| format!("{IMAGE_BASE}{p}")),
                        confidence,
                        media_type,
                    }
                })
                .collect())
        }
    }

    pub async fn fetch_metadata(
        &self,
        source_id: &str,
        media_type: MediaType,
        language: &str,
    ) -> Result<ScrapedMetadata, String> {
        let id = parse_source_numeric_id(source_id)
            .ok_or_else(|| format!("bad tmdb source id: {source_id}"))?;
        let uses_tv = matches!(media_type, MediaType::TvShow | MediaType::Anime);
        let path = if uses_tv {
            format!("/tv/{id}")
        } else {
            format!("/movie/{id}")
        };
        let append = if uses_tv {
            "credits,external_ids,content_ratings,videos,keywords"
        } else {
            "credits,external_ids,videos,keywords,release_dates"
        };
        let url = format!(
            "{BASE}{path}?language={}&append_to_response={append}",
            urlencoding::encode(language)
        );
        let data = self.get_json(&url).await?;
        if uses_tv {
            let detail: TvDetail = serde_json::from_value(data).map_err(|e| e.to_string())?;
            let (seasons, issue) = self.fetch_seasons(id, &detail.seasons, language).await?;
            let mut meta = map_tv_detail(detail, seasons);
            meta.issues.extend(issue);
            Ok(meta)
        } else {
            let detail: MovieDetail = serde_json::from_value(data).map_err(|e| e.to_string())?;
            Ok(map_movie_detail(detail))
        }
    }

    pub async fn fetch_artwork(
        &self,
        source_id: &str,
        media_type: MediaType,
    ) -> Result<ArtworkUrls, String> {
        let id = parse_source_numeric_id(source_id)
            .ok_or_else(|| format!("bad tmdb source id: {source_id}"))?;
        let uses_tv = matches!(media_type, MediaType::TvShow | MediaType::Anime);
        let path = if uses_tv {
            format!("/tv/{id}/images")
        } else {
            format!("/movie/{id}/images")
        };
        let url = format!("{BASE}{path}");
        let data = self.get_json(&url).await?;
        let images: ImageResponse = serde_json::from_value(data).map_err(|e| e.to_string())?;
        Ok(ArtworkUrls {
            poster_url: images
                .posters
                .first()
                .map(|p| format!("{IMAGE_BASE}{}", p.file_path)),
            fanart_url: images
                .backdrops
                .first()
                .map(|p| format!("{IMAGE_BASE}{}", p.file_path)),
            banner_url: None,
        })
    }

    async fn fetch_seasons(
        &self,
        tv_id: &str,
        seasons: &[TvSeasonStub],
        language: &str,
    ) -> Result<(Vec<ScrapedSeason>, Option<String>), String> {
        let mut out = Vec::new();
        let mut failed = Vec::new();
        for stub in seasons {
            if stub.season_number < 0 {
                continue;
            }
            // Keep good seasons and report broken ones; a rate limit fails the whole fetch (retryable).
            match self.fetch_season(tv_id, stub.season_number, language).await {
                Ok(season) => out.push(season),
                Err(err) if err == RATE_LIMITED => return Err(err),
                Err(err) => failed.push((stub.season_number, err)),
            }
        }
        Ok((out, seasons_issue(&failed)))
    }

    /// Fetch a single TV season (title / overview / poster / episodes).
    pub async fn fetch_season(
        &self,
        tv_id: &str,
        season_number: i32,
        language: &str,
    ) -> Result<ScrapedSeason, String> {
        if !self.is_configured() {
            return Err("err.apiKey".into());
        }
        let url = format!(
            "{BASE}/tv/{tv_id}/season/{season_number}?language={}",
            urlencoding::encode(language)
        );
        let data = self.get_json(&url).await?;
        let detail: SeasonDetail = serde_json::from_value(data).map_err(|e| e.to_string())?;
        Ok(ScrapedSeason {
            season_number: detail.season_number,
            title: detail.name,
            overview: detail.overview,
            poster_url: detail.poster_path.map(|p| format!("{IMAGE_BASE}{p}")),
            air_date: detail.air_date,
            episode_count: Some(detail.episodes.len() as i32),
            episodes: detail
                .episodes
                .into_iter()
                .map(|ep| ScrapedEpisode {
                    episode_number: ep.episode_number,
                    title: ep.name,
                    overview: ep.overview,
                    air_date: ep.air_date,
                    still_url: ep.still_path.map(|p| format!("{IMAGE_BASE}{p}")),
                    runtime: ep.runtime,
                    rating: ep.vote_average,
                    director: None,
                    writer: None,
                })
                .collect(),
        })
    }

    async fn get_json(&self, url: &str) -> Result<serde_json::Value, String> {
        let key = self.api_key.trim();
        // v4 Read Access Token (JWT) → Bearer; v3 API Key → query `api_key=`.
        let request = if key.starts_with("eyJ") {
            self.client
                .get(url)
                .header("Authorization", format!("Bearer {key}"))
        } else {
            let sep = if url.contains('?') { '&' } else { '?' };
            let url = format!("{url}{sep}api_key={}", urlencoding::encode(key));
            self.client.get(url)
        };
        // 429 after retries surfaces as `RATE_LIMITED` from `send_with_retry`.
        let response = send_with_retry(request.header("Accept", "application/json")).await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("TMDB HTTP {status}: {body}"));
        }
        response.json().await.map_err(reqwest_err)
    }
}

/// `seasons 2, 5: err.connect` (first error, at most 10 numbers listed).
fn seasons_issue(failed: &[(i32, String)]) -> Option<String> {
    let (_, first) = failed.first()?;
    let mut nums: Vec<String> = failed.iter().take(10).map(|(n, _)| n.to_string()).collect();
    if failed.len() > 10 {
        nums.push("…".into());
    }
    Some(format!("seasons {}: {}", nums.join(", "), humanize_error(first)))
}

fn extract_year(date: Option<&str>) -> Option<i32> {
    date.and_then(|d| d.get(0..4)?.parse().ok())
}

fn map_movie_detail(detail: MovieDetail) -> ScrapedMetadata {
    let director = detail
        .credits
        .as_ref()
        .and_then(|c| c.crew.iter().find(|x| x.job.as_deref() == Some("Director")))
        .and_then(|x| x.name.clone());
    let writers = detail
        .credits
        .as_ref()
        .map(|c| {
            c.crew
                .iter()
                .filter(|x| {
                    matches!(
                        x.job.as_deref(),
                        Some("Writer" | "Screenplay" | "Story")
                    )
                })
                .filter_map(|x| x.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty());
    ScrapedMetadata {
        source_id: format!("tmdb:{}", detail.id),
        title: detail.title,
        original_title: detail.original_title,
        year: extract_year(detail.release_date.as_deref()),
        overview: detail.overview,
        tagline: detail.tagline,
        genres: detail.genres.into_iter().map(|g| g.name).collect(),
        tags: detail
            .keywords
            .as_ref()
            .map(|k| k.keywords.iter().map(|x| x.name.clone()).collect())
            .unwrap_or_default(),
        rating: detail.vote_average,
        rating_votes: detail.vote_count,
        content_rating: None,
        director,
        writer: writers,
        credits: detail
            .credits
            .map(|c| {
                c.cast
                    .into_iter()
                    .take(20)
                    .map(|a| media_core::CastMember {
                        name: a.name.unwrap_or_default(),
                        role: a.character,
                        r#type: Some("Actor".into()),
                        thumb_url: a.profile_path.map(|p| format!("https://image.tmdb.org/t/p/w185{p}")),
                        order: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        studio: detail
            .production_companies
            .and_then(|c| c.into_iter().next().map(|x| x.name)),
        country: detail
            .production_countries
            .and_then(|c| c.into_iter().next().map(|x| x.iso_3166_1)),
        language: detail.original_language,
        premiered: detail.release_date,
        end_date: None,
        runtime: detail.runtime,
        show_status: None,
        collection_name: detail
            .belongs_to_collection
            .as_ref()
            .map(|c| c.name.clone()),
        collection_id: detail
            .belongs_to_collection
            .as_ref()
            .map(|c| c.id.to_string()),
        poster_url: detail.poster_path.map(|p| format!("{IMAGE_BASE}{p}")),
        fanart_url: detail.backdrop_path.map(|p| format!("{IMAGE_BASE}{p}")),
        banner_url: None,
        trailer: extract_trailer(detail.videos.as_ref()),
        imdb_id: detail.external_ids.and_then(|e| e.imdb_id),
        tmdb_id: Some(detail.id.to_string()),
        tvdb_id: None,
        bangumi_id: None,
        seasons: Vec::new(),
        issues: Vec::new(),
    }
}

fn map_tv_detail(detail: TvDetail, seasons: Vec<ScrapedSeason>) -> ScrapedMetadata {
    ScrapedMetadata {
        source_id: format!("tmdb:{}", detail.id),
        title: detail.name,
        original_title: detail.original_name,
        year: extract_year(detail.first_air_date.as_deref()),
        overview: detail.overview,
        tagline: detail.tagline,
        genres: detail.genres.into_iter().map(|g| g.name).collect(),
        tags: detail
            .keywords
            .as_ref()
            .map(|k| k.results.iter().map(|x| x.name.clone()).collect())
            .unwrap_or_default(),
        rating: detail.vote_average,
        rating_votes: detail.vote_count,
        content_rating: None,
        director: None,
        writer: None,
        credits: detail
            .credits
            .map(|c| {
                c.cast
                    .into_iter()
                    .take(20)
                    .map(|a| media_core::CastMember {
                        name: a.name.unwrap_or_default(),
                        role: a.character,
                        r#type: Some("Actor".into()),
                        thumb_url: a.profile_path.map(|p| format!("https://image.tmdb.org/t/p/w185{p}")),
                        order: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        studio: detail.networks.and_then(|n| n.into_iter().next().map(|x| x.name)),
        country: detail.origin_country.and_then(|c| c.into_iter().next()),
        language: detail.original_language,
        premiered: detail.first_air_date,
        end_date: detail.last_air_date,
        runtime: detail.episode_run_time.and_then(|r| r.into_iter().next()),
        show_status: detail.status,
        collection_name: None,
        collection_id: None,
        poster_url: detail.poster_path.map(|p| format!("{IMAGE_BASE}{p}")),
        fanart_url: detail.backdrop_path.map(|p| format!("{IMAGE_BASE}{p}")),
        banner_url: None,
        trailer: extract_trailer(detail.videos.as_ref()),
        imdb_id: detail.external_ids.as_ref().and_then(|e| e.imdb_id.clone()),
        tmdb_id: Some(detail.id.to_string()),
        tvdb_id: detail
            .external_ids
            .and_then(|e| e.tvdb_id.map(|id| id.to_string())),
        bangumi_id: None,
        seasons,
        issues: Vec::new(),
    }
}

fn extract_trailer(videos: Option<&Videos>) -> Option<String> {
    videos.and_then(|v| {
        v.results.iter().find(|x| {
            x.site.as_deref() == Some("YouTube") && x.r#type.as_deref() == Some("Trailer")
        })
    })
    .and_then(|x| x.key.as_ref())
    .map(|k| format!("https://www.youtube.com/watch?v={k}"))
}

#[derive(Deserialize)]
struct MovieSearchResponse {
    results: Vec<MovieSearchItem>,
}
#[derive(Deserialize)]
struct MovieSearchItem {
    id: i64,
    title: String,
    #[serde(default)]
    original_title: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
}
#[derive(Deserialize)]
struct TvSearchResponse {
    results: Vec<TvSearchItem>,
}
#[derive(Deserialize)]
struct TvSearchItem {
    id: i64,
    name: String,
    #[serde(default)]
    original_name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    first_air_date: Option<String>,
}

#[derive(Deserialize)]
struct MovieDetail {
    id: i64,
    title: String,
    #[serde(default)]
    original_title: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    tagline: Option<String>,
    #[serde(default)]
    genres: Vec<Named>,
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    vote_count: Option<i32>,
    #[serde(default)]
    runtime: Option<i32>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    original_language: Option<String>,
    #[serde(default)]
    credits: Option<Credits>,
    #[serde(default)]
    keywords: Option<MovieKeywords>,
    #[serde(default)]
    videos: Option<Videos>,
    #[serde(default)]
    external_ids: Option<ExternalIds>,
    #[serde(default)]
    production_companies: Option<Vec<Named>>,
    #[serde(default)]
    production_countries: Option<Vec<Country>>,
    #[serde(default)]
    belongs_to_collection: Option<Collection>,
}

#[derive(Deserialize)]
struct TvDetail {
    id: i64,
    name: String,
    #[serde(default)]
    original_name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    tagline: Option<String>,
    #[serde(default)]
    genres: Vec<Named>,
    #[serde(default)]
    vote_average: Option<f64>,
    #[serde(default)]
    vote_count: Option<i32>,
    #[serde(default)]
    first_air_date: Option<String>,
    #[serde(default)]
    last_air_date: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    backdrop_path: Option<String>,
    #[serde(default)]
    original_language: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    episode_run_time: Option<Vec<i32>>,
    #[serde(default)]
    origin_country: Option<Vec<String>>,
    #[serde(default)]
    networks: Option<Vec<Named>>,
    #[serde(default)]
    credits: Option<Credits>,
    #[serde(default)]
    keywords: Option<TvKeywords>,
    #[serde(default)]
    videos: Option<Videos>,
    #[serde(default)]
    external_ids: Option<ExternalIds>,
    #[serde(default)]
    seasons: Vec<TvSeasonStub>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}
#[derive(Deserialize)]
struct Country {
    iso_3166_1: String,
}
#[derive(Deserialize)]
struct Collection {
    id: i64,
    name: String,
}
#[derive(Deserialize)]
struct Credits {
    #[serde(default)]
    cast: Vec<CastItem>,
    #[serde(default)]
    crew: Vec<CrewItem>,
}
#[derive(Deserialize)]
struct CastItem {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    character: Option<String>,
    #[serde(default)]
    profile_path: Option<String>,
}
#[derive(Deserialize)]
struct CrewItem {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    job: Option<String>,
}
#[derive(Deserialize)]
struct MovieKeywords {
    #[serde(default)]
    keywords: Vec<Named>,
}
#[derive(Deserialize)]
struct TvKeywords {
    #[serde(default)]
    results: Vec<Named>,
}
#[derive(Deserialize)]
struct Videos {
    #[serde(default)]
    results: Vec<VideoItem>,
}
#[derive(Deserialize)]
struct VideoItem {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    site: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
}
#[derive(Deserialize)]
struct ExternalIds {
    #[serde(default)]
    imdb_id: Option<String>,
    #[serde(default)]
    tvdb_id: Option<i64>,
}
#[derive(Deserialize)]
struct TvSeasonStub {
    season_number: i32,
}
#[derive(Deserialize)]
struct SeasonDetail {
    season_number: i32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    poster_path: Option<String>,
    #[serde(default)]
    air_date: Option<String>,
    #[serde(default)]
    episodes: Vec<EpisodeDetail>,
}
#[derive(Deserialize)]
struct EpisodeDetail {
    episode_number: i32,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    overview: Option<String>,
    #[serde(default)]
    air_date: Option<String>,
    #[serde(default)]
    still_path: Option<String>,
    #[serde(default)]
    runtime: Option<i32>,
    #[serde(default)]
    vote_average: Option<f64>,
}
#[derive(Deserialize)]
struct ImageResponse {
    #[serde(default)]
    posters: Vec<ImageItem>,
    #[serde(default)]
    backdrops: Vec<ImageItem>,
}
#[derive(Deserialize)]
struct ImageItem {
    file_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_seasons_become_humanized_issue() {
        assert_eq!(seasons_issue(&[]), None);
        let failed = [(2, "error sending request".to_string()), (5, "TMDB HTTP 404".to_string())];
        assert_eq!(seasons_issue(&failed).as_deref(), Some("seasons 2, 5: err.connect"));
    }
}
