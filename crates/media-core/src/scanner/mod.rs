//! Library scanning: filename parsing and media discovery.

mod filename;
mod incremental;
mod movies;
mod refresh;
mod shows;

pub use filename::{FileNameParser, ParsedFileName};
pub use incremental::{canonicalize_lossy, strip_windows_verbatim_prefix};
pub use movies::{scan_movies, MovieScanResult, ScanProgress, MEDIA_EXTENSIONS};
pub use refresh::{refresh_items, refresh_items_cancellable, refresh_library, refresh_library_cancellable, ItemRefreshReport, RefreshReport};
pub use shows::{scan_shows, ScannedEpisode, ShowScanResult};

/// Cooperative cancellation between filesystem operations; never aborts a syscall.
fn check_cancel(cancel: &std::sync::atomic::AtomicBool) -> std::io::Result<()> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "scan cancelled"))
    } else { Ok(()) }
}
