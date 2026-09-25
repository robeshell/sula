use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFileName {
    pub title: String,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub sub_group: Option<String>,
}

pub struct FileNameParser;

impl FileNameParser {
    const MEDIA_EXTENSIONS: &[&str] = &[
        "mkv", "mp4", "avi", "m4v", "mov", "wmv", "flv", "ts", "m2ts", "iso",
    ];

    const NOISE_TOKENS: &[&str] = &[
        "1080p", "720p", "4K", "2160p", "480p", "BluRay", "BDRip", "WEB-DL", "WEBDL", "WEBRip",
        "HDRip", "HDTV", "x264", "x265", "H264", "H265", "HEVC", "AVC", "EAC3", "TrueHD", "DTS-HD",
        "DTS-MA", "DTS", "AAC", "AC3", "FLAC", "10bit", "SDR", "HDR", "HDR10", "DV", "DoVi", "Atmos",
        "REMUX", "PROPER", "HD-MA", "HD2160P", "HD1080P", "HD720P", "HD480P", "CHS-ENG", "ENG-CHS",
        "CHS", "CHT", "ENG", "JPN", "KOR", "全集",
    ];

    /// Episode file names: a bare trailing number (`Show 05.mkv`) is read as the episode.
    pub fn parse(filename: &str) -> ParsedFileName {
        Self::parse_with(filename, true)
    }

    /// Movie files and show/anime folder names: a bare trailing number is part of the
    /// title (`Apollo 13`, `District 9`, `The 100`), never an episode.
    pub fn parse_title(name: &str) -> ParsedFileName {
        Self::parse_with(name, false)
    }

    fn parse_with(filename: &str, trailing_episode: bool) -> ParsedFileName {
        let name = Self::drop_extension(filename);
        if name.starts_with('[') {
            Self::parse_anime(&name, trailing_episode)
        } else {
            Self::parse_standard(&name, trailing_episode)
        }
    }

    /// Re-clean a stored/display title for matching (dots already spaces OK).
    pub fn clean_title_for_match(raw: &str) -> String {
        Self::parse_title(&format!("{raw}.mkv")).title
    }

    pub fn extract_season_suffix(dir_name: &str) -> Option<(String, i32)> {
        static RE_SEASON_WORD: OnceLock<Regex> = OnceLock::new();
        static RE_S_NUM: OnceLock<Regex> = OnceLock::new();
        static RE_CN_ARABIC: OnceLock<Regex> = OnceLock::new();
        static RE_CN_HAN: OnceLock<Regex> = OnceLock::new();

        let re_season_word =
            RE_SEASON_WORD.get_or_init(|| Regex::new(r"(?i)^(.+)\s+Season\s*(\d{1,2})\s*$").unwrap());
        if let Some(caps) = re_season_word.captures(dir_name) {
            let base = caps.get(1)?.as_str().trim();
            let season: i32 = caps.get(2)?.as_str().parse().ok()?;
            if !base.is_empty() {
                return Some((base.to_string(), season));
            }
        }

        let re_s_num = RE_S_NUM.get_or_init(|| Regex::new(r"(?i)^(.+)\s+S(\d{1,2})\s*$").unwrap());
        if let Some(caps) = re_s_num.captures(dir_name) {
            let base = caps.get(1)?.as_str().trim();
            let season: i32 = caps.get(2)?.as_str().parse().ok()?;
            if !base.is_empty() {
                return Some((base.to_string(), season));
            }
        }

        let re_cn_arabic =
            RE_CN_ARABIC.get_or_init(|| Regex::new(r"^(.+?)\s*第\s*(\d{1,2})\s*季\s*$").unwrap());
        if let Some(caps) = re_cn_arabic.captures(dir_name) {
            let base = caps.get(1)?.as_str().trim();
            let season: i32 = caps.get(2)?.as_str().parse().ok()?;
            if !base.is_empty() {
                return Some((base.to_string(), season));
            }
        }

        let re_cn_han = RE_CN_HAN
            .get_or_init(|| Regex::new(r"^(.+?)\s*第\s*([一二三四五六七八九十]+)\s*季\s*$").unwrap());
        if let Some(caps) = re_cn_han.captures(dir_name) {
            let base = caps.get(1)?.as_str().trim();
            let season = chinese_numeral(caps.get(2)?.as_str())?;
            if !base.is_empty() {
                return Some((base.to_string(), season));
            }
        }

        None
    }

