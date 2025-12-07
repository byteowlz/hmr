//! Natural language command parsing
//!
//! Parses human-friendly commands into structured actions:
//! - "turn on kitchen light" -> light.turn_on for light.kitchen
//! - "set bedroom temperature to 72" -> climate.set_temperature for climate.bedroom
//! - "dim living room lights to 50%" -> light.turn_on with brightness for light.living_room
//!
//! Supports flexible argument order:
//! - "turn on kitchen light"
//! - "kitchen light on"
//! - "on kitchen light"

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::cache::{Cache, CachedEntity};
use crate::fuzzy::{FuzzyMatcher, Match, MatchResult, MatchType};

/// Action verbs and their mappings to Home Assistant services
#[derive(Debug, Clone)]
pub struct ActionMapping {
    /// Words that trigger this action
    pub trigger_words: Vec<&'static str>,
    /// Default service to call (domain will be determined by entity)
    pub default_service: &'static str,
    /// Domain-specific overrides
    pub domain_overrides: HashMap<&'static str, &'static str>,
    /// Whether this action can infer domain from context
    pub infers_domain: bool,
}

impl ActionMapping {
    pub fn service_for_domain(&self, domain: &str) -> &str {
        self.domain_overrides
            .get(domain)
            .copied()
            .unwrap_or(self.default_service)
    }
}

/// Get all known action mappings
pub fn action_mappings() -> Vec<ActionMapping> {
    let mut mappings = Vec::new();

    // Turn on
    mappings.push(ActionMapping {
        trigger_words: vec!["on", "turn_on", "enable", "activate", "start"],
        default_service: "turn_on",
        domain_overrides: HashMap::new(),
        infers_domain: true,
    });

    // Turn off
    mappings.push(ActionMapping {
        trigger_words: vec!["off", "turn_off", "disable", "deactivate", "stop", "kill"],
        default_service: "turn_off",
        domain_overrides: HashMap::new(),
        infers_domain: true,
    });

    // Toggle
    mappings.push(ActionMapping {
        trigger_words: vec!["toggle", "switch", "flip"],
        default_service: "toggle",
        domain_overrides: HashMap::new(),
        infers_domain: true,
    });

    // Open
    let mut open_overrides = HashMap::new();
    open_overrides.insert("cover", "open_cover");
    open_overrides.insert("lock", "unlock");
    open_overrides.insert("valve", "open_valve");
    mappings.push(ActionMapping {
        trigger_words: vec!["open", "unlock"],
        default_service: "open_cover",
        domain_overrides: open_overrides,
        infers_domain: true,
    });

    // Close
    let mut close_overrides = HashMap::new();
    close_overrides.insert("cover", "close_cover");
    close_overrides.insert("lock", "lock");
    close_overrides.insert("valve", "close_valve");
    mappings.push(ActionMapping {
        trigger_words: vec!["close", "shut", "lock"],
        default_service: "close_cover",
        domain_overrides: close_overrides,
        infers_domain: true,
    });

    // Dim / Brighten
    mappings.push(ActionMapping {
        trigger_words: vec!["dim", "lower", "decrease", "reduce"],
        default_service: "turn_on",
        domain_overrides: HashMap::new(),
        infers_domain: false, // Typically only for lights
    });

    mappings.push(ActionMapping {
        trigger_words: vec!["brighten", "raise", "increase", "brighter"],
        default_service: "turn_on",
        domain_overrides: HashMap::new(),
        infers_domain: false,
    });

    // Set
    mappings.push(ActionMapping {
        trigger_words: vec!["set"],
        default_service: "turn_on",
        domain_overrides: HashMap::new(),
        infers_domain: true,
    });

    mappings
}

