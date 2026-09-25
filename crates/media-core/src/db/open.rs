use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use thiserror::Error;

use super::migrations;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database lock poisoned")]
    Poisoned,
}

/// Read-only connections next to the single writer. WAL lets them read the last
/// committed state while a background write is in progress.
const READERS: usize = 3;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One writer connection for all writes and transactions, plus a small pool of
/// read-only connections for queries. A read sees every write committed before
/// it starts; nothing reads through a reader inside an open write transaction,
/// since transactions live within a single `with_conn` call.
pub struct AppDatabase {
    conn: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next_reader: AtomicUsize,
    path: PathBuf,
}

impl AppDatabase {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DatabaseError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        migrations::migrate(&conn)?;
        // After migration: WAL mode and the schema must exist before readers attach.
        let readers = (0..READERS).map(|_| open_reader(&path).map(Mutex::new)).collect::<Result<_, _>>()?;
        Ok(Self {
            conn: Mutex::new(conn),
            readers,
            next_reader: AtomicUsize::new(0),
            path,
        })
    }

    /// A private in-memory database cannot be shared, so reads use the writer.
    pub fn open_in_memory() -> Result<Self, DatabaseError> {
        let conn = Connection::open_in_memory()?;
        migrations::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            readers: Vec::new(),
            next_reader: AtomicUsize::new(0),
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The writer: every write, every transaction, and reads that must see them.
    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, DatabaseError>,
    ) -> Result<T, DatabaseError> {
        let guard = self.lock()?;
        f(&guard)
    }

    /// A free read-only connection (the writer for in-memory databases).
    pub fn with_read_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, DatabaseError>,
    ) -> Result<T, DatabaseError> {
        if self.readers.is_empty() {
            return self.with_conn(f);
        }
        let start = self.next_reader.fetch_add(1, Ordering::Relaxed);
        for offset in 0..self.readers.len() {
            match self.readers[(start + offset) % self.readers.len()].try_lock() {
                Ok(guard) => return f(&guard),
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Poisoned(_)) => return Err(DatabaseError::Poisoned),
            }
        }
        let guard = self.readers[start % self.readers.len()].lock().map_err(|_| DatabaseError::Poisoned)?;
        f(&guard)
    }

    pub fn library_count(&self) -> Result<i64, DatabaseError> {
        self.with_read_conn(|conn| {
            let count = conn.query_row("SELECT COUNT(*) FROM libraries", [], |row| row.get(0))?;
            Ok(count)
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, DatabaseError> {
        self.conn.lock().map_err(|_| DatabaseError::Poisoned)
    }
}

fn open_reader(path: &Path) -> Result<Connection, DatabaseError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Library, MediaType};

    #[test]
    fn readers_see_committed_writes_and_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDatabase::open(dir.path().join("db.sqlite")).unwrap();
        assert_eq!(db.readers.len(), READERS);
        let library = Library::new("Movies", "/movies", MediaType::Movie);
        db.insert_library(&library).unwrap();
        // Every reader, not just the first one picked.
        for _ in 0..READERS * 2 {
            assert_eq!(db.get_library(&library.id).unwrap().unwrap().name, "Movies");
        }
        let mut renamed = library.clone();
        renamed.name = "Films".into();
        db.update_library(&renamed).unwrap();
        for _ in 0..READERS * 2 {
            assert_eq!(db.list_libraries().unwrap()[0].name, "Films");
        }
        let write = db.with_read_conn(|conn| Ok(conn.execute("DELETE FROM libraries", [])?));
        assert!(write.is_err());
        assert_eq!(db.library_count().unwrap(), 1);
    }

    #[test]
    fn busy_readers_leave_reads_and_writes_available() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDatabase::open(dir.path().join("db.sqlite")).unwrap();
        db.with_read_conn(|_held| {
            assert_eq!(db.library_count()?, 0);
            db.insert_library(&Library::new("A", "/a", MediaType::Movie))?;
            // Committed by the writer: visible to the next read at once.
            assert_eq!(db.library_count()?, 1);
            Ok(())
        }).unwrap();
    }

    #[test]
    fn in_memory_reads_use_the_writer() {
        let db = AppDatabase::open_in_memory().unwrap();
        assert!(db.readers.is_empty());
        let library = Library::new("Movies", "/movies", MediaType::Movie);
        db.insert_library(&library).unwrap();
        assert!(db.get_library(&library.id).unwrap().is_some());
    }
}
