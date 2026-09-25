import { Button } from "./ui/button";
import { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight, File, Folder } from "lucide-react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

type DirectoryEntry = {
  name: string;
  path: string;
  isDirectory: boolean;
  fileSize?: number | null;
  modifiedAt?: string | null;
};

type PathSegment = {
  name: string;
  path: string;
};

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function FolderBrowser({
  rootPath,
  rootName,
  onClose,
}: {
  rootPath: string;
  rootName: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [segments, setSegments] = useState<PathSegment[]>([
    { name: rootName, path: rootPath },
  ]);
  const [entries, setEntries] = useState<DirectoryEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const current = segments[segments.length - 1]?.path ?? rootPath;

  useEffect(() => {
    setSegments([{ name: rootName, path: rootPath }]);
  }, [rootPath, rootName]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      setLoading(true);
      setError(null);
      try {
        const list = await invoke<DirectoryEntry[]>("list_directory", { path: current });
        if (!cancelled) setEntries(list);
      } catch (err) {
        if (!cancelled) {
          setEntries([]);
          setError(String(err));
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [current]);

  function navigateTo(index: number) {
    setSegments((prev) => prev.slice(0, index + 1));
  }

  function enter(entry: DirectoryEntry) {
    if (!entry.isDirectory) return;
    setSegments((prev) => [...prev, { name: entry.name, path: entry.path }]);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex shrink-0 items-center gap-2 border-b border-hairline px-3 py-2">
        <button type="button" className="sl-iconbtn back shrink-0" aria-label={t("browser.back")} title={t("browser.back")} onClick={onClose}>
          <ChevronLeft aria-hidden />
        </button>
        <nav className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto kg-type-body-secondary">
          {segments.map((seg, i) => (
            <span key={seg.path} className="flex shrink-0 items-center gap-1">
              {i > 0 ? <span className="text-fg-muted">/</span> : null}
              {i === segments.length - 1 ? (
                <span className="font-semibold text-fg">{seg.name}</span>
              ) : (
                <Button variant="plain" size="none"
                  type="button"
                  className="text-fg-secondary hover:text-fg"
                  onClick={() => navigateTo(i)}
                >
                  {seg.name}
                </Button>
              )}
            </span>
          ))}
        </nav>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-2 py-2">
        {loading ? (
          <p className="px-2 py-8 text-center kg-type-body-secondary text-fg-muted">{t("browser.loading")}</p>
        ) : error ? (
          <p className="px-2 py-8 text-center kg-type-body-secondary text-error">{error}</p>
        ) : entries.length === 0 ? (
          <p className="px-2 py-8 text-center kg-type-body-secondary text-fg-muted">{t("browser.empty")}</p>
        ) : (
          <ul>
            {entries.map((entry) => {
              const meta = [!entry.isDirectory && entry.fileSize != null ? formatBytes(entry.fileSize) : null, entry.modifiedAt]
                .filter(Boolean)
                .join(" · ");
              return (
              <li key={entry.path}>
                <button type="button" className="sl-file" disabled={!entry.isDirectory} onClick={() => enter(entry)} title={entry.path}>
                  {entry.isDirectory ? <Folder aria-hidden className="dir" /> : <File aria-hidden />}
                  <span className="nm">
                    <b>{entry.name}</b>
                    {meta ? <small>{meta}</small> : null}
                  </span>
                  {entry.isDirectory ? <ChevronRight aria-hidden className="go" /> : null}
                </button>
              </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
