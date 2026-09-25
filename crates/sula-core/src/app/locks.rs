//! File-mutation locks. A library-scoped lock shares the global lock and then takes
//! one mutex per library in sorted order, so two libraries mutate in parallel and
//! a multi-library request can never deadlock against another. `Global` excludes
//! every library lock (arbitrary paths, settings that reset scan state). Every
//! acquisition runs the media-operation recovery check before it is handed out.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use media_core::AppDatabase;
use tokio::sync::{OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockScope {
    Global,
    Libraries(Vec<String>),
}

impl LockScope {
    pub fn library(id: &str) -> Self {
        Self::Libraries(vec![id.to_string()])
    }

    /// Libraries of the items that still exist; a vanished item needs no lock.
    pub fn for_items(db: &AppDatabase, item_ids: &[String]) -> Result<Self, String> {
        let mut libraries = Vec::new();
        for id in item_ids {
            if let Some(item) = db.get_media_item(id).map_err(super::err_string)? {
                libraries.push(item.library_id);
            }
        }
        libraries.sort();
        libraries.dedup();
        Ok(Self::Libraries(libraries))
    }
}

type Recover = dyn Fn() -> Result<(), String> + Send + Sync;
type Expand = dyn Fn(&[String]) -> Vec<String> + Send + Sync;

pub struct MutationLocks {
    global: Arc<RwLock<()>>,
    libraries: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    recover: Box<Recover>,
    expand: Box<Expand>,
}

pub struct MutationGuard {
    // Library guards drop before the global guard they were taken under.
    _libraries: Vec<OwnedMutexGuard<()>>,
    _global: GlobalGuard,
}

enum GlobalGuard {
    Shared(#[allow(dead_code)] OwnedRwLockReadGuard<()>),
    Exclusive(#[allow(dead_code)] OwnedRwLockWriteGuard<()>),
}

impl MutationLocks {
    /// `expand` adds libraries whose roots overlap the requested ones: nested roots
    /// share files, so they must share the lock too.
    pub fn new(
        recover: impl Fn() -> Result<(), String> + Send + Sync + 'static,
        expand: impl Fn(&[String]) -> Vec<String> + Send + Sync + 'static,
    ) -> Self {
        Self { global: Arc::new(RwLock::new(())), libraries: Mutex::new(HashMap::new()), recover: Box::new(recover), expand: Box::new(expand) }
    }

    pub fn for_database(db: Arc<AppDatabase>, recover: impl Fn() -> Result<(), String> + Send + Sync + 'static) -> Self {
        Self::new(recover, move |ids| overlapping_libraries(&db, ids))
    }

    #[cfg(test)]
    pub fn unchecked() -> Self {
        Self::new(|| Ok(()), |ids| ids.to_vec())
    }

    /// Acquire the scope and verify no media operation is left to recover.
    pub async fn lock(&self, scope: &LockScope) -> Result<MutationGuard, String> {
        let guard = self.acquire(scope).await;
        self.recover()?;
        Ok(guard)
    }

    /// Acquire without the recovery check; the caller must run [`Self::recover`].
    pub async fn acquire(&self, scope: &LockScope) -> MutationGuard {
        match scope {
            LockScope::Global => MutationGuard {
                _libraries: Vec::new(),
                _global: GlobalGuard::Exclusive(Arc::clone(&self.global).write_owned().await),
            },
            LockScope::Libraries(ids) => {
                let mut keys = (self.expand)(ids);
                keys.extend(ids.iter().cloned());
                keys.sort();
                keys.dedup();
                let global = GlobalGuard::Shared(Arc::clone(&self.global).read_owned().await);
                let mut libraries = Vec::with_capacity(keys.len());
                for key in keys {
                    let mutex = Arc::clone(self.libraries.lock().unwrap_or_else(|e| e.into_inner()).entry(key).or_default());
                    libraries.push(mutex.lock_owned().await);
                }
                MutationGuard { _libraries: libraries, _global: global }
            }
        }
    }

    pub fn recover(&self) -> Result<(), String> {
        (self.recover)()
    }
}

/// `ids` plus every library whose root contains, or lies inside, one of theirs.
/// Lexical comparison on stored roots: resolving an offline share could hang.
pub fn overlapping_libraries(db: &AppDatabase, ids: &[String]) -> Vec<String> {
    let Ok(libraries) = db.list_libraries() else { return ids.to_vec() };
    let roots: Vec<&str> = libraries.iter().filter(|l| ids.contains(&l.id)).map(|l| l.root_path.as_str()).collect();
    let mut out = ids.to_vec();
    for library in &libraries {
        if roots.iter().any(|root| media_core::db::path_rooted_under(&library.root_path, root) || media_core::db::path_rooted_under(root, &library.root_path)) {
            out.push(library.id.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;

    fn scope(ids: &[&str]) -> LockScope {
        LockScope::Libraries(ids.iter().map(|s| s.to_string()).collect())
    }

    async fn blocked<T>(future: impl std::future::Future<Output = T>) -> bool {
        tokio::time::timeout(Duration::from_millis(100), future).await.is_err()
    }

    #[tokio::test]
    async fn different_libraries_do_not_block_each_other() {
        let locks = MutationLocks::unchecked();
        let _a = locks.lock(&scope(&["a"])).await.unwrap();
        assert!(!blocked(locks.lock(&scope(&["b"]))).await);
    }

    #[tokio::test]
    async fn same_library_is_serialized_and_global_excludes_all() {
        let locks = Arc::new(MutationLocks::unchecked());
        let a = locks.lock(&scope(&["a"])).await.unwrap();
        assert!(blocked(locks.lock(&scope(&["a"]))).await);
        assert!(blocked(locks.lock(&LockScope::Global)).await);
        drop(a);
        let global = locks.lock(&LockScope::Global).await.unwrap();
        assert!(blocked(locks.lock(&scope(&["b"]))).await);
        drop(global);
        // Critical sections of one library never overlap.
        let (inside, overlaps) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU32::new(0)));
        let mut jobs = Vec::new();
        for _ in 0..8 {
            let (locks, inside, overlaps) = (Arc::clone(&locks), Arc::clone(&inside), Arc::clone(&overlaps));
            jobs.push(tokio::spawn(async move {
                let _guard = locks.lock(&scope(&["a"])).await.unwrap();
                if inside.swap(true, Ordering::SeqCst) { overlaps.fetch_add(1, Ordering::SeqCst); }
                tokio::time::sleep(Duration::from_millis(5)).await;
                inside.store(false, Ordering::SeqCst);
            }));
        }
        for job in jobs { job.await.unwrap(); }
        assert_eq!(overlaps.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn cross_library_requests_take_locks_in_one_order() {
        let locks = Arc::new(MutationLocks::unchecked());
        let mut jobs = Vec::new();
        for i in 0..32 {
            let locks = Arc::clone(&locks);
            // Opposite request orders would deadlock without sorting.
            let ids = if i % 2 == 0 { scope(&["a", "b"]) } else { scope(&["b", "a"]) };
            jobs.push(tokio::spawn(async move {
                let _guard = locks.lock(&ids).await.unwrap();
                tokio::task::yield_now().await;
            }));
        }
        tokio::time::timeout(Duration::from_secs(5), async { for job in jobs { job.await.unwrap(); } }).await.expect("no deadlock");
        let _b = locks.lock(&scope(&["b"])).await.unwrap();
        assert!(blocked(locks.lock(&scope(&["a", "b"]))).await);
    }

    #[tokio::test]
    async fn every_acquisition_runs_recovery() {
        let pending = Arc::new(AtomicBool::new(true));
        let check = Arc::clone(&pending);
        let locks = MutationLocks::new(move || if check.load(Ordering::SeqCst) { Err("recovery pending".into()) } else { Ok(()) }, |ids| ids.to_vec());
        assert_eq!(locks.lock(&scope(&["a"])).await.err().as_deref(), Some("recovery pending"));
        assert!(locks.lock(&LockScope::Global).await.is_err());
        pending.store(false, Ordering::SeqCst);
        assert!(locks.lock(&scope(&["a"])).await.is_ok());
    }

    #[tokio::test]
    async fn nested_library_roots_share_a_lock() {
        let db = Arc::new(AppDatabase::open_in_memory().unwrap());
        let outer = media_core::Library::new("All", "/media", media_core::MediaType::Movie);
        let inner = media_core::Library::new("Kids", "/media/kids", media_core::MediaType::Movie);
        let other = media_core::Library::new("Shows", "/media-shows", media_core::MediaType::TvShow);
        for library in [&outer, &inner, &other] { db.insert_library(library).unwrap(); }
        let mut expected = vec![outer.id.clone(), inner.id.clone()];
        expected.sort();
        assert_eq!(overlapping_libraries(&db, std::slice::from_ref(&inner.id)), expected);
        assert_eq!(overlapping_libraries(&db, std::slice::from_ref(&other.id)), vec![other.id.clone()]);
        let locks = MutationLocks::for_database(Arc::clone(&db), || Ok(()));
        let _outer = locks.lock(&LockScope::library(&outer.id)).await.unwrap();
        assert!(blocked(locks.lock(&LockScope::library(&inner.id))).await);
        assert!(!blocked(locks.lock(&LockScope::library(&other.id))).await);
    }
}
