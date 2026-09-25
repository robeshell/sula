import { useTranslation } from "react-i18next";
import { Film, Tv, Sparkles, Plus, Settings, ScrollText, PencilLine } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { TaskActivity } from "./TaskActivity";
import { useAppStore, type MediaType } from "../../store/appStore";

export type MainPage = "library" | "settings" | "logs";

const TYPE_ORDER: MediaType[] = ["movie", "tvShow", "anime"];

export function LibraryTypeIcon({ type }: { type: MediaType }) {
  const Icon = type === "movie" ? Film : type === "tvShow" ? Tv : Sparkles;
  return <Icon aria-hidden />;
}

/** Source list: libraries on top, background work and tools at the bottom. */
export function Sidebar({ page, onSelectLibrary, onOpenLogs, onOpenSettings, onOpenRenamer }: {
  page: MainPage;
  onSelectLibrary: (id: string) => void;
  onOpenLogs: () => void;
  onOpenSettings: () => void;
  onOpenRenamer: () => void;
}) {
  const { t } = useTranslation();
  const libraries = useAppStore((s) => s.libraries);
  const selectedLibraryId = useAppStore((s) => s.selectedLibraryId);
  const itemCount = useAppStore((s) => s.mediaItems.length);
  const addLibrary = useAppStore((s) => s.addLibrary);
  // One string per render: progress ticks must not re-render the sidebar.
  const busyKey = useAppStore((s) =>
    s.tasks
      .filter((task) => (task.status === "pending" || task.status === "running") && task.targetId)
      .map((task) => task.targetId)
      .sort()
      .join("|"),
  );
  const busy = new Set(busyKey ? busyKey.split("|") : []);
  const ordered = libraries
    .slice()
    .sort((a, b) => TYPE_ORDER.indexOf(a.mediaType) - TYPE_ORDER.indexOf(b.mediaType));

  return (
    <aside className="sl-side" onContextMenu={(e) => e.preventDefault()}>
      <div className="sl-side-top" data-tauri-drag-region />
      <div className="sl-sec">
        <span>{t("sidebar.libraries")}</span>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button type="button" aria-label={t("action.addLibrary")} title={t("action.addLibrary")}>
              <Plus aria-hidden />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" sideOffset={6}>
            {TYPE_ORDER.map((type) => (
              <DropdownMenuItem key={type} onSelect={() => void addLibrary(type)}>
                <LibraryTypeIcon type={type} />
                {t(type === "movie" ? "action.addMovie" : type === "tvShow" ? "action.addTv" : "action.addAnime")}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <nav className="sl-libs" aria-label={t("sidebar.libraries")}>
        {ordered.length === 0 ? (
          <p className="sl-empty">{t("sidebar.noLibraries")}</p>
        ) : (
          ordered.map((lib) => {
            const current = page === "library" && lib.id === selectedLibraryId;
            return (
              <button
                key={lib.id}
                type="button"
                className="sl-nav"
                data-selected={current || undefined}
                aria-current={current ? "page" : undefined}
                title={lib.rootPath}
                onClick={() => onSelectLibrary(lib.id)}
              >
                <LibraryTypeIcon type={lib.mediaType} />
                <span className="nm">{lib.name}</span>
                {busy.has(lib.id) ? <span className="busy" aria-label={t("tasks.status.running")} /> : null}
                {lib.id === selectedLibraryId && itemCount > 0 ? <span className="ct">{itemCount}</span> : null}
              </button>
            );
          })
        )}
      </nav>

      <div className="sl-grow" />
      <div className="sl-foot">
        <TaskActivity />
        <button type="button" className="sl-nav plain" onClick={onOpenRenamer}>
          <PencilLine aria-hidden />
          <span className="nm">{t("action.renamer")}</span>
        </button>
        <button type="button" className="sl-nav plain" data-selected={page === "logs" || undefined} onClick={onOpenLogs}>
          <ScrollText aria-hidden />
          <span className="nm">{t("action.logs")}</span>
        </button>
        <button type="button" className="sl-nav plain" data-selected={page === "settings" || undefined} onClick={onOpenSettings}>
          <Settings aria-hidden />
          <span className="nm">{t("action.settings")}</span>
        </button>
      </div>
    </aside>
  );
}
