import { Plus as IconAdd, Settings as IconSettings, Logs as IconLogs, PencilLine as IconRename, RefreshCw as IconRefresh, Download as IconScrape, Ellipsis as IconMore, Film, Tv, Sparkles } from "lucide-react";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "./components/ui/tabs";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./components/ui/dropdown-menu";
import { TasksDock } from "./components/TasksDock";
import { Button } from "./components/ui/button";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { VirtualMediaList } from "./components/VirtualMediaList";
import { EmptyState } from "./components/EmptyState";
import { CleanupSheet, type ResidualCandidate } from "./components/CleanupSheet";
import { DeleteConfirmModal } from "./components/DeleteConfirmModal";
import { ManualMatchModal } from "./components/ManualMatchModal";
import {
  MediaContextMenu,
  type MediaContextMenuTarget,
} from "./components/MediaContextMenu";
import { PosterThumb } from "./components/PosterThumb";
import { ActorAvatar } from "./components/ActorAvatar";
import { SettingsPage } from "./components/SettingsModal";
import { LogPanel } from "./components/LogPanel";
import { FolderBrowser } from "./components/FolderBrowser";
import { ToastHost } from "./components/ToastHost";
import { WindowControls } from "./components/WindowControls";
import appIconUrl from "../src-tauri/icons/icon.png";
import { LibraryToolbar } from "./components/LibraryToolbar";
import { POSTER_THUMB, SEASON_THUMB, EPISODE_STILL } from "./lib/posterLoadQueue";
import {
  clampDetailWidth,
  DETAIL_WIDTH_MAX,
  DETAIL_WIDTH_MIN,
  loadDetailWidth,
  saveDetailWidth,
} from "./lib/detailPanelWidth";
import { localizeUserMessage } from "./lib/localizeMessage";
import i18n from "./i18n";
import {
  useAppStore,
  type CastMember,
  type MediaType,
  type TvEpisode,
  type TvSeason,
} from "./store/appStore";

