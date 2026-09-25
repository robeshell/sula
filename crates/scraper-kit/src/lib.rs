//! Scraper kit — TMDB / Bangumi / OMDb / TVDB matching and scrape pipeline.

pub mod artwork;
pub mod bangumi;
pub mod coordinator;
pub mod engine;
pub mod http;
pub mod matching;
pub mod omdb;
pub mod tmdb;
pub mod tvdb;
pub mod types;

pub use coordinator::{MatchOutcome, ScraperCoordinator, ScraperKeys};
pub use engine::{
    apply_manual_match, scrape_item, scrape_library, scrape_library_cancellable, scrape_season, ScrapeItemOutcome,
    ScrapeOptions, ScrapeProgress, ScrapeSummary,
};
pub use http::{build_client, humanize_error};
pub use matching::{auto_accepted_result, normalize_title, relevance_score};
pub use types::{ArtworkUrls, ScrapedEpisode, ScrapedMetadata, ScrapedSeason, SearchResult};

pub fn crate_name() -> &'static str {
    "scraper-kit"
}
