import { ModalFrame } from "./ModalFrame";
import { DialogTitle } from "./ui/dialog";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";

import { useAppStore, type MediaType, type TaskSnapshot } from "../store/appStore";

export type MatchCandidate = {
  sourceId: string;
  title: string;
  originalTitle?: string | null;
  year?: number | null;
  overview?: string | null;
  posterUrl?: string | null;
  confidence: number;
  mediaType: MediaType;
};

export function ManualMatchModal({
  itemId,
  mediaType,
  initialQuery,
  onClose,
}: {
  itemId: string;
  mediaType: MediaType;
  initialQuery: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const showToast = useAppStore((s) => s.showToast);
  const upsertTask = useAppStore((s) => s.upsertTask);
  const [query, setQuery] = useState(initialQuery);
  const [loading, setLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [candidates, setCandidates] = useState<MatchCandidate[]>([]);
  const [searched, setSearched] = useState(false);

  /** Only the newest search may write results; a slow older one is dropped. */
  const searchRequest = useRef(0);

  const search = async () => {
    const request = ++searchRequest.current;
    const isCurrent = () => request === searchRequest.current;
    setLoading(true);
    try {
      const rows = await invoke<MatchCandidate[]>("search_match_candidates", {
        query,
        mediaType,
      });
      if (!isCurrent()) return;
      setCandidates(rows);
      setSearched(true);
      if (rows.length === 0) {
        showToast(t("match.noResults"));
      }
    } catch (err) {
      if (isCurrent()) showToast(String(err));
    } finally {
      if (isCurrent()) setLoading(false);
    }
  };

  useEffect(() => {
    void search();
    return () => {
      searchRequest.current += 1; // unmounted: ignore anything still in flight
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const apply = async (sourceId: string) => {
    if (applying) return;
    setApplying(true);
    try {
      const task = await invoke<TaskSnapshot>("apply_manual_match", {
        itemId,
        sourceId,
      });
      upsertTask(task);
      showToast(t("toast.manualMatchStarted"));
      onClose();
    } catch (err) {
      showToast(String(err));
      setApplying(false);
    }
  };

  return (
    <ModalFrame onClose={onClose} labelledBy="manual-match-title" className="sm:max-w-2xl">
        <header className="kg-dialog-header flex items-center justify-between gap-3">
          <DialogTitle
            id="manual-match-title"
            className="truncate kg-type-section-title font-extrabold tracking-[-0.25px] text-fg"
          >
            {t("action.manualMatch")}
          </DialogTitle>
          <Button variant="ghost" size="sm" type="button" onClick={onClose}>
            {t("settings.close")}
          </Button>
        </header>

        <div className="flex gap-2 px-6 pb-3">
          <Input
            className="flex-1"
            value={query}
            autoFocus
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !loading && !applying) void search();
            }}
          />
          <Button variant="default" size="sm"
            type="button"
            disabled={loading || applying}
            onClick={() => void search()}
          >
            {loading ? t("match.searching") : t("action.search")}
          </Button>
        </div>

        <ul className="kg-dialog-body !px-3">
          {candidates.length === 0 ? (
            <li className="px-2 py-8 text-center kg-type-body-secondary text-fg-muted">
              {loading
                ? t("match.searching")
                : searched
                  ? t("match.noResults")
                  : t("match.searching")}
            </li>
          ) : (
            candidates.map((c) => (
              <li key={c.sourceId}>
                <Button variant="plain" size="none"
                  type="button"
                  disabled={applying}
                  onClick={() => void apply(c.sourceId)}
                  className="kg-list-row !min-h-[58px] rounded-control"
                >
                  <span className="min-w-0 flex-1">
                    <span className="kg-list-row-title">
                      {c.title}
                      {c.year ? (
                        <span className="ml-2 font-medium text-fg-secondary">({c.year})</span>
                      ) : null}
                    </span>
                    <span className="kg-list-row-subtitle">
                      {c.sourceId} · {(c.confidence * 100).toFixed(0)}%
                    </span>
                    {c.overview ? (
                      <span className="mt-1 line-clamp-2 kg-type-caption text-fg-muted">
                        {c.overview}
                      </span>
                    ) : null}
                  </span>
                </Button>
              </li>
            ))
          )}
        </ul>
    </ModalFrame>
  );
}
