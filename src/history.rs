//! Command history and context memory
//!
//! Tracks:
//! - Command history for recall and "again" functionality
//! - Current context for follow-up commands (e.g., "brighter" after "turn on kitchen light")
//! - Accuracy statistics for fuzzy matching improvement

use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as AnyhowContext, Result};
use serde::{Deserialize, Serialize};

const APP_NAME: &str = env!("CARGO_PKG_NAME");

/// How long context remains valid (5 minutes)
const CONTEXT_TTL_SECS: u64 = 300;

/// Maximum history entries to keep
const MAX_HISTORY_ENTRIES: usize = 1000;

/// A single history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix timestamp
    pub timestamp: u64,
    /// Original input string
    pub input: String,
    /// What it was interpreted as
    pub interpretation: String,
    /// Service that was called
    pub service: Option<String>,
    /// Entities that were targeted
    pub targets: Vec<String>,
    /// Whether the command succeeded
    pub success: bool,
    /// Any error message
    pub error: Option<String>,
    /// Match type used (exact, fuzzy, typo)
    pub match_type: Option<String>,
}

impl HistoryEntry {
    pub fn new(input: &str, interpretation: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            timestamp,
            input: input.to_string(),
            interpretation: interpretation.to_string(),
            service: None,
            targets: Vec::new(),
            success: false,
            error: None,
            match_type: None,
        }
    }

    pub fn with_service(mut self, domain: &str, service: &str) -> Self {
        self.service = Some(format!("{}.{}", domain, service));
        self
    }

    pub fn with_targets(mut self, targets: Vec<String>) -> Self {
        self.targets = targets;
        self
    }

    pub fn with_success(mut self) -> Self {
        self.success = true;
        self
    }

    pub fn with_error(mut self, error: &str) -> Self {
        self.error = Some(error.to_string());
        self
    }

    pub fn with_match_type(mut self, match_type: &str) -> Self {
        self.match_type = Some(match_type.to_string());
        self
    }
}

/// Current command context for follow-up commands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandContext {
    /// Last entities that were targeted
    pub last_entities: Vec<String>,
    /// Last area that was referenced
    pub last_area: Option<String>,
    /// Last domain that was used
    pub last_domain: Option<String>,
    /// Last action that was performed
    pub last_action: Option<String>,
    /// When the context was last updated
    pub updated_at: u64,
}

impl Default for CommandContext {
    fn default() -> Self {
        Self {
            last_entities: Vec::new(),
            last_area: None,
            last_domain: None,
            last_action: None,
            updated_at: 0,
        }
    }
}

impl CommandContext {
    /// Check if context is still valid (not expired)
    pub fn is_valid(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now < self.updated_at + CONTEXT_TTL_SECS
    }

    /// Update context from a command execution
    pub fn update(
        &mut self,
        entities: Vec<String>,
        area: Option<String>,
        domain: Option<String>,
        action: Option<String>,
    ) {
        self.last_entities = entities;
        self.last_area = area;
        self.last_domain = domain;
        self.last_action = action;
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Get age of context
    pub fn age(&self) -> Duration {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Duration::from_secs(now.saturating_sub(self.updated_at))
    }
}

/// Accuracy statistics for fuzzy matching
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccuracyStats {
    /// Total commands executed
    pub total_commands: u64,
    /// Commands with exact matches
    pub exact_matches: u64,
    /// Commands with fuzzy matches
    pub fuzzy_matches: u64,
    /// Commands with typo corrections
    pub typo_corrections: u64,
    /// Commands that required user clarification
    pub ambiguous_prompts: u64,
    /// Commands that failed
    pub failures: u64,
    /// Common typo corrections (typo -> correction)
    pub correction_map: HashMap<String, String>,
    /// Most frequently used entities
    pub entity_frequency: HashMap<String, u64>,
}

impl AccuracyStats {
    pub fn record_exact(&mut self) {
        self.total_commands += 1;
        self.exact_matches += 1;
    }

