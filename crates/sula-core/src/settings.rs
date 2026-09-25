//! Reading and saving the app settings.

use std::sync::atomic::Ordering;

use crate::app::locks::LockScope;
use crate::error::{failed, CoreError, CoreResult};
use crate::config::AppConfig;
use crate::state::AppState;

impl AppState {
    pub async fn config(&self) -> AppConfig {
        self.config.lock().await.snapshot()
    }

    /// The settings once saved API keys have been read, for work that talks to
    /// the metadata sources. Waits while the keychain read is still running.
    pub(crate) async fn config_with_keys(&self) -> AppConfig {
        loop {
            let loaded = self.keys_loaded.notified();
            tokio::pin!(loaded);
            loaded.as_mut().enable();
            {
                let store = self.config.lock().await;
                if !store.keys_loading() {
                    return store.snapshot();
                }
            }
            loaded.await;
        }
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
        // Startup notices were shown when the window opened; don't repeat them.
        store.config.config_notice = None;
        let saved = store.snapshot();
        drop(store);
        self.keep_running_on_close.store(saved.keep_running_on_close, Ordering::Relaxed);
        self.events.config_changed(&saved);
        Ok(saved)
    }
}
