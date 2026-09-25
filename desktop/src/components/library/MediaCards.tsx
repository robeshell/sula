import { useTranslation } from "react-i18next";
import { PosterThumb } from "../PosterThumb";
import { PosterBadge, StatusText } from "../StatusBadge";
import { POSTER_THUMB } from "../../lib/posterLoadQueue";
import type { MediaItem, MediaMetaSummary, ShowListStats } from "../../store/appStore";

type CardProps = {
  item: MediaItem;
  meta?: MediaMetaSummary;
  stats?: ShowListStats;
  selected: boolean;
  onClick: (e: React.MouseEvent) => void;
  onDoubleClick: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
};

export const isShow = (item: Pick<MediaItem, "mediaType">) => item.mediaType === "tvShow" || item.mediaType === "anime";

function subline(item: MediaItem, stats: ShowListStats | undefined, t: (k: string, o?: Record<string, unknown>) => string) {
  const parts: string[] = [];
  if (item.year) parts.push(String(item.year));
  if (isShow(item) && stats) parts.push(t("list.showStatsShort", { seasons: stats.seasonCount, episodes: stats.episodeCount }));
  return parts.join(" · ");
}

// Right-click selects for the context menu without letting the button steal focus styles.
const keepSelection = (e: React.MouseEvent) => { if (e.button === 2) e.preventDefault(); };

export function PosterCard({ item, meta, stats, selected, onClick, onDoubleClick, onContextMenu }: CardProps) {
  const { t } = useTranslation();
  return (
    <button type="button" className="sl-poster" data-selected={selected || undefined} aria-pressed={selected}
      onClick={onClick} onDoubleClick={onDoubleClick} onMouseDown={keepSelection} onContextMenu={onContextMenu}>
      <span className="sl-poster-art">
        <PosterThumb folderPath={item.folderPath} posterPath={meta?.posterPath} width={POSTER_THUMB.width}
          height={POSTER_THUMB.height} className="sl-poster-img" fallbackLabel={item.title} />
        <PosterBadge status={item.status} />
      </span>
      <span className="sl-poster-text">
        <b title={item.title}>{item.title}</b>
        <span>{subline(item, stats, t) || " "}</span>
      </span>
    </button>
  );
}

export function MediaListHeader({ shows }: { shows: boolean }) {
  const { t } = useTranslation();
  return (
    <div className="sl-row sl-row-head" aria-hidden>
      <span>{t("list.col.title")}</span>
      <span className="r">{t("list.col.year")}</span>
      <span>{shows ? t("list.col.episodes") : t("list.col.genre")}</span>
      <span className="r">{t("list.col.rating")}</span>
      <span>{t("list.col.status")}</span>
    </div>
  );
}

export function MediaRow({ item, meta, stats, selected, odd, onClick, onDoubleClick, onContextMenu }: CardProps & { odd: boolean }) {
  const { t } = useTranslation();
  const info = isShow(item)
    ? stats
      ? t("list.episodesLocal", { local: stats.localEpisodeCount, total: stats.episodeCount })
      : "—"
    : meta?.genres?.slice(0, 2).join("、") || "—";
  return (
    <button type="button" className="sl-row" data-selected={selected || undefined} data-odd={odd || undefined} aria-pressed={selected}
      onClick={onClick} onDoubleClick={onDoubleClick} onMouseDown={keepSelection} onContextMenu={onContextMenu}>
      <span className="nm">
        <span className="th">
          <PosterThumb folderPath={item.folderPath} posterPath={meta?.posterPath} width={POSTER_THUMB.width}
            height={POSTER_THUMB.height} fallbackLabel="" />
        </span>
        <span className="ts">
          <b>{item.title}</b>
          {item.originalTitle && item.originalTitle !== item.title ? <span className="l2">{item.originalTitle}</span> : null}
        </span>
      </span>
      <span className="r l2">{item.year ?? "—"}</span>
      <span className="l2 ell">{info}</span>
      <span className="r">{meta?.rating != null ? meta.rating.toFixed(1) : "—"}</span>
      <StatusText status={item.status} />
    </button>
  );
}
