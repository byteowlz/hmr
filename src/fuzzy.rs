//! Fuzzy matching for human-friendly input
//!
//! Provides typo-tolerant matching for:
//! - Entity IDs and friendly names
//! - Area names
//! - Service/domain names
//! - Commands
//!
//! Uses a combination of:
//! - Exact matching (highest priority)
//! - Prefix matching
//! - Levenshtein distance (for typo tolerance)
//! - Fuzzy scoring (skim algorithm)

use std::cmp::Ordering;

use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher as FuzzyMatcherTrait;

use crate::cache::{Cache, CachedArea, CachedEntity, CachedService};

/// Maximum Levenshtein distance for auto-correction
const MAX_EDIT_DISTANCE: usize = 2;

/// Minimum score for fuzzy matches to be considered
const MIN_FUZZY_SCORE: i64 = 50;

/// A match result with confidence information
#[derive(Debug, Clone)]
pub struct Match<T> {
    /// The matched item
    pub item: T,
    /// Match confidence (0.0 to 1.0)
    pub confidence: f64,
    /// How the match was found
    pub match_type: MatchType,
    /// The input that was matched against
    pub matched_input: String,
    /// What part of the item matched
    pub matched_on: String,
}

impl<T> Match<T> {
    pub fn exact(item: T, input: &str, matched_on: &str) -> Self {
        Self {
            item,
            confidence: 1.0,
            match_type: MatchType::Exact,
            matched_input: input.to_string(),
            matched_on: matched_on.to_string(),
        }
    }

    pub fn prefix(item: T, input: &str, matched_on: &str) -> Self {
        Self {
            item,
            confidence: 0.9,
            match_type: MatchType::Prefix,
            matched_input: input.to_string(),
            matched_on: matched_on.to_string(),
        }
    }

    pub fn fuzzy(item: T, input: &str, matched_on: &str, score: i64, max_score: i64) -> Self {
        let confidence = if max_score > 0 {
            (score as f64 / max_score as f64).min(0.85)
        } else {
            0.5
        };
        Self {
            item,
            confidence,
            match_type: MatchType::Fuzzy,
            matched_input: input.to_string(),
            matched_on: matched_on.to_string(),
        }
    }

    pub fn typo(item: T, input: &str, matched_on: &str, distance: usize) -> Self {
        let confidence = match distance {
            1 => 0.8,
            2 => 0.6,
            _ => 0.4,
        };
        Self {
            item,
            confidence,
            match_type: MatchType::Typo { distance },
            matched_input: input.to_string(),
            matched_on: matched_on.to_string(),
        }
    }
}

impl<T: Clone> Match<T> {
    /// Map the inner item to a different type
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Match<U> {
        Match {
            item: f(self.item),
            confidence: self.confidence,
            match_type: self.match_type,
            matched_input: self.matched_input,
            matched_on: self.matched_on,
        }
    }
}

/// How the match was found
#[derive(Debug, Clone, PartialEq)]
pub enum MatchType {
    /// Exact string match
    Exact,
    /// Input is a prefix of the target
    Prefix,
    /// Fuzzy match using skim algorithm
    Fuzzy,
    /// Typo correction using Levenshtein distance
    Typo { distance: usize },
}

impl MatchType {
    /// Get a priority for sorting (lower is better)
    pub fn priority(&self) -> u8 {
        match self {
            MatchType::Exact => 0,
            MatchType::Prefix => 1,
            MatchType::Typo { distance: 1 } => 2,
            MatchType::Typo { distance: 2 } => 3,
            MatchType::Fuzzy => 4,
            MatchType::Typo { .. } => 5,
        }
    }
}

/// Result of attempting to match input
#[derive(Debug)]
pub enum MatchResult<T> {
    /// Single unambiguous match
    Single(Match<T>),
    /// Multiple possible matches (ambiguous)
    Multiple(Vec<Match<T>>),
    /// No match found
    None,
}

impl<T> MatchResult<T> {
    /// Get the best match if unambiguous
    pub fn best(self) -> Option<Match<T>> {
        match self {
            MatchResult::Single(m) => Some(m),
            MatchResult::Multiple(mut matches) if !matches.is_empty() => {
                // If the top match is significantly better, use it
                if matches.len() >= 2 {
                    matches.sort_by(|a, b| {
                        b.confidence
                            .partial_cmp(&a.confidence)
                            .unwrap_or(Ordering::Equal)
                    });
                    if matches[0].confidence > matches[1].confidence + 0.2 {
                        return Some(matches.remove(0));
                    }
                }
                Some(matches.remove(0))
            }
            _ => None,
        }
    }

