use super::AppDatabase;
use crate::{DatabaseError, models::Library};
use rusqlite::{params, OptionalExtension};

impl AppDatabase {
    /// Fail closed when an empty mount point has replaced the original directory.
    /// Network servers that preserve directory identity still require native validation.
    pub fn verify_library_root(&self, library: &Library) -> Result<(), DatabaseError> {
        let root = std::fs::canonicalize(&library.root_path)?;
        let metadata = std::fs::metadata(&root)?;
        if !metadata.is_dir() { return Err(std::io::Error::other("library root is not a directory").into()); }
        std::fs::read_dir(&root)?;
        #[cfg(unix)]
        let identity = { use std::os::unix::fs::MetadataExt; format!("{}:{}", metadata.dev(), metadata.ino()) };
        #[cfg(not(unix))]
        let identity = format!("{:?}", metadata.created()?);
        let path = library.root_path.clone();
        self.with_conn(|conn| {
            let previous: Option<(String, String)> = conn.query_row(
                "SELECT root_path, identity FROM library_root_identity WHERE library_id=?1", [&library.id],
                |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            if let Some((previous_path, previous_identity)) = previous {
                if previous_path == path && previous_identity != identity {
                    return Err(std::io::Error::other("library root identity changed; restore the original mount before refreshing").into());
                }
            }
            conn.execute("INSERT INTO library_root_identity(library_id,root_path,identity) VALUES(?1,?2,?3)
                ON CONFLICT(library_id) DO UPDATE SET root_path=excluded.root_path,identity=excluded.identity",
                params![library.id,path,identity])?;
            Ok(())
        })
    }
}