/// A parsed natural language command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedCommand {
    /// The original input string
    pub original: String,
    /// Detected action (turn_on, turn_off, etc.)
    pub action: Option<String>,
    /// Matched entities
    pub targets: Vec<ParsedTarget>,
    /// Additional parameters (brightness, temperature, etc.)
    pub parameters: HashMap<String, serde_json::Value>,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f64,
    /// What was interpreted (for display)
    pub interpretation: String,
    /// Any warnings or notes
    pub notes: Vec<String>,
    /// Matched area if any
    pub matched_area: Option<String>,
}

/// A matched target (entity or entity pattern)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedTarget {
    /// Entity ID
    pub entity_id: String,
    /// Friendly name if available
    pub friendly_name: Option<String>,
    /// How the match was made
    pub match_type: String,
    /// What the user typed that matched this
    pub matched_input: String,
}

impl From<Match<&CachedEntity>> for ParsedTarget {
    fn from(m: Match<&CachedEntity>) -> Self {
        let match_type_str = format!("{:?}", m.match_type);
        // Use Match::map() to transform entity reference into ParsedTarget
        let mapped = m.map(|entity| ParsedTarget {
            entity_id: entity.entity_id.clone(),
            friendly_name: entity.friendly_name.clone(),
            match_type: match_type_str,
            matched_input: String::new(), // Will be set below
        });

        ParsedTarget {
            matched_input: mapped.matched_input,
            ..mapped.item
        }
    }
}

/// Natural language parser
pub struct NLParser {
    matcher: FuzzyMatcher,
    actions: Vec<ActionMapping>,
}

impl Default for NLParser {
    fn default() -> Self {
        Self::new()
    }
}

impl NLParser {
    pub fn new() -> Self {
        Self {
            matcher: FuzzyMatcher::new(),
            actions: action_mappings(),
        }
    }