    pub fn record_fuzzy(&mut self) {
        self.total_commands += 1;
        self.fuzzy_matches += 1;
    }

    pub fn record_typo(&mut self, original: &str, corrected: &str) {
        self.total_commands += 1;
        self.typo_corrections += 1;
        self.correction_map
            .insert(original.to_lowercase(), corrected.to_lowercase());
    }

    pub fn record_ambiguous(&mut self) {
        self.ambiguous_prompts += 1;
    }

    pub fn record_failure(&mut self) {
        self.failures += 1;
    }

    pub fn record_entity_use(&mut self, entity_id: &str) {
        *self
            .entity_frequency
            .entry(entity_id.to_string())
            .or_default() += 1;
    }

    /// Get success rate as percentage
    pub fn success_rate(&self) -> f64 {
        if self.total_commands == 0 {
            100.0
        } else {
            let successes = self.total_commands - self.failures;
            (successes as f64 / self.total_commands as f64) * 100.0
        }
    }

    /// Get top N most used entities
    pub fn top_entities(&self, n: usize) -> Vec<(&String, &u64)> {
        let mut entries: Vec<_> = self.entity_frequency.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1));
        entries.into_iter().take(n).collect()
    }
}

/// History manager
pub struct History {
    history_path: PathBuf,
    context_path: PathBuf,
    stats_path: PathBuf,
    context: CommandContext,
    stats: AccuracyStats,
}

impl History {
    /// Create a new history manager
    pub fn new() -> Result<Self> {
        let state_dir = state_dir()?;
        fs::create_dir_all(&state_dir)
            .with_context(|| format!("creating state directory {}", state_dir.display()))?;

        let history_path = state_dir.join("history.jsonl");
        let context_path = state_dir.join("context.json");
        let stats_path = state_dir.join("stats.json");

        // Load context
        let context = if context_path.exists() {
            let content = fs::read_to_string(&context_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            CommandContext::default()
        };

        // Load stats
        let stats = if stats_path.exists() {
            let content = fs::read_to_string(&stats_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            AccuracyStats::default()
        };

        Ok(Self {
            history_path,
            context_path,
            stats_path,
            context,
            stats,
        })
    }

    /// Get current context (if valid)
    pub fn context(&self) -> Option<&CommandContext> {
        if self.context.is_valid() {
            Some(&self.context)
        } else {
            None
        }
    }

    /// Get accuracy stats
    pub fn stats(&self) -> &AccuracyStats {
        &self.stats
    }

    /// Get mutable stats
    pub fn stats_mut(&mut self) -> &mut AccuracyStats {
        &mut self.stats
    }

    /// Update context
    pub fn update_context(
        &mut self,
        entities: Vec<String>,
        area: Option<String>,
        domain: Option<String>,
        action: Option<String>,
    ) -> Result<()> {
        self.context.update(entities, area, domain, action);
        self.save_context()
    }

    /// Clear context
    pub fn clear_context(&mut self) -> Result<()> {
        self.context = CommandContext::default();
        self.save_context()
    }

    /// Save context to disk
    fn save_context(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.context)?;
        fs::write(&self.context_path, content)
            .with_context(|| format!("writing context to {}", self.context_path.display()))
    }

    /// Save stats to disk
    pub fn save_stats(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.stats)?;
        fs::write(&self.stats_path, content)
            .with_context(|| format!("writing stats to {}", self.stats_path.display()))
    }

