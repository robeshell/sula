//! Reading and saving the app settings.

use std::sync::atomic::Ordering;

use crate::app::locks::LockScope;
use crate::error::{failed, CoreError, CoreResult};
use crate::config::AppConfig;
use crate::state::AppState;

impl AppState {
    pub async fn config(&self) -> AppConfig {
        self.config.lock().await.config.clone()
    }

    pub(crate) async fn ui_locale(&self) -> String {
        self.config.lock().await.config.ui_locale.clone()
    }

    /// Saves the settings and tells the shell (`Events::config_changed`).
    pub async fn save_config(&self, config: AppConfig) -> CoreResult<AppConfig> {
        // Ordinary settings must not wait for a long scrape. Only a change of scan
        // exclusions resets scan state, which has to stay out of a running refresh.
        let exclusions_changed = self.config.lock().await.config.scan_excluded_folders != config.scan_excluded_folders;
        let mutation_guard = if exclusions_changed { Some(self.tasks.locks().lock(&LockScope::Global).await?) } else { None };
        let mut store = self.config.lock().await;
        let old = store.config.clone();
        let exclusions_changed = old.scan_excluded_folders != config.scan_excluded_folders;
        if exclusions_changed && mutation_guard.is_none() {
            return Err(CoreError::Busy("settings changed concurrently; try again".into()));
        }
        store.config = config;
        if let Err(error) = store.save() {
            store.config = old;
            return Err(failed(error));
        }
        if exclusions_changed {
            for library in self.db.list_libraries().map_err(failed)? {
                self.db.clear_scan_states(&library.id).map_err(failed)?;
            }
        }
        let saved = store.config.clone();
        drop(store);
        self.keep_running_on_close.store(saved.keep_running_on_close, Ordering::Relaxed);
        self.events.config_changed(&saved);
        Ok(saved)
    }
}