    /// Check if this is an exact match
    pub fn is_exact(&self) -> bool {
        matches!(
            self,
            MatchResult::Single(Match {
                match_type: MatchType::Exact,
                ..
            })
        )
    }
}

/// Fuzzy matcher for Home Assistant entities and metadata
pub struct FuzzyMatcher {
    matcher: SkimMatcherV2,
}

impl Default for FuzzyMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyMatcher {
    pub fn new() -> Self {
        Self {
            matcher: SkimMatcherV2::default(),
        }
    }

    /// Find matching entities from cache
    pub fn find_entity<'a>(&self, input: &str, cache: &'a Cache) -> MatchResult<&'a CachedEntity> {
        let input_lower = input.to_lowercase();
        let entities = cache.entities();

        // First pass: exact matches
        for entity in entities {
            // Exact entity_id match
            if entity.entity_id == input || entity.entity_id == input_lower {
                return MatchResult::Single(Match::exact(entity, input, &entity.entity_id));
            }
            // Exact object_id match (e.g., "kitchen" for "light.kitchen")
            if entity.object_id == input_lower {
                return MatchResult::Single(Match::exact(entity, input, &entity.object_id));
            }
            // Exact friendly name match
            if let Some(ref name) = entity.friendly_name {
                if name.to_lowercase() == input_lower {
                    return MatchResult::Single(Match::exact(entity, input, name));
                }
            }
        }

        // Second pass: prefix matches
        let mut prefix_matches = Vec::new();
        for entity in entities {
            for search_name in &entity.search_names {
                let search_lower = search_name.to_lowercase();
                if search_lower.starts_with(&input_lower) {
                    prefix_matches.push(Match::prefix(entity, input, search_name));
                    break;
                }
            }
        }
        if prefix_matches.len() == 1 {
            return MatchResult::Single(prefix_matches.remove(0));
        } else if !prefix_matches.is_empty() {
            return MatchResult::Multiple(prefix_matches);
        }

        // Third pass: typo correction with Levenshtein distance
        let mut typo_matches = Vec::new();
        for entity in entities {
            for search_name in &entity.search_names {
                let search_lower = search_name.to_lowercase();
                let distance = levenshtein(&input_lower, &search_lower);
                if distance <= MAX_EDIT_DISTANCE && distance > 0 {
                    typo_matches.push(Match::typo(entity, input, search_name, distance));
                    break;
                }
            }
        }
        if typo_matches.len() == 1 {
            return MatchResult::Single(typo_matches.remove(0));
        } else if !typo_matches.is_empty() {
            // Sort by distance (lower is better)
            typo_matches.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(Ordering::Equal)
            });
            return MatchResult::Multiple(typo_matches);
        }

        // Fourth pass: fuzzy matching
        let mut fuzzy_matches = Vec::new();
        let max_possible_score = (input.len() as i64) * 16; // Rough estimate

        for entity in entities {
            let mut best_score = 0i64;
            let mut best_name = String::new();

            for search_name in &entity.search_names {
                if let Some(score) = self.matcher.fuzzy_match(search_name, &input_lower) {
                    if score > best_score && score >= MIN_FUZZY_SCORE {
                        best_score = score;
                        best_name = search_name.clone();
                    }
                }
            }

            if best_score >= MIN_FUZZY_SCORE {
                fuzzy_matches.push(Match::fuzzy(
                    entity,
                    input,
                    &best_name,
                    best_score,
                    max_possible_score,
                ));
            }
        }

        if fuzzy_matches.is_empty() {
            return MatchResult::None;
        }

        fuzzy_matches.sort_by(|a, b| {
            // Sort by match type priority first, then by confidence
            match a.match_type.priority().cmp(&b.match_type.priority()) {
                Ordering::Equal => b
                    .confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(Ordering::Equal),
                other => other,
            }
        });

        if fuzzy_matches.len() == 1 {
            MatchResult::Single(fuzzy_matches.remove(0))
        } else {
            MatchResult::Multiple(fuzzy_matches)
        }
    }

    /// Find matching areas from cache
    pub fn find_area<'a>(&self, input: &str, cache: &'a Cache) -> MatchResult<&'a CachedArea> {
        let input_lower = input.to_lowercase();
        let areas = cache.areas();

        // Exact matches
        for area in areas {
            if area.area_id == input_lower || area.name.to_lowercase() == input_lower {
                return MatchResult::Single(Match::exact(area, input, &area.name));
            }
            for alias in &area.aliases {
                if alias.to_lowercase() == input_lower {
                    return MatchResult::Single(Match::exact(area, input, alias));
                }
            }
        }

        // Prefix matches
        let mut prefix_matches = Vec::new();
        for area in areas {
            for search_name in &area.search_names {
                if search_name.to_lowercase().starts_with(&input_lower) {
                    prefix_matches.push(Match::prefix(area, input, search_name));
                    break;
                }
            }
        }
        if prefix_matches.len() == 1 {
            return MatchResult::Single(prefix_matches.remove(0));
        } else if !prefix_matches.is_empty() {
            return MatchResult::Multiple(prefix_matches);
        }

        // Typo correction
        let mut typo_matches = Vec::new();
        for area in areas {
            for search_name in &area.search_names {
                let distance = levenshtein(&input_lower, &search_name.to_lowercase());
                if distance <= MAX_EDIT_DISTANCE && distance > 0 {
                    typo_matches.push(Match::typo(area, input, search_name, distance));
                    break;
                }
            }
        }
        if !typo_matches.is_empty() {
            typo_matches.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(Ordering::Equal)
            });
            if typo_matches.len() == 1 {
                return MatchResult::Single(typo_matches.remove(0));
            }
            return MatchResult::Multiple(typo_matches);
        }

        // Fuzzy matching
        let mut fuzzy_matches = Vec::new();
        let max_score = (input.len() as i64) * 16;

        for area in areas {
            for search_name in &area.search_names {
                if let Some(score) = self.matcher.fuzzy_match(search_name, &input_lower) {
                    if score >= MIN_FUZZY_SCORE {
                        fuzzy_matches.push(Match::fuzzy(
                            area,
                            input,
                            search_name,
                            score,
                            max_score,
                        ));
                        break;
                    }
                }
            }
        }

        if fuzzy_matches.is_empty() {
            return MatchResult::None;
        }

        fuzzy_matches.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(Ordering::Equal)
        });

        if fuzzy_matches.len() == 1 {
            MatchResult::Single(fuzzy_matches.remove(0))
        } else {
            MatchResult::Multiple(fuzzy_matches)
        }
    }

    /// Find matching service from cache
    pub fn find_service<'a>(
        &self,
        input: &str,
        cache: &'a Cache,
    ) -> MatchResult<&'a CachedService> {
        let input_lower = input.to_lowercase();
        let services = cache.services();

        // Parse domain.service format
        let (domain_filter, service_part) = if input.contains('.') {
            let parts: Vec<&str> = input.splitn(2, '.').collect();
            (
                Some(parts[0].to_lowercase()),
                parts.get(1).map(|s| s.to_lowercase()),
            )
        } else {
            (None, Some(input_lower.clone()))
        };

        // Exact matches
        for service in services {
            // Full service name match (domain.service)
            if service.full_name.to_lowercase() == input_lower {
                return MatchResult::Single(Match::exact(service, input, &service.full_name));
            }
            // Service name only match (if domain filter matches or not specified)
            if let Some(ref sp) = service_part {
                if service.service.to_lowercase() == *sp {
                    if domain_filter.is_none()
                        || domain_filter.as_ref() == Some(&service.domain.to_lowercase())
                    {
                        return MatchResult::Single(Match::exact(
                            service,
                            input,
                            &service.full_name,
                        ));
                    }
                }
            }
        }

        // Prefix matches
        let mut prefix_matches = Vec::new();
        for service in services {
            if service.full_name.to_lowercase().starts_with(&input_lower) {
                prefix_matches.push(Match::prefix(service, input, &service.full_name));
            }
        }
        if prefix_matches.len() == 1 {
            return MatchResult::Single(prefix_matches.remove(0));
        } else if !prefix_matches.is_empty() {
            return MatchResult::Multiple(prefix_matches);
        }

        // Typo correction
        let mut typo_matches = Vec::new();
        for service in services {
            let distance = levenshtein(&input_lower, &service.full_name.to_lowercase());
            if distance <= MAX_EDIT_DISTANCE && distance > 0 {
                typo_matches.push(Match::typo(service, input, &service.full_name, distance));
            }
        }
        if !typo_matches.is_empty() {
            typo_matches.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(Ordering::Equal)
            });
            if typo_matches.len() == 1 {
                return MatchResult::Single(typo_matches.remove(0));
            }
            return MatchResult::Multiple(typo_matches);
        }

        // Fuzzy matching
        let mut fuzzy_matches = Vec::new();
        let max_score = (input.len() as i64) * 16;

        for service in services {
            if let Some(score) = self.matcher.fuzzy_match(&service.full_name, &input_lower) {
                if score >= MIN_FUZZY_SCORE {
                    fuzzy_matches.push(Match::fuzzy(
                        service,
                        input,
                        &service.full_name,
                        score,
                        max_score,
                    ));
                }
            }
        }

        if fuzzy_matches.is_empty() {
            return MatchResult::None;
        }

        fuzzy_matches.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(Ordering::Equal)
        });

        if fuzzy_matches.len() == 1 {
            MatchResult::Single(fuzzy_matches.remove(0))
        } else {
            MatchResult::Multiple(fuzzy_matches)
        }
    }

    /// Find matching domain from cache
    pub fn find_domain<'a>(&self, input: &str, cache: &'a Cache) -> MatchResult<String> {
        let input_lower = input.to_lowercase();
        let domains = cache.domains();

        // Handle plural forms (lights -> light, switches -> switch)
        let singular = to_singular(&input_lower);

        // Exact match
        for domain in &domains {
            if *domain == input_lower || *domain == singular {
                return MatchResult::Single(Match::exact(domain.to_string(), input, domain));
            }
        }

        // Prefix match
        let mut prefix_matches = Vec::new();
        for domain in &domains {
            if domain.starts_with(&input_lower) || domain.starts_with(&singular) {
                prefix_matches.push(Match::prefix(domain.to_string(), input, domain));
            }
        }
        if prefix_matches.len() == 1 {
            return MatchResult::Single(prefix_matches.remove(0));
        } else if !prefix_matches.is_empty() {
            return MatchResult::Multiple(prefix_matches);
        }

        // Typo correction
        let mut typo_matches = Vec::new();
        for domain in &domains {
            let distance = levenshtein(&input_lower, domain);
            if distance <= MAX_EDIT_DISTANCE && distance > 0 {
                typo_matches.push(Match::typo(domain.to_string(), input, domain, distance));
            }
        }
        if !typo_matches.is_empty() {
            typo_matches.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(Ordering::Equal)
            });
            if typo_matches.len() == 1 {
                return MatchResult::Single(typo_matches.remove(0));
            }
            return MatchResult::Multiple(typo_matches);
        }

        MatchResult::None
    }

    /// Find entities in a specific domain
    pub fn find_entities_in_domain<'a>(
        &self,
        domain: &str,
        cache: &'a Cache,
    ) -> Vec<&'a CachedEntity> {
        let domain_lower = domain.to_lowercase();
        let singular = to_singular(&domain_lower);

        // Try exact domain match first
        let mut entities = cache.entities_in_domain(&domain_lower);
        
        // If domain might be plural, also try singular form
        if entities.is_empty() && domain_lower != singular {
            entities = cache.entities_in_domain(&singular);
        }
        
        entities
    }

    /// Find entities in a specific area
    pub fn find_entities_in_area<'a>(
        &self,
        area_input: &str,
        cache: &'a Cache,
    ) -> Vec<&'a CachedEntity> {
        match self.find_area(area_input, cache).best() {
            Some(area_match) => cache
                .entities()
                .iter()
                .filter(|e| e.area_id.as_deref() == Some(&area_match.item.area_id))
                .collect(),
            None => Vec::new(),
        }
    }
}

