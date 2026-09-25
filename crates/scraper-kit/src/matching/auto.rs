use media_core::MediaType;

use crate::types::SearchResult;

use super::normalize_title;

/// Based on Swift `AutoMatchEvaluator` — exact normalized title, plus year when present.
/// A candidate in the same year wins; otherwise a single ±1 year (festival vs
/// theatrical, late-December premieres) is accepted. Wider gaps are treated as
/// different works (remakes). Does **not** use a confidence threshold.
pub fn auto_accepted_result(
    query_title: &str,
    query_year: Option<i32>,
    _media_type: MediaType,
    candidates: &[SearchResult],
) -> Option<SearchResult> {
    let mut sorted: Vec<&SearchResult> = candidates
        .iter()
        .filter(|c| title_matches(query_title, c))
        .collect();
    sorted.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let Some(qy) = query_year else {
        return sorted.first().map(|c| (*c).clone());
    };
    if let Some(exact) = sorted.iter().find(|c| c.year == Some(qy)) {
        return Some((*exact).clone());
    }
    let near: Vec<&SearchResult> = sorted
        .into_iter()
        .filter(|c| c.year.is_some_and(|y| (y - qy).abs() == 1))
        .collect();
    // Hits on both sides (qy-1 and qy+1) are ambiguous — leave it to the user.
    let first_year = near.first()?.year;
    if near.iter().all(|c| c.year == first_year) {
        near.first().map(|c| (*c).clone())
    } else {
        None
    }
}

fn title_matches(query_title: &str, candidate: &SearchResult) -> bool {
    let query = normalize_title(query_title);
    let title = normalize_title(&candidate.title);
    let original = normalize_title(candidate.original_title.as_deref().unwrap_or(""));
    query == title || (!original.is_empty() && query == original)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchResult;

    #[test]
    fn requires_exact_title_and_year() {
        let candidates = vec![SearchResult {
            source_id: "tmdb:1".into(),
            title: "Inception".into(),
            original_title: None,
            year: Some(2010),
            overview: None,
            poster_url: None,
            confidence: 0.9,
            media_type: MediaType::Movie,
        }];
        assert!(auto_accepted_result("Inception", Some(2010), MediaType::Movie, &candidates).is_some());
        assert!(auto_accepted_result("Inception", Some(2012), MediaType::Movie, &candidates).is_none());
        assert!(auto_accepted_result("Incept", Some(2010), MediaType::Movie, &candidates).is_none());
    }

    fn candidate(id: &str, year: Option<i32>, confidence: f64) -> SearchResult {
        SearchResult {
            source_id: id.into(),
            title: "Dune".into(),
            original_title: None,
            year,
            overview: None,
            poster_url: None,
            confidence,
            media_type: MediaType::Movie,
        }
    }

    fn accepted(year: Option<i32>, candidates: &[SearchResult]) -> Option<String> {
        auto_accepted_result("Dune", year, MediaType::Movie, candidates).map(|c| c.source_id)
    }

    #[test]
    fn exact_year_beats_adjacent_year() {
        // The ±1 row scores higher but the same-year row must still win.
        let candidates = vec![
            candidate("tmdb:near", Some(2020), 0.95),
            candidate("tmdb:exact", Some(2021), 0.9),
        ];
        assert_eq!(accepted(Some(2021), &candidates).as_deref(), Some("tmdb:exact"));
    }

    #[test]
    fn adjacent_year_accepted_when_alone() {
        let candidates = vec![candidate("tmdb:1", Some(2020), 0.9)];
        assert_eq!(accepted(Some(2021), &candidates).as_deref(), Some("tmdb:1"));
        assert_eq!(accepted(Some(2019), &candidates).as_deref(), Some("tmdb:1"));
    }

    #[test]
    fn two_year_gap_rejected() {
        let candidates = vec![candidate("tmdb:1984", Some(1984), 0.9)];
        assert!(accepted(Some(1986), &candidates).is_none());
        assert!(accepted(Some(1982), &candidates).is_none());
    }

    #[test]
    fn adjacent_years_on_both_sides_are_ambiguous() {
        let candidates = vec![
            candidate("tmdb:a", Some(2020), 0.9),
            candidate("tmdb:b", Some(2022), 0.9),
        ];
        assert!(accepted(Some(2021), &candidates).is_none());
    }

    #[test]
    fn unknown_candidate_year_rejected_when_query_has_year() {
        let candidates = vec![candidate("tmdb:1", None, 0.9)];
        assert!(accepted(Some(2021), &candidates).is_none());
        assert_eq!(accepted(None, &candidates).as_deref(), Some("tmdb:1"));
    }
}
