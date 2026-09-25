import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Search, LayoutGrid, List, X, ArrowUpDown, RefreshCw, ArrowDownToLine, Ellipsis, Check } from "lucide-react";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "../ui/dropdown-menu";
import { SORT_OPTIONS, needsAttention, type MediaSortOption, type MediaStatusFilter } from "../../lib/mediaList";

const SEARCH_DEBOUNCE_MS = 150;

export type ToolbarMenuItem = { label: string; onSelect: () => void; destructive?: boolean; separatorBefore?: boolean };

/** Unified title bar for a library: title, filter, view, sort, search and actions. */
export function LibraryToolbar({ title, statuses, status, onStatus, view, onView, sort, onSort, query, onQuery, scanning, onRefresh, onScrapeAll, menu }: {
  title: string;
  /** Status of every item in the library, for counts. */
  statuses: string[];
  status: MediaStatusFilter;
  onStatus: (v: MediaStatusFilter) => void;
  view: "poster" | "list";
  onView: (v: "poster" | "list") => void;
  sort: MediaSortOption;
  onSort: (v: MediaSortOption) => void;
  query: string;
  onQuery: (v: string) => void;
  scanning: boolean;
  onRefresh: () => void;
  onScrapeAll: () => void;
  menu: ToolbarMenuItem[];
}) {
  const { t } = useTranslation();
  const attention = useMemo(() => statuses.filter(needsAttention).length, [statuses]);
  const segments: { value: MediaStatusFilter; label: string }[] = [
    { value: "all", label: t("filter.status.all") },
    { value: "attention", label: t("filter.status.attention") },
    { value: "scraped", label: t("filter.status.scraped") },
  ];
  // Anything other than the three segments (set by an older version) reads as "all".
  const current = segments.some((s) => s.value === status) ? status : "all";

  // Filtering a large library per keystroke is costly: publish after a short pause.
  const [draft, setDraft] = useState(query);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const published = useRef(query);
  const onQueryRef = useRef(onQuery);
  onQueryRef.current = onQuery;
  const cancelPending = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  useEffect(() => {
    if (query === published.current) return;
    published.current = query;
    cancelPending();
    setDraft(query);
  }, [query]);
  useEffect(() => cancelPending, []);
  const changeDraft = (value: string) => {
    setDraft(value);
    cancelPending();
    timer.current = setTimeout(() => {
      timer.current = null;
      published.current = value;
      onQueryRef.current(value);
    }, SEARCH_DEBOUNCE_MS);
  };
  const clearSearch = () => {
    cancelPending();
    setDraft("");
    published.current = "";
    onQuery("");
  };

  return (
    <header className="sl-toolbar">
      <div data-tauri-drag-region />
      <div className="sl-title">
        <b>{title}</b>
        <span>
          {t("list.itemCount", { count: statuses.length })}
          {attention > 0 ? <> · <span className="warn">{t("list.attentionCount", { count: attention })}</span></> : null}
        </span>
      </div>

      <div className="sl-seg" role="radiogroup" aria-label={t("filter.status.label")}>
        {segments.map((seg) => (
          <button key={seg.value} type="button" role="radio" aria-checked={current === seg.value} onClick={() => onStatus(seg.value)}>
            {seg.value === "attention" && attention > 0 ? <span className="dot" aria-hidden /> : null}
            {seg.label}
            {seg.value === "attention" && attention > 0 ? <em>{attention}</em> : null}
          </button>
        ))}
      </div>

      <div className="sl-seg icons" role="radiogroup" aria-label={t("list.viewMode")}>
        {(["poster", "list"] as const).map((mode) => (
          <button key={mode} type="button" role="radio" aria-checked={view === mode}
            aria-label={t(mode === "poster" ? "list.viewPoster" : "list.viewList")} title={t(mode === "poster" ? "list.viewPoster" : "list.viewList")}
            onClick={() => onView(mode)}>
            {mode === "poster" ? <LayoutGrid aria-hidden /> : <List aria-hidden />}
          </button>
        ))}
      </div>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button type="button" className="sl-iconbtn" aria-label={t("list.sort")} title={t("list.sort")}>
            <ArrowUpDown aria-hidden />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" sideOffset={6}>
          {SORT_OPTIONS.map((option) => (
            <DropdownMenuItem key={option.value} onSelect={() => onSort(option.value)}>
              <Check aria-hidden className={sort === option.value ? "" : "invisible"} />
              {t(`filter.sort.${option.value}`)}
            </DropdownMenuItem>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>

      <label className="sl-search">
        <Search aria-hidden />
        <input aria-label={t("list.searchPlaceholder")} placeholder={t("list.searchPlaceholder")} value={draft}
          onChange={(e) => changeDraft(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Escape" && draft) { e.preventDefault(); clearSearch(); } }} />
        {draft ? (
          <button type="button" className="clear" aria-label={t("list.clearSearch")} onClick={clearSearch}><X aria-hidden /></button>
        ) : null}
      </label>

      <span className="sl-vsep" aria-hidden />
      <button type="button" className="sl-iconbtn" disabled={scanning} onClick={onRefresh}
        aria-label={scanning ? t("action.refreshing") : t("action.refresh")} title={scanning ? t("action.refreshing") : t("action.refresh")}>
        <RefreshCw aria-hidden className={scanning ? "animate-spin" : undefined} />
      </button>
      {menu.length > 0 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button type="button" className="sl-iconbtn" aria-label={t("action.more")} title={t("action.more")}><Ellipsis aria-hidden /></button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" sideOffset={6}>
            {menu.map((item) => (
              <span key={item.label}>
                {item.separatorBefore ? <DropdownMenuSeparator /> : null}
                <DropdownMenuItem variant={item.destructive ? "destructive" : "default"} onSelect={item.onSelect}>{item.label}</DropdownMenuItem>
              </span>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
      <button type="button" className="sl-prim" disabled={scanning} onClick={onScrapeAll}>
        <ArrowDownToLine aria-hidden />
        {t("action.scrapeAll")}
      </button>
    </header>
  );
}