    fn parse_anime(name: &str, trailing_episode: bool) -> ParsedFileName {
        static RE_GROUP: OnceLock<Regex> = OnceLock::new();
        static RE_DASH_EP: OnceLock<Regex> = OnceLock::new();
        static RE_CN_EP: OnceLock<Regex> = OnceLock::new();
        static RE_TRAIL_EP: OnceLock<Regex> = OnceLock::new();
        static RE_TRAIL_TAG: OnceLock<Regex> = OnceLock::new();
        static RE_TRAIL_ROUND: OnceLock<Regex> = OnceLock::new();

        let mut remaining = name.to_string();
        let mut sub_group = None;

        let re_group = RE_GROUP.get_or_init(|| Regex::new(r"^\[([^\]]+)\]").unwrap());
        if let Some(caps) = re_group.captures(&remaining) {
            sub_group = Some(caps.get(1).unwrap().as_str().to_string());
            remaining = remaining[caps.get(0).unwrap().end()..].trim().to_string();
        }

        if remaining.starts_with('[') {
            let parts: Vec<String> = remaining
                .split("][")
                .map(|p| p.trim_matches(|c| c == '[' || c == ']' || c == ' ').to_string())
                .filter(|p| !p.is_empty())
                .collect();
            if parts.len() >= 2 {
                if let Ok(ep) = parts[1].parse::<i32>() {
                    return Self::anime_result(parts[0].clone(), None, Some(ep), sub_group);
                }
            }
        }

        // Trailing `[tags]`, `[CRC32]`, and round-bracket quality tags like `(1080p)` /
        // `(WEB 1080p HEVC)`; a trailing `(2019)` becomes the year.
        let re_trail_tag = RE_TRAIL_TAG.get_or_init(|| Regex::new(r"\s*[\[【][^\]】]*[\]】]$").unwrap());
        let re_trail_round = RE_TRAIL_ROUND.get_or_init(|| Regex::new(r"\s*[(（]([^()（）]*)[)）]$").unwrap());
        let mut year = None;
        loop {
            if let Some(m) = re_trail_tag.find(&remaining) {
                remaining.truncate(m.start());
                continue;
            }
            if let Some(caps) = re_trail_round.captures(&remaining) {
                let inner = caps.get(1).unwrap().as_str().trim();
                let is_year = inner.len() == 4 && (inner.starts_with("19") || inner.starts_with("20"))
                    && inner.bytes().all(|b| b.is_ascii_digit());
                if is_year && year.is_none() {
                    year = inner.parse().ok();
                } else if !is_release_tag(inner) {
                    break;
                }
                remaining.truncate(caps.get(0).unwrap().start());
                continue;
            }
            break;
        }
        remaining = remaining.trim().to_string();

        let re_dash_ep = RE_DASH_EP.get_or_init(|| Regex::new(r"(?i)\s[-–]\s(\d{1,3})(?:v\d{1,2})?$").unwrap());
        let re_cn_ep = RE_CN_EP.get_or_init(|| Regex::new(r"第(\d{1,3})[話话集]").unwrap());
        let re_trail_ep = RE_TRAIL_EP.get_or_init(|| Regex::new(r"\s(\d{1,3})$").unwrap());
        let mut matchers = vec![re_dash_ep, re_cn_ep];
        if trailing_episode {
            matchers.push(re_trail_ep);
        }
        for re in matchers {
            if let Some(caps) = re.captures(&remaining) {
                let ep: i32 = caps.get(1).unwrap().as_str().parse().unwrap();
                let title = remaining[..caps.get(0).unwrap().start()].trim().to_string();
                return Self::anime_result(title, year, Some(ep), sub_group);
            }
        }

        Self::anime_result(remaining, year, None, sub_group)
    }

