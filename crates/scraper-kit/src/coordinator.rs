use media_core::{MediaItem, MediaType};

use crate::bangumi::BangumiScraper;
use crate::matching::auto_accepted_result;
use crate::omdb::OmdbScraper;
use crate::tmdb::TmdbScraper;
use crate::tvdb::TvdbScraper;
use crate::types::{ScrapedMetadata, SearchResult};

#[derive(Debug, Clone, Default)]
pub struct ScraperKeys {
    pub tmdb: String,
    pub bangumi: String,
    pub omdb: String,
    pub tvdb: String,
}

#[derive(Debug, Clone)]
pub enum MatchOutcome {
    Matched(ScrapedMetadata),
    Unmatched { candidates: Vec<SearchResult> },
    Failed(String),
}

#[derive(Clone)]
pub struct ScraperCoordinator {
    tmdb: TmdbScraper,
    bangumi: BangumiScraper,
    omdb: OmdbScraper,
    tvdb: TvdbScraper,
}

impl ScraperCoordinator {
    pub fn new(keys: ScraperKeys) -> Self {
        let client = crate::http::build_client();
        Self {
            tmdb: TmdbScraper::new(client.clone(), keys.tmdb),
            bangumi: BangumiScraper::new(client.clone(), keys.bangumi),
            omdb: OmdbScraper::new(client.clone(), keys.omdb),
            tvdb: TvdbScraper::new(client, keys.tvdb),
        }
    }

    pub async fn search_manual(
        &self,
        query: &str,
        media_type: MediaType,
        language: &str,
    ) -> Result<Vec<SearchResult>, String> {
        let cleaned = media_core::FileNameParser::clean_title_for_match(query);
        let q = if cleaned.is_empty() {
            query.trim()
        } else {
            cleaned.as_str()
        };
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let sources = self.ordered(media_type);
        if sources.is_empty() {
            return Err("err.apiKey".into());
        }
        let mut all = Vec::new();
        let mut last_error: Option<String> = None;
        for scraper in sources {
            // A free-text manual query carries no year; ranking is title-only.
            match self.search_one(scraper, q, None, media_type, language).await {
                Ok(mut rows) => all.append(&mut rows),
                // Includes `rateLimited`: an all-rate-limited search must not look like "no results".
                Err(err) => {
                    last_error = Some(err);
                    continue;
                }
            }
        }
        if all.is_empty() {
            if let Some(err) = last_error {
                return Err(crate::http::humanize_error(&err));
            }
            return Ok(all);
        }
        all.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(all)
    }

    pub async fn match_item(&self, item: &MediaItem, language: &str) -> MatchOutcome {
        let sources = self.ordered(item.media_type);
        if sources.is_empty() {
            return MatchOutcome::Failed("err.apiKey".into());
        }
        let queries = build_queries(item);
        let mut best_candidates = Vec::new();
        let mut last_error: Option<String> = None;
        for query in queries {
            for scraper in &sources {
                let results = match self
                    .search_one(*scraper, &query, item.year, item.media_type, language)
                    .await
                {
                    Ok(rows) => rows,
                    // Includes `rateLimited` so the item ends `Failed(err.rateLimit)`, not Unmatched.
                    Err(err) => {
                        last_error = Some(err);
                        continue;
                    }
                };
                if results.is_empty() {
                    continue;
                }
                merge_candidates(&mut best_candidates, results.clone());
                if let Some(accepted) =
                    auto_accepted_result(&query, item.year, item.media_type, &results)
                {
                    return match self
                        .fetch_by_source(&accepted.source_id, item.media_type, language)
                        .await
                    {
                        Ok(meta) => MatchOutcome::Matched(meta),
                        Err(err) => MatchOutcome::Failed(crate::http::humanize_error(&err)),
                    };
                }
                if let Some(accepted) =
                    auto_accepted_result(&item.title, item.year, item.media_type, &results)
                {
                    return match self
                        .fetch_by_source(&accepted.source_id, item.media_type, language)
                        .await
                    {
                        Ok(meta) => MatchOutcome::Matched(meta),
                        Err(err) => MatchOutcome::Failed(crate::http::humanize_error(&err)),
                    };
                }
            }
        }
        if best_candidates.is_empty() {
            MatchOutcome::Failed(
                last_error
                    .map(|e| crate::http::humanize_error(&e))
                    .unwrap_or_else(|| "noResults".into()),
            )
        } else {
            MatchOutcome::Unmatched {
                candidates: best_candidates,
            }
        }
    }

    pub async fn fetch_by_source(
        &self,
        source_id: &str,
        media_type: MediaType,
        language: &str,
    ) -> Result<ScrapedMetadata, String> {
        if source_id.starts_with("bangumi:") {
            self.bangumi
                .fetch_metadata(source_id, media_type, language)
                .await
        } else if source_id.starts_with("tmdb:") {
            self.tmdb
                .fetch_metadata(source_id, media_type, language)
                .await
        } else if source_id.starts_with("omdb:") {
            self.omdb
                .fetch_metadata(source_id, media_type, language)
                .await
        } else if source_id.starts_with("tvdb:") {
            self.tvdb
                .fetch_metadata(source_id, media_type, language)
                .await
        } else {
            Err(format!("unknown source: {source_id}"))
        }
    }

