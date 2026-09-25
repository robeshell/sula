//! sula media domain core.
//!
//! M0: models, SQLite schema/migrations, filesystem mutation layer.
//! M1: library CRUD, scanning, NFO import-on-refresh.

pub mod cleanup;
pub mod db;
pub mod filesystem;
pub mod models;
pub mod media_files;
pub mod nfo;
pub mod scanner;
pub mod avatar;
pub mod thumbnail;

pub use avatar::{AvatarCache, AvatarError};
pub use cleanup::{
    companion_suffix, find_residuals, perform_cleanup, CleanupError, ResidualCandidate,
    COMPANION_EXTENSIONS,
};
pub use db::{AppDatabase, DatabaseError, DirectoryScanState, MediaMetaSummary, ShowListStats};
pub use filesystem::{
    CollisionPolicy, FilesystemChangeSet, FilesystemError, FilesystemMoveRecord,
    FilesystemService, RemovalStrategy, WriteOptions, entry_name_exists, is_case_only_rename,
};
pub use models::{
    CastMember, Library, MediaItem, MediaMetadata, MediaType, ScrapedStatus, TvEpisode, TvSeason,
};
pub use nfo::{
    import_nfo_for_item, write_emby_nfo, write_kodi_nfo, write_nfo, NfoParsedData, NfoReader,
};
pub use scanner::{
    refresh_items, refresh_items_cancellable, refresh_library, refresh_library_cancellable, FileNameParser, ItemRefreshReport, RefreshReport, ScanProgress,
    ScannedEpisode,
};
pub use thumbnail::{
    ThumbnailCache, ThumbnailError, EPISODE_STILL_HEIGHT, EPISODE_STILL_WIDTH, POSTER_THUMB_HEIGHT,
    POSTER_THUMB_WIDTH, SEASON_THUMB_HEIGHT, SEASON_THUMB_WIDTH,
};
