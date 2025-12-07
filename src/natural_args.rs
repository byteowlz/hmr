//! Natural argument preprocessing for CLI commands
//!
//! Transforms natural variations of commands into the canonical form expected by clap:
//! - "hmr list entities" -> "hmr entity list"
//! - "hmr entities list" -> "hmr entity list"
//! - "hmr list entity" -> "hmr entity list"
//! - "hmr show info" -> "hmr info"
//!
//! This makes the CLI more forgiving and natural for both humans and AI agents.

use std::env;

/// Singular to plural mappings for resource types
const PLURALS: &[(&str, &str)] = &[
    ("entity", "entities"),
    ("service", "services"),
    ("event", "events"),
    ("area", "areas"),
    ("device", "devices"),
];

/// Common action verbs that can be reordered
const ACTIONS: &[&str] = &["list", "show", "get", "call", "watch", "create", "delete", "update", "assign"];

/// Resource types that support actions
const RESOURCES: &[&str] = &[
    "entity", "entities",
    "service", "services", 
    "event", "events",
    "area", "areas",
    "device", "devices",
    "cache",
    "config",
    "history",
    "template",
];

/// Normalize command arguments to canonical form
///
/// Handles:
/// - Singular/plural variations (entity/entities)
/// - Reordering (list entities -> entity list)
/// - Common aliases (show -> info, get -> list)
pub fn normalize_args() -> Vec<String> {
    let args: Vec<String> = env::args().collect();
    
    // Need at least: program_name + command
    if args.len() < 2 {
        return args;
    }
    
    // Skip the program name
    let program_name = args[0].clone();
    let rest = &args[1..];
    
    // Normalize the command portion
    let normalized = normalize_command(rest);
    
    // Rebuild args
    let mut result = vec![program_name];
    result.extend(normalized);
    result
}

fn normalize_command(args: &[String]) -> Vec<String> {
    if args.is_empty() {
        return args.to_vec();
    }
    
    let first = args[0].to_lowercase();
    
    // Handle standalone commands (info, completions)
    if first == "info" || first == "completions" {
        return args.to_vec();
    }
    
    // Normalize action aliases (show -> list, get -> list)
    let normalized_action = normalize_action(&first);
    
    // Check if first word is an action
    if ACTIONS.contains(&normalized_action.as_str()) {
        // Pattern: <action> <resource> [args...]
        // Transform to: <resource> <action> [args...]
        if args.len() >= 2 {
            let second = args[1].to_lowercase();
            
            // Normalize resource (plural -> singular for canonical form)
            let resource = normalize_resource(&second);
            
            if RESOURCES.contains(&resource.as_str()) {
                let mut result = vec![resource, normalized_action];
                result.extend_from_slice(&args[2..]);
                return result;
            }
        }
    }
    
    // Check if first word is a resource
    let normalized_first = normalize_resource(&first);
    if RESOURCES.contains(&normalized_first.as_str()) {
        let mut result = vec![normalized_first];
        // Also normalize any action in the second position
        if args.len() >= 2 {
            let second_action = normalize_action(&args[1].to_lowercase());
            result.push(second_action);
            result.extend_from_slice(&args[2..]);
        } else {
            result.extend_from_slice(&args[1..]);
        }
        return result;
    }
    
    // Special case: "show info" -> "info"
    if (first == "show" || first == "get") && args.len() >= 2 && args[1].to_lowercase() == "info" {
        return vec!["info".to_string()];
    }
    
    // No transformation needed
    args.to_vec()
}

/// Normalize action aliases
fn normalize_action(action: &str) -> String {
    match action {
        "show" => "list".to_string(),
        "get" => "list".to_string(),
        "display" => "list".to_string(),
        _ => action.to_string(),
    }
}

/// Normalize a resource name (handle singular/plural)
fn normalize_resource(resource: &str) -> String {
    let lower = resource.to_lowercase();
    
    // Check if it's already singular
    for (singular, plural) in PLURALS {
        if lower == *plural {
            return singular.to_string();
        }
        if lower == *singular {
            return singular.to_string();
        }
    }
    
    // Return as-is (might be cache, config, etc.)
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize_test_args(args: &[&str]) -> Vec<String> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        normalize_command(&args)
    }

    #[test]
    fn test_list_entities_to_entity_list() {
        let result = normalize_test_args(&["list", "entities"]);
        assert_eq!(result, vec!["entity", "list"]);
    }

    #[test]
    fn test_entities_list_to_entity_list() {
        let result = normalize_test_args(&["entities", "list"]);
        assert_eq!(result, vec!["entity", "list"]);
    }

    #[test]
    fn test_list_entity_to_entity_list() {
        let result = normalize_test_args(&["list", "entity"]);
        assert_eq!(result, vec!["entity", "list"]);
    }

    #[test]
    fn test_show_services_to_service_list() {
        let result = normalize_test_args(&["show", "services"]);
        assert_eq!(result, vec!["service", "list"]);
    }

    #[test]
    fn test_entity_list_unchanged() {
        let result = normalize_test_args(&["entity", "list"]);
        assert_eq!(result, vec!["entity", "list"]);
    }

    #[test]
    fn test_list_areas_to_area_list() {
        let result = normalize_test_args(&["list", "areas"]);
        assert_eq!(result, vec!["area", "list"]);
    }

    #[test]
    fn test_watch_events_to_event_watch() {
        let result = normalize_test_args(&["watch", "events"]);
        assert_eq!(result, vec!["event", "watch"]);
    }

    #[test]
    fn test_show_info_to_info() {
        let result = normalize_test_args(&["show", "info"]);
        assert_eq!(result, vec!["info"]);
    }

    #[test]
    fn test_get_info_to_info() {
        let result = normalize_test_args(&["get", "info"]);
        assert_eq!(result, vec!["info"]);
    }

    #[test]
    fn test_preserves_additional_args() {
        let result = normalize_test_args(&["list", "entities", "--filter", "light"]);
        assert_eq!(result, vec!["entity", "list", "--filter", "light"]);
    }

    #[test]
    fn test_cache_unchanged() {
        let result = normalize_test_args(&["cache", "status"]);
        assert_eq!(result, vec!["cache", "status"]);
    }

    #[test]
    fn test_config_unchanged() {
        let result = normalize_test_args(&["config", "path"]);
        assert_eq!(result, vec!["config", "path"]);
    }

    #[test]
    fn test_show_as_list_alias() {
        let result = normalize_test_args(&["show", "entities"]);
        assert_eq!(result, vec!["entity", "list"]);
    }

    #[test]
    fn test_get_as_list_alias() {
        let result = normalize_test_args(&["get", "services"]);
        assert_eq!(result, vec!["service", "list"]);
    }

    #[test]
    fn test_display_as_list_alias() {
        let result = normalize_test_args(&["display", "areas"]);
        assert_eq!(result, vec!["area", "list"]);
    }

    #[test]
    fn test_entity_show_to_entity_list() {
        let result = normalize_test_args(&["entity", "show"]);
        assert_eq!(result, vec!["entity", "list"]);
    }

    #[test]
    fn test_entities_get_to_entity_list() {
        let result = normalize_test_args(&["entities", "get"]);
        assert_eq!(result, vec!["entity", "list"]);
    }

    #[test]
    fn test_list_device_to_device_list() {
        let result = normalize_test_args(&["list", "device"]);
        assert_eq!(result, vec!["device", "list"]);
    }

    #[test]
    fn test_devices_list_to_device_list() {
        let result = normalize_test_args(&["devices", "list"]);
        assert_eq!(result, vec!["device", "list"]);
    }
}