    /// Append a history entry
    pub fn append(&self, entry: &HistoryEntry) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.history_path)
            .with_context(|| format!("opening history file {}", self.history_path.display()))?;

        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;

        Ok(())
    }

    /// Get recent history entries
    pub fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        if !self.history_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.history_path)?;
        let reader = BufReader::new(file);

        let entries: Vec<HistoryEntry> = reader
            .lines()
            .filter_map(|line| line.ok())
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect();

        // Return last N entries
        let start = entries.len().saturating_sub(limit);
        Ok(entries[start..].to_vec())
    }

    /// Get the most recent entry
    pub fn last_entry(&self) -> Result<Option<HistoryEntry>> {
        let entries = self.recent(1)?;
        Ok(entries.into_iter().next())
    }

    /// Search history by pattern
    pub fn search(&self, pattern: &str) -> Result<Vec<HistoryEntry>> {
        if !self.history_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.history_path)?;
        let reader = BufReader::new(file);
        let pattern_lower = pattern.to_lowercase();

        let entries: Vec<HistoryEntry> = reader
            .lines()
            .filter_map(|line| line.ok())
            .filter_map(|line| serde_json::from_str::<HistoryEntry>(&line).ok())
            .filter(|e| {
                e.input.to_lowercase().contains(&pattern_lower)
                    || e.interpretation.to_lowercase().contains(&pattern_lower)
                    || e.targets
                        .iter()
                        .any(|t| t.to_lowercase().contains(&pattern_lower))
            })
            .collect();

        Ok(entries)
    }

    /// Clear all history
    pub fn clear(&self) -> Result<()> {
        if self.history_path.exists() {
            fs::remove_file(&self.history_path)?;
        }
        Ok(())
    }

    /// Compact history to keep only recent entries
    pub fn compact(&self) -> Result<usize> {
        if !self.history_path.exists() {
            return Ok(0);
        }

        let file = File::open(&self.history_path)?;
        let reader = BufReader::new(file);

        let entries: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();
        let original_count = entries.len();

        if entries.len() <= MAX_HISTORY_ENTRIES {
            return Ok(0);
        }

        // Keep only the last MAX_HISTORY_ENTRIES
        let start = entries.len() - MAX_HISTORY_ENTRIES;
        let kept_entries = &entries[start..];

        // Write back
        let mut file = File::create(&self.history_path)?;
        for entry in kept_entries {
            writeln!(file, "{}", entry)?;
        }

        Ok(original_count - MAX_HISTORY_ENTRIES)
    }
}

/// Get the state directory path (for context and stats)
fn state_dir() -> Result<PathBuf> {
    // Check XDG_STATE_HOME first
    if let Some(dir) = env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir).join(APP_NAME));
    }

    // Use platform-specific state directory
    if let Some(mut dir) = dirs::state_dir() {
        dir.push(APP_NAME);
        return Ok(dir);
    }

    // Fallback to ~/.local/state
    dirs::home_dir()
        .map(|home| home.join(".local").join("state").join(APP_NAME))
        .ok_or_else(|| anyhow::anyhow!("unable to determine state directory"))
}

