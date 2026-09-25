//! Removing items and cleaning up files left behind next to them.

use std::sync::Arc;

use crate::app::cleanup::SystemTrash;
use crate::app::locks::LockScope;
use crate::error::{blocking, failed, CoreError, CoreResult};
use crate::state::AppState;
use crate::task_queue::{TaskKind, TaskSnapshot};
use crate::ui_i18n;

impl AppState {
    /// Files next to the items that look left over (old NFOs, stray artwork …).
    pub async fn scan_residuals(&self, item_ids: Vec<String>) -> CoreResult<Vec<media_core::ResidualCandidate>> {
        if item_ids.is_empty() {
            return Err(CoreError::invalid("no items selected"));
        }
        let db = Arc::clone(&self.db);
        blocking(move || media_core::find_residuals(&db, &item_ids).map_err(failed)).await
    }

    /// Moves the chosen residual files to the trash.
    pub async fn cleanup_residuals(&self, paths: Vec<String>) -> CoreResult<TaskSnapshot> {
        if paths.is_empty() {
            return Err(CoreError::invalid("no residual files selected"));
        }
        let locale = self.ui_locale().await;
        let title = ui_i18n::tf(&locale, "task.cleanupN", &[("n", &paths.len().to_string())]);
        let db = Arc::clone(&self.db);
        // Candidates are revalidated across every library, so the job excludes all of them.
        let snapshot = self
            .tasks
            .enqueue(title, TaskKind::Cleanup, None, Some(LockScope::Global), move |handle| {
                Box::pin(async move { crate::app::cleanup::cleanup_residuals(db, paths, &locale, Arc::new(SystemTrash), &handle).await })
            })
            .await;
        Ok(snapshot)
    }

    /// Removes items from their library and, with `also_trash`, trashes their files.
    pub async fn delete_media_items(&self, item_ids: Vec<String>, also_trash: bool) -> CoreResult<usize> {
        let scope = self.items_scope(&item_ids).await?;
        let mutation_guard = self.tasks.locks().lock(&scope).await?;
        if item_ids.is_empty() {
            return Err(CoreError::invalid("no items selected"));
        }
        let db = Arc::clone(&self.db);
        let events = Arc::clone(&self.events);
        blocking(move || {
            let _mutation_guard = mutation_guard;
            crate::app::cleanup::delete_media_items(&db, &item_ids, also_trash, &SystemTrash, events.as_ref()).map_err(CoreError::from)
        })
        .await
    }
}