/// Calculate Levenshtein distance between two strings
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Early termination: if length difference > MAX_EDIT_DISTANCE, no point calculating
    if a_len.abs_diff(b_len) > MAX_EDIT_DISTANCE {
        return MAX_EDIT_DISTANCE + 1;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0; b_len + 1];

    for (i, a_char) in a_chars.iter().enumerate() {
        curr_row[0] = i + 1;

        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + cost);
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Convert a plural word to singular (basic rules)
fn to_singular(word: &str) -> String {
    if word.ends_with("ies") && word.len() > 3 {
        format!("{}y", &word[..word.len() - 3])
    } else if word.ends_with("es") && word.len() > 2 {
        word[..word.len() - 2].to_string()
    } else if word.ends_with('s') && word.len() > 1 {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}

/// Format a match result for display to the user
pub fn format_correction(original: &str, corrected: &str) -> String {
    if original.to_lowercase() == corrected.to_lowercase() {
        corrected.to_string()
    } else {
        format!("{} -> {}", original, corrected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachedService;

    fn create_test_cache() -> Cache {
        let mut cache = Cache::new();

        // Add test entities
        let entities = vec![
            CachedEntity {
                entity_id: "light.kitchen".to_string(),
                domain: "light".to_string(),
                object_id: "kitchen".to_string(),
                state: "on".to_string(),
                friendly_name: Some("Kitchen Light".to_string()),
                area_id: Some("kitchen".to_string()),
                search_names: vec![
                    "light.kitchen".to_string(),
                    "kitchen".to_string(),
                    "Kitchen Light".to_string(),
                    "kitchen light".to_string(),
                    "kitchen_light".to_string(),
                ],
            },
            CachedEntity {
                entity_id: "light.living_room".to_string(),
                domain: "light".to_string(),
                object_id: "living_room".to_string(),
                state: "off".to_string(),
                friendly_name: Some("Living Room Light".to_string()),
                area_id: Some("living_room".to_string()),
                search_names: vec![
                    "light.living_room".to_string(),
                    "living_room".to_string(),
                    "Living Room Light".to_string(),
                    "living room light".to_string(),
                    "living_room_light".to_string(),
                ],
            },
            CachedEntity {
                entity_id: "switch.bedroom_fan".to_string(),
                domain: "switch".to_string(),
                object_id: "bedroom_fan".to_string(),
                state: "off".to_string(),
                friendly_name: Some("Bedroom Fan".to_string()),
                area_id: Some("bedroom".to_string()),
                search_names: vec![
                    "switch.bedroom_fan".to_string(),
                    "bedroom_fan".to_string(),
                    "Bedroom Fan".to_string(),
                    "bedroom fan".to_string(),
                ],
            },
        ];

        let file =
            crate::cache::CacheFile::new(entities, 3600, "http://localhost:8123".to_string());
        cache.set_entities(file);

        // Add test areas
        let areas = vec![
            CachedArea {
                area_id: "kitchen".to_string(),
                name: "Kitchen".to_string(),
                aliases: vec![],
                search_names: vec!["kitchen".to_string(), "Kitchen".to_string()],
            },
            CachedArea {
                area_id: "living_room".to_string(),
                name: "Living Room".to_string(),
                aliases: vec!["Lounge".to_string()],
                search_names: vec![
                    "living_room".to_string(),
                    "Living Room".to_string(),
                    "living room".to_string(),
                    "Lounge".to_string(),
                    "lounge".to_string(),
                ],
            },
        ];

        let file = crate::cache::CacheFile::new(areas, 3600, "http://localhost:8123".to_string());
        cache.set_areas(file);

        // Add test services
        let services = vec![
            CachedService {
                domain: "light".to_string(),
                service: "turn_on".to_string(),
                full_name: "light.turn_on".to_string(),
                description: "Turn on a light".to_string(),
            },
            CachedService {
                domain: "light".to_string(),
                service: "turn_off".to_string(),
                full_name: "light.turn_off".to_string(),
                description: "Turn off a light".to_string(),
            },
            CachedService {
                domain: "switch".to_string(),
                service: "toggle".to_string(),
                full_name: "switch.toggle".to_string(),
                description: "Toggle a switch".to_string(),
            },
        ];

        let file =
            crate::cache::CacheFile::new(services, 3600, "http://localhost:8123".to_string());
        cache.set_services(file);

        cache
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("light", "ligth"), 2); // Typo
        assert_eq!(levenshtein("kitchen", "kitchn"), 1); // Missing letter
    }

    #[test]
    fn test_levenshtein_early_termination() {
        // Very different strings should return early
        let result = levenshtein("abc", "abcdefghij");
        assert!(result > MAX_EDIT_DISTANCE);
    }

    #[test]
    fn test_to_singular() {
        assert_eq!(to_singular("lights"), "light");
        assert_eq!(to_singular("switches"), "switch"); // -es removed
        assert_eq!(to_singular("entities"), "entity"); // -ies -> y
        assert_eq!(to_singular("fans"), "fan");
        assert_eq!(to_singular("light"), "light");
    }

    #[test]
    fn test_match_type_priority() {
        assert!(MatchType::Exact.priority() < MatchType::Prefix.priority());
        assert!(MatchType::Prefix.priority() < MatchType::Typo { distance: 1 }.priority());
        assert!(
            MatchType::Typo { distance: 1 }.priority() < MatchType::Typo { distance: 2 }.priority()
        );
        assert!(MatchType::Typo { distance: 2 }.priority() < MatchType::Fuzzy.priority());
    }

    #[test]
    fn test_match_exact() {
        let m: Match<&str> = Match::exact("test", "input", "matched");
        assert_eq!(m.confidence, 1.0);
        assert!(matches!(m.match_type, MatchType::Exact));
    }

    #[test]
    fn test_match_prefix() {
        let m: Match<&str> = Match::prefix("test", "input", "matched");
        assert_eq!(m.confidence, 0.9);
        assert!(matches!(m.match_type, MatchType::Prefix));
    }

    #[test]
    fn test_match_typo() {
        let m1: Match<&str> = Match::typo("test", "input", "matched", 1);
        assert_eq!(m1.confidence, 0.8);

        let m2: Match<&str> = Match::typo("test", "input", "matched", 2);
        assert_eq!(m2.confidence, 0.6);
    }

    #[test]
    fn test_find_entity_exact_match() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        // Exact entity_id match
        let result = matcher.find_entity("light.kitchen", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item.entity_id, "light.kitchen");
                assert!(matches!(m.match_type, MatchType::Exact));
            }
            _ => panic!("Expected single match"),
        }
    }

    #[test]
    fn test_find_entity_friendly_name_match() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        // Exact friendly name match
        let result = matcher.find_entity("Kitchen Light", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item.entity_id, "light.kitchen");
            }
            _ => panic!("Expected single match"),
        }
    }

    #[test]
    fn test_find_entity_object_id_match() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        // Object ID match
        let result = matcher.find_entity("kitchen", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item.entity_id, "light.kitchen");
            }
            _ => panic!("Expected single match"),
        }
    }

    #[test]
    fn test_find_entity_no_match() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        let result = matcher.find_entity("nonexistent_entity", &cache);
        assert!(matches!(result, MatchResult::None));
    }

    #[test]
    fn test_find_area_exact_match() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        let result = matcher.find_area("kitchen", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item.area_id, "kitchen");
            }
            _ => panic!("Expected single match"),
        }
    }

    #[test]
    fn test_find_area_alias_match() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        let result = matcher.find_area("lounge", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item.area_id, "living_room");
            }
            _ => panic!("Expected single match for alias"),
        }
    }

    #[test]
    fn test_find_service_exact_match() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        let result = matcher.find_service("light.turn_on", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item.full_name, "light.turn_on");
            }
            _ => panic!("Expected single match"),
        }
    }

    #[test]
    fn test_find_domain() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        let result = matcher.find_domain("light", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item, "light");
            }
            _ => panic!("Expected single match"),
        }
    }

    #[test]
    fn test_find_domain_plural() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        // "lights" should match "light" domain
        let result = matcher.find_domain("lights", &cache);
        match result {
            MatchResult::Single(m) => {
                assert_eq!(m.item, "light");
            }
            _ => panic!("Expected single match for plural"),
        }
    }

    #[test]
    fn test_find_entities_in_domain() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        let lights = matcher.find_entities_in_domain("light", &cache);
        assert_eq!(lights.len(), 2);

        let switches = matcher.find_entities_in_domain("switch", &cache);
        assert_eq!(switches.len(), 1);
    }

    #[test]
    fn test_find_entities_in_area() {
        let cache = create_test_cache();
        let matcher = FuzzyMatcher::new();

        let kitchen_entities = matcher.find_entities_in_area("kitchen", &cache);
        assert_eq!(kitchen_entities.len(), 1);
        assert_eq!(kitchen_entities[0].entity_id, "light.kitchen");
    }

    #[test]
    fn test_match_result_best() {
        let m = Match::exact("test", "input", "matched");
        let result = MatchResult::Single(m);

        let best = result.best();
        assert!(best.is_some());
        assert_eq!(best.unwrap().item, "test");
    }

    #[test]
    fn test_match_result_none_best() {
        let result: MatchResult<&str> = MatchResult::None;
        assert!(result.best().is_none());
    }

    #[test]
    fn test_format_correction() {
        assert_eq!(format_correction("light", "light"), "light");
        assert_eq!(format_correction("ligth", "light"), "ligth -> light");
    }
}
