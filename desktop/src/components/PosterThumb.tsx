import { useEffect, useRef, useState, useSyncExternalStore } from "react";

import { resolvePosterSrc, POSTER_THUMB, subscribePosterCache, posterFolderVersion } from "../lib/posterLoadQueue";

type PosterThumbProps = {
  folderPath: string;
  posterPath?: string | null;
  /** Tried in order before falling back to posterPath / poster.jpg */
  posterCandidates?: string[];
  width?: number;
  height?: number;
  allowFallbacks?: boolean;
  className?: string;
  /** Shown when no poster file exists. */
  fallbackLabel?: string;
};

/**
 * Lazy poster: only starts IPC/decode after the cell enters (near) the viewport.
 * Concurrent loads are capped by posterLoadQueue so large grids stay responsive.
 */
export function PosterThumb({
  folderPath,
  posterPath,
  posterCandidates,
  width = POSTER_THUMB.width,
  height = POSTER_THUMB.height,
  allowFallbacks = true,
  className = "",
  fallbackLabel,
}: PosterThumbProps) {
  const cacheVersion = useSyncExternalStore(subscribePosterCache, () =>
    posterFolderVersion(folderPath),
  );
  const rootRef = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);
  /** undefined = not loaded yet, null = missing, string = url */
  const [src, setSrc] = useState<string | null | undefined>(undefined);
  const candidatesKey = (posterCandidates ?? []).join("|");
  /** What the current `src` depicts; a cache invalidation alone keeps it on screen. */
  const identity = [folderPath, posterPath ?? "", candidatesKey, width, height, allowFallbacks].join("\0");
  const shownIdentity = useRef<string | null>(null);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setVisible(true);
          io.disconnect();
        }
      },
      { root: null, rootMargin: "160px 0px", threshold: 0.01 },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);

  useEffect(() => {
    if (!visible || !folderPath) return;
    let cancelled = false;
    const controller = new AbortController();
    // Different poster → skeleton. Same poster re-resolving after an
    // invalidation → keep the old image until the new one is ready.
    if (shownIdentity.current !== identity) setSrc(undefined);
    const settle = (value: string | null) => {
      shownIdentity.current = identity;
      setSrc(value);
    };

    void (async () => {
      const candidates =
        posterCandidates !== undefined
          ? posterCandidates.filter((c): c is string => Boolean(c?.trim()))
          : [posterPath?.trim() || "poster.jpg"];
      if (candidates.length === 0) {
        if (!cancelled) settle(null);
        return;
      }
      for (const candidate of candidates) {
        if (cancelled) return;
        const url = await resolvePosterSrc({
          folderPath,
          posterPath: candidate,
          width,
          height,
          allowFallbacks,
          signal: controller.signal,
        });
        if (cancelled) return;
        if (url) {
          settle(url);
          return;
        }
      }
      if (!cancelled) settle(null);
    })();

    return () => {
      cancelled = true;
      controller.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- identity covers the poster props
  }, [visible, identity, cacheVersion]);

  return (
    <div ref={rootRef} className={className}>
      {src ? (
        <img
          src={src}
          alt=""
          draggable={false}
          decoding="async"
          className="h-full w-full object-cover"
        />
      ) : src === null ? (
        <span className="flex h-full w-full items-center justify-center overflow-hidden px-2 text-center [overflow-wrap:anywhere] kg-type-caption-small font-semibold leading-tight text-fg-muted">
          {fallbackLabel || "—"}
        </span>
      ) : (
        <span className="kg-skeleton block h-full w-full" aria-hidden />
      )}
    </div>
  );
}
