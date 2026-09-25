/** Trailing debounce that merges a task's completion with its library-updated event. */
const RELOAD_DEBOUNCE_MS = 150;

export type ReloadRequest = {
  libraryId: string;
  posters: "changed" | "all";
  waiters: Array<() => void>;
};

let pendingReload: ReloadRequest | null = null;
let reloadTimer: ReturnType<typeof setTimeout> | null = null;
let reloadRunning: Promise<void> | null = null;

export function scheduleReload(
  run: (req: ReloadRequest) => Promise<void>,
  libraryId: string,
  posters: "changed" | "all",
): Promise<void> {
  if (pendingReload && pendingReload.libraryId !== libraryId) {
    pendingReload.waiters.forEach((resolve) => resolve());
    pendingReload = null;
  }
  const req: ReloadRequest = pendingReload ?? { libraryId, posters, waiters: [] };
  if (posters === "all") req.posters = "all";
  pendingReload = req;
  const done = new Promise<void>((resolve) => req.waiters.push(resolve));
  if (reloadTimer) clearTimeout(reloadTimer);
  reloadTimer = setTimeout(() => {
    reloadTimer = null;
    void flushReload(run);
  }, RELOAD_DEBOUNCE_MS);
  return done;
}

async function flushReload(run: (req: ReloadRequest) => Promise<void>) {
  // A request made mid-flight may postdate the pages already fetched: run again.
  while (reloadRunning) await reloadRunning;
  const req = pendingReload;
  if (!req || reloadTimer) return; // a newer timer owns this request
  pendingReload = null;
  reloadRunning = run(req)
    .catch(() => {})
    .finally(() => {
      reloadRunning = null;
      req.waiters.forEach((resolve) => resolve());
    });
  await reloadRunning;
}