    /// `Title S2` / `Title Season 2` / `Title 第2季` → season from the title suffix.
    fn anime_result(title: String, year: Option<i32>, episode: Option<i32>, sub_group: Option<String>) -> ParsedFileName {
        static RE_TITLE_YEAR: OnceLock<Regex> = OnceLock::new();
        let re_title_year =
            RE_TITLE_YEAR.get_or_init(|| Regex::new(r"\s*[(（]((?:19|20)\d{2})[)）]$").unwrap());
        let (title, year) = match re_title_year.captures(&title) {
            Some(caps) if caps.get(0).unwrap().start() > 0 => {
                (title[..caps.get(0).unwrap().start()].to_string(), year.or(caps[1].parse().ok()))
            }
            _ => (title, year),
        };
        let (title, season) = match Self::extract_season_suffix(&title) {
            Some((base, season)) => (base, Some(season)),
            None => (title, None),
        };
        ParsedFileName { title, year, season, episode, sub_group }
    }

    fn parse_standard(name: &str, trailing_episode: bool) -> ParsedFileName {
        static RE_SE: OnceLock<Regex> = OnceLock::new();
        static RE_NX: OnceLock<Regex> = OnceLock::new();
        static RE_TRAIL_EP: OnceLock<Regex> = OnceLock::new();
        static RE_BRACKETS: OnceLock<Regex> = OnceLock::new();

        let mut s = name.replace(['.', '_'], " ");
        let re_brackets = RE_BRACKETS.get_or_init(|| {
            Regex::new(r"(?i)[\[【][^\]】]*?(?:www|http|\.com|\.net|\.cn|btsj)[^\]】]*?[\]】]").unwrap()
        });
        s = re_brackets.replace_all(&s, " ").to_string();

        let mut season = None;
        let mut episode = None;
        // `S01E01`, plus multi-episode tails `S01E01E02` / `S01E01-E02` (first episode wins).
        let re_se = RE_SE.get_or_init(|| {
            Regex::new(r"(?i)S(\d{1,2})\s?E(\d{1,3})(?:\s?-\s?E\d{1,3}|E\d{1,3})*").unwrap()
        });
        // `1x05` / `01x05`; `x264` / `x265` codecs are not episodes.
        let re_nx = RE_NX.get_or_init(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{1,3})\b").unwrap());
        if let Some(caps) = re_se.captures(&s) {
            season = caps.get(1).and_then(|m| m.as_str().parse().ok());
            episode = caps.get(2).and_then(|m| m.as_str().parse().ok());
            s = re_se.replace(&s, " ").to_string();
        } else if let Some(caps) = re_nx
            .captures_iter(&s)
            .find(|c| !matches!(c.get(2).map(|m| m.as_str()), Some("264" | "265")))
        {
            season = caps.get(1).and_then(|m| m.as_str().parse().ok());
            episode = caps.get(2).and_then(|m| m.as_str().parse().ok());
            let range = caps.get(0).unwrap().range();
            s.replace_range(range, " ");
        }

        let mut year = None;
        if let Some((start, end, y)) = Self::pick_year(&s) {
            year = Some(y);
            s.replace_range(start..end, " ");
        }

        let mut tokens: Vec<&str> = Self::NOISE_TOKENS.to_vec();
        tokens.sort_by_key(|t| std::cmp::Reverse(t.len()));
        for token in tokens {
            let pattern = format!(r"(?i)\b{}\b", regex::escape(token));
            if let Ok(re) = Regex::new(&pattern) {
                s = re.replace_all(&s, " ").to_string();
            }
        }

        let mut title = s
            .split_whitespace()
            .filter(|t| !t.is_empty() && !is_residual_noise_token(t))
            .collect::<Vec<_>>()
            .join(" ");
        // `1917.mkv` / `2012 (2009)`: the number is all there is, so it is the title.
        if title.is_empty() {
            if let Some(y) = year.take() {
                title = y.to_string();
            }
        }

        if season.is_none() {
            if let Some((base, season_num)) = Self::extract_season_suffix(&title) {
                return ParsedFileName {
                    title: base,
                    year,
                    season: Some(season_num),
                    episode,
                    sub_group: None,
                };
            }
        }

        if trailing_episode && season.is_none() && episode.is_none() {
            let re_trail_ep = RE_TRAIL_EP.get_or_init(|| Regex::new(r"\s(\d{1,3})$").unwrap());
            if let Some(caps) = re_trail_ep.captures(&title) {
                let ep: i32 = caps.get(1).unwrap().as_str().parse().unwrap();
                let trimmed = title[..caps.get(0).unwrap().start()].to_string();
                return ParsedFileName {
                    title: trimmed,
                    year,
                    season,
                    episode: Some(ep),
                    sub_group: None,
                };
            }
        }

        ParsedFileName {
            title,
            year,
            season,
            episode,
            sub_group: None,
        }
    }

