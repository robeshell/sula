use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::DatabaseError;

use super::AppDatabase;

#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryScanState {
    pub library_id: String,
    pub directory_path: String,
    pub last_known_modification_time: f64,
    pub last_scanned_at: DateTime<Utc>,
}

impl AppDatabase {
    pub fn list_scan_states(
        &self,
        library_id: &str,
    ) -> Result<Vec<DirectoryScanState>, DatabaseError> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT libraryId, directoryPath, lastKnownModificationTime, lastScannedAt
                 FROM directoryScanState
                 WHERE libraryId = ?1",
            )?;
            let rows = stmt.query_map([library_id], |row| {
                let scanned_at: String = row.get(3)?;
                Ok(DirectoryScanState {
                    library_id: row.get(0)?,
                    directory_path: row.get(1)?,
                    last_known_modification_time: row.get(2)?,
                    last_scanned_at: parse_rfc3339(&scanned_at),
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn upsert_scan_states(&self, rows: &[DirectoryScanState]) -> Result<(), DatabaseError> {
        if rows.is_empty() {
            return Ok(());
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO directoryScanState (
                        libraryId, directoryPath, lastKnownModificationTime, lastScannedAt
                     ) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(libraryId, directoryPath) DO UPDATE SET
                        lastKnownModificationTime = excluded.lastKnownModificationTime,
                        lastScannedAt = excluded.lastScannedAt",
                )?;
                for row in rows {
                    stmt.execute(params![
                        row.library_id,
                        row.directory_path,
                        row.last_known_modification_time,
                        row.last_scanned_at.to_rfc3339(),
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn delete_scan_states(
        &self,
        library_id: &str,
        directory_paths: &[String],
    ) -> Result<usize, DatabaseError> {
        if directory_paths.is_empty() {
            return Ok(0);
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut deleted = 0usize;
            {
                let mut stmt = tx.prepare(
                    "DELETE FROM directoryScanState WHERE libraryId = ?1 AND directoryPath = ?2",
                )?;
                for path in directory_paths {
                    deleted += stmt.execute(params![library_id, path])? as usize;
                }
            }
            tx.commit()?;
            Ok(deleted)
        })
    }

    pub fn clear_scan_states(&self, library_id: &str) -> Result<usize, DatabaseError> {
        self.with_conn(|conn| {
            let n = conn.execute(
                "DELETE FROM directoryScanState WHERE libraryId = ?1",
                params![library_id],
            )?;
            Ok(n)
        })
    }

    /// Delete media whose `folderPath` or `filePath` equals or is nested under any root.
    pub fn delete_media_items_rooted_under(
        &self,
        library_id: &str,
        roots: &[String],
    ) -> Result<usize, DatabaseError> {
        if roots.is_empty() {
            return Ok(0);
        }
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, folderPath, filePath FROM media_items WHERE libraryId = ?1",
            )?;
            let rows = stmt.query_map([library_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            let mut ids = Vec::new();
            for row in rows {
                let (id, folder, file) = row?;
                if roots
                    .iter()
                    .any(|root| path_rooted_under(&folder, root) || path_rooted_under(&file, root))
                {
                    ids.push(id);
                }
            }
            drop(stmt);

            if ids.is_empty() {
                return Ok(0);
            }

            let tx = conn.unchecked_transaction()?;
            let mut deleted = 0usize;
            {
                let mut del = tx.prepare("DELETE FROM media_items WHERE id = ?1")?;
                for id in &ids {
                    deleted += del.execute(params![id])? as usize;
                }
            }
            tx.commit()?;
            Ok(deleted)
        })
    }
}

fn parse_rfc3339(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

pub fn path_rooted_under(path: &str, root: &str) -> bool {
    if path.is_empty() || root.is_empty() {
        return false;
    }
    let path = normalize_path(path);
    let root = normalize_path(root);
    if path == root {
        return true;
    }
    let prefix = format!("{root}/");
    path.starts_with(&prefix)
}

fn normalize_path(path: &str) -> String {
    let mut out = path.replace('\\', "/");
    while out.ends_with('/') && out.len() > 1 {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::path_rooted_under;

    #[test]
    fn rooted_under_matches_self_and_children() {
        assert!(path_rooted_under("/lib/Movie", "/lib/Movie"));
        assert!(path_rooted_under("/lib/Movie/a.mkv", "/lib/Movie"));
        assert!(!path_rooted_under("/lib/MovieOther", "/lib/Movie"));
        assert!(path_rooted_under(r"C:\lib\Movie\a.mkv", r"C:\lib\Movie"));
    }
}
