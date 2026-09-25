//! Application services: the workflows behind the commands (scrape → persist →
//! auto-rename, refresh, organize, cleanup). Nothing here depends on Tauri, so the
//! flows run in tests against a temp dir, an in-memory database and fake reporters.
//! Commands parse arguments, pick a lock scope, enqueue and map results.

pub mod cleanup;
pub mod library;
pub mod locks;
pub mod organize;
pub mod posters;
pub mod scrape;
mod scrape_persist;

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub completed: u32,
    pub total: u32,
    pub current: String,
    pub stage_key: Option<String>,
}

impl TaskProgress {
    pub fn new(completed: u32, total: u32, current: impl Into<String>, stage_key: &str) -> Self {
        Self { completed, total, current: current.into(), stage_key: Some(stage_key.into()) }
    }
}

/// Progress sink of a running job; the task queue's `TaskHandle` implements it.
pub trait Progress: Send + Sync {
    fn update(&self, progress: TaskProgress) -> impl Future<Output = ()> + Send;
    fn scrape_result(&self, _summary: &scrape::ScrapeSummary) -> impl Future<Output = ()> + Send {
        async {}
    }
    fn cancel_flag(&self) -> Arc<AtomicBool>;
    fn is_cancelled(&self) -> bool {
        self.cancel_flag().load(Ordering::SeqCst)
    }
}

/// Notifications for the shell's UI.
pub trait Events: Send + Sync {
    /// Library contents changed; lists should reload.
    fn library_updated(&self);
    /// A task was queued or changed status, progress or result.
    fn task_updated(&self, _task: &crate::task_queue::TaskSnapshot) {}
}

pub fn err_string(err: impl ToString) -> String {
    err.to_string()
}

/// Run filesystem or SQLite work on the blocking pool so slow disks (NAS) and a
/// busy database connection never stall the async workers.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

/// Poll the cancel flag while awaiting async I/O; never wrap detached blocking work.
pub async fn cancellable<T>(cancel: &AtomicBool, future: impl Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::pin!(future);
    loop {
        if cancel.load(Ordering::SeqCst) { return Err("cancelled".into()); }
        tokio::select! {
            result = &mut future => return result,
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use media_core::{AppDatabase, Library, MediaItem, MediaType, ScrapedStatus};

    use super::{Events, Progress, TaskProgress};

    #[derive(Default)]
    pub struct FakeProgress {
        pub updates: Mutex<Vec<TaskProgress>>,
        pub results: Mutex<Vec<(u32, u32, u32)>>,
        pub cancel: Arc<AtomicBool>,
    }

    impl FakeProgress {
        pub fn stages(&self) -> Vec<String> {
            self.updates.lock().unwrap().iter().map(|p| p.stage_key.clone().unwrap_or_default()).collect()
        }
        pub fn last(&self) -> TaskProgress {
            self.updates.lock().unwrap().last().cloned().expect("progress reported")
        }
    }

    impl Progress for FakeProgress {
        async fn update(&self, progress: TaskProgress) {
            self.updates.lock().unwrap().push(progress);
        }
        async fn scrape_result(&self, summary: &super::scrape::ScrapeSummary) {
            self.results.lock().unwrap().push((summary.success_ids.len() as u32, summary.unmatched, summary.failed));
        }
        fn cancel_flag(&self) -> Arc<AtomicBool> {
            Arc::clone(&self.cancel)
        }
    }

    #[derive(Default)]
    pub struct FakeEvents(pub Mutex<u32>);

    impl Events for FakeEvents {
        fn library_updated(&self) {
            *self.0.lock().unwrap() += 1;
        }
    }

    pub fn metadata() -> scraper_kit::ScrapedMetadata {
        serde_json::from_value(serde_json::json!({
            "sourceId": "tmdb:1", "title": "Matched title", "year": 2024,
            "genres": [], "tags": [], "credits": [], "seasons": []
        })).unwrap()
    }

    /// A library in a temp dir with one movie `Original (2000)/Original.mkv`.
    pub fn movie_library(status: ScrapedStatus) -> (tempfile::TempDir, Arc<AppDatabase>, Library, MediaItem) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let db = Arc::new(AppDatabase::open_in_memory().unwrap());
        let lib = Library::new("Movies", root.to_string_lossy(), MediaType::Movie);
        db.insert_library(&lib).unwrap();
        let folder = root.join("Original (2000)");
        std::fs::create_dir(&folder).unwrap();
        let video = folder.join("Original.mkv");
        std::fs::write(&video, b"video").unwrap();
        let item = MediaItem::new_movie("Original", Some(2000), folder.to_string_lossy(), video.to_string_lossy(), lib.id.clone(), status);
        db.insert_media_items(std::slice::from_ref(&item)).unwrap();
        (dir, db, lib, item)
    }
}
