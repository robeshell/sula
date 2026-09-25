/**
 * Browser-only mock of the Tauri backend for visual checks (`/mock.html`).
 * Never imported by the real entry point; the data is a fictional Chinese media
 * library and the posters are generated placeholders, not real artwork.
 */
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

type MediaType = "movie" | "tvShow" | "anime";
type Status = "scraped" | "unscraped" | "unmatched" | "partial";

type Seed = { title: string; original?: string; year: number; status: Status; rating?: number; genres: string[]; overview?: string };

const MOVIES: Seed[] = [
  { title: "沙丘", original: "Dune", year: 2021, status: "scraped", rating: 7.8, genres: ["科幻", "冒险"], overview: "天赋异禀的少年保罗·厄崔迪必须前往宇宙中最危险的星球厄拉科斯，确保家族和人民的未来。" },
  { title: "沙丘：第二部", original: "Dune: Part Two", year: 2024, status: "scraped", rating: 8.2, genres: ["科幻", "冒险"] },
  { title: "奥本海默", original: "Oppenheimer", year: 2023, status: "scraped", rating: 8.1, genres: ["剧情", "历史"] },
  { title: "银翼杀手 2049", original: "Blade Runner 2049", year: 2017, status: "scraped", rating: 7.5, genres: ["科幻", "剧情"] },
  { title: "花样年华", original: "In the Mood for Love", year: 2000, status: "scraped", rating: 8.1, genres: ["剧情", "爱情"] },
  { title: "千与千寻", original: "千と千尋の神隠し", year: 2001, status: "scraped", rating: 8.5, genres: ["动画", "奇幻"] },
  { title: "阿波罗 13 号", original: "Apollo 13", year: 1995, status: "scraped", rating: 7.6, genres: ["剧情", "历史"] },
  { title: "寄生虫", original: "기생충", year: 2019, status: "scraped", rating: 8.5, genres: ["剧情", "惊悚"] },
  { title: "瞬息全宇宙", original: "Everything Everywhere All at Once", year: 2022, status: "scraped", rating: 7.8, genres: ["科幻", "喜剧"] },
  { title: "海上钢琴师", original: "La leggenda del pianista sull'oceano", year: 1998, status: "scraped", rating: 8.5, genres: ["剧情", "音乐"] },
  { title: "让子弹飞", year: 2010, status: "scraped", rating: 8.0, genres: ["剧情", "喜剧"] },
  { title: "流浪地球 2", year: 2023, status: "scraped", rating: 7.9, genres: ["科幻", "灾难"] },
  { title: "卧虎藏龙", original: "Crouching Tiger, Hidden Dragon", year: 2000, status: "scraped", rating: 7.9, genres: ["动作", "武侠"] },
  { title: "星际穿越", original: "Interstellar", year: 2014, status: "scraped", rating: 8.4, genres: ["科幻", "剧情"] },
  { title: "霸王别姬", year: 1993, status: "scraped", rating: 8.3, genres: ["剧情"] },
  { title: "Perfect.Days.2023.1080p.WEB-DL", year: 2023, status: "unscraped", genres: [] },
  { title: "The.Zone.of.Interest.2023", year: 2023, status: "unmatched", genres: [] },
  { title: "坠落的审判", original: "Anatomie d'une chute", year: 2023, status: "partial", rating: 7.7, genres: ["剧情", "悬疑"] },
  { title: "过往人生", original: "Past Lives", year: 2023, status: "scraped", rating: 7.8, genres: ["剧情", "爱情"] },
  { title: "Aftersun", year: 2022, status: "unscraped", genres: [] },
  { title: "一一", year: 2000, status: "scraped", rating: 8.1, genres: ["剧情"] },
  { title: "燃烧", original: "버닝", year: 2018, status: "scraped", rating: 7.5, genres: ["剧情", "悬疑"] },
  { title: "小丑", original: "Joker", year: 2019, status: "scraped", rating: 8.2, genres: ["剧情", "犯罪"] },
  { title: "盗梦空间", original: "Inception", year: 2010, status: "scraped", rating: 8.4, genres: ["科幻", "动作"] },
];

const SHOWS: Seed[] = [
  { title: "安多", original: "Andor", year: 2022, status: "scraped", rating: 8.4, genres: ["科幻", "剧情"], overview: "在义军同盟成立前的黑暗年代，卡西安·安多走上了一条将他变成反抗者的道路。" },
  { title: "孤独的美食家", year: 2012, status: "scraped", rating: 8.0, genres: ["剧情", "美食"] },
  { title: "漫长的季节", year: 2023, status: "scraped", rating: 9.0, genres: ["剧情", "悬疑"] },
  { title: "绝命毒师", original: "Breaking Bad", year: 2008, status: "scraped", rating: 9.5, genres: ["剧情", "犯罪"] },
  { title: "Severance.S02.2160p", year: 2025, status: "unscraped", genres: [] },
  { title: "繁花", year: 2023, status: "partial", rating: 8.3, genres: ["剧情"] },
  { title: "黑镜", original: "Black Mirror", year: 2011, status: "scraped", rating: 8.7, genres: ["科幻", "惊悚"] },
];

