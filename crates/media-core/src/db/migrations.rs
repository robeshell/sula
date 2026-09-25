//! Schema aligned to the Swift GRDB final state (post v8), with optional bookmark columns.

use rusqlite::{Connection, OptionalExtension};

const SCHEMA_VERSION: i32 = 4;

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;

         CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY NOT NULL,
             applied_at TEXT NOT NULL
         );",
    )?;

    let current: i32 = conn
        .query_row(
            "SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0);

    if current < 1 {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "
            CREATE TABLE libraries (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                rootPath TEXT NOT NULL,
                bookmarkData BLOB,
                mediaType TEXT NOT NULL,
                addedAt TEXT NOT NULL
            );

            CREATE TABLE media_items (
                id TEXT PRIMARY KEY NOT NULL,
                type TEXT NOT NULL,
                title TEXT NOT NULL,
                originalTitle TEXT,
                year INTEGER,
                folderPath TEXT NOT NULL,
                filePath TEXT NOT NULL DEFAULT '',
                bookmarkData BLOB,
                status TEXT NOT NULL,
                scrapeIssue TEXT,
                libraryId TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
                addedAt TEXT NOT NULL
            );

            CREATE TABLE media_metadata (
                mediaItemId TEXT PRIMARY KEY NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
                overview TEXT,
                outline TEXT,
                tagline TEXT,
                genres TEXT NOT NULL DEFAULT '[]',
                tags TEXT NOT NULL DEFAULT '[]',
                rating REAL,
                ratingVotes INTEGER,
                contentRating TEXT,
                director TEXT,
                writer TEXT,
                credits TEXT NOT NULL DEFAULT '[]',
                studio TEXT,
                country TEXT,
                language TEXT,
                premiered TEXT,
                endDate TEXT,
                runtime INTEGER,
                showStatus TEXT,
                collectionName TEXT,
                collectionId TEXT,
                sourceId TEXT NOT NULL,
                imdbId TEXT,
                tmdbId TEXT,
                tvdbId TEXT,
                bangumiId TEXT,
                posterPath TEXT,
                fanartPath TEXT,
                bannerPath TEXT,
                logoPath TEXT,
                thumbPath TEXT,
                videoCodec TEXT,
                videoResolution TEXT,
                audioCodec TEXT,
                audioChannels TEXT,
                trailer TEXT,
                scrapedAt TEXT NOT NULL
            );

            CREATE TABLE tv_seasons (
                id TEXT PRIMARY KEY NOT NULL,
                mediaItemId TEXT NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
                seasonNumber INTEGER NOT NULL,
                title TEXT,
                overview TEXT,
                posterPath TEXT,
                airDate TEXT,
                episodeCount INTEGER
            );

            CREATE TABLE tv_episodes (
                id TEXT PRIMARY KEY NOT NULL,
                seasonId TEXT NOT NULL REFERENCES tv_seasons(id) ON DELETE CASCADE,
                episodeNumber INTEGER NOT NULL,
                title TEXT,
                overview TEXT,
                airDate TEXT,
                stillPath TEXT,
                stillURL TEXT,
                filePath TEXT NOT NULL DEFAULT '',
                runtime INTEGER,
                rating REAL,
                director TEXT,
                writer TEXT,
                guestCast TEXT NOT NULL DEFAULT '[]',
                absoluteNumber INTEGER,
                finaleType TEXT,
                videoCodec TEXT,
                videoResolution TEXT,
                audioCodec TEXT
            );

            CREATE TABLE directoryScanState (
                libraryId TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
                directoryPath TEXT NOT NULL,
                lastKnownModificationTime REAL NOT NULL,
                lastScannedAt TEXT NOT NULL,
                PRIMARY KEY (libraryId, directoryPath)
            );

            CREATE INDEX idx_media_items_library ON media_items(libraryId);
            CREATE INDEX idx_media_items_status ON media_items(status);
            CREATE INDEX idx_media_items_type ON media_items(type);
            CREATE INDEX idx_directory_scan_state_library ON directoryScanState(libraryId);
            ",
        )?;

        tx.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, datetime('now'))",
            [1],
        )?;
        tx.commit()?;
    }

    if current < 2 {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch("CREATE TABLE media_operation_journal (
            id TEXT PRIMARY KEY NOT NULL,
            payload TEXT NOT NULL,
            committed INTEGER NOT NULL DEFAULT 0
        );")?;
        tx.execute("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, datetime('now'))", [2])?;
        tx.commit()?;
    }
    if current < 3 {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch("CREATE TABLE library_root_identity (
            library_id TEXT PRIMARY KEY REFERENCES libraries(id) ON DELETE CASCADE,
            root_path TEXT NOT NULL, identity TEXT NOT NULL
        );")?;
        tx.execute("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, datetime('now'))", [3])?;
        tx.commit()?;
    }
    if current < 4 {
        // Sub-mounts under a library root: `paths` is a JSON array; no row means the
        // library was never walked with mount detection yet.
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch("CREATE TABLE library_mount_points (
            library_id TEXT PRIMARY KEY REFERENCES libraries(id) ON DELETE CASCADE,
            paths TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tv_seasons_media_item ON tv_seasons(mediaItemId);
        CREATE INDEX IF NOT EXISTS idx_tv_episodes_season ON tv_episodes(seasonId);")?;
        tx.execute("INSERT INTO schema_migrations (version, applied_at) VALUES (?1, datetime('now'))", [SCHEMA_VERSION])?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn root_identity_upgrade_rolls_back_and_retries() {
        let conn = Connection::open_in_memory().unwrap(); migrate(&conn).unwrap();
        conn.execute_batch("DROP TABLE library_mount_points; DROP TABLE library_root_identity; DELETE FROM schema_migrations WHERE version>=3;
            CREATE TRIGGER fail_v3 BEFORE INSERT ON schema_migrations WHEN NEW.version=3 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        assert!(migrate(&conn).is_err());
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='library_root_identity'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        conn.execute_batch("DROP TRIGGER fail_v3").unwrap(); migrate(&conn).unwrap();
    }

    #[test]
    fn journal_upgrade_rolls_back_and_retries_from_v1() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute_batch("DROP TABLE library_mount_points; DROP TABLE library_root_identity; DROP TABLE media_operation_journal; DELETE FROM schema_migrations WHERE version>=2;
            CREATE TRIGGER fail_v2 BEFORE INSERT ON schema_migrations WHEN NEW.version=2 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        assert!(migrate(&conn).is_err());
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='media_operation_journal'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        conn.execute_batch("DROP TRIGGER fail_v2").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
    }

    #[test]
    fn failed_migration_rolls_back_ddl_and_can_retry() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
            CREATE TRIGGER fail_version BEFORE INSERT ON schema_migrations BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        assert!(migrate(&conn).is_err());
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'libraries'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        conn.execute_batch("DROP TRIGGER fail_version").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
    }

    #[test]
    fn mount_points_upgrade_rolls_back_and_retries() {
        let conn = Connection::open_in_memory().unwrap(); migrate(&conn).unwrap();
        conn.execute_batch("DROP TABLE library_mount_points; DROP INDEX idx_tv_seasons_media_item; DELETE FROM schema_migrations WHERE version=4;
            CREATE TRIGGER fail_v4 BEFORE INSERT ON schema_migrations WHEN NEW.version=4 BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        assert!(migrate(&conn).is_err());
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name IN ('library_mount_points','idx_tv_seasons_media_item')", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        conn.execute_batch("DROP TRIGGER fail_v4").unwrap(); migrate(&conn).unwrap();
    }

    #[test]
    fn migrates_empty_database() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let version: i32 = conn
            .query_row(
                "SELECT version FROM schema_migrations ORDER BY version DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 4);

        let table_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN (
                    'libraries','media_items','media_metadata','tv_seasons','tv_episodes','directoryScanState'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 6);
    }
}
