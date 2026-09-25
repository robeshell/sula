use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use crate::models::{Library, MediaType};
use crate::DatabaseError;

use super::AppDatabase;

impl AppDatabase {
    pub fn list_libraries(&self) -> Result<Vec<Library>, DatabaseError> {
        self.with_read_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, rootPath, bookmarkData, mediaType, addedAt
                 FROM libraries
                 ORDER BY addedAt ASC",
            )?;
            let rows = stmt.query_map([], |row| {
                let media_type: String = row.get(4)?;
                let added_at: String = row.get(5)?;
                Ok(Library {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    root_path: row.get(2)?,
                    bookmark_data: row.get(3)?,
                    media_type: MediaType::parse(&media_type).unwrap_or(MediaType::Movie),
                    added_at: parse_datetime(&added_at),
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn get_library(&self, id: &str) -> Result<Option<Library>, DatabaseError> {
        self.with_read_conn(|conn| {
            conn.query_row(
                "SELECT id, name, rootPath, bookmarkData, mediaType, addedAt
                 FROM libraries WHERE id = ?1",
                [id],
                |row| {
                    let media_type: String = row.get(4)?;
                    let added_at: String = row.get(5)?;
                    Ok(Library {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        root_path: row.get(2)?,
                        bookmark_data: row.get(3)?,
                        media_type: MediaType::parse(&media_type).unwrap_or(MediaType::Movie),
                        added_at: parse_datetime(&added_at),
                    })
                },
            )
            .optional()
            .map_err(DatabaseError::from)
        })
    }

    pub fn insert_library(&self, library: &Library) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO libraries (id, name, rootPath, bookmarkData, mediaType, addedAt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    library.id,
                    library.name,
                    library.root_path,
                    library.bookmark_data,
                    library.media_type.as_str(),
                    library.added_at.to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_library(&self, library: &Library) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE libraries
                 SET name = ?2, rootPath = ?3, bookmarkData = ?4, mediaType = ?5
                 WHERE id = ?1",
                params![
                    library.id,
                    library.name,
                    library.root_path,
                    library.bookmark_data,
                    library.media_type.as_str(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_library(&self, id: &str) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM libraries WHERE id = ?1", [id])?;
            Ok(())
        })
    }
}

fn parse_datetime(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