/// Get the history file path
pub fn history_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("history.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_history_entry_creation() {
        let entry = HistoryEntry::new("turn on kitchen light", "turn_on kitchen light")
            .with_service("light", "turn_on")
            .with_targets(vec!["light.kitchen".to_string()])
            .with_success();

        assert_eq!(entry.input, "turn on kitchen light");
        assert_eq!(entry.service, Some("light.turn_on".to_string()));
        assert!(entry.success);
        assert!(entry.timestamp > 0);
    }

    #[test]
    fn test_history_entry_with_error() {
        let entry = HistoryEntry::new("bad command", "").with_error("No matching entities");

        assert!(!entry.success);
        assert_eq!(entry.error, Some("No matching entities".to_string()));
    }

    #[test]
    fn test_history_entry_with_match_type() {
        let entry = HistoryEntry::new("ligth on", "light on").with_match_type("Typo");

        assert_eq!(entry.match_type, Some("Typo".to_string()));
    }

    #[test]
    fn test_context_default() {
        let ctx = CommandContext::default();
        assert!(ctx.last_entities.is_empty());
        assert!(ctx.last_area.is_none());
        assert!(ctx.last_domain.is_none());
        assert!(ctx.last_action.is_none());
        assert_eq!(ctx.updated_at, 0);
    }

    #[test]
    fn test_context_validity() {
        let mut ctx = CommandContext::default();
        assert!(!ctx.is_valid()); // Not updated yet

        ctx.update(
            vec!["light.kitchen".to_string()],
            Some("kitchen".to_string()),
            Some("light".to_string()),
            Some("turn_on".to_string()),
        );
        assert!(ctx.is_valid()); // Just updated
    }

    #[test]
    fn test_context_update() {
        let mut ctx = CommandContext::default();

        ctx.update(
            vec!["light.kitchen".to_string(), "light.bedroom".to_string()],
            Some("kitchen".to_string()),
            Some("light".to_string()),
            Some("turn_on".to_string()),
        );

        assert_eq!(ctx.last_entities.len(), 2);
        assert_eq!(ctx.last_area, Some("kitchen".to_string()));
        assert_eq!(ctx.last_domain, Some("light".to_string()));
        assert_eq!(ctx.last_action, Some("turn_on".to_string()));
        assert!(ctx.updated_at > 0);
    }

    #[test]
    fn test_context_age() {
        let mut ctx = CommandContext::default();
        ctx.update(vec![], None, None, None);

        // Age should be very small (just updated)
        assert!(ctx.age().as_secs() < 2);
    }

    #[test]
    fn test_accuracy_stats_default() {
        let stats = AccuracyStats::default();
        assert_eq!(stats.total_commands, 0);
        assert_eq!(stats.exact_matches, 0);
        assert_eq!(stats.success_rate(), 100.0); // No commands = 100% success
    }

    #[test]
    fn test_accuracy_stats() {
        let mut stats = AccuracyStats::default();

        stats.record_exact();
        stats.record_exact();
        stats.record_fuzzy();
        stats.record_failure();

        assert_eq!(stats.total_commands, 3); // failure doesn't increment total
        assert_eq!(stats.exact_matches, 2);
        assert_eq!(stats.fuzzy_matches, 1);
        assert_eq!(stats.failures, 1);
        assert!((stats.success_rate() - 66.66).abs() < 1.0);
    }

    #[test]
    fn test_accuracy_stats_typo() {
        let mut stats = AccuracyStats::default();

        stats.record_typo("ligth", "light");
        stats.record_typo("kitchn", "kitchen");

        assert_eq!(stats.typo_corrections, 2);
        assert_eq!(
            stats.correction_map.get("ligth"),
            Some(&"light".to_string())
        );
        assert_eq!(
            stats.correction_map.get("kitchn"),
            Some(&"kitchen".to_string())
        );
    }

    #[test]
    fn test_accuracy_stats_entity_frequency() {
        let mut stats = AccuracyStats::default();

        stats.record_entity_use("light.kitchen");
        stats.record_entity_use("light.kitchen");
        stats.record_entity_use("light.bedroom");

        assert_eq!(*stats.entity_frequency.get("light.kitchen").unwrap(), 2);
        assert_eq!(*stats.entity_frequency.get("light.bedroom").unwrap(), 1);
    }

    #[test]
    fn test_accuracy_stats_top_entities() {
        let mut stats = AccuracyStats::default();

        stats.record_entity_use("light.kitchen");
        stats.record_entity_use("light.kitchen");
        stats.record_entity_use("light.kitchen");
        stats.record_entity_use("light.bedroom");
        stats.record_entity_use("light.bedroom");
        stats.record_entity_use("switch.outlet");

        let top = stats.top_entities(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "light.kitchen");
        assert_eq!(*top[0].1, 3);
        assert_eq!(top[1].0, "light.bedroom");
        assert_eq!(*top[1].1, 2);
    }

    #[test]
    fn test_accuracy_stats_ambiguous() {
        let mut stats = AccuracyStats::default();

        stats.record_ambiguous();
        stats.record_ambiguous();

        assert_eq!(stats.ambiguous_prompts, 2);
    }

    #[test]
    fn test_state_dir() {
        let dir = state_dir().unwrap();
        assert!(dir.to_string_lossy().contains("hmr"));
    }

    #[test]
    fn test_history_path() {
        let path = history_path().unwrap();
        assert!(path.to_string_lossy().contains("hmr"));
        assert!(path.to_string_lossy().ends_with("history.jsonl"));
    }
}
