use media_core::MediaType;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::http::{humanize_error, reqwest_err, send_with_retry, RATE_LIMITED};
use crate::matching::relevance_score;
use crate::types::{
    parse_source_numeric_id, ArtworkUrls, ScrapedEpisode, ScrapedMetadata, ScrapedSeason,
    SearchResult,
};

const BASE: &str = "https://api.bgm.tv";
const EPISODE_PAGE: usize = 200;
/// Bounds pagination when `total` is wrong or missing.
const MAX_EPISODE_PAGES: usize = 20;

#[derive(Clone)]
pub struct BangumiScraper {
    client: Client,
    api_key: String,
    base: String,
}

impl BangumiScraper {
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

    /// `query_year` is the local item's year; it only biases relevance, never filters.
    pub async fn search(
        &self,
        query: &str,
        query_year: Option<i32>,
        media_type: MediaType,
        _language: &str,
    ) -> Result<Vec<SearchResult>, String> {
        if media_type != MediaType::Anime {
            return Ok(Vec::new());
        }
        let url = format!(
            "{}/search/subject/{}?type=2&responseGroup=small",
            self.base,
            urlencoding::encode(query)
        );
        let data = self.get_json(&url).await?;
        let list = data
            .get("list")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        for item in list {
            let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            let name_cn = item
                .get("name_cn")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let title = if name_cn.is_empty() {
                name.clone()
            } else {
                name_cn
            };
            let year = item
                .get("air_date")
                .and_then(|v| v.as_str())
                .and_then(|d| d.get(0..4)?.parse().ok());
            let overview = item
                .get("summary")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let poster_url = item
                .pointer("/images/large")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let confidence = relevance_score(
                query,
                query_year,
                &title,
                Some(&name),
                year,
                MediaType::Anime,
            );
            out.push(SearchResult {
                source_id: format!("bangumi:{id}"),
                title,
                original_title: if name.is_empty() { None } else { Some(name) },
                year,
                overview,
                poster_url,
                confidence,
                media_type: MediaType::Anime,
            });
        }
        Ok(out)
    }

    pub async fn fetch_metadata(
        &self,
        source_id: &str,
        _media_type: MediaType,
        _language: &str,
    ) -> Result<ScrapedMetadata, String> {
        let id = parse_source_numeric_id(source_id)
            .ok_or_else(|| format!("bad bangumi source id: {source_id}"))?;
        let url = format!("{}/v0/subjects/{id}", self.base);
        let detail: SubjectDetail = serde_json::from_value(self.get_json(&url).await?)
            .map_err(|e| e.to_string())?;
        let (episodes, issue) = self.fetch_episodes(id).await?;
        let title = detail
            .name_cn
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| detail.name.clone());
        let tags: Vec<String> = detail
            .tags
            .into_iter()
            .take(5)
            .map(|t| t.name)
            .collect();
        Ok(ScrapedMetadata {
            source_id: format!("bangumi:{id}"),
            title,
            original_title: Some(detail.name),
            year: detail
                .date
                .as_deref()
                .and_then(|d| d.get(0..4)?.parse().ok()),
            overview: detail.summary,
            tagline: None,
            genres: tags.clone(),
            tags,
            rating: detail.rating.as_ref().and_then(|r| r.score),
            rating_votes: detail.rating.as_ref().and_then(|r| r.total),
            content_rating: None,
            director: None,
            writer: None,
            credits: Vec::new(),
            studio: None,
            country: None,
            language: None,
            premiered: detail.date,
            end_date: None,
            runtime: None,
            show_status: None,
            collection_name: None,
            collection_id: None,
            poster_url: detail.images.and_then(|i| i.large.or(i.common)),
            fanart_url: None,
            banner_url: None,
            trailer: None,
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            bangumi_id: Some(id.to_string()),
            seasons: vec![ScrapedSeason {
                season_number: 1,
                title: Some("Season 1".into()),
                overview: None,
                poster_url: None,
                air_date: None,
                episode_count: Some(episodes.len() as i32),
                episodes,
            }],
            issues: issue.into_iter().collect(),
        })
    }

    pub async fn fetch_artwork(
        &self,
        source_id: &str,
        media_type: MediaType,
    ) -> Result<ArtworkUrls, String> {
        let meta = self.fetch_metadata(source_id, media_type, "zh-CN").await?;
        Ok(ArtworkUrls {
            poster_url: meta.poster_url,
            fanart_url: meta.fanart_url,
            banner_url: None,
        })
    }

    /// All episode pages. Only a rate limit fails; other page errors keep what was
    /// fetched and return a humanized issue so the item is saved as partial.
    async fn fetch_episodes(
        &self,
        subject_id: &str,
    ) -> Result<(Vec<ScrapedEpisode>, Option<String>), String> {
        let mut out = Vec::new();
        let mut offset = 0;
        for _ in 0..MAX_EPISODE_PAGES {
            let url = format!(
                "{}/v0/episodes?subject_id={subject_id}&type=0&limit={EPISODE_PAGE}&offset={offset}",
                self.base
            );
            let data = match self.get_json(&url).await {
                Ok(data) => data,
                Err(err) if err == RATE_LIMITED => return Err(err),
                Err(err) => return Ok((out, Some(format!("episodes: {}", humanize_error(&err))))),
            };
            let list = data.get("data").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let total = data.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            offset += list.len();
            let last = list.is_empty() || offset >= total;
            out.extend(list.into_iter().filter_map(parse_episode));
            if last {
                return Ok((out, None));
            }
        }
        Ok((out, Some(format!("episodes: truncated after {MAX_EPISODE_PAGES} pages"))))
    }

    async fn get_json(&self, url: &str) -> Result<Value, String> {
        let mut req = self
            .client
            .get(url)
            .header("User-Agent", "sula/0.1.0")
            .header("Accept", "application/json");
        if !self.api_key.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key.trim()));
        }
        // 429 after retries surfaces as `RATE_LIMITED` from `send_with_retry`.
        let response = send_with_retry(req).await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(format!("Bangumi HTTP {status}: {body}"));
        }
        response.json().await.map_err(reqwest_err)
    }
}

