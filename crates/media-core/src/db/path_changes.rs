use super::{AppDatabase, DatabaseError};
use rusqlite::params;
use std::path::{Path, PathBuf};

impl AppDatabase {
    /// Apply all references and the operation commit marker atomically.
    pub fn commit_media_paths(&self, item_id: &str, old_root: &Path, new_root: &Path,
        file_path: &str, files: &[(PathBuf, PathBuf)], operation: &str) -> Result<(), DatabaseError> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let remap = |value: Option<String>| value.map(|raw| {
                if raw.is_empty() { return raw; }
                let absolute = old_root.join(&raw);
                let mapped = files.iter().find(|(from, _)| *from == absolute).map(|(_, to)| to.clone()).unwrap_or(absolute);
                if Path::new(&raw).is_absolute() { mapped.to_string_lossy().into_owned() }
                else { mapped.strip_prefix(new_root).unwrap_or(&mapped).to_string_lossy().replace('\\', "/") }
            });
            for column in ["posterPath", "fanartPath", "bannerPath", "logoPath", "thumbPath"] {
                let query = format!("SELECT {column} FROM media_metadata WHERE mediaItemId=?1");
                use rusqlite::OptionalExtension;
                if let Some(value) = tx.query_row(&query, [item_id], |r| r.get::<_, Option<String>>(0)).optional()? {
                    tx.execute(&format!("UPDATE media_metadata SET {column}=?1 WHERE mediaItemId=?2"), params![remap(value), item_id])?;
                }
            }
            let mut stmt = tx.prepare("SELECT id,posterPath FROM tv_seasons WHERE mediaItemId=?1")?;
            let seasons = stmt.query_map([item_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            drop(stmt);
            for (season, poster) in seasons {
                tx.execute("UPDATE tv_seasons SET posterPath=?1 WHERE id=?2", params![remap(poster), season])?;
                let mut stmt = tx.prepare("SELECT id,filePath,stillPath FROM tv_episodes WHERE seasonId=?1")?;
                let episodes = stmt.query_map([&season], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
                drop(stmt);
                for (id, path, still) in episodes {
                    let updated = files.iter().find(|(from, _)| *from == Path::new(&path)).map(|(_, to)| to.to_string_lossy().into_owned()).unwrap_or(path);
                    tx.execute("UPDATE tv_episodes SET filePath=?1,stillPath=?2 WHERE id=?3", params![updated, remap(still), id])?;
                }
            }
            if tx.execute("UPDATE media_items SET folderPath=?1,filePath=?2 WHERE id=?3", params![new_root.to_string_lossy(), file_path, item_id])? != 1 {
                return Err(DatabaseError::Io(std::io::Error::other("media item disappeared")));
            }
            if tx.execute("UPDATE media_operation_journal SET committed=1 WHERE id=?1 AND committed=0", [operation])? != 1 {
                return Err(DatabaseError::Io(std::io::Error::other("media journal disappeared")));
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Point every stored reference to `old` (a file or directory renamed in place
    /// outside a media operation, e.g. by the batch renamer) at `new`, including
    /// entries below a renamed directory. Relative artwork paths follow their item
    /// folder and need no change. Returns the number of rewritten values.
    pub fn remap_renamed_path(&self, old: &str, new: &str) -> Result<usize, DatabaseError> {
        const COLUMNS: [(&str, &str, &[&str]); 5] = [
            ("libraries", "id", &["rootPath"]),
            ("media_items", "id", &["folderPath", "filePath"]),
            ("media_metadata", "mediaItemId", &["posterPath", "fanartPath", "bannerPath", "logoPath", "thumbPath"]),
            ("tv_seasons", "id", &["posterPath"]),
            ("tv_episodes", "id", &["filePath", "stillPath"]),
        ];
        if old.is_empty() || new.is_empty() || old == new {
            return Ok(0);
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut changed = 0;
            for (table, key, columns) in COLUMNS {
                for column in columns {
                    let select = format!("SELECT {key},{column} FROM {table} WHERE substr({column},1,length(?1))=?1");
                    let rows = tx.prepare(&select)?
                        .query_map([old], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                        .collect::<Result<Vec<_>, _>>()?;
                    let update = format!("UPDATE {table} SET {column}=?1 WHERE {key}=?2");
                    for (id, value) in rows {
                        let rest = &value[old.len()..];
                        if !(rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\')) {
                            continue;
                        }
                        changed += tx.execute(&update, params![format!("{new}{rest}"), id])?;
                    }
                }
            }
            tx.commit()?;
            Ok(changed)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{Library, MediaItem, MediaType, ScrapedStatus};
    use crate::AppDatabase;

    #[test]
    fn remaps_files_and_directory_prefixes_only_on_path_boundaries() {
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("Movies", "/m".to_string(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let a = MediaItem::new_movie("A", None, "/m/A", "/m/A/a.mkv", &lib.id, ScrapedStatus::Unscraped);
        let b = MediaItem::new_movie("B", None, "/m/AB", "/m/AB/b.mkv", &lib.id, ScrapedStatus::Unscraped);
        db.insert_media_items(&[a.clone(), b.clone()]).unwrap();

        assert_eq!(db.remap_renamed_path("/m/A/a.mkv", "/m/A/Alpha.mkv").unwrap(), 1);
        assert_eq!(db.get_media_item(&a.id).unwrap().unwrap().file_path, "/m/A/Alpha.mkv");

        assert_eq!(db.remap_renamed_path("/m/A", "/m/Alpha (2020)").unwrap(), 2);
        let moved = db.get_media_item(&a.id).unwrap().unwrap();
        assert_eq!(moved.folder_path, "/m/Alpha (2020)");
        assert_eq!(moved.file_path, "/m/Alpha (2020)/Alpha.mkv");
        let untouched = db.get_media_item(&b.id).unwrap().unwrap();
        assert_eq!(untouched.folder_path, "/m/AB");
    }
}
