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
/** Per-folder invalidation counters; cleared on a global invalidation. */
const folderGenerations = new Map<string, number>();
const listeners = new Set<() => void>();
export const posterCacheVersion = () => generation;
/** Version token for one folder: changes on a global or a folder-specific invalidation. */
export function posterFolderVersion(folderPath: string): string {
  return `${generation}:${folderGenerations.get(folderPath) ?? 0}`;
}
export function subscribePosterCache(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function invalidatePosterCache() {
  generation += 1;
  folderGenerations.clear();
  cache.clear();
  listeners.forEach((listener) => listener());
}

/** Invalidate only the posters that live under the given media folders. */
export function invalidatePosterFolders(folderPaths: Iterable<string>) {
  const folders = new Set(folderPaths);
  if (folders.size === 0) return;
  for (const folder of folders) {
    folderGenerations.set(folder, (folderGenerations.get(folder) ?? 0) + 1);
  }
  for (const key of [...cache.keys()]) {
    if (folders.has(key.slice(0, key.indexOf("\0")))) cache.delete(key);
  }
  listeners.forEach((listener) => listener());
}

type Waiter = { start: () => void; cancelled: boolean };

let active = 0;
const waiters: Waiter[] = [];
const cache = new Map<string, Promise<string | null>>();

/**
 * Take a slot, or queue for one. The newest waiter is served first (LIFO) so
 * the cells on screen after a fast scroll do not wait behind ones scrolled past.
 */
function acquire(): { ready: Promise<void>; waiter: Waiter | null } {
  if (active < MAX_CONCURRENT) {
    active += 1;
    return { ready: Promise.resolve(), waiter: null };
  }
  let waiter!: Waiter;
  const ready = new Promise<void>((resolve) => {
    waiter = {
      cancelled: false,
      start: () => {
        active += 1;
        resolve();
      },
    };
  });
  waiters.push(waiter);
  return { ready, waiter };
}

function release() {
  active = Math.max(0, active - 1);
  while (waiters.length > 0) {
    const next = waiters.pop()!;
    if (next.cancelled) continue;
    next.start();
    break;
  }
}

export type ResolvePosterOptions = {
  folderPath: string;
  posterPath?: string | null;
  width?: number;
  height?: number;
  allowFallbacks?: boolean;
  /**
   * Abort when the caller no longer needs the result (e.g. the cell unmounted).
   * A queued request whose every caller aborted never reaches IPC and resolves null.
   */
  signal?: AbortSignal;
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

type JobState = {
  /** Callers still waiting; Infinity once any caller subscribed without a signal. */
  interest: number;
  started: boolean;
  settled: boolean;
  cancel: () => void;
};
const jobStates = new WeakMap<Promise<string | null>, JobState>();

function addInterest(job: Promise<string | null>, signal: AbortSignal | undefined) {
  const state = jobStates.get(job);
  if (!state || state.started || state.settled) return;
  if (!signal) {
    state.interest = Infinity;
    return;
  }
  if (signal.aborted) return;
  state.interest += 1;
  signal.addEventListener(
    "abort",
    () => {
      state.interest -= 1;
      if (state.interest <= 0 && !state.started && !state.settled) state.cancel();
    },
    { once: true },
  );
}

/**
 * Resolve a poster to an asset URL. Results are memoized; work is queued
 * so many visible cells cannot flood the thumbnail pipeline.
 */
export function resolvePosterSrc(opts: ResolvePosterOptions): Promise<string | null> {
  if (opts.signal?.aborted) return Promise.resolve(null);
  const key = cacheKey(opts);
  const hit = cache.get(key);
  if (hit) {
    cache.delete(key);
    cache.set(key, hit);
    addInterest(hit, opts.signal);
    return hit;
  }
  const version = posterFolderVersion(opts.folderPath);
  const state: JobState = { interest: 0, started: false, settled: false, cancel: () => {} };

  const job = new Promise<string | null>((resolveJob) => {
    const { ready, waiter } = acquire();
    state.cancel = () => {
      if (waiter) waiter.cancelled = true;
      state.settled = true;
      if (cache.get(key) === job) cache.delete(key);
      resolveJob(null);
    };
    void ready.then(async () => {
      if (state.settled) {
        // Cancelled after the slot was granted synchronously; hand it back.
        release();
        return;
      }
      state.started = true;
      try {
        if (version !== posterFolderVersion(opts.folderPath)) return resolveJob(null);
        const posterPath = opts.posterPath?.trim() || "poster.jpg";
        const cachePath = await invoke<string | null>("resolve_poster_thumbnail", {
          folderPath: opts.folderPath,
          posterPath,
          width: opts.width ?? POSTER_THUMB.width,
          height: opts.height ?? POSTER_THUMB.height,
          allowFallbacks: opts.allowFallbacks ?? true,
        });
        resolveJob(
          cachePath && version === posterFolderVersion(opts.folderPath)
            ? convertFileSrc(cachePath)
            : null,
        );
      } catch {
        resolveJob(null);
      } finally {
        state.settled = true;
        release();
      }
    });
  });

  jobStates.set(job, state);
  addInterest(job, opts.signal);
  cache.set(key, job);
  while (cache.size > MAX_CACHE_ENTRIES) cache.delete(cache.keys().next().value!);
  void job.then((value) => {
    // Missing/error results are retryable, and an old request must not evict a new one.
    if (!value && cache.get(key) === job) cache.delete(key);
  });
  return job;
}
