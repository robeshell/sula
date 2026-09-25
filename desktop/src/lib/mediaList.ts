/** Mirrors Swift `MediaSortOption` / `MediaStatusFilter` in MediaListViewModel.swift */

export type MediaSortOption =
  | "nameAscending"
  | "nameDescending"
  | "yearDescending"
  | "yearAscending"
  | "addedAtDescending"
  | "addedAtAscending"
  | "unscrapedFirst";

export type MediaStatusFilter =
  | "all"
  | "unscraped"
  | "scraped"
  | "partial"
  | "unmatched";

export const SORT_OPTIONS: { value: MediaSortOption; label: string }[] = [
  { value: "nameAscending", label: "Name (A-Z)" },
  { value: "nameDescending", label: "Name (Z-A)" },
  { value: "yearDescending", label: "Year (Newest First)" },
  { value: "yearAscending", label: "Year (Oldest First)" },
  { value: "addedAtDescending", label: "Recently Added" },
  { value: "addedAtAscending", label: "Earliest Added" },
  { value: "unscrapedFirst", label: "Unscraped First" },
];

export const STATUS_FILTERS: { value: MediaStatusFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "unscraped", label: "Unscraped" },
  { value: "scraped", label: "Scraped" },
  { value: "partial", label: "Incomplete" },
  { value: "unmatched", label: "Unmatched" },
];

export type SortableMedia = {
  id: string;
  title: string;
  originalTitle?: string | null;
  year?: number | null;
  status: string;
  addedAt: string;
};

function statusRank(status: string): number {
  switch (status) {
    case "unscraped":
      return 0;
    case "partial":
      return 1;
    case "unmatched":
      return 2;
    case "scraped":
      return 3;
    default:
      return 99;
  }
}

/** Approximate Swift folding + numeric compare for titles. */
function titleKey(title: string): string {
  return title.normalize("NFKD").replace(/\p{M}/gu, "").toLocaleLowerCase();
}

/** Same ordering as `a.localeCompare(b, undefined, { numeric: true })`, built once. */
const titleCollator = new Intl.Collator(undefined, { numeric: true });

/** Folded title per item object; items are immutable snapshots, so this is stable. */
const titleKeyCache = new WeakMap<object, string>();
function cachedTitleKey(item: SortableMedia): string {
  let key = titleKeyCache.get(item);
  if (key === undefined) {
    key = titleKey(item.title);
    titleKeyCache.set(item, key);
  }
  return key;
}

type Decorated<T> = { item: T; key: string };

function compareDecorated<T extends SortableMedia>(
  sortOption: MediaSortOption,
): (lhs: Decorated<T>, rhs: Decorated<T>) => number {
  const byTitle = (lhs: Decorated<T>, rhs: Decorated<T>) =>
    titleCollator.compare(lhs.key, rhs.key);
  switch (sortOption) {
    case "nameAscending":
      return byTitle;
    case "nameDescending":
      return (lhs, rhs) => titleCollator.compare(rhs.key, lhs.key);
    case "yearDescending":
      return (lhs, rhs) => {
        const ly = lhs.item.year ?? Number.MIN_SAFE_INTEGER;
        const ry = rhs.item.year ?? Number.MIN_SAFE_INTEGER;
        if (ly === ry) return byTitle(lhs, rhs);
        return ry - ly;
      };
    case "yearAscending":
      return (lhs, rhs) => {
        const ly = lhs.item.year ?? Number.MAX_SAFE_INTEGER;
        const ry = rhs.item.year ?? Number.MAX_SAFE_INTEGER;
        if (ly === ry) return byTitle(lhs, rhs);
        return ly - ry;
      };
    case "addedAtDescending":
      return (lhs, rhs) => {
        if (lhs.item.addedAt === rhs.item.addedAt) return byTitle(lhs, rhs);
        return lhs.item.addedAt < rhs.item.addedAt ? 1 : -1;
      };
    case "addedAtAscending":
      return (lhs, rhs) => {
        if (lhs.item.addedAt === rhs.item.addedAt) return byTitle(lhs, rhs);
        return lhs.item.addedAt > rhs.item.addedAt ? 1 : -1;
      };
    case "unscrapedFirst":
      return (lhs, rhs) => {
        const lr = statusRank(lhs.item.status);
        const rr = statusRank(rhs.item.status);
        if (lr !== rr) return lr - rr;
        // Same status: newest first so refresh additions surface at the top.
        if (lhs.item.addedAt !== rhs.item.addedAt) {
          return lhs.item.addedAt < rhs.item.addedAt ? 1 : -1;
        }
        return byTitle(lhs, rhs);
      };
  }
}

export function filterAndSortMedia<T extends SortableMedia>(
  items: T[],
  query: string,
  statusFilter: MediaStatusFilter,
  sortOption: MediaSortOption,
): T[] {
  const lowered = query.trim().toLowerCase();
  let next = items;
  if (lowered) {
    next = next.filter(
      (item) =>
        item.title.toLowerCase().includes(lowered) ||
        (item.originalTitle?.toLowerCase().includes(lowered) ?? false),
    );
  }
  if (statusFilter !== "all") {
    next = next.filter((item) => item.status === statusFilter);
  }

  // Decorate-sort-undecorate: fold each title once, not once per comparison.
  const decorated = next.map((item) => ({ item, key: cachedTitleKey(item) }));
  decorated.sort(compareDecorated<T>(sortOption));
  return decorated.map((entry) => entry.item);
}
