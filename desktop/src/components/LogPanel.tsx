import { NativeSelect } from "./ui/native-select";
import { Button } from "./ui/button";
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

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="kg-page-shell flex min-h-0 flex-1 flex-col !pb-6">
        <header className="kg-page-header !mb-4">
          <div className="min-w-0 flex-1">
            <h2 className="kg-page-header-title">{t("logs.title")}</h2>
          </div>
          <div className="kg-page-header-actions">
            <NativeSelect
              value={filter}
              onChange={(e) => setFilter(e.target.value as LogLevel | "all")}
            >
              <option value="all">{t("logs.filter.all")}</option>
              <option value="debug">{t("logs.filter.debug")}</option>
              <option value="info">{t("logs.filter.info")}</option>
              <option value="warning">{t("logs.filter.warning")}</option>
              <option value="error">{t("logs.filter.error")}</option>
            </NativeSelect>
            <Button variant="ghost" size="sm" type="button" onClick={() => void clear()}>
              {t("logs.clear")}
            </Button>
          </div>
        </header>

        <div className="kg-settings-group min-h-0 flex-1 overflow-auto">
          {filtered.length === 0 ? (
            <p className="px-3.5 py-10 text-center kg-type-body-secondary text-fg-muted">{t("logs.empty")}</p>
          ) : (
            <ul className="kg-log-list">
              {filtered.map((entry) => (
                <li key={entry.id} className="kg-log-row">
                  <span className="kg-log-time">{entry.timestamp}</span>
                  <span className={`kg-log-level ${levelClass(entry.level)}`}>
                    {entry.level === "warning" ? "warn" : entry.level}
                  </span>
                  <span
                    className={`kg-log-msg ${
                      entry.level === "error" ? "text-error" : "text-fg"
                    }`}
                  >
                    {entry.message}
                  </span>
                </li>
              ))}
              <div ref={bottomRef} />
            </ul>
          )}
        </div>
      </div>
    </div>
  );
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

function levelClass(level: LogLevel): string {
  switch (level) {
    case "error":
      return "text-error";
    case "warning":
      return "text-warning";
    case "info":
      return "text-accent";
    default:
      return "text-fg-muted";
  }
}
