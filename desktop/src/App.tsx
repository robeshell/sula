import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { Sidebar, type MainPage } from "./components/shell/Sidebar";
import { LibraryToolbar, type ToolbarMenuItem } from "./components/library/LibraryToolbar";
import { MediaListHeader, MediaRow, PosterCard, isShow } from "./components/library/MediaCards";
import { BatchInspector, ItemInspector } from "./components/detail/Inspector";
import { ShowPage } from "./components/detail/ShowPage";
import { VirtualMediaList } from "./components/VirtualMediaList";
import { EmptyState } from "./components/EmptyState";
import { CleanupSheet, type ResidualCandidate } from "./components/CleanupSheet";
import { DeleteConfirmModal } from "./components/DeleteConfirmModal";
import { ManualMatchModal } from "./components/ManualMatchModal";
import { MediaContextMenu, type MediaContextMenuTarget } from "./components/MediaContextMenu";
import { SettingsPage } from "./components/SettingsModal";
import { LogPanel } from "./components/LogPanel";
import { FolderBrowser } from "./components/FolderBrowser";
import { ToastHost } from "./components/ToastHost";
import { WindowControls } from "./components/WindowControls";
import { Button } from "./components/ui/button";
import { useAppStore, type MediaItem } from "./store/appStore";

function App() {
  const { t } = useTranslation();
  const [page, setPage] = useState<MainPage>("library");
  const [showPageId, setShowPageId] = useState<string | null>(null);
  const [folderBrowser, setFolderBrowser] = useState<{ rootPath: string; rootName: string } | null>(null);
  const [manualMatchOpen, setManualMatchOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [cleanupCandidates, setCleanupCandidates] = useState<ResidualCandidate[] | null>(null);
  const [contextMenu, setContextMenu] = useState<MediaContextMenuTarget | null>(null);
  const contextSelectionSnapshot = useRef<{
    selectedMediaIds: string[];
    selectedMediaId: string | null;
    detail: ReturnType<typeof useAppStore.getState>["detail"];
    detailLoading: boolean;
    posterUrl: string | null;
  } | null>(null);

  const libraries = useAppStore((s) => s.libraries);
  const selectedLibraryId = useAppStore((s) => s.selectedLibraryId);
  const mediaItems = useAppStore((s) => s.mediaItems);
  const metadataById = useAppStore((s) => s.metadataById);
  const showStatsById = useAppStore((s) => s.showStatsById);
  const selectedMediaIds = useAppStore((s) => s.selectedMediaIds);
  const selectedMediaId = useAppStore((s) => s.selectedMediaId);
  const detail = useAppStore((s) => s.detail);
  const searchQuery = useAppStore((s) => s.searchQuery);
  const sortOption = useAppStore((s) => s.sortOption);
  const statusFilter = useAppStore((s) => s.statusFilter);
  const listViewMode = useAppStore((s) => s.listViewMode);
  // Boolean selector: progress ticks must not re-render the grid while scanning.
  const isLibraryScanning = useAppStore((s) =>
    Boolean(s.selectedLibraryId && s.tasks.some((task) => task.kind === "refresh" && task.targetId === s.selectedLibraryId
      && (task.status === "pending" || task.status === "running"))),
  );
  const refreshStatus = useAppStore((s) => s.refreshStatus);
  const refreshLibraries = useAppStore((s) => s.refreshLibraries);
  const refreshTasks = useAppStore((s) => s.refreshTasks);
  const upsertTask = useAppStore((s) => s.upsertTask);
  const selectLibrary = useAppStore((s) => s.selectLibrary);
  const toggleMediaSelection = useAppStore((s) => s.toggleMediaSelection);
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
  const renameSelectedItems = useAppStore((s) => s.renameSelectedItems);
  const organizeSelectedItems = useAppStore((s) => s.organizeSelectedItems);
  const consolidateSelectedShows = useAppStore((s) => s.consolidateSelectedShows);
  const consolidateSelectedLibraryShows = useAppStore((s) => s.consolidateSelectedLibraryShows);
  const scanResidualsForSelected = useAppStore((s) => s.scanResidualsForSelected);
  const cleanupResiduals = useAppStore((s) => s.cleanupResiduals);
  const refreshSelectedItemsFromDisk = useAppStore((s) => s.refreshSelectedItemsFromDisk);
  const revealSelectedItem = useAppStore((s) => s.revealSelectedItem);
  const deleteSelectedItems = useAppStore((s) => s.deleteSelectedItems);
  const visibleMediaItems = useAppStore((s) => s.visibleMediaItems);

  const selected = libraries.find((l) => l.id === selectedLibraryId) ?? null;
  const visible = useMemo(
    () => visibleMediaItems(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [mediaItems, searchQuery, sortOption, statusFilter, visibleMediaItems],
  );
  const statuses = useMemo(() => mediaItems.map((m) => m.status), [mediaItems]);

  const openSettings = useCallback(async () => {
    try {
      await invoke("open_settings_window");
    } catch {
      // Builds without a separate settings window show it in place.
      setPage("settings");
    }
  }, []);

  const openRenamer = async () => {
    try {
      await invoke("open_renamer_window");
    } catch (err) {
      useAppStore.getState().showToast(String(err));
    }
  };

  const openManualMatch = async () => {
    if (!(await ensureScrapeReady())) return;
    setManualMatchOpen(true);
  };

  const openCleanupSheet = async () => {
    const candidates = await scanResidualsForSelected();
    if (candidates.length > 0) setCleanupCandidates(candidates);
  };

  const goToLibrary = (id: string) => {
    setPage("library");
    setShowPageId(null);
    setFolderBrowser(null);
    void selectLibrary(id);
  };

  const openShow = (item: MediaItem) => {
    if (!isShow(item)) return;
    setFolderBrowser(null);
    if (selectedMediaId !== item.id || selectedMediaIds.length !== 1) void toggleMediaSelection(item.id, false);
    setShowPageId(item.id);
  };

  const openContextMenu = (e: React.MouseEvent, itemId: string) => {
    e.preventDefault();
    e.stopPropagation();
    // Right-click must not leave a text selection highlight under the menu.
    window.getSelection()?.removeAllRanges();
    // Select for context actions only — never open/fetch detail (avoids layout jitter).
    // Snapshot the prior selection so dismissing the menu can undo a right-click-only select.
    if (!selectedMediaIds.includes(itemId)) {
      const s = useAppStore.getState();
      contextSelectionSnapshot.current = {
        selectedMediaIds: s.selectedMediaIds,
        selectedMediaId: s.selectedMediaId,
        detail: s.detail,
        detailLoading: s.detailLoading,
        posterUrl: s.posterUrl,
      };
      useAppStore.setState({ selectedMediaIds: [itemId], selectedMediaId: null, detail: null, detailLoading: false, posterUrl: null });
    } else {
      contextSelectionSnapshot.current = null;
    }
    setContextMenu({ x: e.clientX, y: e.clientY, itemId });
  };

  const closeContextMenu = (commitSelection = false) => {
    setContextMenu(null);
    if (!commitSelection && contextSelectionSnapshot.current) useAppStore.setState(contextSelectionSnapshot.current);
    contextSelectionSnapshot.current = null;
  };

  const contextSelectionIds = contextMenu && selectedMediaIds.includes(contextMenu.itemId)
    ? selectedMediaIds
    : contextMenu ? [contextMenu.itemId] : [];
  const contextItems = mediaItems.filter((m) => contextSelectionIds.includes(m.id));
  const matchTarget = mediaItems.find((m) => m.id === selectedMediaId)
    ?? (detail ? detail.item : null)
    ?? (selectedMediaIds.length === 1 ? (mediaItems.find((m) => m.id === selectedMediaIds[0]) ?? null) : null);

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
    }).then((fn) => { if (disposed) fn(); else unlistenTask = fn; });
    void listen("library-updated", () => {
      if (disposed) return;
      // Items reload in place (selection/detail kept); the store coalesces this
      // with the reload the finishing task already requested.
      const { selectedLibraryId: libraryId, reloadLibraryItems } = useAppStore.getState();
      if (libraryId) void reloadLibraryItems(libraryId);
      // Coalesce bursts so a finishing scan doesn't thrash the library list.
      if (libTimer) clearTimeout(libTimer);
      libTimer = setTimeout(() => {
        libTimer = null;
        void refreshLibraries();
        void refreshStatus();
      }, 200);
    }).then((fn) => { if (disposed) fn(); else unlistenLib = fn; });
    return () => {
      disposed = true;
      unlistenTask?.();
      unlistenLib?.();
      if (libTimer) clearTimeout(libTimer);
    };
  }, [refreshStatus, refreshLibraries, refreshTasks, upsertTask]);

  // ⌘, / Ctrl+, opens Settings like any native app.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === ",") {
        e.preventDefault();
        void openSettings();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openSettings]);

  // Leaving the show page when its item disappears or the library changes.
  useEffect(() => {
    if (showPageId && !mediaItems.some((m) => m.id === showPageId)) setShowPageId(null);
  }, [mediaItems, showPageId]);

  const libraryMenu: ToolbarMenuItem[] = selected ? [
    ...(selected.mediaType !== "movie" ? [{ label: t("action.mergeDuplicates"), onSelect: () => void consolidateSelectedLibraryShows() }] : []),
    { label: t("action.browseFolder"), onSelect: () => setFolderBrowser({ rootPath: selected.rootPath, rootName: selected.name }) },
    { label: t("action.deleteLibrary"), onSelect: () => void deleteSelectedLibrary(), destructive: true, separatorBefore: true },
  ] : [];

  const selectedItems = mediaItems.filter((m) => selectedMediaIds.includes(m.id));
  const inspectorItem = selectedMediaIds.length <= 1 && selectedMediaId ? mediaItems.find((m) => m.id === selectedMediaId) ?? null : null;
  const inspectorCallbacks = {
    onManualMatch: () => void openManualMatch(),
    onCleanup: () => void openCleanupSheet(),
    onDelete: () => setDeleteOpen(true),
    onBrowseFolder: (item: MediaItem) => setFolderBrowser({ rootPath: item.folderPath, rootName: item.title }),
    onOpenShow: openShow,
  };
  const showPageDetail = showPageId && detail?.item.id === showPageId ? detail : null;
  const resetKey = `${selectedLibraryId}:${searchQuery}:${sortOption}:${statusFilter}`;

  const renderItem = (item: MediaItem, index: number) => {
    const props = {
      item,
      meta: metadataById[item.id],
      stats: showStatsById[item.id],
      selected: selectedMediaIds.includes(item.id),
      onClick: (e: React.MouseEvent) => void toggleMediaSelection(item.id, e.metaKey || e.ctrlKey),
      onDoubleClick: () => openShow(item),
      onContextMenu: (e: React.MouseEvent) => openContextMenu(e, item.id),
    };
    return listViewMode === "poster" ? <PosterCard {...props} /> : <MediaRow {...props} odd={index % 2 === 1} />;
  };

  let content: React.ReactNode;
  if (page === "settings") {
    content = <div className="sl-main"><SettingsPage onClose={() => setPage("library")} /></div>;
  } else if (page === "logs") {
    content = <div className="sl-main"><LogPanel onClose={() => setPage("library")} /></div>;
  } else if (showPageId) {
    content = showPageDetail ? (
      <ShowPage detail={showPageDetail} libraryName={selected?.name ?? ""} onBack={() => setShowPageId(null)} />
    ) : (
      <div className="sl-main"><header className="sl-toolbar"><div data-tauri-drag-region /></header>
        <EmptyState className="flex-1" title={t("detail.loading")} message="" /></div>
    );
  } else {
    content = (
      <div className="sl-main">
        {selected ? (
          <LibraryToolbar
            title={selected.name}
            statuses={statuses}
            status={statusFilter}
            onStatus={setStatusFilter}
            view={listViewMode}
            onView={setListViewMode}
            sort={sortOption}
            onSort={setSortOption}
            query={searchQuery}
            onQuery={setSearchQuery}
            scanning={isLibraryScanning}
            onRefresh={() => void refreshSelectedLibrary()}
            onScrapeAll={() => void scrapeSelectedLibrary()}
            menu={libraryMenu}
          />
        ) : (
          <header className="sl-toolbar"><div data-tauri-drag-region /><div className="sl-title"><b>{t("app.brand")}</b></div></header>
        )}
        <div className="sl-body">
          <div className="sl-content">
            {!selected ? (
              <EmptyState
                className="h-full"
                title={t("empty.welcomeTitle")}
                message={libraries.length === 0 ? t("empty.welcomeMessage") : t("list.pickLibrary")}
                action={libraries.length === 0 ? (
                  <div className="flex flex-wrap justify-center gap-2">
                    <Button size="sm" onClick={() => void addLibrary("movie")}>{t("action.addMovie")}</Button>
                    <Button variant="outline" size="sm" onClick={() => void addLibrary("tvShow")}>{t("action.addTv")}</Button>
                    <Button variant="outline" size="sm" onClick={() => void addLibrary("anime")}>{t("action.addAnime")}</Button>
                  </div>
                ) : undefined}
              />
            ) : visible.length === 0 ? (
              <EmptyState
                className="h-full"
                title={isLibraryScanning ? t("empty.scanningTitle") : mediaItems.length === 0 ? t("empty.scanTitle") : t("empty.filterTitle")}
                message={isLibraryScanning ? t("empty.scanningMessage") : mediaItems.length === 0 ? t("list.emptyScan") : t("list.emptyFilter")}
                action={mediaItems.length === 0 && !isLibraryScanning ? (
                  <Button size="sm" onClick={() => void refreshSelectedLibrary()}>{t("action.refresh")}</Button>
                ) : undefined}
              />
            ) : (
              <VirtualMediaList items={visible} mode={listViewMode} resetKey={resetKey}
                header={listViewMode === "list" ? <MediaListHeader shows={selected.mediaType !== "movie"} /> : undefined}>
                {renderItem}
              </VirtualMediaList>
            )}
          </div>
          {folderBrowser ? (
            <aside className="sl-insp">
              <FolderBrowser rootPath={folderBrowser.rootPath} rootName={folderBrowser.rootName} onClose={() => setFolderBrowser(null)} />
            </aside>
          ) : selectedItems.length > 1 ? (
            <BatchInspector items={selectedItems} callbacks={inspectorCallbacks} />
          ) : inspectorItem ? (
            <ItemInspector item={inspectorItem} detail={detail} meta={metadataById[inspectorItem.id]}
              stats={showStatsById[inspectorItem.id]} callbacks={inspectorCallbacks} />
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <div className="sl-app">
      <div className="kg-titlebar absolute right-0 top-0 z-30 h-[var(--kg-titlebar-height)]" aria-hidden>
        <WindowControls />
      </div>
      <Sidebar
        page={page}
        onSelectLibrary={goToLibrary}
        onOpenLogs={() => { setShowPageId(null); setPage(page === "logs" ? "library" : "logs"); }}
        onOpenSettings={() => void openSettings()}
        onOpenRenamer={() => void openRenamer()}
      />
      {content}

      {manualMatchOpen && matchTarget ? (
        <ManualMatchModal itemId={matchTarget.id} mediaType={matchTarget.mediaType} initialQuery={matchTarget.title}
          onClose={() => setManualMatchOpen(false)} />
      ) : null}
      {deleteOpen ? (
        <DeleteConfirmModal
          title={selectedMediaIds.length > 1 ? t("delete.multiTitle", { count: selectedMediaIds.length }) : (detail?.item.title ?? matchTarget?.title ?? "")}
          onClose={() => setDeleteOpen(false)}
          onConfirm={(alsoTrash) => { setDeleteOpen(false); void deleteSelectedItems(alsoTrash); }}
        />
      ) : null}
      {cleanupCandidates ? (
        <CleanupSheet candidates={cleanupCandidates} onClose={() => setCleanupCandidates(null)}
          onConfirm={(paths) => { setCleanupCandidates(null); void cleanupResiduals(paths); }} />
      ) : null}
      {contextMenu ? (
        <MediaContextMenu
          menu={contextMenu}
          canScrapeAuto={contextItems.some((m) => m.status === "unscraped" || m.status === "partial")}
          canRescrape={contextItems.some((m) => m.status === "scraped")}
          canManualMatch={contextSelectionIds.length === 1}
          canRename={contextItems.some((m) => m.status === "scraped")}
          canOrganize={contextItems.some((m) => m.status === "scraped" && isShow(m))}
          canMergeDuplicates={contextItems.some(isShow)}
          canCleanResiduals={contextItems.some((m) => m.status === "scraped")}
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

export default App;