    pub async fn fetch_artwork_urls(
        &self,
        source_id: &str,
        media_type: MediaType,
    ) -> Result<crate::types::ArtworkUrls, String> {
        if source_id.starts_with("bangumi:") {
            self.bangumi.fetch_artwork(source_id, media_type).await
        } else if source_id.starts_with("tmdb:") {
            self.tmdb.fetch_artwork(source_id, media_type).await
        } else if source_id.starts_with("omdb:") {
            self.omdb.fetch_artwork(source_id, media_type).await
        } else if source_id.starts_with("tvdb:") {
            self.tvdb.fetch_artwork(source_id, media_type).await
        } else {
            Err(format!("unknown source: {source_id}"))
        }
    }

    async fn search_one(
        &self,
        scraper: Source,
        query: &str,
        year: Option<i32>,
        media_type: MediaType,
        language: &str,
    ) -> Result<Vec<SearchResult>, String> {
        match scraper {
            Source::Bangumi => self.bangumi.search(query, year, media_type, language).await,
            Source::Tmdb => self.tmdb.search(query, year, media_type, language).await,
            Source::Omdb => self.omdb.search(query, year, media_type, language).await,
            Source::Tvdb => self.tvdb.search(query, year, media_type, language).await,
        }
    }

    fn ordered(&self, media_type: MediaType) -> Vec<Source> {
        let candidates = match media_type {
            MediaType::Anime => vec![Source::Bangumi, Source::Tmdb, Source::Tvdb],
            MediaType::TvShow => vec![Source::Tmdb, Source::Tvdb],
            MediaType::Movie => vec![Source::Tmdb, Source::Omdb],
        };
        // Skip unconfigured providers. Otherwise a missing OMDb/TVDB key becomes
        // the final `err.apiKey` even when TMDB is filled and working/empty.
        candidates
            .into_iter()
            .filter(|source| self.source_ready(*source))
            .collect()
    }

    fn source_ready(&self, source: Source) -> bool {
        match source {
            Source::Tmdb => self.tmdb.is_configured(),
            // Bangumi public search works without a token.
            Source::Bangumi => true,
            Source::Omdb => self.omdb.is_configured(),
            Source::Tvdb => self.tvdb.is_configured(),
        }
    }
}

#[derive(Clone, Copy)]
enum Source {
    Tmdb,
    Bangumi,
    Omdb,
    Tvdb,
}

/// Upper bound on candidates offered to the user after an unmatched auto-scrape.
const MAX_CANDIDATES: usize = 20;

/// Folds one query/source result set into the running candidate list:
/// dedupes by `source_id` (keeping the higher confidence), sorts best-first
/// and caps the list, so the user sees the strongest hits across all queries.
fn merge_candidates(into: &mut Vec<SearchResult>, rows: Vec<SearchResult>) {
    for row in rows {
        match into.iter_mut().find(|c| c.source_id == row.source_id) {
            Some(existing) => {
                if row.confidence > existing.confidence {
                    *existing = row;
                }
            }
            None => into.push(row),
        }
    }
    into.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    into.truncate(MAX_CANDIDATES);
}

fn build_queries(item: &MediaItem) -> Vec<String> {
    let mut queries = Vec::new();
    let cleaned = media_core::FileNameParser::clean_title_for_match(&item.title);
    if !cleaned.is_empty() {
        queries.push(cleaned);
    }
    if !item.title.is_empty() && !queries.iter().any(|q| q == &item.title) {
        queries.push(item.title.clone());
    }
    if let Some(original) = &item.original_title {
        let cleaned_original = media_core::FileNameParser::clean_title_for_match(original);
        if !cleaned_original.is_empty() && !queries.iter().any(|q| q == &cleaned_original) {
            queries.push(cleaned_original);
        }
        if !original.is_empty() && !queries.iter().any(|q| q == original) {
            queries.push(original.clone());
        }
    }
    queries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(source_id: &str, confidence: f64) -> SearchResult {
        SearchResult {
            source_id: source_id.into(),
            title: source_id.into(),
            original_title: None,
            year: None,
            overview: None,
            poster_url: None,
            confidence,
            media_type: MediaType::Movie,
        }
    }

    fn ids(rows: &[SearchResult]) -> Vec<&str> {
        rows.iter().map(|r| r.source_id.as_str()).collect()
    }

    #[test]
    fn later_weaker_result_set_does_not_replace_better_candidates() {
        let mut best = Vec::new();
        merge_candidates(&mut best, vec![row("tmdb:1", 0.95), row("tmdb:2", 0.4)]);
        merge_candidates(&mut best, vec![row("omdb:tt9", 0.3)]);
        assert_eq!(ids(&best), ["tmdb:1", "tmdb:2", "omdb:tt9"]);
    }

    #[test]
    fn duplicates_keep_highest_confidence() {
        let mut best = vec![];
        merge_candidates(&mut best, vec![row("tmdb:1", 0.5), row("tvdb:1", 0.6)]);
        merge_candidates(&mut best, vec![row("tmdb:1", 0.9)]);
        assert_eq!(ids(&best), ["tmdb:1", "tvdb:1"]);
        assert_eq!(best[0].confidence, 0.9);
        merge_candidates(&mut best, vec![row("tmdb:1", 0.1)]);
        assert_eq!(best.len(), 2);
        assert_eq!(best[0].confidence, 0.9);
    }

    #[test]
    fn candidates_are_capped() {
        let mut best = Vec::new();
        let rows = (0..MAX_CANDIDATES + 5)
            .map(|i| row(&format!("tmdb:{i}"), i as f64 / 100.0))
            .collect();
        merge_candidates(&mut best, rows);
        assert_eq!(best.len(), MAX_CANDIDATES);
        assert_eq!(best[0].source_id, format!("tmdb:{}", MAX_CANDIDATES + 4));
    }
}
