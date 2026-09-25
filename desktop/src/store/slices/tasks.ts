import { invoke } from "@tauri-apps/api/core";

import { localizeUserMessage } from "../../lib/localizeMessage";
import { notifyTaskDone } from "../../lib/notify";
import { tt } from "../shared";
import type { SliceCreator, TaskSnapshot, TasksSlice } from "../types";

function scrapeUnmatchedCount(summary: string): number {
  const m =
    summary.match(/(?:^|\s)unmatched=(\d+)/) ||
    summary.match(/未自动匹配\s+(\d+)/) ||
    summary.match(/未自動マッチ\s+(\d+)/) ||
    summary.match(/\bunmatched\s+(\d+)/i);
  return m ? Number(m[1]) : 0;
}

function localizeScrapeSummary(summary: string): string {
  const m = summary.match(/^success=(\d+) unmatched=(\d+) failed=(\d+)$/);
  if (m) {
    return tt("toast.scrapeSummary", {
      success: m[1],
      unmatched: m[2],
      failed: m[3],
    });
  }
  return summary;
}

export const createTasksSlice: SliceCreator<TasksSlice> = (set, get) => ({
  tasks: [],

  refreshTasks: async () => {
    try {
      const tasks = await invoke<TaskSnapshot[]>("list_tasks");
      set({ tasks });
    } catch (err) {
      const message = localizeUserMessage(String(err));
      set({ error: message });
      get().showToast(message);
    }
  },

  upsertTask: (task) => {
    const prev = get().tasks.find((t) => t.id === task.id);
    if (
      prev &&
      prev.status === task.status &&
      prev.errorMessage === task.errorMessage &&
      JSON.stringify(prev.result) === JSON.stringify(task.result) &&
      prev.progress?.completed === task.progress?.completed &&
      prev.progress?.total === task.progress?.total &&
      prev.progress?.current === task.progress?.current &&
      prev.progress?.stageKey === task.progress?.stageKey
    ) {
      return;
    }
    set((state) => {
      const rest = state.tasks.filter((t) => t.id !== task.id);
      return { tasks: [task, ...rest] };
    });
    if (prev?.status === task.status) return;
    if (task.status === "completed") {
      if (task.kind === "refresh") {
        const detail = task.progress?.current?.trim();
        if (task.progress?.stageKey === "refreshItems") {
          const title = detail
            ? tt("toast.itemsRefreshDoneDetail", { detail })
            : tt("toast.itemsRefreshDone");
          get().showToast(title, 2800);
          void notifyTaskDone(tt("toast.itemsRefreshDone"), detail);
          const libraryId = get().selectedLibraryId;
          if (libraryId) void get().reloadLibraryItems(libraryId);
        } else {
          const added =
            task.progress?.stageKey === "saveResults"
              ? Number(task.progress?.completed ?? 0)
              : 0;
          const title =
            added > 0
              ? tt("toast.refreshAdded", { n: added })
              : detail
                ? tt("toast.refreshDoneDetail", { detail })
                : tt("toast.refreshDone");
          get().showToast(title, 3200);
          void notifyTaskDone(tt("toast.refreshDone"), detail || title);
          const targetId = task.targetId;
          if (targetId && get().selectedLibraryId === targetId) {
            if (added > 0) {
              set({
                sortOption: "unscrapedFirst",
                statusFilter: "all",
                searchQuery: "",
              });
            }
            void get().reloadLibraryItems(targetId);
          }
        }
      } else if (
        task.kind === "batchScrape" ||
        task.kind === "scrape" ||
        task.kind === "rescrape" ||
        task.kind === "manualMatch"
      ) {
        const raw = task.progress?.current?.trim() ?? "";
        const summary = raw ? localizeScrapeSummary(raw) : "";
        const label =
          task.kind === "rescrape"
            ? tt("toast.rescrapeDone")
            : task.kind === "manualMatch"
              ? tt("toast.manualMatchDone")
              : tt("toast.scrapeDone");
        if (summary && task.kind !== "manualMatch") {
          const unmatched = scrapeUnmatchedCount(raw || summary);
          const hint = unmatched > 0 ? tt("toast.scrapeUnmatchedHint") : "";
          get().showToast(`${label} · ${summary}${hint}`);
          void notifyTaskDone(label, summary);
        } else {
          get().showToast(label);
          void notifyTaskDone(label);
        }
        const libraryId = get().selectedLibraryId;
        // Metadata fingerprints include scrapedAt, so only rescraped posters reload.
        if (libraryId) void get().reloadLibraryItems(libraryId);
      } else if (task.kind === "rename") {
        const detail = task.progress?.current?.trim();
        get().showToast(
          detail ? tt("toast.renameDoneDetail", { detail }) : tt("toast.renameDone"),
        );
        void notifyTaskDone(tt("toast.renameDone"), detail);
        const libraryId = get().selectedLibraryId;
        if (libraryId) void get().reloadLibraryItems(libraryId);
      } else if (task.kind === "organize") {
        const detail = task.progress?.current?.trim();
        get().showToast(
          detail
            ? tt("toast.organizeDoneDetail", { detail })
            : tt("toast.organizeDone"),
        );
        void notifyTaskDone(tt("toast.organizeDone"), detail);
        const libraryId = get().selectedLibraryId;
        if (libraryId) void get().reloadLibraryItems(libraryId);
      } else if (task.kind === "cleanup") {
        const detail = task.progress?.current?.trim();
        get().showToast(
          detail ? tt("toast.cleanupDoneDetail", { detail }) : tt("toast.cleanupDone"),
        );
        void notifyTaskDone(tt("toast.cleanupDone"), detail);
      }
    } else if (task.status === "failed") {
      const err = localizeUserMessage(task.errorMessage || tt("toast.taskFailed"));
      get().showToast(err);
      void notifyTaskDone(tt("toast.taskFailed"), err);
    }
  },
});
