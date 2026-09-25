import { useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { X, Star, Ellipsis, ArrowDownToLine, RefreshCw, TextCursorInput, FolderTree, HardDrive, Trash2, Merge, Eraser } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { PosterThumb } from "../PosterThumb";
import { ActorAvatar } from "../ActorAvatar";
import { StatusText } from "../StatusBadge";
import { isShow } from "../library/MediaCards";
import { POSTER_THUMB } from "../../lib/posterLoadQueue";
import { useAppStore, type MediaDetail, type MediaItem, type MediaMetaSummary, type ShowListStats } from "../../store/appStore";

export type InspectorCallbacks = {
  onManualMatch: () => void;
  onCleanup: () => void;
  onDelete: () => void;
  onBrowseFolder: (item: MediaItem) => void;
  onOpenShow: (item: MediaItem) => void;
};

type MenuEntry = { label: string; onSelect: () => void; destructive?: boolean } | "separator";

function MoreMenu({ entries }: { entries: MenuEntry[] }) {
  const { t } = useTranslation();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button type="button" className="sl-btn" aria-label={t("action.more")} title={t("action.more")}><Ellipsis aria-hidden /></button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" sideOffset={6}>
        {entries.map((entry, i) => entry === "separator"
          ? <DropdownMenuSeparator key={`sep-${i}`} />
          : <DropdownMenuItem key={entry.label} variant={entry.destructive ? "destructive" : "default"} onSelect={entry.onSelect}>{entry.label}</DropdownMenuItem>)}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function formatRuntime(minutes: number | null | undefined, t: (key: string, opts?: Record<string, unknown>) => string) {
  if (minutes == null || minutes <= 0) return null;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  if (h > 0 && m > 0) return t("detail.runtime", { h, m });
  if (h > 0) return t("detail.runtimeHours", { h });
  return t("detail.runtimeMinutes", { m });
}

/** "tmdb:872585" → "TMDB 872585"; anything unrecognised is shown as-is. */
export function formatSource(sourceId: string | null | undefined): string | null {
  if (!sourceId) return null;
  const [provider, id] = sourceId.split(":");
  if (!id) return sourceId;
  const names: Record<string, string> = { tmdb: "TMDB", tvdb: "TVDB", omdb: "OMDb", imdb: "IMDb", bangumi: "Bangumi" };
  return `${names[provider.toLowerCase()] ?? provider} ${id}`;
}

function Facts({ children }: { children: ReactNode[] }) {
  const parts = children.filter(Boolean);
  return (
    <div className="sl-facts">
      {parts.map((part, i) => (
        <span key={i} className="contents">
          {i > 0 ? <span className="dot" aria-hidden /> : null}
          {part}
        </span>
      ))}
    </div>
  );
}

export function ItemInspector({ item, detail, meta, stats, callbacks }: {
  item: MediaItem;
  detail: MediaDetail | null;
  meta?: MediaMetaSummary;
  stats?: ShowListStats;
  callbacks: InspectorCallbacks;
}) {
  const { t } = useTranslation();
  const clearMediaSelection = useAppStore((s) => s.clearMediaSelection);
  const scrapeSelectedItems = useAppStore((s) => s.scrapeSelectedItems);
  const rescrapeSelectedItems = useAppStore((s) => s.rescrapeSelectedItems);
  const renameSelectedItem = useAppStore((s) => s.renameSelectedItem);
  const organizeSelectedItems = useAppStore((s) => s.organizeSelectedItems);
  const consolidateSelectedShows = useAppStore((s) => s.consolidateSelectedShows);
  const refreshSelectedItemsFromDisk = useAppStore((s) => s.refreshSelectedItemsFromDisk);
  const revealSelectedItem = useAppStore((s) => s.revealSelectedItem);

  const metadata = detail?.item.id === item.id ? detail.metadata : null;
  const seasons = detail?.item.id === item.id ? detail.seasons : [];
  const episodes = detail?.item.id === item.id ? detail.episodes : [];
  const show = isShow(item);
  const scraped = item.status === "scraped";
  const issue = (item as MediaItem & { scrapeIssue?: string | null }).scrapeIssue;
  const runtime = formatRuntime(metadata?.runtime ?? null, t);
  const rating = metadata?.rating ?? meta?.rating ?? null;
  const genres = (metadata?.genres?.length ? metadata.genres : meta?.genres) ?? [];

  const more: MenuEntry[] = [
    ...(scraped ? [{ label: t("action.applyRename"), onSelect: () => void renameSelectedItem() }] : []),
    ...(scraped && show ? [
      { label: t("action.organizeSeasons"), onSelect: () => void organizeSelectedItems() },
      { label: t("action.mergeDuplicates"), onSelect: () => void consolidateSelectedShows() },
    ] : []),
    ...(scraped ? [{ label: t("action.cleanResiduals"), onSelect: callbacks.onCleanup }] : []),
    { label: t("action.refreshFromDisk"), onSelect: () => void refreshSelectedItemsFromDisk() },
    { label: t("action.revealInFinder"), onSelect: () => void revealSelectedItem() },
    { label: t("action.browseFolder"), onSelect: () => callbacks.onBrowseFolder(item) },
    "separator",
    { label: t("action.deleteItem"), onSelect: callbacks.onDelete, destructive: true },
  ];

  const note = item.status === "unscraped"
    ? t("inspector.note.unscraped")
    : item.status === "unmatched"
      ? t("inspector.note.unmatched")
      : item.status === "partial"
        ? (issue ? t("inspector.note.partialWith", { issue }) : t("inspector.note.partial"))
        : null;

  const cast = useMemo(() => (metadata?.credits ?? [])
    .filter((c) => c.name.trim() && (!c.type || c.type === "Actor"))
    .sort((a, b) => (a.order ?? 999) - (b.order ?? 999))
    .slice(0, 12), [metadata?.credits]);

  return (
    <aside className="sl-insp" aria-label={t("detail.title")}>
      <button type="button" className="sl-insp-close" aria-label={t("detail.close")} title={t("detail.close")} onClick={() => clearMediaSelection()}>
        <X aria-hidden />
      </button>
      <div className="sl-insp-head">
        <span className="sl-poster-art">
          <PosterThumb folderPath={item.folderPath} posterPath={metadata?.posterPath ?? meta?.posterPath} width={POSTER_THUMB.width}
            height={POSTER_THUMB.height} className="sl-poster-img" fallbackLabel={item.title} />
        </span>
        <div className="sl-insp-title">
          <b>{item.title}</b>
          <span>
            {[item.originalTitle && item.originalTitle !== item.title ? item.originalTitle : null, item.year].filter(Boolean).join(" · ")}
          </span>
        </div>
      </div>

      {scraped ? (
        <Facts>
          {[
            rating != null ? <span className="rate"><Star aria-hidden />{rating.toFixed(1)}</span> : null,
            show
              ? (stats ? t("list.showStatsShort", { seasons: stats.seasonCount, episodes: stats.episodeCount }) : null)
              : runtime,
            show && stats && stats.localEpisodeCount < stats.episodeCount
              ? <span className="warn">{t("inspector.localOf", { local: stats.localEpisodeCount, total: stats.episodeCount })}</span>
              : null,
            metadata?.contentRating ?? null,
            genres.length ? genres.slice(0, 3).join("、") : null,
          ]}
        </Facts>
      ) : (
        <div className="sl-facts"><StatusText status={item.status} /></div>
      )}
      {note ? <p className="sl-note">{note}</p> : null}

      <div className="sl-btns">
        {item.status === "unscraped" ? (
          <>
            <button type="button" className="sl-btn grow acc" onClick={() => void scrapeSelectedItems()}>{t("action.scrapeItem")}</button>
            <button type="button" className="sl-btn grow" onClick={callbacks.onManualMatch}>{t("action.manualMatch")}</button>
          </>
        ) : item.status === "unmatched" ? (
          <>
            <button type="button" className="sl-btn grow acc" onClick={callbacks.onManualMatch}>{t("action.manualMatch")}</button>
            <button type="button" className="sl-btn grow" onClick={() => void scrapeSelectedItems()}>{t("inspector.retryAuto")}</button>
          </>
        ) : item.status === "partial" ? (
          <>
            <button type="button" className="sl-btn grow acc" onClick={() => void rescrapeSelectedItems()}>{t("inspector.complete")}</button>
            <button type="button" className="sl-btn grow" onClick={callbacks.onManualMatch}>{t("action.manualMatch")}</button>
          </>
        ) : (
          <>
            <button type="button" className="sl-btn grow" onClick={() => void rescrapeSelectedItems()}>{t("action.rescrape")}</button>
            <button type="button" className="sl-btn grow" onClick={callbacks.onManualMatch}>{t("action.manualMatch")}</button>
          </>
        )}
        <MoreMenu entries={more} />
      </div>

      {metadata?.overview ? (
        <section className="sl-sect">
          <h4>{t("detail.overview")}</h4>
          <p className="clamp" title={metadata.overview}>{metadata.overview}</p>
        </section>
      ) : null}

      {show && seasons.length > 0 ? (
        <section className="sl-sect">
          <h4>{t("detail.seasons")}<button type="button" onClick={() => callbacks.onOpenShow(item)}>{t("inspector.openShow")}</button></h4>
          <dl className="sl-info">
            {seasons.slice().sort((a, b) => a.seasonNumber - b.seasonNumber).map((season) => {
              const eps = episodes.filter((e) => e.seasonId === season.id);
              const local = eps.filter((e) => Boolean(e.filePath)).length;
              return (
                <span key={season.id} className="contents">
                  <dt>{t("detail.seasonLabel", { n: season.seasonNumber })}</dt>
                  <dd className={local < eps.length ? "text-[color:var(--kg-status-orange)]" : undefined}>
                    {t("inspector.localOf", { local, total: eps.length })}
                  </dd>
                </span>
              );
            })}
          </dl>
        </section>
      ) : null}

      {cast.length > 0 ? (
        <section className="sl-sect">
          <h4>{t("detail.cast")}</h4>
          <div className="sl-cast">
            {cast.map((actor, i) => (
              <div key={`${actor.name}-${actor.order ?? i}`}>
                <ActorAvatar url={actor.thumbUrl} name={actor.name} size={38} />
                <span className="nm">{actor.name}</span>
                {actor.role ? <small>{actor.role}</small> : null}
              </div>
            ))}
          </div>
        </section>
      ) : null}

      <section className="sl-sect">
        <h4>{t("inspector.info")}</h4>
        <dl className="sl-info">
          {!show && item.filePath ? (<><dt>{t("inspector.file")}</dt><dd>{item.filePath.split(/[/\\]/).pop()}</dd></>) : null}
          {metadata?.director && !show ? (<><dt>{t("detail.director")}</dt><dd>{metadata.director}</dd></>) : null}
          <dt>{t("inspector.location")}</dt>
          <dd className="mono">{item.folderPath}</dd>
          {formatSource(metadata?.sourceId) ? (<><dt>{t("inspector.source")}</dt><dd>{formatSource(metadata?.sourceId)}</dd></>) : null}
        </dl>
      </section>
    </aside>
  );
}

export function BatchInspector({ items, callbacks }: { items: MediaItem[]; callbacks: Pick<InspectorCallbacks, "onCleanup" | "onDelete"> }) {
  const { t } = useTranslation();
  const clearMediaSelection = useAppStore((s) => s.clearMediaSelection);
  const scrapeSelectedItems = useAppStore((s) => s.scrapeSelectedItems);
  const rescrapeSelectedItems = useAppStore((s) => s.rescrapeSelectedItems);
  const renameSelectedItems = useAppStore((s) => s.renameSelectedItems);
  const organizeSelectedItems = useAppStore((s) => s.organizeSelectedItems);
  const consolidateSelectedShows = useAppStore((s) => s.consolidateSelectedShows);
  const refreshSelectedItemsFromDisk = useAppStore((s) => s.refreshSelectedItemsFromDisk);
  const scraped = items.filter((i) => i.status === "scraped").length;
  const todo = items.length - scraped;
  const shows = items.some(isShow);

  return (
    <aside className="sl-insp" aria-label={t("list.selectedCount", { count: items.length })}>
      <button type="button" className="sl-insp-close" aria-label={t("list.clearSelection")} title={t("list.clearSelection")} onClick={() => clearMediaSelection()}>
        <X aria-hidden />
      </button>
      <div className="sl-batch">
        <h2>{t("list.selectedCount", { count: items.length })}</h2>
        <p className="sum">{t("inspector.batchSummary", { scraped, todo })}</p>
        <div className="list">
          {items.map((item) => (
            <div key={item.id}><span>{item.title}</span><StatusText status={item.status} /></div>
          ))}
        </div>
        <div className="acts">
          {todo > 0 ? (
            <button type="button" className="sl-btn acc" onClick={() => void scrapeSelectedItems()}>
              <ArrowDownToLine aria-hidden />{t("inspector.scrapeTodo", { count: todo })}
            </button>
          ) : null}
          {scraped > 0 ? (
            <button type="button" className="sl-btn" onClick={() => void rescrapeSelectedItems()}><RefreshCw aria-hidden />{t("inspector.rescrapeN", { count: scraped })}</button>
          ) : null}
          {scraped > 0 ? (
            <button type="button" className="sl-btn" onClick={() => void renameSelectedItems()}><TextCursorInput aria-hidden />{t("action.applyRename")}</button>
          ) : null}
          {shows && scraped > 0 ? (
            <button type="button" className="sl-btn" onClick={() => void organizeSelectedItems()}><FolderTree aria-hidden />{t("action.organizeSeasons")}</button>
          ) : null}
          {shows ? (
            <button type="button" className="sl-btn" onClick={() => void consolidateSelectedShows()}><Merge aria-hidden />{t("action.mergeDuplicates")}</button>
          ) : null}
          {scraped > 0 ? (
            <button type="button" className="sl-btn" onClick={callbacks.onCleanup}><Eraser aria-hidden />{t("action.cleanResiduals")}</button>
          ) : null}
          <button type="button" className="sl-btn" onClick={() => void refreshSelectedItemsFromDisk()}><HardDrive aria-hidden />{t("action.refreshFromDisk")}</button>
          <hr />
          <button type="button" className="sl-btn danger" onClick={callbacks.onDelete}><Trash2 aria-hidden />{t("action.deleteItem")}…</button>
        </div>
      </div>
    </aside>
  );
}

export function InspectorSkeleton({ item }: { item: MediaItem }) {
  const { t } = useTranslation();
  return (
    <aside className="sl-insp" aria-busy="true">
      <div className="sl-insp-head">
        <span className="sl-poster-art" />
        <div className="sl-insp-title"><b>{item.title}</b><span>{t("detail.loading")}</span></div>
      </div>
      <div className="grid gap-2 px-[18px] pt-4">
        <div className="kg-skeleton h-3 w-3/4 rounded" />
        <div className="kg-skeleton h-3 w-1/2 rounded" />
      </div>
    </aside>
  );
}
