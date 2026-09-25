//! Warms poster thumbnails after a task changes what a library shows, so the
//! grid opens with thumbnails ready instead of generating them on scroll.

use std::sync::Arc;

use crate::task_queue::TaskKind;

/// Must run inside the tokio runtime (the task queue's worker calls it).
pub fn warm_after_task(
    db: Arc<media_core::AppDatabase>,
    thumbs: Arc<media_core::ThumbnailCache>,
    kind: TaskKind,
    target_id: Option<String>,
) {
    let Some(target_id) = target_id else {
        return;
    };
    tokio::spawn(async move {
        let warm = tokio::task::spawn_blocking(move || match kind {
            TaskKind::Refresh | TaskKind::BatchScrape => {
                warm_library_posters(&db, &thumbs, &target_id)
            }
            TaskKind::Scrape | TaskKind::ManualMatch => {
                warm_item_poster(&db, &thumbs, &target_id)
            }
            _ => 0,
        })
        .await;
        match warm {
            Ok(n) if n > 0 => tracing::info!(count = n, "poster thumbs warmed"),
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %err, "poster warm task join failed"),
        }
    });
}

fn warm_library_posters(
    db: &media_core::AppDatabase,
    thumbs: &media_core::ThumbnailCache,
    library_id: &str,
) -> usize {
    let Ok(items) = db.list_media_items(library_id) else {
        return 0;
    };
    let Ok(metas) = db.list_metadata_summaries(library_id) else {
        return 0;
    };
    let mut by_id = std::collections::HashMap::new();
    for item in &items {
        by_id.insert(item.id.clone(), item);
    }
    let mut jobs = Vec::new();
    for meta in metas {
        let Some(poster) = meta.poster_path.as_deref().filter(|p| !p.is_empty()) else {
            continue;
        };
        let Some(item) = by_id.get(&meta.media_item_id) else {
            continue;
        };
        let Some(source) =
            media_core::ThumbnailCache::resolve_poster_source(&item.folder_path, poster)
        else {
            continue;
        };
        jobs.push(source);
    }
    if jobs.is_empty() {
        return 0;
    }

    use std::sync::atomic::{AtomicUsize, Ordering};
    let warmed = AtomicUsize::new(0);
    let workers = 4usize.min(jobs.len());
    let chunk = jobs.len().div_ceil(workers);
    std::thread::scope(|scope| {
        for piece in jobs.chunks(chunk.max(1)) {
            let piece = piece.to_vec();
            let warmed = &warmed;
            scope.spawn(move || {
                for source in &piece {
                    if thumbs.ensure_poster_thumb(source).is_ok() {
                        warmed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    warmed.load(Ordering::Relaxed)
}

fn warm_item_poster(
    db: &media_core::AppDatabase,
    thumbs: &media_core::ThumbnailCache,
    item_id: &str,
) -> usize {
    let Ok(Some(item)) = db.get_media_item(item_id) else {
        return 0;
    };
    let Ok(Some(meta)) = db.fetch_metadata(item_id) else {
        return 0;
    };
    let Some(poster) = meta.poster_path.as_deref().filter(|p| !p.is_empty()) else {
        return 0;
    };
    let Some(source) =
        media_core::ThumbnailCache::resolve_poster_source(&item.folder_path, poster)
    else {
        return 0;
    };
    if thumbs.ensure_poster_thumb(&source).is_ok() {
        1
    } else {
        0
    }
}
