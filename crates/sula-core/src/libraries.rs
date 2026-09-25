//! Adding, renaming, removing and relocating libraries, and merging duplicate shows.

use std::sync::Arc;

use media_core::{Library, MediaType};

use crate::app::locks::LockScope;
use crate::app::organize::{ShowMergePair, ShowMergePlanDto};
use crate::error::{blocking, failed, CoreError, CoreResult};
use crate::state::AppState;

impl AppState {
    pub async fn libraries(&self) -> CoreResult<Vec<Library>> {
        self.db.list_libraries().map_err(failed)
    }

    /// Inserts the library and queues its first refresh; no files change here.
    pub async fn add_library(&self, name: String, root_path: String, media_type: MediaType) -> CoreResult<Library> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(CoreError::invalid("library name is empty"));
        }
        if root_path.trim().is_empty() {
            return Err(CoreError::invalid("library path is empty"));
        }
        let library = Library::new(name, root_path, media_type);
        self.db.insert_library(&library).map_err(failed)?;
        self.refresh_library(library.id.clone()).await?;
        // Every window keeps its own library list (settings edits them too).
        self.events.library_updated();
        Ok(library)
    }

    pub async fn rename_library(&self, id: String, name: String) -> CoreResult<Library> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(CoreError::invalid("library name is empty"));
        }
        let mut library = self
            .db
            .get_library(&id)
            .map_err(failed)?
            .ok_or_else(|| CoreError::not_found("library", &id))?;
        library.name = name;
        self.db.update_library(&library).map_err(failed)?;
        self.events.library_updated();
        Ok(library)
    }

    pub async fn delete_library(&self, id: String) -> CoreResult<()> {
        let _mutation_guard = self.tasks.locks().lock(&LockScope::library(&id)).await?;
        self.db.delete_library(&id).map_err(failed)?;
        self.events.library_updated();
        Ok(())
    }

    /// Points a library at a new root (LIB-08: the old path moved or went stale).
    pub async fn rebind_library(&self, id: String, root_path: String) -> CoreResult<Library> {
        // A new root can overlap other libraries, so rebinding excludes all of them.
        let _mutation_guard = self.tasks.locks().lock(&LockScope::Global).await?;
        let root_path = root_path.trim().to_string();
        if root_path.is_empty() {
            return Err(CoreError::invalid("library path is empty"));
        }
        if !std::path::Path::new(&root_path).is_dir() {
            return Err(CoreError::invalid("selected path is not a directory"));
        }
        let mut library = self
            .db
            .get_library(&id)
            .map_err(failed)?
            .ok_or_else(|| CoreError::not_found("library", &id))?;
        library.root_path = root_path;
        library.bookmark_data = None;
        self.db.update_library(&library).map_err(failed)?;
        // Path changed → wipe scan state so next refresh re-bootstraps.
        if let Err(error) = self.db.clear_scan_states(&library.id) {
            tracing::warn!(library_id = %library.id, %error, "scan state not cleared after rebind");
        }
        self.refresh_library(library.id.clone()).await?;
        self.events.library_updated();
        Ok(library)
    }

    /// Read-only preview of duplicate-show merges for a library or selected items, so
    /// the user sees exactly which folders would be absorbed before anything moves.
    pub async fn plan_show_merges(&self, library_id: Option<String>, item_ids: Option<Vec<String>>) -> CoreResult<Vec<ShowMergePlanDto>> {
        let db = Arc::clone(&self.db);
        blocking(move || crate::app::organize::plan_show_merges(&db, library_id, item_ids).map_err(CoreError::from)).await
    }

    /// Executes merges the user confirmed from `plan_show_merges`. Each pair is checked
    /// again against the current index and skipped if the match changed meanwhile.
    pub async fn merge_planned_shows(&self, pairs: Vec<ShowMergePair>) -> CoreResult<u32> {
        let db = Arc::clone(&self.db);
        // Every library of every pair, locked in one sorted acquisition.
        let scope = self.items_scope(&crate::app::organize::merge_item_ids(&pairs)).await?;
        let mutation_guard = self.tasks.locks().lock(&scope).await?;
        let templates = self.config.lock().await.config.rename_templates();
        let events = Arc::clone(&self.events);
        blocking(move || {
            let _mutation_guard = mutation_guard;
            crate::app::organize::merge_planned_shows(&db, &pairs, &templates, events.as_ref()).map_err(CoreError::from)
        })
        .await
    }
}