    /// Picks the release year and returns its byte range (brackets included) in `s`.
    ///
    /// A bracketed `(2017)` / `[2017]` wins; otherwise the last standalone year token, so
    /// title numbers (`Blade Runner 2049 2017`, `1917 2019`) stay in the title. A leading
    /// year-like number counts only when it is the sole candidate.
    fn pick_year(s: &str) -> Option<(usize, usize, i32)> {
        static RE_YEAR: OnceLock<Regex> = OnceLock::new();
        let re_year = RE_YEAR.get_or_init(|| Regex::new(r"(?:19|20)\d{2}").unwrap());
        let bytes = s.as_bytes();
        // (start, end, year, bracketed, leading)
        let candidates: Vec<(usize, usize, i32, bool, bool)> = re_year
            .find_iter(s)
            .filter(|m| {
                let before = m.start().checked_sub(1).map(|i| bytes[i]);
                !before.is_some_and(|b| b.is_ascii_alphanumeric())
                    && !bytes.get(m.end()).is_some_and(|b| b.is_ascii_alphanumeric())
            })
            .map(|m| {
                let open = m.start() > 0 && matches!(bytes[m.start() - 1], b'(' | b'[');
                let close = matches!(bytes.get(m.end()), Some(b')' | b']'));
                let start = if open { m.start() - 1 } else { m.start() };
                let end = if close { m.end() + 1 } else { m.end() };
                let leading = s[..start].trim().is_empty();
                (start, end, m.as_str().parse().unwrap(), open && close, leading)
            })
            .collect();
        let pick = candidates
            .iter()
            .rev()
            .find(|c| c.3)
            .or_else(|| candidates.iter().rev().find(|c| !c.4))
            .or(candidates.first())?;
        Some((pick.0, pick.1, pick.2))
    }

    fn drop_extension(filename: &str) -> String {
        let path = std::path::Path::new(filename);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if Self::MEDIA_EXTENSIONS.contains(&ext.as_str()) {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(filename)
                .to_string()
        } else {
            path.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(filename)
                .to_string()
        }
    }
}

fn is_residual_noise_token(token: &str) -> bool {
    let upper = token.to_ascii_uppercase();
    if upper == "HD" || token == "全集" {
        return true;
    }
    // HD2160P / 2160P / H264 / CHS-ENG leftover fragments
    if Regex::new(r"(?i)^(?:HD)?(?:480|720|1080|2160)P$")
        .ok()
        .is_some_and(|re| re.is_match(token))
    {
        return true;
    }
    if Regex::new(r"(?i)^H\.?26[45]$")
        .ok()
        .is_some_and(|re| re.is_match(token))
    {
        return true;
    }
    if upper.contains("世界网") || upper.contains("论坛") || upper.contains("BTSJ") {
        return true;
    }
    false
}

