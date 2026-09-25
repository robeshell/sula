use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskKind {
    Smoke,
    Refresh,
    BatchScrape,
    Scrape,
    Rescrape,
    ManualMatch,
    Rename,
    Organize,
    Cleanup,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub completed: u32,
    pub total: u32,
    pub current: String,
    pub stage_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub success: u32,
    pub unmatched: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    pub id: String,
    pub title: String,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub progress: Option<TaskProgress>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub result: Option<TaskResult>,
    #[serde(default)]
    pub target_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

struct TaskRecord {
    snapshot: TaskSnapshot,
    scope: Option<String>,
    cancel: Arc<AtomicBool>,
    work: Option<TaskWork>,
}

type TaskWork = Box<dyn FnOnce(TaskHandle) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> + Send>;

#[derive(Clone)]
pub struct TaskHandle {
    pub id: String,
    cancel: Arc<AtomicBool>,
    queue: Arc<TaskQueueInner>,
}

impl TaskHandle {
    pub fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub async fn record_scrape_result(&self, summary: &scraper_kit::ScrapeSummary) {
        self.queue.update(&self.id, |snap| {
            snap.result = Some(TaskResult { success: summary.success_ids.len() as u32, unmatched: summary.unmatched, failed: summary.failed });
        }).await;
    }

    /// Use only for cancellable async I/O, never for detached blocking workers.
    pub async fn run_cancellable<T>(&self, future: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
        tokio::pin!(future);
        loop {
            if self.is_cancelled() { return Err("cancelled".into()); }
            tokio::select! {
                result = &mut future => return result,
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
            }
        }
    }

    pub async fn update_progress(&self, progress: TaskProgress) {
        self.queue
            .update(&self.id, |snap| {
                snap.progress = Some(progress);
                snap.updated_at = Utc::now();
            })
            .await;
    }
}

struct TaskQueueInner {
    tasks: Mutex<Vec<TaskRecord>>,
    wake: Notify,
    mutation_gate: Arc<Mutex<()>>,
    recover: Arc<dyn Fn() -> Result<(), String> + Send + Sync>,
    history: Option<std::path::PathBuf>,
}

pub struct TaskQueue {
    inner: Arc<TaskQueueInner>,
}

impl TaskQueue {
    #[cfg(test)]
    pub fn new() -> Self { Self::with_recovery(|| Ok(())) }

    #[cfg(test)]
    pub fn with_recovery(recover: impl Fn() -> Result<(), String> + Send + Sync + 'static) -> Self {
        Self::start(recover, None, Vec::new())
    }

    pub fn open(history: std::path::PathBuf, recover: impl Fn() -> Result<(), String> + Send + Sync + 'static) -> Result<Self, String> {
        let snapshots: Vec<TaskSnapshot> = match std::fs::read(&history) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(snapshots) => snapshots,
                Err(error) => {
                    let backup = history.with_extension(format!("invalid-{}.json", Uuid::new_v4()));
                    std::fs::rename(&history, &backup).map_err(|e| e.to_string())?;
                    tracing::error!(%error, backup = %backup.display(), "invalid task history preserved");
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        let records = snapshots.into_iter().rev().take(256).collect::<Vec<_>>().into_iter().rev().map(|mut snapshot| {
            if matches!(snapshot.status, TaskStatus::Pending | TaskStatus::Running) {
                snapshot.status = TaskStatus::Failed;
                snapshot.error_message = Some("interrupted by application exit; completed changes were preserved".into());
                snapshot.updated_at = Utc::now();
            }
            TaskRecord { snapshot, scope: None, cancel: Arc::new(AtomicBool::new(false)), work: None }
        }).collect();
        Ok(Self::start(recover, Some(history), records))
    }

    fn start(recover: impl Fn() -> Result<(), String> + Send + Sync + 'static, history: Option<std::path::PathBuf>, records: Vec<TaskRecord>) -> Self {
        let inner = Arc::new(TaskQueueInner {
            tasks: Mutex::new(records),
            wake: Notify::new(),
            mutation_gate: Arc::new(Mutex::new(())),
            recover: Arc::new(recover),
            history,
        });
        let worker = Arc::clone(&inner);
        tauri::async_runtime::spawn(async move {
            worker_loop(worker).await;
        });
        Self { inner }
    }

    /// Shared by queued jobs and direct file-mutating commands.
    pub async fn lock_mutations(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
        let guard = Arc::clone(&self.inner.mutation_gate).lock_owned().await;
        (self.inner.recover)()?;
        Ok(guard)
    }

    pub async fn list(&self) -> Vec<TaskSnapshot> {
        self.inner
            .tasks
            .lock()
            .await
            .iter()
            .map(|t| t.snapshot.clone())
            .collect()
    }

    pub async fn enqueue_smoke(&self, title: impl Into<String>) -> TaskSnapshot {
        self.enqueue(title, TaskKind::Smoke, None, |_handle| {
            Box::pin(async move {
                for step in 1..=5u32 {
                    if _handle.is_cancelled() {
                        return Err("cancelled".into());
                    }
                    _handle
                        .update_progress(TaskProgress {
                            completed: step,
                            total: 5,
                            current: format!("smoke step {step}/5"),
                            stage_key: Some("smoke".into()),
                        })
                        .await;
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                Ok(())
            })
        })
        .await
    }

    pub async fn find_active(&self, kind: TaskKind, target_id: &str) -> Option<TaskSnapshot> {
        self.list().await.into_iter().find(|t| {
            t.kind == kind
                && t.target_id.as_deref() == Some(target_id)
                && matches!(t.status, TaskStatus::Pending | TaskStatus::Running)
        })
    }

    pub async fn enqueue<F>(
        &self,
        title: impl Into<String>,
        kind: TaskKind,
        target_id: Option<String>,
        work: F,
    ) -> TaskSnapshot
    where
        F: FnOnce(TaskHandle) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
            + Send
            + 'static,
    {
        let scope = if matches!(kind, TaskKind::BatchScrape) { target_id.clone() } else { None };
        self.enqueue_scoped(title, kind, target_id, scope, work).await
    }

    pub async fn enqueue_scoped<F>(&self, title: impl Into<String>, kind: TaskKind,
        target_id: Option<String>, scope: Option<String>, work: F) -> TaskSnapshot
    where F: FnOnce(TaskHandle) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> + Send + 'static,
    {
        let title = title.into();
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut snapshot = TaskSnapshot {
            id: id.clone(),
            title,
            kind,
            status: TaskStatus::Pending,
            progress: None,
            error_message: None,
            result: None,
            target_id,
            created_at: now,
            updated_at: now,
        };

        let work: TaskWork = Box::new(work);

        {
            let mut tasks = self.inner.tasks.lock().await;
            if scope.is_some() {
                if let Some(existing) = tasks.iter().find(|t| t.snapshot.kind == kind
                    && t.scope == scope
                    && matches!(t.snapshot.status, TaskStatus::Pending | TaskStatus::Running)) {
                    return existing.snapshot.clone();
                }
            }
            while tasks.len() >= 256 {
                if let Some(index) = tasks.iter().position(|t| !matches!(t.snapshot.status, TaskStatus::Pending | TaskStatus::Running)) { tasks.remove(index); }
                else {
                    snapshot.status = TaskStatus::Failed;
                    snapshot.error_message = Some("task queue is full; retry after active tasks finish".into());
                    return snapshot;
                }
            }
            tasks.push(TaskRecord {
                snapshot: snapshot.clone(),
                scope,
                cancel,
                work: Some(work),
            });
            if let Err(error) = self.inner.persist(&tasks) {
                snapshot.status = TaskStatus::Failed;
                snapshot.error_message = Some(error);
                if let Some(last) = tasks.last_mut() { last.snapshot = snapshot.clone(); last.work = None; }
            }
        }
        self.inner.wake.notify_one();
        snapshot
    }

    pub async fn cancel(&self, id: &str) -> bool {
        let mut tasks = self.inner.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.snapshot.id == id) {
            if !matches!(task.snapshot.status, TaskStatus::Pending | TaskStatus::Running) { return false; }
            task.cancel.store(true, Ordering::SeqCst);
            if task.snapshot.status == TaskStatus::Pending {
                task.snapshot.status = TaskStatus::Cancelled;
                task.snapshot.updated_at = Utc::now();
                task.work = None;
            }
            if let Err(error) = self.inner.persist(&tasks) { tracing::error!(%error, "task cancellation persistence failed"); }
            return true;
        }
        false
    }

    pub async fn cancel_active(&self) -> bool {
        let id = {
            let tasks = self.inner.tasks.lock().await;
            tasks
                .iter()
                .find(|t| {
                    matches!(
                        t.snapshot.status,
                        TaskStatus::Pending | TaskStatus::Running
                    )
                })
                .map(|t| t.snapshot.id.clone())
        };
        match id {
            Some(id) => self.cancel(&id).await,
            None => false,
        }
    }
}

impl TaskQueueInner {
    fn persist(&self, tasks: &[TaskRecord]) -> Result<(), String> {
        let Some(path) = &self.history else { return Ok(()); };
        let snapshots: Vec<_> = tasks.iter().map(|t| &t.snapshot).collect();
        let bytes = serde_json::to_vec(&snapshots).map_err(|e| e.to_string())?;
        media_core::FilesystemService::new().write_file(&bytes, path, media_core::WriteOptions {
            collision_policy: media_core::CollisionPolicy::Replace, ..Default::default()
        }).map_err(|e| e.to_string())?;
        Ok(())
    }
    async fn update(&self, id: &str, f: impl FnOnce(&mut TaskSnapshot)) {
        let mut tasks = self.tasks.lock().await;
        if let Some(task) = tasks.iter_mut().find(|t| t.snapshot.id == id) {
            f(&mut task.snapshot);
        }
        if let Err(error) = self.persist(&tasks) { tracing::error!(%error, "task history write failed"); }
    }
}

async fn worker_loop(inner: Arc<TaskQueueInner>) {
    loop {
        let next = {
            let mut tasks = inner.tasks.lock().await;
            tasks
                .iter_mut()
                .find(|t| t.snapshot.status == TaskStatus::Pending && t.work.is_some())
                .map(|t| {
                    t.snapshot.status = TaskStatus::Running;
                    t.snapshot.updated_at = Utc::now();
                    let work = t.work.take().expect("work present");
                    let handle = TaskHandle {
                        id: t.snapshot.id.clone(),
                        cancel: Arc::clone(&t.cancel),
                        queue: Arc::clone(&inner),
                    };
                    (work, handle)
                })
        };

        let Some((work, handle)) = next else {
            inner.wake.notified().await;
            continue;
        };

        let gate = Arc::clone(&inner.mutation_gate);
        let recover = Arc::clone(&inner.recover);
        let job_handle = handle.clone();
        // A panic belongs to this job, not the long-lived queue worker.
        let result = tauri::async_runtime::spawn(async move {
            let _guard = gate.lock_owned().await;
            if job_handle.is_cancelled() { return Err("cancelled".into()); }
            recover()?;
            work(job_handle).await
        }).await.unwrap_or_else(|_| Err("task stopped unexpectedly".into()));
        {
            let mut tasks = inner.tasks.lock().await;
            if let Some(task) = tasks.iter_mut().find(|t| t.snapshot.id == handle.id) {
                task.snapshot.updated_at = Utc::now();
                if handle.is_cancelled() {
                    task.snapshot.status = TaskStatus::Cancelled;
                } else if let Err(err) = result {
                    task.snapshot.status = TaskStatus::Failed;
                    task.snapshot.error_message = Some(err);
                } else {
                    task.snapshot.status = TaskStatus::Completed;
                }
            }
            if let Err(error) = inner.persist(&tasks) { tracing::error!(%error, "task completion persistence failed"); }
        }

    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn terminal(queue: &TaskQueue, id: &str) -> TaskSnapshot {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let task = queue.list().await.into_iter().find(|t| t.id == id).unwrap();
                if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled) { return task; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap()
    }

    #[tokio::test]
    async fn pending_recovery_blocks_direct_and_queued_mutations() {
        let blocked = Arc::new(AtomicBool::new(true));
        let check = Arc::clone(&blocked);
        let queue = TaskQueue::with_recovery(move || if check.load(Ordering::SeqCst) { Err("recovery pending".into()) } else { Ok(()) });
        assert!(queue.lock_mutations().await.is_err());
        let task = queue.enqueue("blocked", TaskKind::Rename, None, |_| Box::pin(async { panic!("must not run") })).await;
        let failed = terminal(&queue, &task.id).await;
        assert_eq!(failed.status, TaskStatus::Failed);
        assert_eq!(failed.error_message.as_deref(), Some("recovery pending"));
        blocked.store(false, Ordering::SeqCst);
        assert!(queue.lock_mutations().await.is_ok());
        let next = queue.enqueue("ready", TaskKind::Rename, None, |_| Box::pin(async { Ok(()) })).await;
        assert_eq!(terminal(&queue, &next.id).await.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn panic_does_not_stop_the_queue() {
        let queue = TaskQueue::new();
        let bad = queue.enqueue("bad", TaskKind::Smoke, None, |_| Box::pin(async { panic!("injected") })).await;
        let good = queue.enqueue("good", TaskKind::Smoke, None, |_| Box::pin(async { Ok(()) })).await;
        assert_eq!(terminal(&queue, &bad.id).await.status, TaskStatus::Failed);
        assert_eq!(terminal(&queue, &good.id).await.status, TaskStatus::Completed);
        assert!(!queue.cancel(&good.id).await);
    }

    #[tokio::test]
    async fn direct_mutation_gate_blocks_jobs_and_cancellation_skips_work() {
        let queue = TaskQueue::new();
        let guard = queue.lock_mutations().await.unwrap();
        let ran = Arc::new(AtomicBool::new(false));
        let ran_job = Arc::clone(&ran);
        let task = queue.enqueue("job", TaskKind::Rename, None, move |_| Box::pin(async move {
            ran_job.store(true, Ordering::SeqCst); Ok(())
        })).await;
        tokio::task::yield_now().await;
        assert!(!ran.load(Ordering::SeqCst));
        queue.cancel(&task.id).await;
        drop(guard);
        assert_eq!(terminal(&queue, &task.id).await.status, TaskStatus::Cancelled);
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn batch_scrape_enqueue_deduplicates_under_lock() {
        let queue = TaskQueue::new();
        let _guard = queue.lock_mutations().await.unwrap();
        let a = queue.enqueue("a", TaskKind::BatchScrape, Some("library".into()), |_| Box::pin(async { Ok(()) }));
        let b = queue.enqueue("b", TaskKind::BatchScrape, Some("library".into()), |_| Box::pin(async { Ok(()) }));
        let (a, b) = tokio::join!(a, b);
        assert_eq!(a.id, b.id);
        queue.cancel(&a.id).await;
    }
    #[tokio::test]
    async fn cancelled_async_io_releases_queue() {
        let queue = TaskQueue::new();
        let started = Arc::new(Notify::new()); let signal = started.clone();
        let task = queue.enqueue("io", TaskKind::Scrape, None, move |handle| Box::pin(async move {
            signal.notify_one();
            handle.run_cancellable(std::future::pending::<Result<(), String>>()).await
        })).await;
        started.notified().await;
        queue.cancel(&task.id).await;
        assert_eq!(terminal(&queue, &task.id).await.status, TaskStatus::Cancelled);
        let next = queue.enqueue("next", TaskKind::Smoke, None, |_| Box::pin(async { Ok(()) })).await;
        assert_eq!(terminal(&queue, &next.id).await.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn exact_scope_dedup_preserves_different_batches() {
        let queue = TaskQueue::new(); let _guard = queue.lock_mutations().await.unwrap();
        let a = queue.enqueue_scoped("a", TaskKind::Scrape, Some("first".into()), Some("first,second".into()), |_| Box::pin(async { Ok(()) })).await;
        let b = queue.enqueue_scoped("b", TaskKind::Scrape, Some("first".into()), Some("first,second".into()), |_| Box::pin(async { Ok(()) })).await;
        let c = queue.enqueue_scoped("c", TaskKind::Scrape, Some("first".into()), Some("first,third".into()), |_| Box::pin(async { Ok(()) })).await;
        assert_eq!(a.id, b.id); assert_ne!(a.id, c.id);
        queue.cancel(&a.id).await; queue.cancel(&c.id).await;
    }

    #[tokio::test]
    async fn history_recovers_interrupted_jobs_without_replaying_and_is_bounded() {
        let dir = std::env::temp_dir().join(format!("sula-task-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap(); let path = dir.join("history.json");
        let now = Utc::now();
        let snapshots: Vec<_> = (0..300).map(|i| TaskSnapshot {
            id: i.to_string(), title: "old".into(), kind: TaskKind::Rename,
            status: if i == 299 { TaskStatus::Running } else { TaskStatus::Completed },
            progress: None, error_message: None, result: None, target_id: None, created_at: now, updated_at: now,
        }).collect();
        std::fs::write(&path, serde_json::to_vec(&snapshots).unwrap()).unwrap();
        let queue = TaskQueue::open(path.clone(), || Ok(())).unwrap();
        let tasks = queue.list().await;
        assert_eq!(tasks.len(), 256); assert_eq!(tasks.last().unwrap().status, TaskStatus::Failed);
        let next = queue.enqueue("next", TaskKind::Smoke, None, |_| Box::pin(async { Ok(()) })).await;
        terminal(&queue, &next.id).await;
        assert_eq!(queue.list().await.len(), 256);
        let recovered = TaskQueue::open(path.clone(), || Ok(())).unwrap();
        assert_eq!(recovered.list().await.last().unwrap().status, TaskStatus::Completed);
        std::fs::write(&path, "broken").unwrap();
        assert!(TaskQueue::open(path, || Ok(())).unwrap().list().await.is_empty());
        assert!(std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("invalid-")));
        std::fs::remove_dir_all(dir).unwrap();
    }

}
