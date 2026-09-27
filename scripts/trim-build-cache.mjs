#!/usr/bin/env node
// Keeps the Rust build cache (target/) from growing without bound. Everything
// removed here is rebuilt on demand; sources, Git and app data are never touched.
//
//   node scripts/trim-build-cache.mjs        prune stale files; clear all if over the cap
//   node scripts/trim-build-cache.mjs --all  clear the whole cache now (like cargo clean)
//
// Runs before every `pnpm tauri …` (at most every 12 hours) so nothing is being
// built while it deletes. Tunables:
//   SULA_CACHE_MAX_AGE_DAYS  incremental caches and object files older than this go (default 3)
//   SULA_TARGET_CAP_GB       above this size the whole cache is cleared (default 8)

import { existsSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const target = join(root, "target");
const clearAll = process.argv.includes("--all");
const maxAgeDays = Number(process.env.SULA_CACHE_MAX_AGE_DAYS ?? 3);
const capBytes = Number(process.env.SULA_TARGET_CAP_GB ?? 8) * 1024 ** 3;
const stamp = join(target, ".sula-trim-stamp");
const HOUR = 3600 * 1000;

// Only ever touch a directory cargo itself marked as a build cache.
if (!existsSync(join(target, "CACHEDIR.TAG"))) process.exit(0);
if (!clearAll && existsSync(stamp) && Date.now() - statSync(stamp).mtimeMs < 12 * HOUR) process.exit(0);

function sizeOf(path) {
  let total = 0;
  const stack = [path];
  while (stack.length) {
    const current = stack.pop();
    let entries;
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      const child = join(current, entry.name);
      if (entry.isDirectory()) stack.push(child);
      else if (entry.isFile()) {
        try {
          total += statSync(child).size;
        } catch {
          // Removed meanwhile.
        }
      }
    }
  }
  return total;
}

function remove(path) {
  const size = sizeOf(path);
  rmSync(path, { recursive: true, force: true });
  return size;
}

function olderThan(path, cutoff) {
  try {
    return statSync(path).mtimeMs < cutoff;
  } catch {
    return false;
  }
}

// debug/, release/ and per-target-triple directories hold the artifacts.
const profiles = readdirSync(target, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => join(target, entry.name));

let freed = 0;
let cleared = clearAll;
if (!clearAll) {
  const cutoff = Date.now() - maxAgeDays * 24 * HOUR;
  for (const profile of profiles) {
    // One directory per crate and session; old sessions are never reused.
    const incremental = join(profile, "incremental");
    if (existsSync(incremental)) {
      for (const entry of readdirSync(incremental)) {
        const path = join(incremental, entry);
        if (olderThan(path, cutoff)) freed += remove(path);
      }
    }
    // Object files kept for debug info pile up with every rebuild.
    const deps = join(profile, "deps");
    if (existsSync(deps)) {
      for (const entry of readdirSync(deps)) {
        const path = join(deps, entry);
        if (entry.endsWith(".o") && olderThan(path, cutoff)) freed += remove(path);
      }
    }
  }
  cleared = sizeOf(target) > capBytes;
}
if (cleared) {
  for (const profile of profiles) freed += remove(profile);
}

writeFileSync(stamp, "");
if (freed > 0) {
  const gb = (freed / 1024 ** 3).toFixed(1);
  console.log(`trim-build-cache: freed ${gb} GB${cleared ? " (cache cleared; next build starts fresh)" : ""}`);
}