fn parse_episode(ep: Value) -> Option<ScrapedEpisode> {
    let ep_num = ep.get("ep").and_then(|v| v.as_f64()).map(|n| n as i32)?;
    Some(ScrapedEpisode {
        episode_number: ep_num,
        title: ep
            .get("name_cn")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| ep.get("name").and_then(|v| v.as_str()).map(str::to_string)),
        overview: ep.get("desc").and_then(|v| v.as_str()).map(str::to_string),
        air_date: ep.get("airdate").and_then(|v| v.as_str()).map(str::to_string),
        still_url: None,
        runtime: ep.get("duration_seconds").and_then(|v| v.as_i64()).map(|n| (n / 60) as i32),
        rating: None,
        director: None,
        writer: None,
    })
}

#[derive(Deserialize)]
struct SubjectDetail {
    name: String,
    #[serde(default)]
    name_cn: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    images: Option<Images>,
    #[serde(default)]
    rating: Option<Rating>,
    #[serde(default)]
    tags: Vec<Tag>,
}

#[derive(Deserialize)]
struct Images {
    #[serde(default)]
    large: Option<String>,
    #[serde(default)]
    common: Option<String>,
}

#[derive(Deserialize)]
struct Rating {
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    total: Option<i32>,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(line: &str, total: usize) -> String {
        let offset: usize = line
            .split("offset=")
            .nth(1)
            .and_then(|s| s.split([' ', '&']).next()?.parse().ok())
            .unwrap();
        let eps: Vec<_> = (offset..total.min(offset + EPISODE_PAGE))
            .map(|n| serde_json::json!({"ep": n + 1, "name": format!("E{}", n + 1)}))
            .collect();
        serde_json::json!({"data": eps, "total": total, "limit": EPISODE_PAGE, "offset": offset}).to_string()
    }

    fn scraper(base: &str) -> BangumiScraper {
        BangumiScraper::new(Client::builder().no_proxy().build().unwrap(), "").with_base(base)
    }

    #[tokio::test]
    async fn episodes_paginate_by_offset_until_total() {
        let (base, server) = crate::http::serve_routes(6, |line| page(line, 1100));
        let (episodes, issue) = scraper(&base).fetch_episodes("975").await.unwrap();
        let seen = server.join().unwrap();
        assert_eq!(episodes.len(), 1100);
        assert_eq!(episodes.last().unwrap().episode_number, 1100);
        assert!(issue.is_none());
        assert!(seen[5].contains("offset=1000"), "{seen:?}");
    }

    #[tokio::test]
    async fn later_page_failure_is_partial_and_bogus_total_is_bounded() {
        let (base, server) = crate::http::serve_routes(2, |line| {
            if line.contains("offset=0") {
                page(line, 400)
            } else {
                "HTTP/1.1 500 Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
            }
        });
        let (episodes, issue) = scraper(&base).fetch_episodes("1").await.unwrap();
        server.join().unwrap();
        assert_eq!(episodes.len(), EPISODE_PAGE);
        assert!(issue.unwrap().starts_with("episodes:"));

        let (base, server) = crate::http::serve_routes(MAX_EPISODE_PAGES, |line| page(line, usize::MAX));
        let (episodes, issue) = scraper(&base).fetch_episodes("1").await.unwrap();
        server.join().unwrap();
        assert_eq!(episodes.len(), EPISODE_PAGE * MAX_EPISODE_PAGES);
        assert!(issue.unwrap().contains("truncated"));
    }
}
