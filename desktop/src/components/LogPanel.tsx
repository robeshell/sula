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
    void invoke<LogEntry[]>("list_logs")
      .then(setEntries)
      .catch(() => setEntries([]));

    let unlistenEntry: (() => void) | undefined;
    let unlistenClear: (() => void) | undefined;
    void listen<LogEntry>("log://entry", (event) => {
      setEntries((prev) => {
        const next = [...prev, event.payload];
        return next.length > 500 ? next.slice(next.length - 500) : next;
      });
    }).then((fn) => {
      unlistenEntry = fn;
    });
    void listen("log://cleared", () => {
      setEntries([]);
    }).then((fn) => {
      unlistenClear = fn;
    });
    return () => {
      unlistenEntry?.();
      unlistenClear?.();
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