function App() {
  const { t } = useTranslation();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [logsOpen, setLogsOpen] = useState(false);
  const [folderBrowser, setFolderBrowser] = useState<{
    rootPath: string;
    rootName: string;
  } | null>(null);
  const [manualMatchOpen, setManualMatchOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [cleanupCandidates, setCleanupCandidates] = useState<ResidualCandidate[] | null>(
    null,
  );
  const [contextMenu, setContextMenu] = useState<MediaContextMenuTarget | null>(null);
  const contextSelectionSnapshot = useRef<{
    selectedMediaIds: string[];
    selectedMediaId: string | null;
    detail: ReturnType<typeof useAppStore.getState>["detail"];
    detailLoading: boolean;
    posterUrl: string | null;
  } | null>(null);
  const [detailWidth, setDetailWidth] = useState(loadDetailWidth);
  const [resizingDetail, setResizingDetail] = useState(false);
  const [tasksOpen, setTasksOpen] = useState(false);
  const detailPaneRef = useRef<HTMLElement | null>(null);
  const prevActiveTaskCount = useRef(0);
  const libraries = useAppStore((s) => s.libraries);
  const selectedLibraryId = useAppStore((s) => s.selectedLibraryId);
  const mediaItems = useAppStore((s) => s.mediaItems);
  const metadataById = useAppStore((s) => s.metadataById);
  const showStatsById = useAppStore((s) => s.showStatsById);
  const selectedMediaIds = useAppStore((s) => s.selectedMediaIds);
  const selectedMediaId = useAppStore((s) => s.selectedMediaId);
  const detail = useAppStore((s) => s.detail);
  const detailLoading = useAppStore((s) => s.detailLoading);
  const posterUrl = useAppStore((s) => s.posterUrl);
  const searchQuery = useAppStore((s) => s.searchQuery);
  const sortOption = useAppStore((s) => s.sortOption);
  const statusFilter = useAppStore((s) => s.statusFilter);
  const listViewMode = useAppStore((s) => s.listViewMode);
  // Boolean selector: progress ticks won't re-render the list while still scanning.
  const isLibraryScanning = useAppStore((s) =>
    Boolean(
      s.selectedLibraryId &&
        s.tasks.some(
          (t) =>
            t.kind === "refresh" &&
            t.targetId === s.selectedLibraryId &&
            (t.status === "pending" || t.status === "running"),
        ),
    ),
  );
  const activeTaskCount = useAppStore(
    (s) =>
      s.tasks.filter((t) => t.status === "pending" || t.status === "running").length,
  );
  const refreshStatus = useAppStore((s) => s.refreshStatus);
  const refreshLibraries = useAppStore((s) => s.refreshLibraries);
  const selectLibrary = useAppStore((s) => s.selectLibrary);
  const toggleMediaSelection = useAppStore((s) => s.toggleMediaSelection);
  const clearMediaSelection = useAppStore((s) => s.clearMediaSelection);
  const setSearchQuery = useAppStore((s) => s.setSearchQuery);
  const setSortOption = useAppStore((s) => s.setSortOption);
  const setStatusFilter = useAppStore((s) => s.setStatusFilter);
  const setListViewMode = useAppStore((s) => s.setListViewMode);
  const addLibrary = useAppStore((s) => s.addLibrary);
  const deleteSelectedLibrary = useAppStore((s) => s.deleteSelectedLibrary);
  const refreshSelectedLibrary = useAppStore((s) => s.refreshSelectedLibrary);
  const scrapeSelectedLibrary = useAppStore((s) => s.scrapeSelectedLibrary);
  const scrapeSelectedItems = useAppStore((s) => s.scrapeSelectedItems);
  const rescrapeSelectedItems = useAppStore((s) => s.rescrapeSelectedItems);
  const ensureScrapeReady = useAppStore((s) => s.ensureScrapeReady);
  const renameSelectedItem = useAppStore((s) => s.renameSelectedItem);
  const renameSelectedItems = useAppStore((s) => s.renameSelectedItems);
  const organizeSelectedItems = useAppStore((s) => s.organizeSelectedItems);
  const consolidateSelectedShows = useAppStore((s) => s.consolidateSelectedShows);
  const consolidateSelectedLibraryShows = useAppStore(
    (s) => s.consolidateSelectedLibraryShows,
  );
  const scanResidualsForSelected = useAppStore((s) => s.scanResidualsForSelected);
  const cleanupResiduals = useAppStore((s) => s.cleanupResiduals);
  const refreshSelectedItemsFromDisk = useAppStore((s) => s.refreshSelectedItemsFromDisk);
  const revealSelectedItem = useAppStore((s) => s.revealSelectedItem);
  const deleteSelectedItems = useAppStore((s) => s.deleteSelectedItems);
  const refreshTasks = useAppStore((s) => s.refreshTasks);
  const upsertTask = useAppStore((s) => s.upsertTask);
  const visibleMediaItems = useAppStore((s) => s.visibleMediaItems);

  const openManualMatch = async () => {
    if (!(await ensureScrapeReady())) return;
    setManualMatchOpen(true);
  };

  const openCleanupSheet = async () => {
    const candidates = await scanResidualsForSelected();
    if (candidates.length > 0) setCleanupCandidates(candidates);
  };

  const openRenamer = async () => {
    try {
      await invoke("open_renamer_window");
    } catch (err) {
      useAppStore.getState().showToast(String(err));
    }
  };

  const openContextMenu = (e: React.MouseEvent, itemId: string) => {
    e.preventDefault();
    e.stopPropagation();
    // Right-click must not leave a text selection highlight under the menu.
    window.getSelection()?.removeAllRanges();
    // Select for context actions only — never open/fetch detail (avoids layout jitter).
    // Snapshot prior selection so dismissing the menu can undo a right-click-only select.
    if (!selectedMediaIds.includes(itemId)) {
      const s = useAppStore.getState();
      contextSelectionSnapshot.current = {
        selectedMediaIds: s.selectedMediaIds,
        selectedMediaId: s.selectedMediaId,
        detail: s.detail,
        detailLoading: s.detailLoading,
        posterUrl: s.posterUrl,
      };
      useAppStore.setState({
        selectedMediaIds: [itemId],
        selectedMediaId: null,
        detail: null,
        detailLoading: false,
        posterUrl: null,
      });
    } else {
      contextSelectionSnapshot.current = null;
    }
    setContextMenu({ x: e.clientX, y: e.clientY, itemId });
  };

  const closeContextMenu = (commitSelection = false) => {
    setContextMenu(null);
    if (!commitSelection && contextSelectionSnapshot.current) {
      useAppStore.setState(contextSelectionSnapshot.current);
    }
    contextSelectionSnapshot.current = null;
  };

  const contextSelectionIds =
    contextMenu && selectedMediaIds.includes(contextMenu.itemId)
      ? selectedMediaIds
      : contextMenu
        ? [contextMenu.itemId]
        : [];
  const contextItems = mediaItems.filter((m) => contextSelectionIds.includes(m.id));
  const contextSingle = contextSelectionIds.length === 1;
  const contextCanAuto = contextItems.some(
    (m) => m.status === "unscraped" || m.status === "partial",
  );
  const contextCanRescrape = contextItems.some((m) => m.status === "scraped");
  const contextCanRename = contextItems.some((m) => m.status === "scraped");
  const contextCanOrganize = contextItems.some(
    (m) =>
      m.status === "scraped" && (m.mediaType === "tvShow" || m.mediaType === "anime"),
  );
  const contextCanMerge = contextItems.some(
    (m) => m.mediaType === "tvShow" || m.mediaType === "anime",
  );
  const contextCanClean = contextItems.some((m) => m.status === "scraped");
  const matchTarget =
    mediaItems.find((m) => m.id === selectedMediaId) ??
    (detail ? detail.item : null) ??
    (selectedMediaIds.length === 1
      ? (mediaItems.find((m) => m.id === selectedMediaIds[0]) ?? null)
      : null);

  useEffect(() => {
    if (!resizingDetail) return;
    const onMove = (e: PointerEvent) => {
      const pane = detailPaneRef.current;
      if (!pane) return;
      const right = pane.getBoundingClientRect().right;
      setDetailWidth(clampDetailWidth(right - e.clientX));
    };
    const onUp = () => {
      setResizingDetail(false);
      setDetailWidth((w) => {
        saveDetailWidth(w);
        return w;
      });
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, [resizingDetail]);

  useEffect(() => {
    void refreshStatus();
    void refreshLibraries();
    void refreshTasks();
    let disposed = false;
    let unlistenTask: (() => void) | undefined;
    let unlistenLib: (() => void) | undefined;
    let libTimer: ReturnType<typeof setTimeout> | null = null;
    void listen("task-updated", (event) => {
      if (disposed) return;
      upsertTask(event.payload as Parameters<typeof upsertTask>[0]);
    }).then((fn) => {
      if (disposed) fn(); else unlistenTask = fn;
    });
    void listen("library-updated", () => {
      if (disposed) return;
      // Coalesce bursts so a finishing scan doesn't thrash the list.
      if (libTimer) clearTimeout(libTimer);
      libTimer = setTimeout(() => {
        libTimer = null;
        void refreshLibraries();
        void refreshStatus();
      }, 200);
    }).then((fn) => {
      if (disposed) fn(); else unlistenLib = fn;
    });
    return () => {
      disposed = true;
      unlistenTask?.();
      unlistenLib?.();
      if (libTimer) clearTimeout(libTimer);
    };
  }, [refreshStatus, refreshLibraries, refreshTasks, upsertTask]);

  useEffect(() => {
    if (prevActiveTaskCount.current === 0 && activeTaskCount > 0) {
      setTasksOpen(true);
    }
    prevActiveTaskCount.current = activeTaskCount;
  }, [activeTaskCount]);

  const selected = libraries.find((l) => l.id === selectedLibraryId) ?? null;
  const showDetailPane = Boolean(
    folderBrowser || (selectedMediaId && selectedMediaIds.length <= 1),
  );
  const grouped = groupLibraries(libraries);
  const visible = useMemo(
    () => visibleMediaItems(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mediaItems, searchQuery, sortOption, statusFilter, visibleMediaItems],
  );

  return (
    <div className="kg-shell flex h-full text-fg">
      <div
        className="kg-titlebar absolute inset-x-0 top-0 z-30 h-[var(--kg-titlebar-height)]"
        aria-hidden
      >
        <div data-tauri-drag-region className="absolute inset-0" />
        <WindowControls />
      </div>
      <aside
        className="kg-chrome-rail flex w-[var(--kg-sidebar-width)] shrink-0 flex-col overflow-hidden border-r border-hairline px-2.5 pb-3 pt-[var(--kg-rail-top)]"
        onContextMenu={(e) => e.preventDefault()}
      >
        <div className="kg-rail-brand select-none">
          <img
            className="kg-rail-brand-logo"
            src={appIconUrl}
            alt=""
            draggable={false}
          />
          <p className="kg-rail-brand-name">{t("app.brand")}</p>
        </div>

        <div className="kg-rail-nav min-h-0 flex-1 overflow-auto select-none">
          {(["movie", "tvShow", "anime"] as MediaType[]).map((type) => (
            <div key={type} className="kg-rail-group">
              <p className="kg-rail-group-label">{typeLabel(type, t)}</p>
              {(grouped[type] ?? []).length === 0 ? (
                <p className="kg-rail-empty">{t("sidebar.empty")}</p>
              ) : (
                <ul className="kg-rail-list">
                  {(grouped[type] ?? []).map((lib) => {
                    const active = lib.id === selectedLibraryId;
                    return (
                      <li key={lib.id}>
                        <Button variant="plain" size="none"
                          type="button"
                          onClick={() => {
                            setSettingsOpen(false);
                            setLogsOpen(false);
                            setFolderBrowser(null);
                            void selectLibrary(lib.id);
                          }}
                          onContextMenu={(e) => e.preventDefault()}
                          data-selected={!settingsOpen && !logsOpen && active}
                          data-type={type}
                          className="kg-side-item"
                        >
                          <span className="kg-side-item-icon" aria-hidden>
                            <LibraryTypeIcon type={type} />
                          </span>
                          <span className="kg-side-item-text">
                            <span className="kg-side-item-title">{lib.name}</span>
                            <span className="kg-side-item-sub" title={lib.rootPath}>
                              {shortLibraryPath(lib.rootPath)}
                            </span>
                          </span>
                        </Button>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          ))}
        </div>

        <nav className="kg-rail-actions" aria-label={t("app.title")}>
          <AddMenu onAdd={(type) => void addLibrary(type)} />
          <TileButton
            icon={<IconSettings />}
            label={t("action.settings")}
            selected={settingsOpen}
            onClick={() => {
              setLogsOpen(false);
              setFolderBrowser(null);
              setSettingsOpen((v) => !v);
            }}
          />
          <TileButton
            icon={<IconLogs />}
            label={t("action.logs")}
            selected={logsOpen}
            onClick={() => {
              setSettingsOpen(false);
              setFolderBrowser(null);
              setLogsOpen((v) => !v);
            }}
          />
          <TileButton
            icon={<IconRename />}
            label={t("action.renamer")}
            onClick={() => void openRenamer()}
          />
        </nav>
      </aside>

      <div className="flex min-h-0 min-w-0 flex-1 flex-col pt-[var(--kg-titlebar-height)]">
        {settingsOpen ? (
          <SettingsPage onClose={() => setSettingsOpen(false)} />
        ) : logsOpen ? (
          <LogPanel onClose={() => setLogsOpen(false)} />
        ) : (
          <div
            className={showDetailPane ? "grid min-h-0 flex-1" : "flex min-h-0 flex-1"}
            style={
              showDetailPane
                ? { gridTemplateColumns: `minmax(0, 1fr) ${detailWidth}px` }
                : undefined
            }
          >
            <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
              {!selected ? (
                <EmptyState
                  className="h-full"
                  title={t("empty.welcomeTitle")}
                  message={
                    libraries.length === 0 ? t("empty.welcomeMessage") : t("list.pickLibrary")
                  }
                  action={
                    libraries.length === 0 ? (
                      <div className="flex flex-wrap justify-center gap-2">
                        <Button variant="default" size="sm"
                          type="button"
                          onClick={() => void addLibrary("movie")}
                        >
                          {t("action.addMovie")}
                        </Button>
                        <Button variant="outline" size="sm"
                          type="button"
                          onClick={() => void addLibrary("tvShow")}
                        >
                          {t("action.addTv")}
                        </Button>
                        <Button variant="outline" size="sm"
                          type="button"
                          onClick={() => void addLibrary("anime")}
                        >
                          {t("action.addAnime")}
                        </Button>
                      </div>
                    ) : undefined
                  }
                />
              ) : (
                <>
                  <LibraryToolbar title={selected.name} path={selected.rootPath}
                    count={visible.length} total={mediaItems.length} query={searchQuery} onQuery={setSearchQuery}
                    status={statusFilter} onStatus={setStatusFilter} sort={sortOption} onSort={setSortOption}
                    view={listViewMode} onView={setListViewMode} actions={
                    <div className="kg-toolbar-group">
                      <TileButton
                        icon={<IconRefresh />}
                        label={isLibraryScanning ? t("action.refreshing") : t("action.refresh")}
                        disabled={isLibraryScanning}
                        onClick={() => void refreshSelectedLibrary()}
                      />
                      <TileButton
                        icon={<IconScrape />}
                        label={t("action.scrapeAll")}
                        primary
                        disabled={isLibraryScanning}
                        onClick={() => void scrapeSelectedLibrary()}
                      />
                      <MenuButton
                        label={t("action.more")}
                        icon={<IconMore />}
                        items={[
                          ...(selected.mediaType === "tvShow" ||
                          selected.mediaType === "anime"
                            ? [
                                {
                                  label: t("action.mergeDuplicates"),
                                  onClick: () => void consolidateSelectedLibraryShows(),
                                },
                              ]
                            : []),
                          {
                            label: t("action.browseFolder"),
                            onClick: () =>
                              setFolderBrowser({
                                rootPath: selected.rootPath,
                                rootName: selected.name,
                              }),
                          },
                          {
                            label: t("action.deleteLibrary"),
                            destructive: true,
                            onClick: () => void deleteSelectedLibrary(),
                          },
                        ]}
                      />
                    </div>
                  } />

                  <div className="relative min-h-0 flex-1">
                  {selectedMediaIds.length > 1 ? (
                    <div className="ion-bar">
                      <span className="kg-type-body-secondary font-semibold text-fg-secondary">
                        {t("list.selectedCount", { count: selectedMediaIds.length })}
                      </span>
                      <Button variant="default" size="sm"
                        type="button"
                        onClick={() => void scrapeSelectedItems()}
                      >
                        {t("action.scrapeAuto")}
                      </Button>
                      <MenuButton
                        label={t("action.more")}
                        items={[
                          ...(selectedMediaIds.some((id) => {
                            const item = mediaItems.find((m) => m.id === id);
                            return item?.status === "scraped";
                          })
                            ? [
                                {
                                  label: t("action.rescrape"),
                                  onClick: () => void rescrapeSelectedItems(),
                                },
                                {
                                  label: t("action.applyRename"),
                                  onClick: () => void renameSelectedItems(),
                                },
                              ]
                            : []),
                          ...(selectedMediaIds.some((id) => {
                            const item = mediaItems.find((m) => m.id === id);
                            return (
                              item?.status === "scraped" &&
                              (item.mediaType === "tvShow" || item.mediaType === "anime")
                            );
                          })
                            ? [
                                {
                                  label: t("action.organizeSeasons"),
                                  onClick: () => void organizeSelectedItems(),
                                },
                                {
                                  label: t("action.mergeDuplicates"),
                                  onClick: () => void consolidateSelectedShows(),
                                },
                              ]
                            : selectedMediaIds.some((id) => {
                                  const item = mediaItems.find((m) => m.id === id);
                                  return (
                                    item?.mediaType === "tvShow" || item?.mediaType === "anime"
                                  );
                                })
                              ? [
                                  {
                                    label: t("action.mergeDuplicates"),
                                    onClick: () => void consolidateSelectedShows(),
                                  },
                                ]
                              : []),
                          ...(selectedMediaIds.some((id) => {
                            const item = mediaItems.find((m) => m.id === id);
                            return item?.status === "scraped";
                          })
                            ? [
                                {
                                  label: t("action.cleanResiduals"),
                                  onClick: () => void openCleanupSheet(),
                                },
                              ]
                            : []),
                          {
                            label: t("action.refreshFromDisk"),
                            onClick: () => void refreshSelectedItemsFromDisk(),
                          },
                          {
                            label: t("action.deleteItem"),
                            destructive: true,
                            onClick: () => setDeleteOpen(true),
                          },
                        ]}
                      />
                      <Button variant="ghost" size="sm"
                        type="button"
                        onClick={() => clearMediaSelection()}
                      >
                        {t("list.clearSelection")}
                      </Button>
                    </div>
                  ) : null}

                  <div className="h-full min-h-0 overflow-auto">
                    {visible.length === 0 ? (
                      <EmptyState
                        className="h-full"
                        title={
                          isLibraryScanning
                            ? t("empty.scanningTitle")
                            : mediaItems.length === 0
                              ? t("empty.scanTitle")
                              : t("empty.filterTitle")
                        }
                        message={
                          isLibraryScanning
                            ? t("empty.scanningMessage")
                            : mediaItems.length === 0
                              ? t("list.emptyScan")
                              : t("list.emptyFilter")
                        }
                        action={
                          mediaItems.length === 0 && !isLibraryScanning ? (
                            <Button variant="default" size="sm"
                              type="button"
                              onClick={() => void refreshSelectedLibrary()}
                            >
                              {t("action.refresh")}
                            </Button>
                          ) : undefined
                        }
                      />
                    ) : listViewMode === "poster" ? (
                      <VirtualMediaList items={visible} mode="poster" resetKey={`${selectedLibraryId}:${searchQuery}:${sortOption}:${statusFilter}`}>
                        {(item) => {
                          const meta = metadataById[item.id];
                          const stats = showStatsById[item.id];
                          const active = selectedMediaIds.includes(item.id);
                          const isShow =
                            item.mediaType === "tvShow" || item.mediaType === "anime";
                          return (
                            <Button variant="plain" size="none"
                              key={item.id}
                              type="button"
                              data-selected={active}
                              className="kg-poster-card"
                              style={{ width: "100%" }}
                              onClick={(e) =>
                                void toggleMediaSelection(item.id, e.metaKey || e.ctrlKey)
                              }
                              onMouseDown={(e) => {
                                if (e.button === 2) e.preventDefault();
                              }}
                              onContextMenu={(e) => openContextMenu(e, item.id)}
                            >
                              <PosterThumb
                                folderPath={item.folderPath}
                                posterPath={meta?.posterPath}
                                width={POSTER_THUMB.width}
                                height={POSTER_THUMB.height}
                                className="kg-poster-card-art"
                                fallbackLabel={item.title.slice(0, 1)}
                              />
                              <span className="kg-poster-card-title">{item.title}</span>
                              <span className="kg-poster-card-sub">
                                {isShow && stats
                                  ? t("list.showStatsShort", {
                                      seasons: stats.seasonCount,
                                      episodes: stats.episodeCount,
                                    })
                                  : [
                                      item.year,
                                      t(`status.${item.status}`, {
                                        defaultValue: item.status,
                                      }),
                                    ]
                                      .filter(Boolean)
                                      .join(" · ")}
                              </span>
                            </Button>
                          );
                        }}
                      </VirtualMediaList>
                    ) : (
                      <VirtualMediaList items={visible} mode="list" resetKey={`${selectedLibraryId}:${searchQuery}:${sortOption}:${statusFilter}`}>
                        {(item) => {
                          const meta = metadataById[item.id];
                          const stats = showStatsById[item.id];
                          const active = selectedMediaIds.includes(item.id);
                          const isShow =
                            item.mediaType === "tvShow" || item.mediaType === "anime";
                          return (
                            <div key={item.id}>
                              <Button variant="plain" size="none"
                                type="button"
                                onClick={(e) =>
                                  void toggleMediaSelection(item.id, e.metaKey || e.ctrlKey)
                                }
                                onMouseDown={(e) => {
                                  if (e.button === 2) e.preventDefault();
                                }}
                                onContextMenu={(e) => openContextMenu(e, item.id)}
                                data-selected={active}
                                className="kg-list-row"
                              >
                                <span className="min-w-0 flex-1">
                                  <span className="flex min-w-0 items-center gap-1.5">
                                    <span className="kg-list-row-title min-w-0">
                                      {item.title}
                                      {item.year ? (
                                        <span className="ml-2 font-medium text-fg-secondary">
                                          ({item.year})
                                        </span>
                                      ) : null}
                                    </span>
                                  </span>
                                  {isShow ? (
                                    <span className="kg-list-row-subtitle">
                                      {stats
                                        ? t("list.showStats", {
                                            seasons: stats.seasonCount,
                                            episodes: stats.episodeCount,
                                            local: stats.localEpisodeCount,
                                          })
                                        : t("list.showPending")}
                                    </span>
                                  ) : meta?.overview ? (
                                    <span className="kg-list-row-subtitle">{meta.overview}</span>
                                  ) : (
                                    <span className="kg-list-row-subtitle font-mono kg-type-caption-small text-fg-muted">
                                      {item.filePath || item.folderPath}
                                    </span>
                                  )}
                                </span>
                                {meta?.rating != null ? (
                                  <span className="shrink-0 kg-type-body-secondary font-medium text-warning">
                                    {meta.rating.toFixed(1)}
                                  </span>
                                ) : null}
                                <span className="shrink-0 kg-type-body-secondary font-medium text-fg-secondary">
                                  {t(`status.${item.status}`, { defaultValue: item.status })}
                                </span>
                              </Button>
                            </div>
                          );
                        }}
                      </VirtualMediaList>
                    )}
                  </div>
                  </div>
                </>
              )}
            </main>

            {showDetailPane ? (
            <section
              ref={detailPaneRef}
              className="relative flex min-h-0 flex-col overflow-hidden border-l border-hairline"
            >
              <div
                role="separator"
                aria-orientation="vertical"
                aria-valuemin={DETAIL_WIDTH_MIN}
                aria-valuemax={DETAIL_WIDTH_MAX}
                aria-valuenow={detailWidth}
                aria-label={t("detail.resize")}
                data-active={resizingDetail}
                className="kg-detail-resizer"
                onPointerDown={(e) => {
                  e.preventDefault();
                  (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
                  setResizingDetail(true);
                }}
              />
              {folderBrowser ? (
                <FolderBrowser
                  rootPath={folderBrowser.rootPath}
                  rootName={folderBrowser.rootName}
                  onClose={() => setFolderBrowser(null)}
                />
              ) : detail && !detailLoading ? (
                <div className="flex min-h-0 flex-1 flex-col">
                  <div className="kg-detail-chrome">
                    {detail.item.status === "scraped" ? (
                      <Button variant="default" size="sm"
                        type="button"
                        onClick={() => void rescrapeSelectedItems()}
                      >
                        {t("action.rescrape")}
                      </Button>
                    ) : (
                      <Button variant="default" size="sm"
                        type="button"
                        onClick={() => void openManualMatch()}
                      >
                        {t("action.scrapeItem")}
                      </Button>
                    )}
                    <MenuButton
                      label={t("action.more")}
                      align="left"
                      items={[
                        ...(detail.item.status !== "scraped"
                          ? [
                              {
                                label: t("action.scrapeAuto"),
                                onClick: () => void scrapeSelectedItems(),
                              },
                            ]
                          : []),
                        ...(detail.item.status === "scraped"
                          ? [
                              {
                                label: t("action.manualMatch"),
                                onClick: () => void openManualMatch(),
                              },
                              {
                                label: t("action.applyRename"),
                                onClick: () => void renameSelectedItem(),
                              },
                              ...(detail.item.mediaType === "tvShow" ||
                              detail.item.mediaType === "anime"
                                ? [
                                    {
                                      label: t("action.organizeSeasons"),
                                      onClick: () => void organizeSelectedItems(),
                                    },
                                    {
                                      label: t("action.mergeDuplicates"),
                                      onClick: () => void consolidateSelectedShows(),
                                    },
                                  ]
                                : []),
                              {
                                label: t("action.cleanResiduals"),
                                onClick: () => void openCleanupSheet(),
                              },
                            ]
                          : [
                              {
                                label: t("action.manualMatch"),
                                onClick: () => void openManualMatch(),
                              },
                            ]),
                        {
                          label: t("action.refreshFromDisk"),
                          onClick: () => void refreshSelectedItemsFromDisk(),
                        },
                        {
                          label: t("action.revealInFinder"),
                          onClick: () => void revealSelectedItem(),
                        },
                        {
                          label: t("action.browseFolder"),
                          onClick: () =>
                            setFolderBrowser({
                              rootPath: detail.item.folderPath,
                              rootName: detail.item.title,
                            }),
                        },
                        {
                          label: t("action.deleteItem"),
                          destructive: true,
                          onClick: () => setDeleteOpen(true),
                        },
                      ]}
                    />
                    <div className="kg-detail-chrome-spacer" />
                    <Button variant="ghost" size="sm"
                      type="button"
                      aria-label={t("detail.close")}
                      onClick={() => clearMediaSelection()}
                    >
                      {t("settings.close")}
                    </Button>
                  </div>
                  <div className="min-h-0 flex-1 overflow-auto p-4">
                    <DetailPanel detail={detail} posterUrl={posterUrl} />
                  </div>
                </div>
              ) : (
                <DetailSkeleton
                  title={mediaItems.find((m) => m.id === selectedMediaId)?.title}
                  mediaType={mediaItems.find((m) => m.id === selectedMediaId)?.mediaType}
                />
              )}
            </section>
            ) : null}
          </div>
        )}
      </div>

      {!settingsOpen && !logsOpen ? (
        <TasksDock open={tasksOpen} onOpenChange={setTasksOpen} activeCount={activeTaskCount} />
      ) : null}

      {manualMatchOpen && matchTarget ? (
        <ManualMatchModal
          itemId={matchTarget.id}
          mediaType={matchTarget.mediaType}
          initialQuery={matchTarget.title}
          onClose={() => setManualMatchOpen(false)}
        />
      ) : null}
      {deleteOpen ? (
        <DeleteConfirmModal
          title={
            selectedMediaIds.length > 1
              ? t("delete.multiTitle", { count: selectedMediaIds.length })
              : (detail?.item.title ?? matchTarget?.title ?? "")
          }
          onClose={() => setDeleteOpen(false)}
          onConfirm={(alsoTrash) => {
            setDeleteOpen(false);
            void deleteSelectedItems(alsoTrash);
          }}
        />
      ) : null}
      {cleanupCandidates ? (
        <CleanupSheet
          candidates={cleanupCandidates}
          onClose={() => setCleanupCandidates(null)}
          onConfirm={(paths) => {
            setCleanupCandidates(null);
            void cleanupResiduals(paths);
          }}
        />
      ) : null}
      {contextMenu ? (
        <MediaContextMenu
          menu={contextMenu}
          canScrapeAuto={contextCanAuto}
          canRescrape={contextCanRescrape}
          canManualMatch={contextSingle}
          canRename={contextCanRename}
          canOrganize={contextCanOrganize}
          canMergeDuplicates={contextCanMerge}
          canCleanResiduals={contextCanClean}
          canDelete={contextSelectionIds.length > 0}
          onClose={() => closeContextMenu(false)}
          onScrapeConfirm={() => void openManualMatch()}
          onScrapeAuto={() => void scrapeSelectedItems()}
          onRescrape={() => void rescrapeSelectedItems()}
          onManualMatch={() => void openManualMatch()}
          onRename={() => void renameSelectedItems()}
          onOrganize={() => void organizeSelectedItems()}
          onMergeDuplicates={() => void consolidateSelectedShows()}
          onCleanResiduals={() => void openCleanupSheet()}
          onRefreshFromDisk={() => void refreshSelectedItemsFromDisk()}
          onReveal={() => void revealSelectedItem()}
          onDelete={() => setDeleteOpen(true)}
          onAction={() => closeContextMenu(true)}
        />
      ) : null}
      <ToastHost />
    </div>
  );
}

function SeasonBrowser({
  mediaItemId,
  canScrapeSeason,
  folderPath,
  seasons,
  episodes,
}: {
  mediaItemId: string;
  canScrapeSeason: boolean;
  folderPath: string;
  seasons: TvSeason[];
  episodes: TvEpisode[];
}) {
  const { t } = useTranslation();
  const scrapeSeason = useAppStore((s) => s.scrapeSeason);
  const ordered = useMemo(
    () => seasons.slice().sort((a, b) => a.seasonNumber - b.seasonNumber),
    [seasons],
  );
  const [activeId, setActiveId] = useState(ordered[0]?.id ?? "");
  const [seasonMenu, setSeasonMenu] = useState<{
    x: number;
    y: number;
    seasonId: string;
    seasonNumber: number;
  } | null>(null);

  useEffect(() => {
    if (!ordered.some((s) => s.id === activeId)) {
      setActiveId(ordered[0]?.id ?? "");
    }
  }, [ordered, activeId]);

  if (ordered.length === 0) {
    return (
      <section>
        <h3 className="kg-section-label">{t("detail.seasons")}</h3>
        <p className="kg-type-body-secondary text-fg-muted">{t("detail.noSeasons")}</p>
      </section>
    );
  }

  const active = ordered.find((s) => s.id === activeId) ?? ordered[0];
  const eps = episodes
    .filter((e) => e.seasonId === active.id)
    .slice()
    .sort((a, b) => a.episodeNumber - b.episodeNumber);
  const local = eps.filter((e) => Boolean(e.filePath)).length;

  return (
    <Tabs value={activeId} onValueChange={setActiveId}>
      <h3 className="kg-section-label">{t("detail.seasons")}</h3>
      <TabsList className="kg-season-strip h-auto group-data-[orientation=horizontal]/tabs:h-auto justify-start bg-transparent" aria-label={t("detail.seasons")}>
        {ordered.map((season) => {
          const selected = season.id === active.id;
          const seasonEps = episodes.filter((e) => e.seasonId === season.id);
          const seasonLocal = seasonEps.filter((e) => Boolean(e.filePath)).length;
          const n = season.seasonNumber;
          const padded = String(n).padStart(2, "0");
          const candidates = [
            season.posterPath,
            `season${n}-poster.jpg`,
            `season${padded}-poster.jpg`,
            `season${n}-poster.png`,
            `season${padded}-poster.png`,
          ].filter((v): v is string => Boolean(v));
          return (
            <TabsTrigger value={season.id}
              key={season.id}
              type="button"
              role="tab"
              aria-selected={selected}
              data-selected={selected}
              className="kg-season-tab h-auto flex-none"
              onClick={() => setActiveId(season.id)}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                window.getSelection()?.removeAllRanges();
                setActiveId(season.id);
                setSeasonMenu({
                  x: e.clientX,
                  y: e.clientY,
                  seasonId: season.id,
                  seasonNumber: season.seasonNumber,
                });
              }}
            >
              <PosterThumb
                folderPath={folderPath}
                posterCandidates={candidates}
                width={SEASON_THUMB.width}
                height={SEASON_THUMB.height}
                allowFallbacks={false}
                className="kg-season-tab-poster"
                fallbackLabel={`S${padded}`}
              />
              <span className="kg-season-tab-meta">
                <span className="kg-season-tab-title">
                  {t("detail.seasonLabel", { n: season.seasonNumber })}
                </span>
                <span className="kg-season-tab-sub">
                  {t("detail.seasonEps", {
                    total: seasonEps.length,
                    local: seasonLocal,
                  })}
                </span>
              </span>
            </TabsTrigger>
          );
        })}
      </TabsList>

      <TabsContent value={active.id} className="kg-season-panel">
        <p className="kg-season-panel-head">
          <span className="kg-season-panel-title">
            {active.title ?? t("detail.seasonLabel", { n: active.seasonNumber })}
          </span>
          <span className="kg-season-panel-meta">
            {t("detail.seasonEps", { total: eps.length, local })}
          </span>
        </p>
        {active.overview ? (
          <p className="kg-season-panel-overview">{active.overview}</p>
        ) : null}
        <ul className="max-h-[min(320px,40vh)] space-y-0.5 overflow-auto">
          {eps.length === 0 ? (
            <li className="kg-type-caption text-fg-muted">{t("detail.noEpisodes")}</li>
          ) : (
            eps.map((ep) => {
              const stillCandidates = [
                ep.stillPath,
                stillCandidateFromFile(ep.filePath),
              ].filter((v): v is string => Boolean(v));
              return (
                <li key={ep.id} className="kg-episode-row">
                  <PosterThumb
                    folderPath={folderPath}
                    posterCandidates={stillCandidates}
                    width={EPISODE_STILL.width}
                    height={EPISODE_STILL.height}
                    allowFallbacks={false}
                    className="kg-episode-still"
                    fallbackLabel={`E${String(ep.episodeNumber).padStart(2, "0")}`}
                  />
                  <div className="kg-episode-meta">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span
                        className={ep.filePath ? "text-accent" : "text-fg-muted"}
                        title={ep.filePath ? t("detail.epLocal") : t("detail.epMissing")}
                      >
                        {ep.filePath ? "●" : "○"}
                      </span>
                      <span className="shrink-0 kg-type-caption font-semibold text-fg-muted">
                        E{String(ep.episodeNumber).padStart(2, "0")}
                      </span>
                      <span className="min-w-0 truncate kg-type-body-secondary font-semibold text-fg">
                        {ep.title || t("detail.episodeFallback", { n: ep.episodeNumber })}
                      </span>
                    </div>
                    {ep.overview ? (
                      <p className="line-clamp-2 kg-type-caption leading-[1.35] text-fg-secondary">
                        {ep.overview}
                      </p>
                    ) : null}
                  </div>
                </li>
              );
            })
          )}
        </ul>
      </TabsContent>

      {seasonMenu && <DropdownMenu open modal={false} onOpenChange={open => { if (!open) setSeasonMenu(null); }}>
        <DropdownMenuTrigger asChild><span aria-hidden style={{ position:'fixed', left:seasonMenu.x, top:seasonMenu.y, width:1, height:1 }} /></DropdownMenuTrigger>
        <DropdownMenuContent align="start" sideOffset={0} collisionPadding={8} onCloseAutoFocus={event => event.preventDefault()}>
          <DropdownMenuItem onSelect={() => { const {seasonId,seasonNumber} = seasonMenu; setSeasonMenu(null); void revealSeasonInFinder(folderPath, seasonId, seasonNumber, episodes); }}>{t("action.revealInFinder")}</DropdownMenuItem>
          {canScrapeSeason && <DropdownMenuItem onSelect={() => { const n=seasonMenu.seasonNumber; setSeasonMenu(null); void scrapeSeason(mediaItemId,n); }}>{t("action.scrapeSeason")}</DropdownMenuItem>}
        </DropdownMenuContent>
      </DropdownMenu>}

    </Tabs>
  );
}

async function revealSeasonInFinder(
  showFolder: string,
  seasonId: string,
  seasonNumber: number,
  episodes: TvEpisode[],
) {
  const ep = episodes.find((e) => e.seasonId === seasonId && e.filePath?.trim());
  let target = showFolder;
  if (ep?.filePath?.trim()) {
    target = ep.filePath.trim();
  } else {
    const padded = String(seasonNumber).padStart(2, "0");
    const root = showFolder.replace(/[/\\]+$/, "");
    target = `${root}/Season ${padded}`;
  }
  if (!target) {
    useAppStore.getState().showToast(i18n.t("toast.noPath"));
    return;
  }
  try {
    await invoke("reveal_in_file_manager", { path: target });
  } catch (err) {
    // Season folder may be missing — fall back to the show root.
    if (target !== showFolder) {
      try {
        await invoke("reveal_in_file_manager", { path: showFolder });
        return;
      } catch {
        /* show toast below */
      }
    }
    useAppStore.getState().showToast(localizeUserMessage(String(err)));
  }
}

/** Prefer `{stem}-thumb.jpg` next to the episode file when stillPath is empty. */
function stillCandidateFromFile(filePath?: string | null): string | null {
  if (!filePath) return null;
  const base = filePath.split(/[/\\]/).pop();
  if (!base) return null;
  const stem = base.replace(/\.[^.]+$/, "");
  if (!stem) return null;
  // still_path is stored relative to show root; file may live in Season XX/.
  const dir = filePath.slice(0, Math.max(0, filePath.length - base.length));
  // If absolute path under folder, PosterThumb joins folder+relative — pass relative if possible.
  // Prefer basename-relative forms that resolve via join under folder:
  // 1) Season 01/foo-thumb.jpg when file is .../Season 01/foo.mkv — hard without folder.
  // Use absolute path: resolve_poster_source accepts absolute poster_path.
  return `${dir}${stem}-thumb.jpg`;
}

/** Show basename (or relative path) when the file lives under the folder. */
function fileNameUnderFolder(folderPath: string, filePath: string): string {
  const folder = folderPath.replace(/[/\\]+$/, "");
  const prefixes = [`${folder}/`, `${folder}\\`];
  for (const prefix of prefixes) {
    if (filePath.startsWith(prefix)) {
      return filePath.slice(prefix.length) || filePath;
    }
  }
  return filePath.split(/[/\\]/).pop() || filePath;
}

function DetailSkeleton({
  title,
  mediaType,
}: {
  title?: string;
  mediaType?: MediaType;
}) {
  const { t } = useTranslation();
  const isShow = mediaType === "tvShow" || mediaType === "anime";

  return (
    <div className="min-h-0 flex-1 overflow-auto p-4" aria-busy="true" aria-live="polite">
      <p className="mb-3 kg-type-caption text-fg-muted">{t("detail.loading")}</p>
      <div className="flex gap-3">
        <div className="kg-skeleton h-[148px] w-[100px] shrink-0 rounded-card" />
        <div className="min-w-0 flex-1 space-y-2 pt-1">
          {title ? (
            <p className="kg-type-title font-extrabold leading-tight tracking-[-0.25px] text-fg">
              {title}
            </p>
          ) : (
            <div className="kg-skeleton h-5 w-[75%] rounded-control" />
          )}
          {isShow ? (
            <span className="kg-type-badge" data-type={mediaType}>
              {mediaType === "anime" ? t("type.anime") : t("type.tvShow")}
            </span>
          ) : null}
          <div className="kg-skeleton h-3 w-[48%] rounded-control" />
          <div className="kg-skeleton h-3 w-[62%] rounded-control" />
        </div>
      </div>
      <div className="mt-5 space-y-2">
        <div className="kg-skeleton h-3 w-20 rounded-control" />
        <div className="kg-skeleton h-16 w-full rounded-control" />
      </div>
      {isShow ? (
        <div className="mt-5 space-y-2">
          <div className="kg-skeleton h-3 w-24 rounded-control" />
          <div className="kg-skeleton h-20 w-full rounded-control" />
          <div className="kg-skeleton h-20 w-full rounded-control" />
        </div>
      ) : null}
    </div>
  );
}

function DetailPanel({
  detail,
  posterUrl,
}: {
  detail: NonNullable<ReturnType<typeof useAppStore.getState>["detail"]>;
  posterUrl: string | null;
}) {
  const { t } = useTranslation();
  const { item, metadata, seasons, episodes } = detail;
  const runtime = formatRuntime(metadata?.runtime ?? null, t);
  const isShow = item.mediaType === "tvShow" || item.mediaType === "anime";
  const localEpisodes = episodes.filter((e) => Boolean(e.filePath)).length;

  return (
    <div className="space-y-4 kg-type-body">
      <div className="flex gap-3">
        <div className="h-[148px] w-[100px] shrink-0 overflow-hidden rounded-card border border-hairline bg-subtle">
          {posterUrl ? (
            <img src={posterUrl} alt="" className="h-full w-full object-cover" />
          ) : (
            <div className="flex h-full items-center justify-center kg-type-caption-small text-fg-muted">
              {t("detail.noPoster")}
            </div>
          )}
        </div>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="kg-type-title font-extrabold leading-tight tracking-[-0.25px] text-fg">
              {item.title}
            </h2>
            {isShow ? (
              <span className="kg-type-badge" data-type={item.mediaType}>
                {item.mediaType === "anime" ? t("type.anime") : t("type.tvShow")}
              </span>
            ) : (
              <span className="kg-type-badge" data-type="movie">
                {t("type.movie")}
              </span>
            )}
          </div>
          {metadata?.tagline ? (
            <p className="mt-1 kg-type-caption italic text-fg-secondary">{metadata.tagline}</p>
          ) : null}
          {item.originalTitle && item.originalTitle !== item.title ? (
            <p className="mt-1 kg-type-caption text-fg-secondary">{item.originalTitle}</p>
          ) : null}
          <p className="mt-2 kg-type-caption text-fg-secondary">
            {isShow
              ? [
                  item.year,
                  t("detail.showMeta", {
                    seasons: seasons.length,
                    episodes: episodes.length,
                    local: localEpisodes,
                  }),
                  metadata?.showStatus,
                  t(`status.${item.status}`, { defaultValue: item.status }),
                ]
                  .filter(Boolean)
                  .join(" · ")
              : [
                  item.year,
                  runtime,
                  metadata?.contentRating,
                  t(`status.${item.status}`, { defaultValue: item.status }),
                ]
                  .filter(Boolean)
                  .join(" · ")}
          </p>
          {metadata?.genres?.length ? (
            <p className="mt-2 kg-type-caption text-fg-muted">{metadata.genres.join(" / ")}</p>
          ) : null}
          {metadata?.rating != null ? (
            <p className="mt-1 kg-type-body-secondary font-medium text-warning">
              ★ {metadata.rating.toFixed(1)}
              {metadata.ratingVotes ? ` (${metadata.ratingVotes})` : null}
            </p>
          ) : null}
          {!isShow && metadata?.director ? (
            <p className="mt-1 kg-type-caption text-fg-secondary">
              {t("detail.director")}: {metadata.director}
            </p>
          ) : null}
        </div>
      </div>

      {isShow ? (
        <SeasonBrowser
          mediaItemId={item.id}
          canScrapeSeason={item.status === "scraped"}
          folderPath={item.folderPath}
          seasons={seasons}
          episodes={episodes}
        />
      ) : null}

      {metadata?.overview ? (
        <section>
          <h3 className="kg-section-label">{t("detail.overview")}</h3>
          <p className="whitespace-pre-wrap kg-type-body-secondary leading-relaxed text-fg-secondary">
            {metadata.overview}
          </p>
        </section>
      ) : null}

      {metadata?.credits && metadata.credits.length > 0 ? (
        <CastStrip credits={metadata.credits} />
      ) : null}

      <section>
        <h3 className="kg-section-label">{t("detail.file")}</h3>
        <p className="truncate font-mono kg-type-caption text-fg-secondary" title={item.folderPath}>
          {item.folderPath}
        </p>
        {!isShow && item.filePath && item.filePath !== item.folderPath ? (
          <p className="mt-1 truncate font-mono kg-type-caption text-fg-muted" title={item.filePath}>
            {fileNameUnderFolder(item.folderPath, item.filePath)}
          </p>
        ) : null}
        {metadata?.sourceId ? (
          <p className="mt-2 kg-type-caption text-fg-muted">source: {metadata.sourceId}</p>
        ) : null}
      </section>
    </div>
  );
}

function CastStrip({ credits }: { credits: CastMember[] }) {
  const { t } = useTranslation();
  const actors = useMemo(() => {
    const list = credits
      .filter((c) => c.name.trim().length > 0)
      .filter((c) => !c.type || c.type === "Actor")
      .slice()
      .sort((a, b) => (a.order ?? 999) - (b.order ?? 999));
    return list.slice(0, 20);
  }, [credits]);

  if (actors.length === 0) return null;

  return (
    <section>
      <h3 className="kg-section-label">{t("detail.cast")}</h3>
      <ul className="flex gap-3 overflow-x-auto pb-1">
        {actors.map((actor, i) => (
          <li
            key={`${actor.name}-${actor.order ?? i}`}
            className="flex w-[72px] shrink-0 flex-col items-center gap-1.5"
          >
            <ActorAvatar url={actor.thumbUrl} name={actor.name} size={56} />
            <p className="w-full truncate text-center kg-type-caption font-semibold text-fg">
              {actor.name}
            </p>
            {actor.role ? (
              <p className="w-full truncate text-center kg-type-caption-small text-fg-muted">{actor.role}</p>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}

function formatRuntime(
  minutes: number | null | undefined,
  t: (key: string, opts?: Record<string, unknown>) => string,
) {
  if (minutes == null || minutes <= 0) return null;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h > 0 && m > 0) return t("detail.runtime", { h, m });
  if (h > 0) return t("detail.runtimeHours", { h });
  return t("detail.runtimeMinutes", { m });
}

function TileButton({
  icon,
  label,
  onClick,
  disabled,
  selected,
  primary,
}: {
  icon: ReactNode;
  label: string;
  onClick?: () => void;
  disabled?: boolean;
  selected?: boolean;
  primary?: boolean;
}) {
  return (
    <Button variant={primary ? "default" : "ghost"} size="sm"
      type="button"
      className="kg-tile-btn"
      disabled={disabled}
      data-selected={selected || undefined}
      data-primary={primary || undefined}
      aria-pressed={selected}
      onClick={onClick}
    >
      <span className="kg-tile-btn-icon" aria-hidden>
        {icon}
      </span>
      <span className="kg-tile-btn-label">{label}</span>
    </Button>
  );
}

function MenuButton({
  label,
  items,
  icon,
  align = "right",
}: {
  label: string;
  items: Array<{ label: string; onClick: () => void; destructive?: boolean }>;
  icon?: ReactNode;
  align?: "left" | "right";
}) {
  if (!items.length) return null;
  return <DropdownMenu>
    <DropdownMenuTrigger asChild>
      <Button variant="ghost" size="sm" className="gap-2">{icon}<span>{label}</span></Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align={align === "left" ? "start" : "end"} sideOffset={6}>
      {items.map(item => <DropdownMenuItem key={item.label} variant={item.destructive ? "destructive" : "default"} onSelect={item.onClick}>{item.label}</DropdownMenuItem>)}
    </DropdownMenuContent>
  </DropdownMenu>;
}

function AddMenu({ onAdd }: { onAdd: (t: MediaType) => void }) {
  const { t } = useTranslation();
  return <MenuButton label={t("action.addLibrary")} icon={<IconAdd />} align="left" items={([
    ["movie", "action.addMovie"], ["tvShow", "action.addTv"], ["anime", "action.addAnime"],
  ] as const).map(([type, key]) => ({label:t(key), onClick:()=>onAdd(type)}))} />;
}

function groupLibraries(libraries: ReturnType<typeof useAppStore.getState>["libraries"]) {
  return libraries.reduce(
    (acc, lib) => {
      (acc[lib.mediaType] ??= []).push(lib);
      return acc;
    },
    {} as Record<MediaType, typeof libraries>,
  );
}

function typeLabel(type: MediaType, t: (key: string) => string) {
  switch (type) {
    case "movie":
      return t("sidebar.movies");
    case "tvShow":
      return t("sidebar.tv");
    case "anime":
      return t("sidebar.anime");
  }
}

/** Compact path for sidebar: drop Volumes/Users noise, keep last 1–2 segments. */
function shortLibraryPath(rootPath: string): string {
  const parts = rootPath.replace(/[/\\]+$/, "").split(/[/\\]/).filter(Boolean);
  if (parts.length === 0) return rootPath;
  const start =
    parts[0] === "Volumes" || parts[0] === "Users" || parts[0] === "home" ? 1 : 0;
  const useful = parts.slice(start);
  if (useful.length === 0) return parts[parts.length - 1] ?? rootPath;
  if (useful.length === 1) return useful[0];
  return `${useful[useful.length - 2]} · ${useful[useful.length - 1]}`;
}

function LibraryTypeIcon({ type }: { type: MediaType }) {
  const Icon = type === "movie" ? Film : type === "tvShow" ? Tv : Sparkles;
  return <Icon className="size-4" aria-hidden />;
}

export default App;
