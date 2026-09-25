import { convertFileSrc, invoke } from "@tauri-apps/api/core";

/** Must match media-core POSTER_THUMB_* — list grid + detail share one cache key. */
export const POSTER_THUMB = { width: 140, height: 210 } as const;
/** Same 2:3 ratio as cover poster. */
export const SEASON_THUMB = { width: 72, height: 108 } as const;
/** Episode still — landscape 16:9. */
export const EPISODE_STILL = { width: 160, height: 90 } as const;

/** Cap concurrent thumbnail IPC/decode so a full grid does not stall the UI. */
const MAX_CONCURRENT = 6;
const MAX_CACHE_ENTRIES = 512;
let generation = 0;
const listeners = new Set<() => void>();
export const posterCacheVersion = () => generation;
export function subscribePosterCache(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function invalidatePosterCache() {
  generation += 1;
  cache.clear();
  listeners.forEach((listener) => listener());
}

let active = 0;
const waiters: Array<() => void> = [];
const cache = new Map<string, Promise<string | null>>();

function acquire(): Promise<void> {
  if (active < MAX_CONCURRENT) {
    active += 1;
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    waiters.push(() => {
      active += 1;
      resolve();
    });
  });
}

function release() {
  active = Math.max(0, active - 1);
  const next = waiters.shift();
  if (next) next();
}

export type ResolvePosterOptions = {
  folderPath: string;
  posterPath?: string | null;
  width?: number;
  height?: number;
  allowFallbacks?: boolean;
};

function cacheKey(opts: ResolvePosterOptions): string {
  return [
    opts.folderPath,
    opts.posterPath ?? "",
    opts.width ?? POSTER_THUMB.width,
    opts.height ?? POSTER_THUMB.height,
    opts.allowFallbacks === false ? "0" : "1",
  ].join("\0");
}

/**
 * Resolve a poster to an asset URL. Results are memoized; work is queued
 * so many visible cells cannot flood the thumbnail pipeline.
 */
export function resolvePosterSrc(opts: ResolvePosterOptions): Promise<string | null> {
  const key = cacheKey(opts);
  const hit = cache.get(key);
  if (hit) {
    cache.delete(key);
    cache.set(key, hit);
    return hit;
  }
  const version = generation;

  const job = (async () => {
    await acquire();
    try {
      if (version !== generation) return null;
      const posterPath = opts.posterPath?.trim() || "poster.jpg";
      const cachePath = await invoke<string | null>("resolve_poster_thumbnail", {
        folderPath: opts.folderPath,
        posterPath,
        width: opts.width ?? POSTER_THUMB.width,
        height: opts.height ?? POSTER_THUMB.height,
        allowFallbacks: opts.allowFallbacks ?? true,
      });
      return cachePath && version === generation ? convertFileSrc(cachePath) : null;
    } catch {
      return null;
    } finally {
      release();
    }
  })();

  cache.set(key, job);
  while (cache.size > MAX_CACHE_ENTRIES) cache.delete(cache.keys().next().value!);
  void job.then((value) => {
    // Missing/error results are retryable, and an old request must not evict a new one.
    if (!value && cache.get(key) === job) cache.delete(key);
  });
  return job;
}
