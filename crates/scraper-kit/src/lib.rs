//! Scraper kit — TMDB / Bangumi / OMDb / TVDB clients and matching. Returns data only;
//! persisting metadata, NFO and artwork is the application's job.

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
pub use engine::{ScrapeClient, ScrapeOptions};
pub use http::{build_client, humanize_error, redact_secrets};
pub use matching::{auto_accepted_result, normalize_title, relevance_score};
pub use types::{ArtworkUrls, ScrapedEpisode, ScrapedMetadata, ScrapedSeason, SearchResult};

pub fn crate_name() -> &'static str {
    "scraper-kit"
}