const ANIME: Seed[] = [
  { title: "葬送的芙莉莲", original: "葬送のフリーレン", year: 2023, status: "scraped", rating: 9.1, genres: ["奇幻", "冒险"] },
  { title: "间谍过家家", original: "SPY×FAMILY", year: 2022, status: "scraped", rating: 8.5, genres: ["喜剧", "动作"] },
  { title: "[SubsPlease] Dandadan", year: 2024, status: "unmatched", genres: [] },
  { title: "孤独摇滚！", original: "ぼっち・ざ・ろっく！", year: 2022, status: "scraped", rating: 8.8, genres: ["音乐", "喜剧"] },
  { title: "进击的巨人", original: "進撃の巨人", year: 2013, status: "scraped", rating: 9.0, genres: ["动作", "奇幻"] },
];

const LIBRARIES = [
  { id: "lib-movies", name: "电影", rootPath: "/Volumes/Media/电影", mediaType: "movie" as MediaType, addedAt: "2026-03-02T10:00:00Z" },
  { id: "lib-shows", name: "剧集", rootPath: "/Volumes/Media/剧集", mediaType: "tvShow" as MediaType, addedAt: "2026-03-02T10:01:00Z" },
  { id: "lib-anime", name: "动漫", rootPath: "/Volumes/Media/动漫", mediaType: "anime" as MediaType, addedAt: "2026-03-02T10:02:00Z" },
];

function hash(text: string): number {
  let h = 2166136261;
  for (const ch of text) h = Math.imul(h ^ ch.codePointAt(0)!, 16777619);
  return h >>> 0;
}

