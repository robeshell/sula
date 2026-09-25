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
}
