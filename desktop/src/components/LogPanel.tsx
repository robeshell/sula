import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type LogLevel = "debug" | "info" | "warning" | "error";

export type LogEntry = {
  id: string;
  timestamp: string;
  level: LogLevel;
  message: string;
};

export function LogPanel({ onClose }: { onClose: () => void }) {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [filter, setFilter] = useState<LogLevel | "all">("all");
  const bottomRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    // Events that arrive before the snapshot are buffered, then merged by id,
    // so nothing logged between `listen` and `list_logs` is lost or doubled.
    let snapshotLoaded = false;
    let buffered: LogEntry[] = [];
    let clearedBeforeSnapshot = false;
    const track = (registration: Promise<() => void>) =>
      registration.then(
        (unlisten) => {
          if (disposed) unlisten();
          else unlisteners.push(unlisten);
        },
        () => {},
      );

    void (async () => {
      // Register first: `listen` is async, and a snapshot taken before it
      // resolves would miss entries emitted in between.
      await Promise.all([
        track(
          listen<LogEntry>("log://entry", (event) => {
            if (disposed) return;
            if (!snapshotLoaded) {
              buffered.push(event.payload);
              return;
            }
            setEntries((prev) => mergeLogEntries(prev, [event.payload]));
          }),
        ),
        track(
          listen("log://cleared", () => {
            if (disposed) return;
            if (!snapshotLoaded) {
              buffered = [];
              clearedBeforeSnapshot = true;
              return;
            }
            setEntries([]);
          }),
        ),
      ]);
      if (disposed) return;
      let snapshot: LogEntry[] = [];
      try {
        snapshot = await invoke<LogEntry[]>("list_logs");
      } catch {
        snapshot = [];
      }
      if (disposed) return;
      snapshotLoaded = true;
      // A clear during loading may predate the snapshot; only post-clear events are safe.
      setEntries(mergeLogEntries(clearedBeforeSnapshot ? [] : snapshot, buffered));
      buffered = [];
    })();

    return () => {
      disposed = true;
      unlisteners.splice(0).forEach((unlisten) => unlisten());
    };
  }, []);

  const filtered = useMemo(() => {
    if (filter === "all") return entries;
    return entries.filter((e) => e.level === filter);
  }, [entries, filter]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [filtered.length]);

  async function clear() {
    try {
      await invoke("clear_logs");
      setEntries([]);
    } catch {
      /* ignore */
    }
  }

  const levels: (LogLevel | "all")[] = ["all", "info", "warning", "error", "debug"];
  return (
    <>
      <header className="sl-toolbar">
        <div data-tauri-drag-region />
        <div className="sl-title"><b>{t("logs.title")}</b><span>{t("list.itemCount", { count: filtered.length })}</span></div>
        <div className="sl-seg" role="radiogroup" aria-label={t("logs.title")}>
          {levels.map((level) => (
            <button key={level} type="button" role="radio" aria-checked={filter === level} onClick={() => setFilter(level)}>
              {t(`logs.filter.${level}`)}
            </button>
          ))}
        </div>
        <span className="sl-vsep" aria-hidden />
        <button type="button" className="sl-btn" disabled={entries.length === 0} onClick={() => void clear()}>{t("logs.clear")}</button>
      </header>
      <div className="sl-logs">
        {filtered.length === 0 ? (
          <p className="sl-rn-empty">{t("logs.empty")}</p>
        ) : (
          <ul className="kg-log-list">
            {filtered.map((entry, i) => (
              <li key={entry.id} className="kg-log-row" data-odd={i % 2 === 1 || undefined}>
                <span className="kg-log-time" title={entry.timestamp}>{formatLogTime(entry.timestamp)}</span>
                <span className="kg-log-level" data-level={entry.level}>{entry.level === "warning" ? "warn" : entry.level}</span>
                <span className="kg-log-msg" data-level={entry.level}>{entry.message}</span>
              </li>
            ))}
            <div ref={bottomRef} />
          </ul>
        )}
      </div>
    </>
  );
}

/** Local wall-clock time; the full timestamp stays in the tooltip. */
function formatLogTime(timestamp: string): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return timestamp;
  return date.toLocaleTimeString(undefined, { hour12: false });
}

const MAX_LOG_ENTRIES = 500;

/** Append entries not already present (by id), keeping the newest MAX_LOG_ENTRIES. */
function mergeLogEntries(base: LogEntry[], extra: LogEntry[]): LogEntry[] {
  if (extra.length === 0) return base.slice(-MAX_LOG_ENTRIES);
  const seen = new Set(base.map((entry) => entry.id));
  const next = [...base];
  for (const entry of extra) {
    if (seen.has(entry.id)) continue;
    seen.add(entry.id);
    next.push(entry);
  }
  return next.length > MAX_LOG_ENTRIES ? next.slice(next.length - MAX_LOG_ENTRIES) : next;
}