/** Abstract placeholder "poster": a hue field plus the title, clearly not real art. */
function posterSvg(title: string): string {
  const h = hash(title);
  const hue = h % 360;
  const hue2 = (hue + 40 + (h >> 9) % 80) % 360;
  const esc = title.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="300" height="450" viewBox="0 0 300 450">
<defs><linearGradient id="g" x1="0" y1="0" x2="0.6" y2="1"><stop offset="0" stop-color="hsl(${hue} 45% 32%)"/><stop offset="1" stop-color="hsl(${hue2} 55% 14%)"/></linearGradient></defs>
<rect width="300" height="450" fill="url(#g)"/>
<circle cx="${60 + (h % 180)}" cy="${120 + (h >> 5) % 140}" r="${60 + (h >> 11) % 70}" fill="hsl(${hue2} 70% 60% / 0.18)"/>
<text x="24" y="400" fill="rgba(255,255,255,0.92)" font-family="system-ui, PingFang SC, sans-serif" font-size="26" font-weight="600">${esc.slice(0, 12)}</text>
</svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

type Item = {
  id: string; mediaType: MediaType; title: string; originalTitle: string | null; year: number;
  folderPath: string; filePath: string; status: Status; libraryId: string; addedAt: string;
};

const items: Item[] = [];
const seeds = new Map<string, Seed>();
for (const [lib, list] of [[LIBRARIES[0], MOVIES], [LIBRARIES[1], SHOWS], [LIBRARIES[2], ANIME]] as const) {
  list.forEach((seed, index) => {
    const id = `${lib.id}-${index}`;
    const folder = `${lib.rootPath}/${seed.title} (${seed.year})`;
    items.push({
      id, mediaType: lib.mediaType, title: seed.title, originalTitle: seed.original ?? null, year: seed.year,
      folderPath: folder, filePath: lib.mediaType === "movie" ? `${folder}/${seed.title}.mkv` : folder,
      status: seed.status, libraryId: lib.id, addedAt: new Date(Date.UTC(2026, 7, 1 + index)).toISOString(),
    });
    seeds.set(id, seed);
  });
}

function summary(item: Item) {
  const seed = seeds.get(item.id)!;
  const hasMeta = item.status !== "unscraped" && item.status !== "unmatched";
  return hasMeta ? {
    mediaItemId: item.id, posterPath: "poster.jpg", fanartPath: "fanart.jpg", overview: seed.overview ?? null,
    rating: seed.rating ?? null, genres: seed.genres, scrapedAt: "2026-09-20T08:00:00Z",
  } : null;
}

function showStats(item: Item) {
  if (item.mediaType === "movie") return null;
  const seasons = item.title.includes("孤独的美食家") ? 10 : item.status === "unscraped" ? 1 : 2;
  return { mediaItemId: item.id, seasonCount: seasons, episodeCount: seasons * 10, localEpisodeCount: seasons * 10 - (item.status === "partial" ? 4 : 0) };
}

function detail(id: string) {
  const item = items.find((i) => i.id === id)!;
  const seed = seeds.get(id)!;
  const meta = summary(item);
  const seasons = item.mediaType === "movie" ? [] : [1, 2].map((n) => ({
    id: `${id}-s${n}`, mediaItemId: id, seasonNumber: n, title: `第 ${n} 季`, overview: null,
    posterPath: null, airDate: `${item.year + n - 1}-09-21`, episodeCount: 10,
  }));
  const episodes = seasons.flatMap((season) => Array.from({ length: 10 }, (_, e) => ({
    id: `${season.id}-e${e + 1}`, seasonId: season.id, episodeNumber: e + 1,
    title: item.status === "unscraped" ? null : `第 ${e + 1} 集`, overview: null,
    airDate: `${item.year + season.seasonNumber - 1}-10-${String(e + 1).padStart(2, "0")}`,
    stillPath: null, filePath: `${item.folderPath}/Season 0${season.seasonNumber}/S0${season.seasonNumber}E${String(e + 1).padStart(2, "0")}.mkv`,
    runtime: 45, rating: null, director: null,
  })));
  return {
    item,
    metadata: meta && {
      ...meta, tagline: null, ratingVotes: 12840, contentRating: "PG-13", director: item.mediaType === "movie" ? "丹尼斯·维伦纽瓦" : null,
      runtime: item.mediaType === "movie" ? 155 : 45, showStatus: item.mediaType === "movie" ? null : "Returning Series",
      sourceId: `tmdb:${hash(item.title) % 900000}`,
      credits: ["张三", "李四", "王五", "赵六", "孙七"].map((name, order) => ({ name, role: `角色 ${order + 1}`, type: "Actor", thumbUrl: null, order })),
      overview: seed.overview ?? "简介暂缺。",
    },
    seasons,
    episodes,
  };
}

const config = {
  configNotice: null, scrapeConcurrency: 4, metadataLanguage: "zh-CN", nfoFormat: "kodi", scanExcludedFolders: ["@eaDir", "Extras"],
  renameAutoAfterScrape: true, renameCreateSeasonFolders: true,
  renameMovieFolderTemplate: "{title} ({year})", renameMovieFileTemplate: "{title} ({year})",
  renameTvShowFolderTemplate: "{title} ({year})", renameSeasonFolderTemplate: "Season {season:2}",
  renameEpisodeFileTemplate: "{title} - S{season:2}E{episode:2} - {episodeTitle}",
  appearance: "system", accent: "indigo", trayEnabled: true, keepRunningOnClose: true, uiLocale: "zh-Hans",
  apiKeys: { tmdb: "mock-token", tvdb: "", omdb: "", bangumi: "" },
};

const now = () => new Date().toISOString();
const tasks = [
  { id: "task-1", title: "刮削 电影", kind: "batchScrape", status: "running", progress: { completed: 7, total: 24, current: "奥本海默", stageKey: "scrape" }, errorMessage: null, result: null, targetId: "lib-movies", createdAt: now(), updatedAt: now() },
  { id: "task-0", title: "刷新 剧集", kind: "refresh", status: "completed", progress: { completed: 7, total: 7, current: "", stageKey: null }, errorMessage: null, result: null, targetId: "lib-shows", createdAt: now(), updatedAt: now() },
];

function page(libraryId: string, offset = 0, limit = 256) {
  const list = items.filter((i) => i.libraryId === libraryId);
  const slice = list.slice(offset, offset + limit);
  return {
    items: slice,
    metadata: slice.map(summary).filter(Boolean),
    showStats: slice.map(showStats).filter(Boolean),
    nextOffset: offset + limit < list.length ? offset + limit : null,
  };
}

export function installMockBackend() {
  // mock.html?window=settings|renamer previews the secondary windows.
  const label = new URLSearchParams(location.search).get("window") ?? "main";
  mockWindows(label, ...["main", "settings", "renamer"].filter((l) => l !== label));
  const accent = new URLSearchParams(location.search).get("accent");
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    switch (cmd) {
      case "get_config": return config;
      case "save_config": Object.assign(config, args.config); return config;
      case "app_status": return { appName: "Sula", version: "1.0.1", dataDir: "~/Library/Application Support/sula", databasePath: "~/Library/Application Support/sula/sula.sqlite3", libraryCount: LIBRARIES.length, config, crates: { mediaCore: "1.0.1", scraperKit: "1.0.1", renamer: "1.0.1" } };
      case "list_libraries": return LIBRARIES;
      case "list_media_page": return page(args.libraryId, args.offset, args.limit);
      case "list_media_items": return items.filter((i) => i.libraryId === args.libraryId);
      case "get_media_detail": return detail(args.id);
      case "resolve_poster_thumbnail": {
        const item = items.find((i) => i.folderPath === args.folderPath);
        return item && summary(item) ? posterSvg(item.title) : null;
      }
      case "resolve_actor_avatar": return null;
      case "get_system_accent": return accent ? `#${accent}` : null;
      case "open_settings_window": window.open("/mock.html?window=settings", "_blank", "width=700,height=610"); return null;
      case "list_tasks": return tasks;
      case "list_logs": return [
        { id: 1, level: "INFO", target: "sula", message: "database opened", timestamp: now() },
        { id: 2, level: "WARN", target: "scraper", message: "TMDB 请求被限流，2 秒后重试", timestamp: now() },
      ];
      case "search_match_candidates": return [
        { source: "tmdb", sourceId: "tmdb:1", title: "完美的日子", originalTitle: "Perfect Days", year: 2023, overview: "东京公厕清洁工平山的日常。", posterUrl: null, confidence: 0.92, mediaType: "movie" },
        { source: "tmdb", sourceId: "tmdb:2", title: "完美的日子", originalTitle: "Perfect Days", year: 2024, overview: null, posterUrl: null, confidence: 0.61, mediaType: "movie" },
      ];
      case "plan_show_merges": return [];
      case "scan_media_residuals": return (args.itemIds ?? []).slice(0, 1).flatMap((id: string) => {
        const item = items.find((i) => i.id === id);
        if (!item) return [];
        return [
          { path: `${item.folderPath}/${item.title}.2019.1080p.nfo`, itemId: id, itemTitle: item.title, reason: "orphanNfo", size: 4213 },
          { path: `${item.folderPath}/poster (1).jpg`, itemId: id, itemTitle: item.title, reason: "duplicateArtwork", size: 845_120 },
        ];
      });
      case "renamer_snapshot_count": return 0;
      case "renamer_list_presets": return [];
      case "renamer_auto_load_pipeline": return null;
      case "plugin:dialog|open": return ["/Volumes/Media/动漫/葬送的芙莉莲"];
      case "renamer_collect_files": return Array.from({ length: 14 }, (_, i) => {
        const n = String(i + 1).padStart(2, "0");
        const name = i >= 12 ? `葬送的芙莉莲 - S01E${n}.mkv` : `[SubsPlease] Sousou no Frieren - ${n}${i === 4 ? "v2" : ""} (1080p) [A1B2].mkv`;
        return { id: `f${i}`, originalName: name, path: `/Volumes/Media/动漫/葬送的芙莉莲/${name}` };
      });
      case "renamer_preview": return (args.files ?? []).map((f: any, i: number) => {
        const newName = f.originalName.replace(/^\[[^\]]*\]\s*/, "").replace(/ \[[^\]]*\]/, "").replace("Sousou no Frieren", "葬送的芙莉莲");
        return { id: f.id, originalName: f.originalName, newName, path: f.path, hasConflict: i === 4, hasInvalidChars: false };
      });
      case "list_directory": return [
        { name: "Season 01", path: `${args.path}/Season 01`, isDirectory: true, modifiedAt: now().slice(0, 10) },
        { name: "extrafanart", path: `${args.path}/extrafanart`, isDirectory: true, modifiedAt: now().slice(0, 10) },
        { name: "poster.jpg", path: `${args.path}/poster.jpg`, isDirectory: false, fileSize: 845_120, modifiedAt: now().slice(0, 10) },
        { name: "tvshow.nfo", path: `${args.path}/tvshow.nfo`, isDirectory: false, fileSize: 6_210, modifiedAt: now().slice(0, 10) },
      ];
      case "path_is_dir": return true;
      case "refresh_library": case "scrape_library": case "scrape_items": case "rescrape_items":
      case "refresh_media_items": case "apply_rename_templates": case "organize_season_folders": case "scrape_season": {
        const task = { id: `task-${Date.now()}`, title: "模拟任务", kind: "scrape", status: "pending", progress: null, errorMessage: null, result: null, targetId: null, createdAt: now(), updatedAt: now() };
        setTimeout(() => void emit("task-updated", { ...task, status: "completed" }), 800);
        return task;
      }
      default:
        if (cmd.startsWith("plugin:")) return null;
        console.info("[mock] unhandled command", cmd, args);
        return null;
    }
  }, { shouldMockEvents: true });
  // Posters are data URIs here; pass them through instead of the asset protocol.
  (window as any).__TAURI_INTERNALS__.convertFileSrc = (path: string) => path;
}
