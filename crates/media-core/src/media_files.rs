//! File ownership checks shared by destructive media operations.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{AppDatabase, MediaItem, MediaType};

/// A directory is owned only when it is inside its library, contains no other
/// library/item, and every video on disk belongs to this item. Unknown files and
/// symlinks must never expand the scope of a destructive operation.
pub fn owns_folder(db: &AppDatabase, item: &MediaItem) -> anyhow::Result<bool> {
    let folder = Path::new(&item.folder_path).canonicalize()?;
    let library = db
        .get_library(&item.library_id)?
        .ok_or_else(|| anyhow::anyhow!("library not found"))?;
    let root = Path::new(&library.root_path).canonicalize()?;
    if folder == root || !folder.starts_with(&root) {
        return Ok(false);
    }
    for library in db.list_libraries()? {
        let root = Path::new(&library.root_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&library.root_path));
        if root.starts_with(&folder) {
            return Ok(false);
        }
        for other in db.list_media_items(&library.id)? {
            if other.id == item.id {
                continue;
            }
            let other_folder = Path::new(&other.folder_path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&other.folder_path));
            if other_folder.starts_with(&folder)
                || (other.media_type != MediaType::Movie && folder.starts_with(&other_folder))
            {
                return Ok(false);
            }
        }
    }
    let mut owned = HashSet::new();
    if item.media_type == MediaType::Movie {
        owned.insert(Path::new(&item.file_path).canonicalize()?);
    } else {
        for season in db.fetch_seasons(&item.id)? {
            for episode in db.fetch_episodes(&season.id)? {
                if !episode.file_path.is_empty() {
                    owned.insert(Path::new(&episode.file_path).canonicalize()?);
                }
            }
        }
    }
    for entry in walkdir::WalkDir::new(&folder).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Ok(false);
        }
        let extension = entry
            .path()
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if entry.file_type().is_file()
            && crate::scanner::MEDIA_EXTENSIONS.contains(&extension.as_str())
            && !owned.contains(&entry.path().canonicalize()?)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Shared folders are never deleted. For a loose movie only its video is a
/// deletion target; ambiguous sidecars remain available for manual inspection.
pub fn deletion_target(db: &AppDatabase, item: &MediaItem) -> anyhow::Result<PathBuf> {
    if owns_folder(db, item)? {
        return Ok(PathBuf::from(&item.folder_path));
    }
    anyhow::ensure!(
        item.media_type == MediaType::Movie,
        "cannot delete a shared show folder"
    );
    let file = Path::new(&item.file_path).canonicalize()?;
    let library = db
        .get_library(&item.library_id)?
        .ok_or_else(|| anyhow::anyhow!("library not found"))?;
    let root = Path::new(&library.root_path).canonicalize()?;
    anyhow::ensure!(
        file.is_file() && file.starts_with(root),
        "media file is outside the library"
    );
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Library, ScrapedStatus};

    #[test]
    fn root_and_shared_movies_never_own_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("Movies", dir.path().to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        for folder in [dir.path().to_path_buf(), dir.path().join("Shared")] {
            std::fs::create_dir_all(&folder).unwrap();
            let a = folder.join("A.mkv");
            std::fs::write(&a, b"a").unwrap();
            let item = MediaItem::new_movie(
                "A",
                None,
                folder.to_string_lossy(),
                a.to_string_lossy(),
                lib.id.clone(),
                ScrapedStatus::Scraped,
            );
            db.insert_media_items(&[item.clone()]).unwrap();
            if folder != dir.path() {
                std::fs::write(folder.join("Unindexed.mkv"), b"b").unwrap();
            }
            assert!(!owns_folder(&db, &item).unwrap());
            assert_eq!(
                deletion_target(&db, &item).unwrap(),
                a.canonicalize().unwrap()
            );
            db.delete_media_item(&item.id).unwrap();
        }
    }

    #[test]
    fn exclusive_movie_folder_can_be_managed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("A");
        std::fs::create_dir(&folder).unwrap();
        let a = folder.join("A.mkv");
        std::fs::write(&a, b"a").unwrap();
        let db = AppDatabase::open_in_memory().unwrap();
        let lib = Library::new("Movies", dir.path().to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let item = MediaItem::new_movie(
            "A",
            None,
            folder.to_string_lossy(),
            a.to_string_lossy(),
            lib.id,
            ScrapedStatus::Scraped,
        );
        db.insert_media_items(&[item.clone()]).unwrap();
        assert!(owns_folder(&db, &item).unwrap());
    }
}