/// Round-bracket release tags in anime names: `1080p`, `WEB 1080p HEVC`, `BD x265 FLAC`, CRC32.
fn is_release_tag(inner: &str) -> bool {
    static RE_TAG: OnceLock<Regex> = OnceLock::new();
    static RE_CRC: OnceLock<Regex> = OnceLock::new();
    let re_tag = RE_TAG.get_or_init(|| {
        Regex::new(r"(?i)\b(?:\d{3,4}p|\d{3,4}x\d{3,4}|[248]k|x26[45]|h\.?26[45]|hevc|avc|av1|aac|flac|opus|e?ac-?3|dts|\d{1,2}-?bit|hdr|web(?:-?dl|rip)?|bd(?:rip)?|blu-?ray|dvd(?:rip)?|hdtv|remux|mkv|mp4|chs|cht|big5|gb|jpsc|jptc)\b").unwrap()
    });
    let re_crc = RE_CRC.get_or_init(|| Regex::new(r"^[0-9A-Fa-f]{8}$").unwrap());
    re_tag.is_match(inner) || re_crc.is_match(inner)
}

fn chinese_numeral(s: &str) -> Option<i32> {
    let map = |c: char| -> Option<i32> {
        match c {
            '一' => Some(1),
            '二' => Some(2),
            '三' => Some(3),
            '四' => Some(4),
            '五' => Some(5),
            '六' => Some(6),
            '七' => Some(7),
            '八' => Some(8),
            '九' => Some(9),
            '十' => Some(10),
            _ => None,
        }
    };
    if s == "十" {
        return Some(10);
    }
    if s.chars().count() == 1 {
        return map(s.chars().next()?);
    }
    if let Some(idx) = s.find('十') {
        let before = &s[..idx];
        let after = &s[idx + '十'.len_utf8()..];
        let tens = if before.is_empty() {
            1
        } else {
            map(before.chars().next()?)?
        };
        let ones = if after.is_empty() {
            0
        } else {
            map(after.chars().next()?)?
        };
        return Some(tens * 10 + ones);
    }
    map(s.chars().next()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_movie_with_year() {
        let p = FileNameParser::parse("Oppenheimer.2023.1080p.BluRay.mkv");
        assert_eq!(p.title, "Oppenheimer");
        assert_eq!(p.year, Some(2023));
    }

    #[test]
    fn parses_tv_se() {
        let p = FileNameParser::parse("Show.Name.S02E05.mkv");
        assert_eq!(p.season, Some(2));
        assert_eq!(p.episode, Some(5));
    }

    #[test]
    fn parses_anime_dash_episode() {
        let p = FileNameParser::parse("[SubGroup] Title - 28.mkv");
        assert_eq!(p.sub_group.as_deref(), Some("SubGroup"));
        assert_eq!(p.title, "Title");
        assert_eq!(p.episode, Some(28));
    }

    #[test]
    fn extracts_cn_season_suffix() {
        let (base, season) = FileNameParser::extract_season_suffix("IT狂人 第 1 季").unwrap();
        assert_eq!(base, "IT狂人");
        assert_eq!(season, 1);
    }

    #[test]
    fn strips_chinese_release_noise() {
        let p = FileNameParser::parse(
            "火遮眼.2026.HD2160P.AAC.H264.CHS-ENG.BT世界网.[www.btsj6.com].mp4",
        );
        assert_eq!(p.title, "火遮眼");
        assert_eq!(p.year, Some(2026));
        assert_eq!(
            FileNameParser::clean_title_for_match(
                "火遮眼 HD2160P H264 CHS-ENG BT世界网 [www btsj6 com]"
            ),
            "火遮眼"
        );
    }

    #[test]
    fn prefers_last_or_bracketed_year() {
        let p = FileNameParser::parse_title("Blade.Runner.2049.2017.1080p.mkv");
        assert_eq!((p.title.as_str(), p.year), ("Blade Runner 2049", Some(2017)));
        let p = FileNameParser::parse_title("2001.A.Space.Odyssey.1968.mkv");
        assert_eq!((p.title.as_str(), p.year), ("2001 A Space Odyssey", Some(1968)));
        let p = FileNameParser::parse_title("1917.2019.mkv");
        assert_eq!((p.title.as_str(), p.year), ("1917", Some(2019)));
        let p = FileNameParser::parse_title("Blade Runner 2049 (2017) 2160p.mkv");
        assert_eq!((p.title.as_str(), p.year), ("Blade Runner 2049", Some(2017)));
        let p = FileNameParser::parse_title("1917.mkv");
        assert_eq!((p.title.as_str(), p.year), ("1917", None));
        let p = FileNameParser::parse_title("Movie.2019x.2020.mkv");
        assert_eq!((p.title.as_str(), p.year), ("Movie 2019x", Some(2020)));
    }

    #[test]
    fn trailing_number_is_title_for_movies_and_folders() {
        let p = FileNameParser::parse_title("Apollo 13 (1995).mkv");
        assert_eq!((p.title.as_str(), p.year, p.episode), ("Apollo 13", Some(1995), None));
        let p = FileNameParser::parse_title("District.9.mkv");
        assert_eq!((p.title.as_str(), p.episode), ("District 9", None));
        let p = FileNameParser::parse_title("The 100.mkv");
        assert_eq!((p.title.as_str(), p.episode), ("The 100", None));
        let p = FileNameParser::parse_title("[Group] Mob Psycho 100 [BD 1080p]");
        assert_eq!((p.title.as_str(), p.episode), ("Mob Psycho 100", None));
        assert_eq!(FileNameParser::clean_title_for_match("Apollo 13"), "Apollo 13");
        // Episode files keep the bare-number fallback.
        let p = FileNameParser::parse("Show 05.mkv");
        assert_eq!((p.title.as_str(), p.episode), ("Show", Some(5)));
    }

    #[test]
    fn parses_anime_release_names() {
        let p = FileNameParser::parse("[SubsPlease] Title - 05 (1080p) [ABCD1234].mkv");
        assert_eq!(p.sub_group.as_deref(), Some("SubsPlease"));
        assert_eq!((p.title.as_str(), p.season, p.episode), ("Title", None, Some(5)));
        let p = FileNameParser::parse("[Group] Title - 05v2 (WEB 1080p HEVC).mkv");
        assert_eq!((p.title.as_str(), p.episode), ("Title", Some(5)));
        let p = FileNameParser::parse("[Group] Title S2 - 05 [1080p].mkv");
        assert_eq!((p.title.as_str(), p.season, p.episode), ("Title", Some(2), Some(5)));
        let p = FileNameParser::parse("[Group] Title (2019) - 12 (BD 1080p x265 FLAC).mkv");
        assert_eq!((p.title.as_str(), p.year, p.episode), ("Title", Some(2019), Some(12)));
        let p = FileNameParser::parse("[Group] Title - 07 (BD 1920x1080 AVC).mkv");
        assert_eq!((p.title.as_str(), p.episode), ("Title", Some(7)));
        let p = FileNameParser::parse("[Group] Title (TV) - 03.mkv");
        assert_eq!((p.title.as_str(), p.episode), ("Title (TV)", Some(3)));
    }

    #[test]
    fn parses_nx_and_multi_episode_markers() {
        let p = FileNameParser::parse("Show.Name.1x05.720p.mkv");
        assert_eq!((p.title.as_str(), p.season, p.episode), ("Show Name", Some(1), Some(5)));
        let p = FileNameParser::parse("Show Name - 01x05 - Pilot.mkv");
        assert_eq!((p.season, p.episode), (Some(1), Some(5)));
        let p = FileNameParser::parse("Show.Name.1080p.x264.mkv");
        assert_eq!((p.title.as_str(), p.season, p.episode), ("Show Name", None, None));
        let p = FileNameParser::parse("Show.Name.S01E01E02.1080p.mkv");
        assert_eq!((p.title.as_str(), p.season, p.episode), ("Show Name", Some(1), Some(1)));
        let p = FileNameParser::parse("Show.Name.S01E01-E02.mkv");
        assert_eq!((p.title.as_str(), p.season, p.episode), ("Show Name", Some(1), Some(1)));
    }
}