    /// Parse a natural language command
    pub fn parse(&self, input: &str, cache: &Cache) -> Result<ParsedCommand> {
        let input = input.trim();
        if input.is_empty() {
            return Err(anyhow!("Empty command"));
        }

        let tokens = tokenize(input);
        if tokens.is_empty() {
            return Err(anyhow!("No tokens in command"));
        }

        // Check for service-based command format: "call <domain> <service>"
        // Examples: "call light turn_on", "call switch toggle"
        if tokens.len() >= 3 && (tokens[0] == "call" || tokens[0] == "run") {
            return self.parse_service_based(input, &tokens, cache);
        }

        let mut result = ParsedCommand {
            original: input.to_string(),
            action: None,
            targets: Vec::new(),
            parameters: HashMap::new(),
            confidence: 0.0,
            interpretation: String::new(),
            notes: Vec::new(),
            matched_area: None,
        };

        // Classify each token
        let mut action_found = false;
        let mut action_mapping: Option<&ActionMapping> = None;
        let mut domain_hint: Option<String> = None;
        let mut area_hint: Option<String> = None;
        let mut remaining_tokens: Vec<&str> = Vec::new();

        for token in &tokens {
            // Check if it's an action verb
            if let Some((action, mapping)) = self.find_action_with_mapping(token) {
                if !action_found {
                    result.action = Some(action);
                    action_mapping = Some(mapping);
                    action_found = true;
                    continue;
                }
            }

            // Check if it's a number (parameter)
            if let Some(num) = parse_number(token) {
                // Could be brightness, temperature, etc.
                result.parameters.insert("value".to_string(), num.into());
                continue;
            }

            // Check if it's a percentage
            if let Some(pct) = parse_percentage(token) {
                result
                    .parameters
                    .insert("brightness_pct".to_string(), pct.into());
                continue;
            }

            // Check if it's a domain (light, switch, etc.)
            if let Some(domain) = self.find_domain(token, cache) {
                domain_hint = Some(domain);
                continue;
            }

            // Check if it's an area
            if let MatchResult::Single(area_match) = self.matcher.find_area(token, cache) {
                area_hint = Some(area_match.item.area_id.clone());
                result.matched_area = Some(area_match.item.area_id.clone());
                continue;
            }

            // Otherwise, it might be part of an entity name
            remaining_tokens.push(token);
        }

        // Try to find entities from remaining tokens
        let target_string = remaining_tokens.join(" ");
        if !target_string.is_empty() {
            // First try the full string
            match self.matcher.find_entity(&target_string, cache) {
                MatchResult::Single(m) => {
                    // Only accept single matches with reasonable confidence
                    // Exact and prefix matches are always fine, fuzzy needs higher threshold
                    let min_confidence = match m.match_type {
                        MatchType::Exact | MatchType::Prefix => 0.0,
                        MatchType::Typo { .. } => 0.6,
                        MatchType::Fuzzy => 0.65,
                    };

                    if m.confidence >= min_confidence {
                        result.targets.push(m.into());
                    }
                }
                MatchResult::Multiple(matches) => {
                    // If we have a domain hint, filter by it
                    let filtered: Vec<_> = if let Some(ref domain) = domain_hint {
                        matches
                            .into_iter()
                            .filter(|m| &m.item.domain == domain)
                            .collect()
                    } else {
                        matches
                    };

                    if filtered.len() == 1 {
                        let m = filtered.into_iter().next().unwrap();
                        // Apply same confidence threshold for single filtered match
                        let min_confidence = match m.match_type {
                            MatchType::Exact | MatchType::Prefix => 0.0,
                            MatchType::Typo { .. } => 0.6,
                            MatchType::Fuzzy => 0.65,
                        };
                        if m.confidence >= min_confidence {
                            result.targets.push(m.into());
                        }
                    } else if !filtered.is_empty() {
                        // For multiple matches, only include high-confidence ones
                        for m in filtered.into_iter().take(5) {
                            if m.confidence >= 0.5 {
                                result.targets.push(m.into());
                            }
                        }
                        if !result.targets.is_empty() {
                            result.notes.push("Multiple matches found".to_string());
                        }
                    }
                }
                MatchResult::None => {
                    // Try individual tokens, but only accept high-confidence matches
                    for token in &remaining_tokens {
                        match self.matcher.find_entity(token, cache) {
                            MatchResult::Single(m) => {
                                // Only accept if confidence is reasonably high (> 0.7)
                                // This prevents matching "schreibtisch" to "sun"
                                if m.confidence > 0.7 {
                                    result.targets.push(m.into());
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // If no domain hint but action doesn't infer domain, set a default
        // For example, "dim" and "brighten" are light-specific
        if domain_hint.is_none() {
            if let Some(mapping) = action_mapping {
                if !mapping.infers_domain {
                    // Actions like dim/brighten are light-specific
                    domain_hint = Some("light".to_string());
                }
            }
        }

        // If we have area + domain hints but no entities, find all matching
        if result.targets.is_empty() {
            if let Some(ref area) = area_hint {
                let entities = self.matcher.find_entities_in_area(area, cache);
                let filtered: Vec<_> = if let Some(ref domain) = domain_hint {
                    entities
                        .into_iter()
                        .filter(|e| &e.domain == domain)
                        .collect()
                } else {
                    entities
                };

                for entity in filtered.into_iter().take(10) {
                    result.targets.push(ParsedTarget {
                        entity_id: entity.entity_id.clone(),
                        friendly_name: entity.friendly_name.clone(),
                        match_type: "area_match".to_string(),
                        matched_input: area.clone(),
                    });
                }
            }
        }

        // If we have a domain hint but no targets, get all in domain
        // This is a fallback when no specific entity was matched
        if result.targets.is_empty() && domain_hint.is_some() {
            let domain = domain_hint.as_ref().unwrap();
            let entities = self.matcher.find_entities_in_domain(domain, cache);
            let entity_count = entities.len();

            // Only use domain-based fallback if there are a reasonable number of entities
            // Don't target all 50+ lights just because we couldn't find a specific match
            if !entities.is_empty() && entity_count <= 15 {
                for entity in entities.into_iter().take(10) {
                    result.targets.push(ParsedTarget {
                        entity_id: entity.entity_id.clone(),
                        friendly_name: entity.friendly_name.clone(),
                        match_type: "domain_match".to_string(),
                        matched_input: domain.clone(),
                    });
                }
                if !result.notes.is_empty() {
                    result.notes.push(format!(
                        "No specific entity found, targeting all {} entities in domain '{}'",
                        entity_count.min(10),
                        domain
                    ));
                }
            } else if entity_count > 15 {
                result.notes.push(format!(
                    "No specific entity matched. Domain '{}' has {} entities - please be more specific.",
                    domain,
                    entity_count
                ));
            }
        }

        // Calculate confidence
        result.confidence = self.calculate_confidence(&result);

        // Build interpretation string
        result.interpretation = self.build_interpretation(&result, &domain_hint);

        Ok(result)
    }

    fn find_action(&self, token: &str) -> Option<String> {
        self.find_action_with_mapping(token)
            .map(|(action, _)| action)
    }

    fn find_action_with_mapping(&self, token: &str) -> Option<(String, &ActionMapping)> {
        let token_lower = token.to_lowercase();
        for mapping in &self.actions {
            for trigger in &mapping.trigger_words {
                if *trigger == token_lower {
                    return Some((mapping.default_service.to_string(), mapping));
                }
            }
        }
        None
    }

    fn find_domain(&self, token: &str, cache: &Cache) -> Option<String> {
        match self.matcher.find_domain(token, cache) {
            MatchResult::Single(m) => Some(m.item),
            _ => None,
        }
    }

    fn calculate_confidence(&self, result: &ParsedCommand) -> f64 {
        let mut score = 0.0;

        // Action found: +0.3
        if result.action.is_some() {
            score += 0.3;
        }

        // Targets found: +0.4 (scaled by number)
        if !result.targets.is_empty() {
            score += 0.4 * (1.0 / result.targets.len() as f64).min(1.0);
        }

        // Parameters found: +0.2
        if !result.parameters.is_empty() {
            score += 0.2;
        }

        // No warnings: +0.1
        if result.notes.is_empty() {
            score += 0.1;
        }

        score
    }

    fn build_interpretation(&self, result: &ParsedCommand, domain_hint: &Option<String>) -> String {
        let mut parts = Vec::new();

        if let Some(ref action) = result.action {
            parts.push(action.clone());
        }

        if !result.targets.is_empty() {
            let targets: Vec<String> = result
                .targets
                .iter()
                .map(|t| {
                    t.friendly_name
                        .clone()
                        .unwrap_or_else(|| t.entity_id.clone())
                })
                .collect();
            parts.push(targets.join(", "));
        } else if let Some(ref domain) = domain_hint {
            parts.push(format!("all {}s", domain));
        }

        for (key, value) in &result.parameters {
            parts.push(format!("{}={}", key, value));
        }

        parts.join(" ")
    }

    /// Parse service-based command: "call <domain> <service> [entity] [params]"
    fn parse_service_based(
        &self,
        input: &str,
        tokens: &[&str],
        cache: &Cache,
    ) -> Result<ParsedCommand> {
        // Format: call <domain> <service> [rest...]
        // Examples:
        //   call light turn_on kitchen
        //   call switch toggle bedroom_fan
        //   call climate set_temperature --temperature=72

        let domain_token = tokens[1];
        let service_token = tokens[2];

        // Find domain (with fuzzy matching)
        let domain = match self.matcher.find_domain(domain_token, cache) {
            MatchResult::Single(m) => m.item,
            _ => {
                // Try as-is if not found
                domain_token.to_string()
            }
        };

        // Try to find the service using fuzzy matching first
        let full_service_name = format!("{}.{}", domain, service_token);
        let action = if let MatchResult::Single(service_match) =
            self.matcher.find_service(&full_service_name, cache)
        {
            // Use the matched service name
            service_match.item.service.clone()
        } else {
            // Check if this service exists for the domain using cache
            let domain_services = cache.services_for_domain(&domain);
            if let Some(matching_service) = domain_services
                .iter()
                .find(|s| s.eq_ignore_ascii_case(service_token))
            {
                matching_service.clone()
            } else {
                // Look up the action mapping for the service
                match self.find_action(service_token) {
                    Some(a) => a,
                    None => service_token.to_string(), // Use the service name directly
                }
            }
        };

        let mut result = ParsedCommand {
            original: input.to_string(),
            action: Some(action),
            targets: Vec::new(),
            parameters: HashMap::new(),
            confidence: 0.8, // High confidence for explicit service syntax
            interpretation: String::new(),
            notes: Vec::new(),
            matched_area: None,
        };

        // Parse remaining tokens for entity targets and parameters
        let remaining = &tokens[3..];
        let mut remaining_tokens: Vec<&str> = Vec::new();

        for token in remaining {
            // Check if it's a parameter
            if let Some(pct) = parse_percentage(token) {
                result
                    .parameters
                    .insert("brightness_pct".to_string(), pct.into());
                continue;
            }
            if let Some(num) = parse_number(token) {
                result.parameters.insert("value".to_string(), num.into());
                continue;
            }

            // Otherwise it's part of entity name
            remaining_tokens.push(token);
        }

        // Find entities
        if !remaining_tokens.is_empty() {
            let target_string = remaining_tokens.join(" ");
            match self.matcher.find_entity(&target_string, cache) {
                MatchResult::Single(m) => {
                    result.targets.push(m.into());
                }
                MatchResult::Multiple(matches) => {
                    // Filter by domain if possible
                    let filtered: Vec<_> = matches
                        .into_iter()
                        .filter(|m| m.item.domain == domain)
                        .collect();

                    if filtered.len() == 1 {
                        result
                            .targets
                            .push(filtered.into_iter().next().unwrap().into());
                    } else if !filtered.is_empty() {
                        for m in filtered.into_iter().take(5) {
                            result.targets.push(m.into());
                        }
                        result.notes.push("Multiple matches found".to_string());
                    } else {
                        result
                            .notes
                            .push("No entities found in specified domain".to_string());
                    }
                }
                MatchResult::None => {
                    // Try to get all entities in domain
                    let entities = self.matcher.find_entities_in_domain(&domain, cache);
                    for entity in entities.into_iter().take(10) {
                        result.targets.push(ParsedTarget {
                            entity_id: entity.entity_id.clone(),
                            friendly_name: entity.friendly_name.clone(),
                            match_type: "domain_match".to_string(),
                            matched_input: domain.clone(),
                        });
                    }
                    if result.targets.is_empty() {
                        result.notes.push("No entities found".to_string());
                    }
                }
            }
        } else {
            // No specific entity - target all in domain
            let entities = self.matcher.find_entities_in_domain(&domain, cache);
            for entity in entities.into_iter().take(10) {
                result.targets.push(ParsedTarget {
                    entity_id: entity.entity_id.clone(),
                    friendly_name: entity.friendly_name.clone(),
                    match_type: "domain_match".to_string(),
                    matched_input: domain.clone(),
                });
            }
        }

        result.interpretation = format!(
            "{}.{} on {}",
            domain,
            result.action.as_deref().unwrap_or("unknown"),
            if result.targets.is_empty() {
                "all entities".to_string()
            } else {
                format!("{} entities", result.targets.len())
            }
        );

        Ok(result)
    }
}

/// Tokenize input into words, handling punctuation
fn tokenize(input: &str) -> Vec<&str> {
    input
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|s| s.trim_matches(|c: char| !c.is_alphanumeric() && c != '%' && c != '_' && c != '.'))
        .filter(|s| !s.is_empty())
        .filter(|s| !is_stop_word(s))
        .collect()
}

/// Check if a word is a stop word (the, a, an, to, etc.)
fn is_stop_word(word: &str) -> bool {
    matches!(
        word.to_lowercase().as_str(),
        "the" | "a" | "an" | "to" | "in" | "at" | "for" | "and" | "my" | "please"
    )
}

/// Parse a number from a string
fn parse_number(s: &str) -> Option<i64> {
    s.parse().ok()
}

/// Parse a percentage (e.g., "50%", "75")
fn parse_percentage(s: &str) -> Option<i64> {
    let s = s.trim_end_matches('%');
    let num: i64 = s.parse().ok()?;
    if (0..=100).contains(&num) {
        Some(num)
    } else {
        None
    }
}

/// Build a service call from a parsed command
#[derive(Debug, Clone, Serialize)]
pub struct ServiceCall {
    pub domain: String,
    pub service: String,
    pub target: ServiceTarget,
    pub data: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceTarget {
    pub entity_id: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area_id: Option<Vec<String>>,
}

impl ParsedCommand {
    /// Convert to a service call
    pub fn to_service_call(&self) -> Result<ServiceCall> {
        if self.targets.is_empty() {
            return Err(anyhow!("No targets specified"));
        }

        let action = self.action.as_deref().unwrap_or("turn_on");

        // Get domain from first target
        let first_entity = &self.targets[0].entity_id;
        let domain = first_entity
            .split('.')
            .next()
            .ok_or_else(|| anyhow!("Invalid entity ID: {}", first_entity))?
            .to_string();

        // Check if action has domain-specific overrides
        let mappings = action_mappings();
        let service_name = mappings
            .iter()
            .find(|m| m.default_service == action)
            .map(|m| m.service_for_domain(&domain))
            .unwrap_or(action);

        let entity_ids: Vec<String> = self.targets.iter().map(|t| t.entity_id.clone()).collect();

        let mut data = serde_json::Map::new();

        // Convert parameters
        for (key, value) in &self.parameters {
            match key.as_str() {
                "brightness_pct" => {
                    // Convert percentage to 0-255 range
                    if let Some(pct) = value.as_i64() {
                        let brightness = (pct as f64 * 255.0 / 100.0).round() as i64;
                        data.insert("brightness".to_string(), brightness.into());
                    }
                }
                "value" => {
                    // Could be brightness, temperature, etc. - context dependent
                    if domain == "light" {
                        if let Some(val) = value.as_i64() {
                            if val <= 100 {
                                // Treat as percentage
                                let brightness = (val as f64 * 255.0 / 100.0).round() as i64;
                                data.insert("brightness".to_string(), brightness.into());
                            } else {
                                data.insert("brightness".to_string(), value.clone());
                            }
                        }
                    } else if domain == "climate" {
                        data.insert("temperature".to_string(), value.clone());
                    } else {
                        data.insert("value".to_string(), value.clone());
                    }
                }
                _ => {
                    data.insert(key.clone(), value.clone());
                }
            }
        }

        Ok(ServiceCall {
            domain,
            service: service_name.to_string(),
            target: ServiceTarget {
                entity_id: entity_ids,
                area_id: None,
            },
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{Cache, CacheFile, CachedArea, CachedEntity};

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

        let file = CacheFile::new(entities, 3600, "http://localhost:8123".to_string());
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
                aliases: vec![],
                search_names: vec![
                    "living_room".to_string(),
                    "Living Room".to_string(),
                    "living room".to_string(),
                ],
            },
        ];

        let file = CacheFile::new(areas, 3600, "http://localhost:8123".to_string());
        cache.set_areas(file);

        cache
    }

    #[test]
    fn test_tokenize() {
        assert_eq!(
            tokenize("turn on the kitchen light"),
            vec!["turn", "on", "kitchen", "light"]
        );
        assert_eq!(tokenize("kitchen light on"), vec!["kitchen", "light", "on"]);
        assert_eq!(
            tokenize("set brightness to 50%"),
            vec!["set", "brightness", "50%"]
        );
    }

    #[test]
    fn test_tokenize_punctuation() {
        assert_eq!(tokenize("light.kitchen"), vec!["light.kitchen"]);
        assert_eq!(tokenize("kitchen, bedroom"), vec!["kitchen", "bedroom"]);
        assert_eq!(
            tokenize("  multiple   spaces  "),
            vec!["multiple", "spaces"]
        );
    }

    #[test]
    fn test_tokenize_stop_words() {
        // Stop words should be filtered out
        assert_eq!(
            tokenize("please turn on the light"),
            vec!["turn", "on", "light"]
        );
        assert_eq!(tokenize("a light in the kitchen"), vec!["light", "kitchen"]);
    }

    #[test]
    fn test_is_stop_word() {
        assert!(is_stop_word("the"));
        assert!(is_stop_word("The"));
        assert!(is_stop_word("a"));
        assert!(is_stop_word("please"));
        assert!(is_stop_word("my"));
        assert!(!is_stop_word("light"));
        assert!(!is_stop_word("on"));
    }

    #[test]
    fn test_parse_percentage() {
        assert_eq!(parse_percentage("50%"), Some(50));
        assert_eq!(parse_percentage("100"), Some(100));
        assert_eq!(parse_percentage("0%"), Some(0));
        assert_eq!(parse_percentage("150"), None); // Out of range
        assert_eq!(parse_percentage("-10"), None); // Negative
        assert_eq!(parse_percentage("abc"), None);
    }

    #[test]
    fn test_parse_number() {
        assert_eq!(parse_number("42"), Some(42));
        assert_eq!(parse_number("-10"), Some(-10));
        assert_eq!(parse_number("0"), Some(0));
        assert_eq!(parse_number("abc"), None);
        assert_eq!(parse_number("12.5"), None); // Float not supported
    }

    #[test]
    fn test_action_mappings() {
        let mappings = action_mappings();
        assert!(!mappings.is_empty());

        // Check turn_on mapping
        let turn_on = mappings.iter().find(|m| m.trigger_words.contains(&"on"));
        assert!(turn_on.is_some());
        assert_eq!(turn_on.unwrap().default_service, "turn_on");

        // Check turn_off mapping
        let turn_off = mappings.iter().find(|m| m.trigger_words.contains(&"off"));
        assert!(turn_off.is_some());
        assert_eq!(turn_off.unwrap().default_service, "turn_off");

        // Check toggle mapping
        let toggle = mappings
            .iter()
            .find(|m| m.trigger_words.contains(&"toggle"));
        assert!(toggle.is_some());
        assert_eq!(toggle.unwrap().default_service, "toggle");
    }

    #[test]
    fn test_action_mapping_service_for_domain() {
        let mappings = action_mappings();
        let open = mappings
            .iter()
            .find(|m| m.trigger_words.contains(&"open"))
            .unwrap();

        // Default service
        assert_eq!(open.service_for_domain("unknown"), "open_cover");

        // Domain-specific override
        assert_eq!(open.service_for_domain("lock"), "unlock");
        assert_eq!(open.service_for_domain("cover"), "open_cover");
    }

    #[test]
    fn test_nl_parser_new() {
        let parser = NLParser::new();
        assert!(!parser.actions.is_empty());
    }

    #[test]
    fn test_parse_simple_command() {
        let cache = create_test_cache();
        let parser = NLParser::new();

        let result = parser.parse("on kitchen", &cache).unwrap();
        assert_eq!(result.action, Some("turn_on".to_string()));
        assert!(!result.targets.is_empty());
    }

    #[test]
    fn test_parse_with_action_verb() {
        let cache = create_test_cache();
        let parser = NLParser::new();

        let result = parser.parse("turn off kitchen", &cache).unwrap();
        assert_eq!(result.action, Some("turn_off".to_string()));
    }

    #[test]
    fn test_parse_with_percentage() {
        let cache = create_test_cache();
        let parser = NLParser::new();

        let result = parser.parse("kitchen 50%", &cache).unwrap();
        assert!(result.parameters.contains_key("brightness_pct"));
        assert_eq!(result.parameters["brightness_pct"], 50);
    }

    #[test]
    fn test_parse_empty_command() {
        let cache = create_test_cache();
        let parser = NLParser::new();

        let result = parser.parse("", &cache);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_only_stop_words() {
        let cache = create_test_cache();
        let parser = NLParser::new();

        // All stop words should result in empty tokens
        let result = parser.parse("the a an", &cache);
        assert!(result.is_err());
    }

    #[test]
    fn test_parsed_command_to_service_call() {
        let parsed = ParsedCommand {
            original: "on kitchen".to_string(),
            action: Some("turn_on".to_string()),
            targets: vec![ParsedTarget {
                entity_id: "light.kitchen".to_string(),
                friendly_name: Some("Kitchen Light".to_string()),
                match_type: "Exact".to_string(),
                matched_input: "kitchen".to_string(),
            }],
            parameters: HashMap::new(),
            confidence: 1.0,
            interpretation: "turn_on Kitchen Light".to_string(),
            notes: vec![],
            matched_area: None,
        };

        let call = parsed.to_service_call().unwrap();
        assert_eq!(call.domain, "light");
        assert_eq!(call.service, "turn_on");
        assert_eq!(call.target.entity_id, vec!["light.kitchen".to_string()]);
    }

    #[test]
    fn test_parsed_command_to_service_call_no_targets() {
        let parsed = ParsedCommand {
            original: "on".to_string(),
            action: Some("turn_on".to_string()),
            targets: vec![],
            parameters: HashMap::new(),
            confidence: 0.3,
            interpretation: "turn_on".to_string(),
            notes: vec![],
            matched_area: None,
        };

        let result = parsed.to_service_call();
        assert!(result.is_err());
    }

    #[test]
    fn test_parsed_command_brightness_conversion() {
        let mut params = HashMap::new();
        params.insert("brightness_pct".to_string(), serde_json::json!(50));

        let parsed = ParsedCommand {
            original: "on kitchen 50%".to_string(),
            action: Some("turn_on".to_string()),
            targets: vec![ParsedTarget {
                entity_id: "light.kitchen".to_string(),
                friendly_name: None,
                match_type: "Exact".to_string(),
                matched_input: "kitchen".to_string(),
            }],
            parameters: params,
            confidence: 1.0,
            interpretation: "turn_on kitchen 50%".to_string(),
            notes: vec![],
            matched_area: None,
        };

        let call = parsed.to_service_call().unwrap();
        // 50% should convert to brightness ~128
        assert!(call.data.contains_key("brightness"));
        let brightness = call.data["brightness"].as_i64().unwrap();
        assert_eq!(brightness, 128); // 50% of 255 rounded
    }

    #[test]
    fn test_parsed_command_multiple_targets() {
        let parsed = ParsedCommand {
            original: "on lights".to_string(),
            action: Some("turn_on".to_string()),
            targets: vec![
                ParsedTarget {
                    entity_id: "light.kitchen".to_string(),
                    friendly_name: None,
                    match_type: "domain_match".to_string(),
                    matched_input: "lights".to_string(),
                },
                ParsedTarget {
                    entity_id: "light.living_room".to_string(),
                    friendly_name: None,
                    match_type: "domain_match".to_string(),
                    matched_input: "lights".to_string(),
                },
            ],
            parameters: HashMap::new(),
            confidence: 0.7,
            interpretation: "turn_on lights".to_string(),
            notes: vec![],
            matched_area: None,
        };

        let call = parsed.to_service_call().unwrap();
        assert_eq!(call.target.entity_id.len(), 2);
        assert!(call.target.entity_id.contains(&"light.kitchen".to_string()));
        assert!(call
            .target
            .entity_id
            .contains(&"light.living_room".to_string()));
    }
}
