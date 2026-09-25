import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, FolderOpen, Ellipsis, Star, Check, CircleDashed } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { PosterThumb } from "../PosterThumb";
import { ActorAvatar } from "../ActorAvatar";
import { formatSource } from "./Inspector";
import { POSTER_THUMB } from "../../lib/posterLoadQueue";
import { localizeUserMessage } from "../../lib/localizeMessage";
import { useAppStore, type MediaDetail, type TvEpisode } from "../../store/appStore";

async function revealSeason(showFolder: string, seasonId: string, seasonNumber: number, episodes: TvEpisode[]) {
  const ep = episodes.find((e) => e.seasonId === seasonId && e.filePath?.trim());
  const target = ep?.filePath?.trim() || `${showFolder.replace(/[/\\]+$/, "")}/Season ${String(seasonNumber).padStart(2, "0")}`;
  try {
    await invoke("reveal_in_file_manager", { path: target });
  } catch (err) {
    // The season folder may not exist (flat layout): fall back to the show folder.
    try {
      await invoke("reveal_in_file_manager", { path: showFolder });
    } catch {
      useAppStore.getState().showToast(localizeUserMessage(String(err)));
    }
  }
}

/** Full page for a show: seasons, every episode with its local file, cast and info. */
export function ShowPage({ detail, onBack, libraryName }: { detail: MediaDetail; onBack: () => void; libraryName: string }) {
  const { t } = useTranslation();
  const rescrapeSelectedItems = useAppStore((s) => s.rescrapeSelectedItems);
  const organizeSelectedItems = useAppStore((s) => s.organizeSelectedItems);
  const revealSelectedItem = useAppStore((s) => s.revealSelectedItem);
  const scrapeSeason = useAppStore((s) => s.scrapeSeason);
  const { item, metadata, seasons, episodes } = detail;
  const ordered = useMemo(() => seasons.slice().sort((a, b) => a.seasonNumber - b.seasonNumber), [seasons]);
  const [activeId, setActiveId] = useState(ordered[0]?.id ?? "");
  useEffect(() => {
    if (!ordered.some((s) => s.id === activeId)) setActiveId(ordered[0]?.id ?? "");
  }, [ordered, activeId]);
  const active = ordered.find((s) => s.id === activeId) ?? ordered[0];
  const seasonEpisodes = (seasonId: string) => episodes.filter((e) => e.seasonId === seasonId).sort((a, b) => a.episodeNumber - b.episodeNumber);
  const activeEps = active ? seasonEpisodes(active.id) : [];
  const local = episodes.filter((e) => Boolean(e.filePath)).length;
  const scraped = item.status === "scraped";
  const cast = (metadata?.credits ?? []).filter((c) => c.name.trim() && (!c.type || c.type === "Actor"))
    .sort((a, b) => (a.order ?? 999) - (b.order ?? 999)).slice(0, 8);
  const years = [item.year, metadata?.showStatus].filter(Boolean).join(" · ");

  return (
    <div className="sl-main">
      <header className="sl-toolbar">
        <div data-tauri-drag-region />
        <button type="button" className="sl-iconbtn back" aria-label={t("detail.back", { name: libraryName })} title={t("detail.back", { name: libraryName })} onClick={onBack}>
          <ChevronLeft aria-hidden />
        </button>
        <div className="sl-title"><b>{item.title}</b><span>{libraryName}</span></div>
        <button type="button" className="sl-iconbtn" aria-label={t("action.revealInFinder")} title={t("action.revealInFinder")} onClick={() => void revealSelectedItem()}>
          <FolderOpen aria-hidden />
        </button>
      </header>
      <div className="sl-page">
        <div className="sl-page-head">
          <span className="sl-poster-art">
            <PosterThumb folderPath={item.folderPath} posterPath={metadata?.posterPath} width={POSTER_THUMB.width} height={POSTER_THUMB.height}
              className="sl-poster-img" fallbackLabel={item.title} />
          </span>
          <div className="sl-page-meta">
            <h1>{item.title}</h1>
            <span className="sub">{[item.originalTitle && item.originalTitle !== item.title ? item.originalTitle : null, years].filter(Boolean).join(" · ")}</span>
            <div className="sl-facts">
              {metadata?.rating != null ? <span className="rate"><Star aria-hidden />{metadata.rating.toFixed(1)}</span> : null}
              {metadata?.rating != null ? <span className="dot" aria-hidden /> : null}
              <span>{t("list.showStatsShort", { seasons: seasons.length, episodes: episodes.length })}</span>
              <span className="dot" aria-hidden />
              <span className={local < episodes.length ? "warn" : undefined}>{t("inspector.localOf", { local, total: episodes.length })}</span>
              {metadata?.genres?.length ? <><span className="dot" aria-hidden /><span>{metadata.genres.slice(0, 3).join("、")}</span></> : null}
              {metadata?.contentRating ? <><span className="dot" aria-hidden /><span>{metadata.contentRating}</span></> : null}
            </div>
            {metadata?.overview ? <p>{metadata.overview}</p> : null}
          </div>
          <div className="sl-page-acts">
            {scraped ? <button type="button" className="sl-btn" onClick={() => void organizeSelectedItems()}>{t("action.organizeSeasons")}</button> : null}
            <button type="button" className="sl-btn" onClick={() => void rescrapeSelectedItems()}>{t("action.rescrape")}</button>
          </div>
        </div>

        <div className="sl-page-cols">
          <section>
            <div className="sl-page-bar">
              <b>{t("detail.seasons")}</b>
              {ordered.length > 0 ? (
                <div className="sl-seg" role="radiogroup" aria-label={t("detail.seasons")} style={{ maxWidth: "100%", overflowX: "auto" }}>
                  {ordered.map((season) => {
                    const eps = seasonEpisodes(season.id);
                    const have = eps.filter((e) => Boolean(e.filePath)).length;
                    return (
                      <button key={season.id} type="button" role="radio" aria-checked={season.id === active?.id} onClick={() => setActiveId(season.id)}>
                        {t("detail.seasonLabel", { n: season.seasonNumber })}
                        <em className={have < eps.length ? "warn" : undefined}>{have}/{eps.length}</em>
                      </button>
                    );
                  })}
                </div>
              ) : null}
              {active ? (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <button type="button" className="sl-iconbtn" aria-label={t("action.more")} title={t("action.more")}><Ellipsis aria-hidden /></button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" sideOffset={6}>
                    <DropdownMenuItem onSelect={() => void revealSeason(item.folderPath, active.id, active.seasonNumber, episodes)}>{t("action.revealInFinder")}</DropdownMenuItem>
                    {scraped ? <DropdownMenuItem onSelect={() => void scrapeSeason(item.id, active.seasonNumber)}>{t("action.scrapeSeason")}</DropdownMenuItem> : null}
                  </DropdownMenuContent>
                </DropdownMenu>
              ) : null}
            </div>
            {ordered.length === 0 ? (
              <p className="kg-type-body-secondary text-fg-secondary">{t("detail.noSeasons")}</p>
            ) : (
              <div className="sl-eps" role="table" aria-label={t("detail.seasonLabel", { n: active?.seasonNumber ?? 1 })}>
                <div className="sl-ep head" role="row">
                  <span>{t("detail.col.episode")}</span><span>{t("detail.col.title")}</span><span>{t("detail.col.aired")}</span>
                  <span className="text-right">{t("detail.col.runtime")}</span><span>{t("detail.col.file")}</span>
                </div>
                {activeEps.length === 0 ? (
                  <p className="px-2 py-3 kg-type-body-secondary text-fg-secondary">{t("detail.noEpisodes")}</p>
                ) : activeEps.map((ep, i) => {
                  const missing = !ep.filePath;
                  return (
                    <div key={ep.id} className="sl-ep" role="row" data-odd={i % 2 === 1 || undefined} data-missing={missing || undefined}>
                      <span className="no">{String(ep.episodeNumber).padStart(2, "0")}</span>
                      <span className="et" title={ep.title ?? undefined}>{ep.title || t("detail.episodeFallback", { n: ep.episodeNumber })}</span>
                      <span className="l2 tabular-nums">{ep.airDate ?? "—"}</span>
                      <span className="l2 text-right tabular-nums">{ep.runtime ? t("detail.runtimeMinutes", { m: ep.runtime }) : "—"}</span>
                      <span className="fl" title={ep.filePath || undefined}>
                        {missing ? <CircleDashed aria-hidden /> : <Check aria-hidden />}
                        <span>{missing ? t("detail.epMissingFile") : ep.filePath.split(/[/\\]/).pop()}</span>
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </section>
          <aside className="sl-page-side">
            {cast.length > 0 ? (
              <div>
                <h4>{t("detail.cast")}</h4>
                <div className="sl-people">
                  {cast.map((actor, i) => (
                    <div key={`${actor.name}-${i}`} className="sl-person">
                      <ActorAvatar url={actor.thumbUrl} name={actor.name} size={30} />
                      <div>{actor.name}{actor.role ? <small>{actor.role}</small> : null}</div>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
            <div>
              <h4>{t("inspector.info")}</h4>
              <dl className="sl-info">
                <dt>{t("inspector.location")}</dt><dd className="mono">{item.folderPath}</dd>
                {formatSource(metadata?.sourceId) ? (<><dt>{t("inspector.source")}</dt><dd>{formatSource(metadata?.sourceId)}</dd></>) : null}
              </dl>
            </div>
          </aside>
        </div>
      </div>
    </div>
  );
}
