use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use media_core::{AppDatabase, AvatarCache, ThumbnailCache};
use renamer::{PresetManager, RenameUndoManager};
use tokio::sync::Mutex;

use crate::app::locks::MutationLocks;
use crate::config::{AppConfig, ConfigStore};
use crate::log_store::LogStore;
use crate::task_queue::TaskQueue;

pub struct AppState {
    _instance_lock: std::fs::File,
    pub db: Arc<AppDatabase>,
    pub config: Arc<Mutex<ConfigStore>>,
    pub tasks: Arc<TaskQueue>,
    pub thumbs: Arc<ThumbnailCache>,
    pub avatars: Arc<AvatarCache>,
    pub rename_undo: Arc<RenameUndoManager>,
    pub rename_presets: Arc<PresetManager>,
    pub logs: Arc<LogStore>,
    pub data_dir: PathBuf,
    pub keep_running_on_close: AtomicBool,
    pub tray_enabled: AtomicBool,
}

#[derive(Debug)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("another Sula instance is using this library database")
    }
}

impl std::error::Error for AlreadyRunning {}

impl AppState {
    /// Opens everything under `data_dir` (shells pass [`app_data_dir`] so existing
    /// libraries are found) and starts the task queue on `runtime`.
    pub fn bootstrap(data_dir: PathBuf, runtime: &tokio::runtime::Handle, logs: Arc<LogStore>) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;

        let instance_lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(data_dir.join("application.lock"))?;
        instance_lock.try_lock().map_err(|_| AlreadyRunning)?;
        let db_path = data_dir.join("sula.sqlite3");
        let db = Arc::new(AppDatabase::open(&db_path)?);
        tracing::info!(path = %db_path.display(), "database opened");

        let config_path = data_dir.join("config.toml");
        let config_store = ConfigStore::load_or_default(&config_path)?;
        let keep_running_on_close = AtomicBool::new(config_store.config.keep_running_on_close);
        let tray_enabled = AtomicBool::new(config_store.config.tray_enabled);
        let config = Arc::new(Mutex::new(config_store));

        if let Err(error) = renamer::recover_media_operations(&db) {
            tracing::error!(%error, "media recovery pending; mutations remain blocked");
        }
        let recovery_db = Arc::clone(&db);
        let locks = Arc::new(MutationLocks::for_database(Arc::clone(&db), move || renamer::recover_media_operations(&recovery_db)));
        let tasks = Arc::new(TaskQueue::open(runtime, data_dir.join("task_history.json"), locks).map_err(anyhow::Error::msg)?);
        let thumbs = Arc::new(ThumbnailCache::open_default()?);
        let avatars = Arc::new(AvatarCache::open_default()?);
        let rename_undo = Arc::new(RenameUndoManager::open(
            data_dir.join("rename_snapshots"),
        )?);
        let rename_presets = Arc::new(PresetManager::open(data_dir.join("rename_presets"))?);

        Ok(Self {
            _instance_lock: instance_lock,
            db,
            config,
            tasks,
            thumbs,
            avatars,
            rename_undo,
            rename_presets,
            logs,
            data_dir,
            keep_running_on_close,
            tray_enabled,
        })
    }
}

/// Where Sula keeps its database, config and history. Moving it would make
/// existing users' libraries disappear, so it is pinned by a test.
pub fn app_data_dir() -> anyhow::Result<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| anyhow::anyhow!("no data directory"))?;
    Ok(base.join("sula"))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatusDto {
    pub app_name: String,
    pub version: String,
    pub data_dir: String,
    pub database_path: String,
    pub library_count: i64,
    pub config: AppConfig,
    pub crates: CratesDto,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CratesDto {
    pub media_core: String,
    pub scraper_kit: String,
    pub renamer: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn data_dir_stays_under_the_platform_data_dir() {
        let expected = dirs::data_dir().expect("platform data dir").join("sula");
        assert_eq!(super::app_data_dir().unwrap(), expected);
    }
}
