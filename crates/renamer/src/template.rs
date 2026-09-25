//! Template engine for rename patterns (`{title}`, `{season:02}`, …).

use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

static TOKEN_RE: OnceLock<Regex> = OnceLock::new();

fn token_re() -> &'static Regex {
    TOKEN_RE.get_or_init(|| {
        Regex::new(r"\{(\w+)(?::0(\d+))?\}").expect("template token regex")
    })
}

pub struct TemplateEngine;

impl TemplateEngine {
    pub const MOVIE_FOLDER: &'static str = "{title} ({year})";
    pub const TV_SHOW_FOLDER: &'static str = "{title} ({year})";
    pub const SEASON_FOLDER: &'static str = "Season {season:02}";
    pub const MOVIE_FILE: &'static str = "{title} ({year})";
    pub const EPISODE_FILE: &'static str =
        "{title} - S{season:02}E{episode:02} - {episodeTitle}";

    pub fn render(template: &str, values: &HashMap<String, String>) -> String {
        let re = token_re();
        let mut result = String::with_capacity(template.len());
        let mut last = 0;
        for caps in re.captures_iter(template) {
            let m = caps.get(0).expect("full match");
            result.push_str(&template[last..m.start()]);
            let field = caps.get(1).map(|c| c.as_str()).unwrap_or("");
            let pad_width = caps
                .get(2)
                .and_then(|c| c.as_str().parse::<usize>().ok());
            let raw = values.get(field).map(String::as_str).unwrap_or("");
            if raw.is_empty() {
                // leave empty
            } else if let (Some(width), Ok(num)) = (pad_width, raw.parse::<i64>()) {
                result.push_str(&format!("{num:0width$}"));
            } else {
                result.push_str(raw);
            }
            last = m.end();
        }
        result.push_str(&template[last..]);

        let mut cleaned = result.replace(" -  - ", " - ");
        cleaned = cleaned.replace(" ()", "");
        while cleaned.contains("  ") {
            cleaned = cleaned.replace("  ", " ");
        }
        cleaned.trim().to_string()
    }

    pub fn sanitize_filename(name: &str) -> String {
        let mut s: String = name
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '/' | ':' | '\\' | '?' | '*' | '"' | '<' | '>' | '|'))
            .collect();
        while s.ends_with('.') || s.ends_with(' ') {
            s.pop();
        }
        // A leading dot (`.hack//SIGN`) would hide the entry from Finder and the scanner.
        s.trim_start_matches(['.', ' ']).to_string()
    }

    pub fn extract_variables(template: &str) -> Vec<String> {
        token_re()
            .captures_iter(template)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn basic_template() {
        let result = TemplateEngine::render(
            "{title} ({year})",
            &vals(&[("title", "Oppenheimer"), ("year", "2023")]),
        );
        assert_eq!(result, "Oppenheimer (2023)");
    }

    #[test]
    fn zero_padding() {
        let result = TemplateEngine::render(
            "{title} - S{season:02}E{episode:02} - {episodeTitle}",
            &vals(&[
                ("title", "Breaking Bad"),
                ("season", "2"),
                ("episode", "5"),
                ("episodeTitle", "Breakage"),
            ]),
        );
        assert_eq!(result, "Breaking Bad - S02E05 - Breakage");
    }

    #[test]
    fn missing_values() {
        let result = TemplateEngine::render("{title} ({year})", &vals(&[("title", "Movie")]));
        assert_eq!(result, "Movie");
    }

    #[test]
    fn anime_template() {
        let result = TemplateEngine::render(
            "{title} - {episode:03}",
            &vals(&[("title", "Frieren"), ("episode", "3")]),
        );
        assert_eq!(result, "Frieren - 003");
    }

    #[test]
    fn sanitize_filename() {
        assert_eq!(TemplateEngine::sanitize_filename("Movie: Title"), "Movie Title");
        assert_eq!(TemplateEngine::sanitize_filename("A/B\\C"), "ABC");
        assert_eq!(TemplateEngine::sanitize_filename("test..."), "test");
        assert_eq!(TemplateEngine::sanitize_filename("hello "), "hello");
        assert_eq!(TemplateEngine::sanitize_filename(".hack//SIGN (2002)"), "hackSIGN (2002)");
        assert_eq!(TemplateEngine::sanitize_filename("..."), "");
    }

    #[test]
    fn extract_variables() {
        let vars = TemplateEngine::extract_variables(
            "{title} - S{season:02}E{episode:02} - {episodeTitle}",
        );
        assert_eq!(vars, vec!["title", "season", "episode", "episodeTitle"]);
    }

    #[test]
    fn season_folder_template() {
        let result =
            TemplateEngine::render(TemplateEngine::SEASON_FOLDER, &vals(&[("season", "3")]));
        assert_eq!(result, "Season 03");
    }
}
