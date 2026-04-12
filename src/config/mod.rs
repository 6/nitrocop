pub mod gem_path;
pub mod lockfile;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::RegexSet;
use serde_yml::Value;

use crate::cop::registry::CopRegistry;
use crate::cop::{CopConfig, EnabledState};
use crate::diagnostic::Severity;

/// Policy for handling `Enabled: pending` cops, controlled by `AllCops.NewCops`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewCopsPolicy {
    Enable,
    Disable,
}

/// Department-level configuration (e.g., `RSpec:`, `Rails:`).
///
/// Plugin default configs use bare department keys to set Include/Exclude
/// patterns and Enabled state for all cops in that department.
#[derive(Debug, Clone, Default)]
struct DepartmentConfig {
    enabled: EnabledState,
    include: Vec<String>,
    exclude: Vec<String>,
}

/// Controls how arrays are merged during config inheritance.
///
/// By default, Exclude arrays are appended and Include arrays are replaced.
/// `inherit_mode` lets configs override this per-key.
#[derive(Debug, Clone, Default)]
struct InheritMode {
    /// Keys whose arrays should be appended (merged) instead of replaced.
    merge: HashSet<String>,
    /// Keys whose arrays should be replaced instead of appended.
    override_keys: HashSet<String>,
}

/// Pre-compiled glob filter for a single cop.
///
/// Built once at startup from resolved config + cop defaults. Avoids
/// recompiling glob patterns on every `is_cop_enabled` call.
pub struct CopFilter {
    enabled: bool,
    include_set: Option<GlobSet>, // None = match all files
    exclude_set: Option<GlobSet>, // None = exclude no files
    include_re: Option<RegexSet>, // Ruby regexp include patterns
    exclude_re: Option<RegexSet>, // Ruby regexp exclude patterns
}

impl CopFilter {
    /// Returns true if the cop is enabled in config (Enabled: true).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check whether this cop should run on the given file path.
    pub fn is_match(&self, path: &Path) -> bool {
        if !self.enabled {
            return false;
        }
        if let Some(ref inc) = self.include_set {
            if !inc.is_match(path) {
                return false;
            }
        }
        if let Some(ref exc) = self.exclude_set {
            if exc.is_match(path) {
                return false;
            }
        }
        true
    }

    /// Returns true if this cop always matches any file (enabled, no Include/Exclude patterns).
    /// Universal cops can skip per-file glob matching entirely.
    pub fn is_universal(&self) -> bool {
        self.enabled
            && self.include_set.is_none()
            && self.exclude_set.is_none()
            && self.include_re.is_none()
            && self.exclude_re.is_none()
    }

    /// Check whether the given path matches this cop's Include patterns.
    fn is_included(&self, path: &Path) -> bool {
        let has_globs = self.include_set.is_some();
        let has_regexes = self.include_re.is_some();
        if !has_globs && !has_regexes {
            return true; // no Include = match all
        }
        let path_str = path.to_string_lossy();
        if let Some(ref inc) = self.include_set {
            if inc.is_match(path) {
                return true;
            }
        }
        if let Some(ref re) = self.include_re {
            if re.is_match(path_str.as_ref()) {
                return true;
            }
        }
        false
    }

    /// Check whether the given path matches this cop's Exclude patterns.
    fn is_excluded(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        if let Some(ref exc) = self.exclude_set {
            if exc.is_match(path) {
                return true;
            }
        }
        if let Some(ref re) = self.exclude_re {
            if re.is_match(path_str.as_ref()) {
                return true;
            }
        }
        false
    }
}

/// Pre-compiled filter set for all cops + global excludes.
///
/// Built once from `ResolvedConfig` + `CopRegistry`, then shared across
/// all rayon worker threads. Eliminates per-file glob compilation overhead.
pub struct CopFilterSet {
    global_exclude: GlobSet,
    global_exclude_patterns: Vec<String>, // Raw glob patterns for Ruby-like confirmation
    global_exclude_re: Option<RegexSet>,  // Ruby regexp global exclude patterns
    filters: Vec<CopFilter>,              // indexed by cop position in registry
    /// Config directory for relativizing file paths before glob matching.
    /// Cop Include/Exclude patterns are relative to the project root
    /// (where `.rubocop.yml` lives), but file paths may include a prefix
    /// when running from outside the project root.
    config_dir: Option<PathBuf>,
    /// Base directory for path pattern resolution (RuboCop's `base_dir_for_path_parameters`).
    /// For `.rubocop*` configs: config file's parent dir (absolute).
    /// For other configs (e.g., `baseline_rubocop.yml`): current working directory.
    /// Used to relativize absolute file paths before glob matching.
    base_dir: Option<PathBuf>,
    /// Root of the scanned target repo/directory, if one was provided on the CLI.
    /// For external configs, some cop Include patterns (for example
    /// `db/**/*.rb`) still need to match relative to the inspected repo root
    /// rather than the caller's CWD. Do not use this for Exclude matching:
    /// RuboCop resolves Exclude patterns relative to the config file/base_dir,
    /// not the scan target.
    scan_root: Option<PathBuf>,
    /// Sub-directories containing their own `.rubocop.yml` files.
    /// Sorted deepest-first so `nearest_config_dir` finds the most specific match.
    /// RuboCop resolves Include/Exclude patterns relative to the nearest config
    /// directory, so files in `db/migrate/` with a local `.rubocop.yml` have their
    /// paths relativized to `db/migrate/` rather than the project root.
    sub_config_dirs: Vec<PathBuf>,
    /// Indices of cops that always match any file (enabled, no Include/Exclude).
    /// These skip per-file glob matching entirely in the filter loop.
    universal_cop_indices: Vec<usize>,
    /// Indices of enabled cops that need per-file Include/Exclude pattern matching.
    pattern_cop_indices: Vec<usize>,
    /// AllCops.MigratedSchemaVersion — when set, files whose basename contains a
    /// 14-digit run that is <= this value have all offenses suppressed.
    /// Implements rubocop-rails' MigrationFileSkippable behavior.
    migrated_schema_version: Option<String>,
}

impl CopFilterSet {
    /// Set the scan root for Include-pattern resolution.
    /// Called by the linter when a target directory is provided on the CLI.
    pub fn set_scan_root(&mut self, root: PathBuf) {
        self.scan_root = Some(root);
    }

    fn matches_global_exclude_glob(&self, path: &Path) -> bool {
        self.global_exclude.is_match(path)
            && self
                .global_exclude_patterns
                .iter()
                .any(|pattern| glob_matches(pattern, path))
    }

    /// Check whether a file is globally excluded (AllCops.Exclude).
    pub fn is_globally_excluded(&self, path: &Path) -> bool {
        if self.matches_global_exclude_glob(path) {
            return true;
        }
        // Check Ruby regexp patterns against the path string
        if let Some(ref re) = self.global_exclude_re {
            let path_str = path.to_string_lossy();
            if re.is_match(path_str.as_ref()) {
                return true;
            }
        }
        // Strip `./` prefix: file discovery produces `./vendor/foo.rb` but
        // exclude patterns use `vendor/**/*`.
        if let Ok(stripped) = path.strip_prefix("./") {
            if self.matches_global_exclude_glob(stripped) {
                return true;
            }
            if let Some(ref re) = self.global_exclude_re {
                let stripped_str = stripped.to_string_lossy();
                if re.is_match(stripped_str.as_ref()) {
                    return true;
                }
            }
        }
        // Try matching against the path relativized to base_dir and config_dir.
        // base_dir follows RuboCop's `base_dir_for_path_parameters`:
        //   - `.rubocop*` configs → config file's parent (absolute)
        //   - other configs → current working directory
        // This handles absolute file paths (e.g., `/abs/path/vendor/foo.rb`
        // needs to match a pattern like `vendor/**`).
        // NOTE: Do NOT add a "scan_roots" fallback that resolves AllCops.Exclude
        // patterns relative to CLI target directories. RuboCop resolves these
        // patterns relative to the config file's directory (base_dir/config_dir),
        // not the scan target. Adding scan_roots caused smoke test regressions:
        // when `--config bench/corpus/baseline.yml /tmp/repo` was used, patterns
        // like `vendor/**/*` incorrectly excluded `/tmp/repo/vendor/` files that
        // RuboCop still lints (since RuboCop resolves relative to the config dir).
        // See commit a44c8fc2 (reverted) for the failed attempt.
        for dir in [&self.base_dir, &self.config_dir].into_iter().flatten() {
            if let Ok(rel) = path.strip_prefix(dir) {
                if self.matches_global_exclude_glob(rel) {
                    return true;
                }
                if let Some(ref re) = self.global_exclude_re {
                    let rel_str = rel.to_string_lossy();
                    if re.is_match(rel_str.as_ref()) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Check if a file is a "migrated" migration file that should have all offenses
    /// suppressed. Implements rubocop-rails' MigrationFileSkippable behavior:
    /// extracts the first 14-digit run from the basename and compares it to
    /// `AllCops.MigratedSchemaVersion` (string comparison).
    pub fn is_migrated_file(&self, path: &Path) -> bool {
        let Some(ref version) = self.migrated_schema_version else {
            return false;
        };
        let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Match Ruby's /(?<timestamp>\d{14})/ — first run of exactly 14+ digits
        let bytes = basename.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit() {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i - start >= 14 {
                    // Take the first 14 digits (Ruby's named capture gets the full match,
                    // but the comparison is string-based so longer runs still work)
                    let timestamp = &basename[start..start + 14];
                    return timestamp <= version.as_str();
                }
            } else {
                i += 1;
            }
        }
        false
    }

    /// Get the pre-compiled filter for a cop by its registry index.
    pub fn cop_filter(&self, index: usize) -> &CopFilter {
        &self.filters[index]
    }

    /// Indices of cops that always match any file (enabled, no Include/Exclude).
    pub fn universal_cop_indices(&self) -> &[usize] {
        &self.universal_cop_indices
    }

    /// Indices of enabled cops that need per-file Include/Exclude pattern matching.
    pub fn pattern_cop_indices(&self) -> &[usize] {
        &self.pattern_cop_indices
    }

    /// Find the nearest sub-config directory for a file path.
    /// Returns the deepest `.rubocop.yml` directory that is an ancestor of `path`,
    /// falling back to the root `config_dir`.
    fn nearest_config_dir(&self, path: &Path) -> Option<&Path> {
        // sub_config_dirs is sorted deepest-first, so the first match is most specific
        for dir in &self.sub_config_dirs {
            if path.starts_with(dir) {
                return Some(dir.as_path());
            }
        }
        self.config_dir.as_deref()
    }

    /// Check whether a cop (by registry index) should run on the given file.
    /// Checks both the original path and the path relativized to the nearest
    /// config directory (supports per-directory `.rubocop.yml` path relativity):
    /// - Include: matches if EITHER path matches (supports absolute + relative patterns)
    /// - Exclude: matches only RuboCop-resolved config/base-dir path forms
    pub fn is_cop_match(&self, index: usize, path: &Path) -> bool {
        let filter = &self.filters[index];
        if !filter.enabled {
            return false;
        }

        let rel_path = self
            .nearest_config_dir(path)
            .and_then(|cd| path.strip_prefix(cd).ok());

        // Also relativize against base_dir when it differs from config_dir.
        // For non-.rubocop configs (e.g., baseline_rubocop.yml), base_dir is cwd,
        // which may differ from the config file's parent directory.
        let rel_to_base = self.base_dir.as_deref().and_then(|bd| {
            // Skip if base_dir == config_dir (already covered by rel_path)
            if self.config_dir.as_deref() == Some(bd) {
                return None;
            }
            path.strip_prefix(bd).ok()
        });
        // Match against the explicit scan root when one was provided by the CLI.
        // This fixes external baseline configs that lint an out-of-tree repo:
        // plugin defaults like `db/**/*.rb` must still be relative to the repo
        // being scanned, not just the config file's base_dir or the process CWD.
        let rel_to_scan_root = self
            .scan_root
            .as_deref()
            .and_then(|root| path.strip_prefix(root).ok());

        // Strip `./` prefix for matching: file discovery produces `./test/foo.rb`
        // but cop Exclude patterns use `test/**/*`. Without stripping, patterns
        // that don't start with `./` won't match.
        let stripped = path.strip_prefix("./").ok();

        // Include: file must match on at least one path form.
        // This supports both absolute patterns (/tmp/test/db/**) and
        // relative patterns (db/migrate/**).
        let included = filter.is_included(path)
            || rel_path.is_some_and(|rel| filter.is_included(rel))
            || rel_to_base.is_some_and(|rel| filter.is_included(rel))
            || rel_to_scan_root.is_some_and(|rel| filter.is_included(rel))
            || stripped.is_some_and(|s| filter.is_included(s));
        if !included {
            return false;
        }

        // Exclude: file is excluded if EITHER RuboCop-resolved path form
        // matches. Do NOT use scan_root here: external configs resolve Exclude
        // patterns from the config/base dir, not the scanned repo root.
        let excluded = filter.is_excluded(path)
            || rel_path.is_some_and(|rel| filter.is_excluded(rel))
            || rel_to_base.is_some_and(|rel| filter.is_excluded(rel))
            || stripped.is_some_and(|s| filter.is_excluded(s));
        if excluded {
            return false;
        }

        true
    }

    /// Check whether a cop is explicitly excluded from a file by its Exclude
    /// patterns. Only checks Exclude — does NOT check Include patterns or
    /// enabled status. Used for RedundantCopDisableDirective to distinguish
    /// "cop excluded from this file" (safe to flag) from "cop didn't match
    /// Include" (sub-config path issues, not safe to flag).
    pub fn is_cop_excluded(&self, index: usize, path: &Path) -> bool {
        let filter = &self.filters[index];

        let nearest_dir = self.nearest_config_dir(path);
        let rel_to_nearest = nearest_dir.and_then(|cd| path.strip_prefix(cd).ok());

        // Also check against the root config dir for Exclude patterns
        // relative to the project root (e.g., **/app/controllers/**/*.rb).
        let rel_to_root = self
            .config_dir
            .as_deref()
            .filter(|root| nearest_dir.is_some_and(|n| n != *root))
            .and_then(|cd| path.strip_prefix(cd).ok());

        // Also try base_dir when it differs from config_dir.
        let rel_to_base = self
            .base_dir
            .as_deref()
            .filter(|bd| self.config_dir.as_deref() != Some(*bd))
            .and_then(|bd| path.strip_prefix(bd).ok());
        let stripped = path.strip_prefix("./").ok();
        filter.is_excluded(path)
            || rel_to_nearest.is_some_and(|rel| filter.is_excluded(rel))
            || rel_to_root.is_some_and(|rel| filter.is_excluded(rel))
            || rel_to_base.is_some_and(|rel| filter.is_excluded(rel))
            || stripped.is_some_and(|s| filter.is_excluded(s))
    }

    /// Check whether a file path would be matched (not excluded) by a cop's
    /// Include/Exclude patterns from its `CopConfig`. This is used for cops
    /// NOT in the registry (unimplemented cops from gem configs) to determine
    /// if a `rubocop:disable` directive is redundant.
    ///
    /// Returns true if the file WOULD be matched (cop would run on it),
    /// false if the file is excluded by Include/Exclude patterns.
    pub fn is_path_matched_by_cop_config(&self, cop_config: &CopConfig, path: &Path) -> bool {
        let include_pats: Vec<&str> = cop_config.include.iter().map(|s| s.as_str()).collect();
        let exclude_pats: Vec<&str> = cop_config.exclude.iter().map(|s| s.as_str()).collect();
        let include_set = build_glob_set(&include_pats);
        let exclude_set = build_glob_set(&exclude_pats);
        let include_re = build_regex_set(&include_pats);
        let exclude_re = build_regex_set(&exclude_pats);

        let rel_path = self
            .nearest_config_dir(path)
            .and_then(|cd| path.strip_prefix(cd).ok());
        let rel_to_base = self
            .base_dir
            .as_deref()
            .filter(|bd| self.config_dir.as_deref() != Some(*bd))
            .and_then(|bd| path.strip_prefix(bd).ok());
        // Include check: if patterns exist, file must match at least one form
        let has_include = include_set.is_some() || include_re.is_some();
        if has_include {
            let path_str = path.to_string_lossy();
            let glob_match = include_set.as_ref().is_some_and(|inc| {
                inc.is_match(path)
                    || rel_path.is_some_and(|rel| inc.is_match(rel))
                    || rel_to_base.is_some_and(|rel| inc.is_match(rel))
            });
            let re_match = include_re.as_ref().is_some_and(|re| {
                re.is_match(path_str.as_ref())
                    || rel_path.is_some_and(|rel| re.is_match(&rel.to_string_lossy()))
                    || rel_to_base.is_some_and(|rel| re.is_match(&rel.to_string_lossy()))
            });
            if !glob_match && !re_match {
                return false;
            }
        }

        // Exclude check: file is excluded if either path form matches
        if let Some(ref exc) = exclude_set {
            let excluded = exc.is_match(path)
                || rel_path.is_some_and(|rel| exc.is_match(rel))
                || rel_to_base.is_some_and(|rel| exc.is_match(rel));
            if excluded {
                return false;
            }
        }
        if let Some(ref re) = exclude_re {
            let path_str = path.to_string_lossy();
            let excluded = re.is_match(path_str.as_ref())
                || rel_path.is_some_and(|rel| re.is_match(&rel.to_string_lossy()))
                || rel_to_base.is_some_and(|rel| re.is_match(&rel.to_string_lossy()));
            if excluded {
                return false;
            }
        }

        true
    }
}

/// Walk the project tree and find directories containing `.rubocop.yml` files
/// (excluding the root). Returns directories sorted deepest-first so that
/// `nearest_config_dir` finds the most specific match first.
fn discover_sub_config_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .build();

    for entry in walker.flatten() {
        if entry.file_type().is_some_and(|ft| ft.is_file()) && entry.file_name() == ".rubocop.yml" {
            if let Some(parent) = entry.path().parent() {
                // Skip the root directory itself
                if parent != root {
                    dirs.push(parent.to_path_buf());
                }
            }
        }
    }

    // Sort deepest-first: longer paths first
    dirs.sort_by_key(|b| std::cmp::Reverse(b.as_os_str().len()));
    dirs
}

/// Load per-directory config layers from nested `.rubocop.yml` files.
///
/// For each subdirectory containing a `.rubocop.yml`, parses the local config
/// keys from that file (without following `inherit_from`). The returned layer
/// is later merged on top of the root resolved config for files under that
/// directory, so nested `Enabled`, `DisabledByDefault`, department settings,
/// and path filters affect execution.
/// Returns a list of (directory, config_layer) pairs sorted deepest-first.
fn load_dir_overrides(root: &Path) -> Vec<(PathBuf, ConfigLayer)> {
    let sub_dirs = discover_sub_config_dirs(root);
    let mut overrides = Vec::new();

    for dir in sub_dirs {
        let config_path = dir.join(".rubocop.yml");
        let contents = match std::fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let contents = contents.replace("!ruby/regexp ", "");
        let raw: Value = match serde_yml::from_str(&contents) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "warning: failed to parse nested config {}: {e}",
                    config_path.display()
                );
                continue;
            }
        };
        let layer = parse_config_layer(&raw);
        let has_effect = !layer.cop_configs.is_empty()
            || !layer.department_configs.is_empty()
            || !layer.global_excludes.is_empty()
            || layer.new_cops.is_some()
            || layer.disabled_by_default.is_some()
            || layer.target_ruby_version.is_some()
            || layer.target_rails_version.is_some()
            || layer.active_support_extensions_enabled.is_some()
            || layer.migrated_schema_version.is_some();
        if has_effect {
            overrides.push((dir, layer));
        }
    }

    overrides
}

/// Check if a pattern string is a Ruby regexp (from `!ruby/regexp /pattern/`).
/// Returns the inner regex pattern if it is, stripping the surrounding `/` delimiters.
fn extract_ruby_regexp(s: &str) -> Option<&str> {
    let s = s.trim();
    if s.starts_with('/') && s.len() > 1 {
        // Find the closing `/`, which may be followed by flags like `i`, `x`, `m`
        if let Some(end) = s[1..].rfind('/') {
            // After the closing `/`, only valid Ruby regex flags may follow.
            // If anything else is present (e.g., `*` from a glob), this is an
            // absolute filesystem path, not a regexp.
            let after_close = &s[end + 2..];
            if after_close
                .bytes()
                .all(|b| matches!(b, b'i' | b'x' | b'm' | b'u' | b's' | b'n' | b'o' | b'e'))
            {
                return Some(&s[1..end + 1]);
            }
        }
    }
    None
}

/// Build a `GlobSet` from a list of pattern strings, skipping any that are
/// Ruby regexp patterns (these are handled separately by `build_regex_set`).
/// Returns `None` if no glob patterns remain.
fn build_glob_set(patterns: &[&str]) -> Option<GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    let mut count = 0;
    for pat in patterns {
        if extract_ruby_regexp(pat).is_some() {
            continue; // Skip regex patterns — handled by build_regex_set
        }
        if let Ok(glob) = GlobBuilder::new(pat).literal_separator(true).build() {
            builder.add(glob);
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    builder.build().ok()
}

/// Build a `RegexSet` from Ruby regexp patterns in the list.
/// Only patterns that look like `/pattern/` are included.
/// Returns `None` if no regex patterns are found.
fn build_regex_set(patterns: &[&str]) -> Option<RegexSet> {
    let regexes: Vec<&str> = patterns
        .iter()
        .filter_map(|p| extract_ruby_regexp(p))
        .collect();
    if regexes.is_empty() {
        return None;
    }
    RegexSet::new(&regexes).ok()
}

/// Resolved configuration from .rubocop.yml with full inheritance support.
///
/// Supports `inherit_from` (local YAML files), `inherit_gem` (via
/// `bundle info --path`), `require:` (plugin default configs), department-level
/// configs, `Enabled: pending` / `AllCops.NewCops`, `AllCops.DisabledByDefault`,
/// and `inherit_mode`.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Per-cop configs keyed by cop name (e.g. "Style/FrozenStringLiteralComment")
    cop_configs: HashMap<String, CopConfig>,
    /// Department-level configs keyed by department name (e.g. "RSpec", "Rails")
    department_configs: HashMap<String, DepartmentConfig>,
    global_excludes: Vec<String>,
    /// Directory containing the resolved config file (for relative path resolution).
    config_dir: Option<PathBuf>,
    /// How to handle `Enabled: pending` cops.
    new_cops: NewCopsPolicy,
    /// When true, cops without explicit `Enabled: true` are disabled.
    disabled_by_default: bool,
    /// All cop names mentioned in `require:` gem default configs.
    /// Cops from plugin departments not in this set are treated as non-existent
    /// (the installed gem version doesn't include them).
    require_known_cops: HashSet<String>,
    /// Department names that had gems loaded via `require:`.
    require_departments: HashSet<String>,
    /// Target Ruby version from AllCops.TargetRubyVersion (e.g. 3.1, 3.2).
    /// None means not specified (cops should default to 2.7 per RuboCop convention).
    target_ruby_version: Option<f64>,
    /// Target Rails version from AllCops.TargetRailsVersion (e.g. 7.1, 8.0).
    /// None means not specified (cops should default to 5.0 per RuboCop convention).
    target_rails_version: Option<f64>,
    /// Whether ActiveSupport extensions are enabled (AllCops.ActiveSupportExtensionsEnabled).
    /// Set to true by rubocop-rails. Affects cops like Style/CollectionQuerying.
    active_support_extensions_enabled: bool,
    /// All cop names found in the installed rubocop gem's config/default.yml.
    /// When non-empty, core cops (Layout, Lint, Style, etc.) not in this set
    /// are treated as non-existent in the project's rubocop version.
    rubocop_known_cops: HashSet<String>,
    /// Cops mentioned in the project config layer (inherit_from, inherit_gem,
    /// local config — but NOT from require: gem defaults).
    /// Used by department-level Enabled:false to distinguish user-explicit cops
    /// from rubocop default cops.
    project_mentioned_cops: HashSet<String>,
    /// Departments mentioned in the project config layer (inherit_from,
    /// inherit_gem, local config — but NOT from require: gem defaults).
    project_mentioned_depts: HashSet<String>,
    /// Departments that have `Enabled: true` explicitly in the project config.
    /// Distinguished from departments merely mentioned with other keys (e.g., Exclude).
    /// Used for DisabledByDefault: cops in these departments get their default
    /// enabled state restored (matching RuboCop's handle_disabled_by_default).
    project_enabled_depts: HashSet<String>,
    /// Per-directory config layers from nested `.rubocop.yml` files.
    /// Keyed by directory path (sorted deepest-first for lookup).
    dir_overrides: Vec<(PathBuf, ConfigLayer)>,
    /// Whether the `railties` gem was found in the project's Gemfile.lock.
    /// RuboCop 1.84+ uses `requires_gem 'railties'` to gate Rails cops — if
    /// `railties` is not in the lockfile, cops with `minimum_target_rails_version`
    /// are silently disabled regardless of `TargetRailsVersion` in config.
    railties_in_lockfile: bool,
    /// The `rack` gem version from Gemfile.lock (e.g. 3.1 for "3.1.8").
    /// Used by `Rails/HttpStatusNameConsistency` and `RSpecRails/HttpStatusNameConsistency`
    /// which require `rack >= 3.1.0` (via RuboCop's `requires_gem 'rack', '>= 3.1.0'`).
    rack_version: Option<f64>,
    /// Base directory for resolving Include/Exclude path patterns.
    /// RuboCop's `base_dir_for_path_parameters`: if the config filename starts
    /// with `.rubocop`, this is the config file's parent (canonical). Otherwise
    /// (e.g., `baseline_rubocop.yml`), this is the current working directory.
    /// This distinction matters because non-dotfile configs use cwd-relative patterns.
    base_dir: Option<PathBuf>,
    /// AllCops.MigratedSchemaVersion from rubocop-rails.
    /// When set, files whose basename contains a 14-digit "timestamp" <= this value
    /// have ALL offenses suppressed (rubocop-rails' MigrationFileSkippable).
    /// Default sentinel from rubocop-rails: `'19700101000000'`.
    migrated_schema_version: Option<String>,
}

impl ResolvedConfig {
    pub fn empty() -> Self {
        Self {
            cop_configs: HashMap::new(),
            department_configs: HashMap::new(),
            global_excludes: Vec::new(),
            config_dir: None,
            new_cops: NewCopsPolicy::Disable,
            disabled_by_default: false,
            require_known_cops: HashSet::new(),
            require_departments: HashSet::new(),
            target_ruby_version: None,
            target_rails_version: None,
            active_support_extensions_enabled: false,
            rubocop_known_cops: HashSet::new(),
            project_mentioned_cops: HashSet::new(),
            project_mentioned_depts: HashSet::new(),
            project_enabled_depts: HashSet::new(),
            dir_overrides: Vec::new(),
            railties_in_lockfile: false,
            rack_version: None,
            base_dir: None,
            migrated_schema_version: None,
        }
    }

    /// Register plugin departments so their cops are enabled during
    /// `build_cop_filters`. Used with `--force-default-config --only` to
    /// ensure plugin cops (RSpec, Rails, etc.) run in isolation.
    pub fn register_departments_from_only(&mut self, only: &[String]) {
        for cop_name in only {
            if let Some(dept) = cop_name.split('/').next() {
                if is_plugin_department(dept) {
                    self.require_departments.insert(dept.to_string());
                }
            }
        }
    }
}

/// A single parsed config layer (before merging).
#[derive(Debug, Clone)]
struct ConfigLayer {
    cop_configs: HashMap<String, CopConfig>,
    department_configs: HashMap<String, DepartmentConfig>,
    global_excludes: Vec<String>,
    new_cops: Option<String>,
    disabled_by_default: Option<bool>,
    inherit_mode: InheritMode,
    /// Cop names where Enabled:true came from `require:` gem defaults
    /// (used to distinguish user-explicit enables from gem defaults under DisabledByDefault).
    require_enabled_cops: HashSet<String>,
    /// Department names where Enabled:true came from `require:` gem defaults.
    require_enabled_depts: HashSet<String>,
    /// ALL cop names mentioned in `require:` gem default configs (regardless of enabled state).
    /// Used to determine which cops exist in the installed gem version.
    require_known_cops: HashSet<String>,
    /// Department names that had gems loaded via `require:`.
    require_departments: HashSet<String>,
    /// Cop names explicitly mentioned in user config files (inherit_from, inherit_gem,
    /// local config), as opposed to only from `require:` gem defaults.
    user_mentioned_cops: HashSet<String>,
    /// Same for departments.
    user_mentioned_depts: HashSet<String>,
    /// Target Ruby version from AllCops.TargetRubyVersion.
    target_ruby_version: Option<f64>,
    /// Target Rails version from AllCops.TargetRailsVersion.
    target_rails_version: Option<f64>,
    /// AllCops.ActiveSupportExtensionsEnabled (set by rubocop-rails).
    active_support_extensions_enabled: Option<bool>,
    /// AllCops.MigratedSchemaVersion (set by rubocop-rails).
    /// When set, files whose basename contains a 14+ digit "timestamp" <= this value
    /// have ALL offenses suppressed (MigrationFileSkippable).
    migrated_schema_version: Option<String>,
}

impl ConfigLayer {
    fn empty() -> Self {
        Self {
            cop_configs: HashMap::new(),
            department_configs: HashMap::new(),
            global_excludes: Vec::new(),
            new_cops: None,
            disabled_by_default: None,
            inherit_mode: InheritMode::default(),
            require_enabled_cops: HashSet::new(),
            require_enabled_depts: HashSet::new(),
            require_known_cops: HashSet::new(),
            user_mentioned_cops: HashSet::new(),
            user_mentioned_depts: HashSet::new(),
            require_departments: HashSet::new(),
            target_ruby_version: None,
            target_rails_version: None,
            active_support_extensions_enabled: None,
            migrated_schema_version: None,
        }
    }
}

/// Walk up from `start_dir` looking for a config file name.
fn walk_up_for(start_dir: &Path, filename: &str) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let candidate = dir.join(filename);
        if candidate.exists() {
            return Some(candidate);
        }
        if !dir.pop() {
            break;
        }
    }
    // Fallback: try with canonicalized path (resolves symlinks, ..)
    if let Ok(canonical) = std::fs::canonicalize(start_dir) {
        if canonical != start_dir {
            let mut dir = canonical;
            loop {
                let candidate = dir.join(filename);
                if candidate.exists() {
                    return Some(candidate);
                }
                if !dir.pop() {
                    break;
                }
            }
        }
    }
    None
}

/// Walk up from `start_dir` to find `.rubocop.yml`, falling back to
/// `.standard.yml` for pure-standardrb projects.
///
/// First tries with the original path (preserving relative paths), then falls
/// back to canonicalized path to handle symlinks and `..` components.
fn find_config(start_dir: &Path) -> Option<PathBuf> {
    // Prefer .rubocop.yml
    if let Some(path) = walk_up_for(start_dir, ".rubocop.yml") {
        return Some(path);
    }
    // Fallback: .standard.yml for pure-standardrb projects
    walk_up_for(start_dir, ".standard.yml")
}

/// Convert a `.standard.yml` file into a synthetic `.rubocop.yml`-compatible
/// YAML string. This allows the existing config loading pipeline to handle
/// pure-standardrb projects without modification.
///
/// Mapping:
///   ruby_version: X.Y         → AllCops.TargetRubyVersion: X.Y
///   plugins: [standard-rails] → require: [standard, standard-rails]
///   ignore: [patterns]        → AllCops.Exclude + per-cop Enabled: false
///   extend_config: [files]    → inherit_from: [files]
fn convert_standard_yml(standard_path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(standard_path)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", standard_path.display()))?;
    let doc: serde_yml::Value = serde_yml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", standard_path.display()))?;
    let empty_mapping = serde_yml::Mapping::new();
    let map = doc.as_mapping().unwrap_or(&empty_mapping);

    let mut lines: Vec<String> = Vec::new();

    // require: always include "standard" itself, plus any plugins
    let mut requires = vec!["standard".to_string()];
    if let Some(plugins) = map.get(serde_yml::Value::String("plugins".into())) {
        if let Some(seq) = plugins.as_sequence() {
            for v in seq {
                if let Some(s) = v.as_str() {
                    requires.push(s.to_string());
                }
            }
        }
    }
    lines.push(format!(
        "require:\n{}",
        requires
            .iter()
            .map(|r| format!("  - {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    ));

    // AllCops section
    let mut all_cops_lines: Vec<String> = Vec::new();

    // ruby_version → AllCops.TargetRubyVersion
    if let Some(rv) = map.get(serde_yml::Value::String("ruby_version".into())) {
        if let Some(f) = rv.as_f64() {
            all_cops_lines.push(format!("  TargetRubyVersion: {f}"));
        } else if let Some(s) = rv.as_str() {
            all_cops_lines.push(format!("  TargetRubyVersion: {s}"));
        }
    }

    // Standard's DEFAULT_IGNORES: always applied unless `default_ignores: false`.
    // These match the Ruby standard gem's ConfiguresIgnoredPaths::DEFAULT_IGNORES.
    // Note: .git/**/*, node_modules/**/*, vendor/**/*, tmp/**/* are already in
    // RuboCop's own AllCops.Exclude defaults, so we only need the standard-specific ones.
    let default_ignores_disabled = map
        .get(serde_yml::Value::String("default_ignores".into()))
        .and_then(|v| v.as_bool())
        == Some(false);

    // ignore → AllCops.Exclude (simple patterns) + per-cop disables
    let mut exclude_patterns: Vec<String> = Vec::new();
    if !default_ignores_disabled {
        exclude_patterns.push("bin/*".to_string());
        exclude_patterns.push("db/schema.rb".to_string());
    }
    let mut cop_disables: Vec<(String, String)> = Vec::new(); // (cop_name, glob)
    if let Some(ignore) = map.get(serde_yml::Value::String("ignore".into())) {
        if let Some(seq) = ignore.as_sequence() {
            for item in seq {
                match item {
                    // Simple string pattern → AllCops.Exclude
                    serde_yml::Value::String(pattern) => {
                        exclude_patterns.push(pattern.clone());
                    }
                    // Mapping: { 'glob': [cop_names] } → per-cop disable/exclude
                    serde_yml::Value::Mapping(m) => {
                        for (k, v) in m {
                            if let (Some(glob), Some(cops)) = (k.as_str(), v.as_sequence()) {
                                for cop in cops {
                                    if let Some(cop_name) = cop.as_str() {
                                        cop_disables.push((cop_name.to_string(), glob.to_string()));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if !exclude_patterns.is_empty() {
        all_cops_lines.push("  Exclude:".into());
        for p in &exclude_patterns {
            all_cops_lines.push(format!("    - '{p}'"));
        }
    }

    if !all_cops_lines.is_empty() {
        lines.push(format!("AllCops:\n{}", all_cops_lines.join("\n")));
    }

    // Per-cop disables from ignore glob+cop entries
    for (cop_name, glob) in &cop_disables {
        if glob == "**/*" {
            // Global disable
            lines.push(format!("{cop_name}:\n  Enabled: false"));
        } else {
            lines.push(format!("{cop_name}:\n  Exclude:\n    - '{glob}'"));
        }
    }

    // Standard's ignores are appended on top of plugin gem configs (like standard-rails)
    // rather than replacing them. Emit inherit_mode to merge Exclude arrays so that
    // AllCops.Exclude from standard-rails (e.g., db/*schema.rb) is preserved.
    if !exclude_patterns.is_empty() {
        lines.push("inherit_mode:\n  merge:\n    - Exclude".to_string());
    }

    // extend_config → inherit_from
    if let Some(extend) = map.get(serde_yml::Value::String("extend_config".into())) {
        if let Some(seq) = extend.as_sequence() {
            let files: Vec<String> = seq
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if !files.is_empty() {
                lines.push(format!(
                    "inherit_from:\n{}",
                    files
                        .iter()
                        .map(|f| format!("  - {f}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ));
            }
        }
    }

    Ok(lines.join("\n\n"))
}

/// Load config from the given path, or auto-discover `.rubocop.yml` by walking
/// up from `target_dir`. Returns an empty config if no config file is found.
///
/// Resolves `inherit_from`, `inherit_gem`, and `require:` recursively, merging
/// layers bottom-up with RuboCop-compatible merge rules.
pub fn load_config(
    path: Option<&Path>,
    target_dir: Option<&Path>,
    gem_cache: Option<&HashMap<String, PathBuf>>,
) -> Result<ResolvedConfig> {
    let start_dir = target_dir
        .map(|p| {
            if p.is_file() {
                p.parent().unwrap_or(p).to_path_buf()
            } else {
                p.to_path_buf()
            }
        })
        .or_else(|| std::env::current_dir().ok());

    let config_path = match path {
        Some(p) => {
            if p.exists() {
                Some(p.to_path_buf())
            } else {
                return Ok(ResolvedConfig::empty());
            }
        }
        None => start_dir.as_ref().and_then(|dir| find_config(dir)),
    };

    let config_path = match config_path {
        Some(p) => p,
        None => {
            let Some(config_dir) = start_dir else {
                return Ok(ResolvedConfig::empty());
            };
            // With no config file, RuboCop still resolves path parameters from
            // the current working directory. This matters for explicit file
            // paths in subdirectories: lockfile-derived project metadata (for
            // example TargetRailsVersion from Gemfile.lock) must come from the
            // repo root CWD, not the file's own parent directory.
            let base_dir = std::env::current_dir().unwrap_or_else(|_| config_dir.clone());
            // Even without a config file, apply RuboCop's default AllCops.Exclude
            // patterns (vendor/**/* etc.). RuboCop always loads config/default.yml
            // which includes these excludes, regardless of whether a project has
            // .rubocop.yml. Without this, repos without config files get zero file
            // exclusion, causing false positives on vendored code.
            let defaults = fallback_default_excludes();
            let mut railties_in_lockfile = false;
            let mut target_rails_version = None;
            let mut rack_version = None;
            for lock_name in &["Gemfile.lock", "gems.locked"] {
                let lock_path = base_dir.join(lock_name);
                if let Ok(content) = std::fs::read_to_string(&lock_path) {
                    if target_rails_version.is_none() {
                        if let Some(ver) = parse_gem_version_from_lockfile(&content, "railties") {
                            railties_in_lockfile = true;
                            target_rails_version = Some(ver);
                        }
                    }
                    if rack_version.is_none() {
                        rack_version = parse_gem_version_from_lockfile(&content, "rack");
                    }
                    if target_rails_version.is_some() && rack_version.is_some() {
                        break;
                    }
                }
            }
            return Ok(ResolvedConfig {
                config_dir: Some(config_dir.clone()),
                dir_overrides: load_dir_overrides(&config_dir),
                base_dir: Some(base_dir),
                global_excludes: defaults.global_excludes,
                target_rails_version,
                railties_in_lockfile,
                rack_version,
                ..ResolvedConfig::empty()
            });
        }
    };

    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    // RuboCop's `base_dir_for_path_parameters`: config files named `.rubocop*`
    // resolve Include/Exclude patterns relative to the config file's directory.
    // All other config files (e.g., `baseline_rubocop.yml`) resolve relative to
    // the current working directory. This matters for `--config path/to/custom.yml`.
    let base_dir = {
        let is_rubocop_dotfile = config_path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|name| name.starts_with(".rubocop"));
        if is_rubocop_dotfile {
            // Canonicalize to absolute path (matching RuboCop's File.expand_path)
            config_dir
                .canonicalize()
                .unwrap_or_else(|_| config_dir.clone())
        } else {
            std::env::current_dir().unwrap_or_else(|_| config_dir.clone())
        }
    };

    // Load rubocop's own config/default.yml as the lowest-priority base layer.
    // This provides correct default Enabled states, EnforcedStyle values, etc.
    // Also collect the set of known cops for version awareness.
    let (mut base, rubocop_known_cops) = try_load_rubocop_defaults(&config_dir, gem_cache);

    let mut visited = HashSet::new();
    let is_standard_yml = config_path
        .file_name()
        .is_some_and(|f| f == ".standard.yml");
    let project_layer = if is_standard_yml {
        let synthetic_yaml = convert_standard_yml(&config_path)?;
        // Use config_path so inherit_from / gem resolution is relative to .standard.yml's dir
        load_config_recursive_inner(
            &config_path,
            &config_dir,
            &mut visited,
            gem_cache,
            Some(&synthetic_yaml),
        )?
    } else {
        load_config_recursive(&config_path, &config_dir, &mut visited, gem_cache)?
    };

    // Collect cop/department names explicitly mentioned in user config files
    // (inherit_from, inherit_gem, local config), excluding require: gem defaults.
    let project_mentioned_cops = project_layer.user_mentioned_cops.clone();
    let project_mentioned_depts = project_layer.user_mentioned_depts.clone();

    // Collect departments that explicitly have Enabled: true in the project config,
    // EXCLUDING departments where Enabled: true came from require:/plugins: gem defaults.
    // This distinguishes "Security: Enabled: true" (user wrote it) from
    // "Performance: Enabled: true" (came from rubocop-performance gem defaults).
    // Used for DisabledByDefault: when a department is user-enabled, its
    // default-enabled cops should be restored (matching RuboCop's handle_disabled_by_default).
    let project_enabled_depts: HashSet<String> = project_layer
        .department_configs
        .iter()
        .filter(|(name, cfg)| {
            cfg.enabled == EnabledState::True
                && !project_layer.require_enabled_depts.contains(name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();

    // Merge project config on top of rubocop defaults
    merge_layer_into(&mut base, &project_layer, None);

    let disabled_by_default = base.disabled_by_default.unwrap_or(false);

    // DisabledByDefault handling: for each cop/department in rubocop's defaults
    // that is NOT mentioned in the user config, reset Enabled to Unset.
    // This matches RuboCop's handle_disabled_by_default behavior where require:
    // gem defaults go into default_config (disabled), while cops in user config
    // keep their enabled state.
    if disabled_by_default {
        for (cop_name, cop_cfg) in base.cop_configs.iter_mut() {
            if cop_cfg.enabled == EnabledState::True && !project_mentioned_cops.contains(cop_name) {
                cop_cfg.enabled = EnabledState::Unset;
            }
        }
        for (dept_name, dept_cfg) in base.department_configs.iter_mut() {
            if dept_cfg.enabled == EnabledState::True
                && !project_mentioned_depts.contains(dept_name)
            {
                dept_cfg.enabled = EnabledState::Unset;
            }
        }
    }

    // Fall back to gemspec required_ruby_version, then .ruby-version file.
    // RuboCop's resolution order: EnvVar → Config → Gemspec → .ruby-version → ...
    let target_ruby_version = base
        .target_ruby_version
        .or_else(|| resolve_ruby_version_from_gemspec(&config_dir))
        .or_else(|| {
            let ruby_version_path = config_dir.join(".ruby-version");
            if let Ok(content) = std::fs::read_to_string(&ruby_version_path) {
                let trimmed = content.trim();
                // Parse version like "3.4.4" -> 3.4
                let parts: Vec<&str> = trimmed.split('.').collect();
                if parts.len() >= 2 {
                    if let (Ok(major), Ok(minor)) =
                        (parts[0].parse::<u64>(), parts[1].parse::<u64>())
                    {
                        return Some(major as f64 + minor as f64 / 10.0);
                    }
                }
            }
            None
        });

    // Fall back to Gemfile.lock if TargetRailsVersion wasn't set in config.
    // RuboCop looks for the 'railties' gem in the lockfile.
    //
    // Use `base_dir` (not `config_dir`) for lockfile resolution to match
    // RuboCop's `bundler_lock_file_path` which uses `base_dir_for_path_parameters`:
    // CWD for non-dotfile configs (e.g. `--config baseline_rubocop.yml`),
    // config file's parent for `.rubocop*` dotfiles. This prevents reading
    // a Gemfile.lock from an unrelated directory when `--config` points to
    // a config file outside the target project (e.g. corpus oracle CI).
    let lockfile_dir = &base_dir;
    let mut railties_in_lockfile = false;
    let target_rails_version = base.target_rails_version.or_else(|| {
        for lock_name in &["Gemfile.lock", "gems.locked"] {
            let lock_path = lockfile_dir.join(lock_name);
            if let Ok(content) = std::fs::read_to_string(&lock_path) {
                if let Some(ver) = parse_gem_version_from_lockfile(&content, "railties") {
                    railties_in_lockfile = true;
                    return Some(ver);
                }
            }
        }
        None
    });

    // If TargetRailsVersion was set in config (not from lockfile), still check
    // the lockfile for railties presence. RuboCop 1.84+ uses `requires_gem
    // 'railties'` which gates cops based on actual lockfile presence, independent
    // of the TargetRailsVersion config option.
    if !railties_in_lockfile && base.target_rails_version.is_some() {
        for lock_name in &["Gemfile.lock", "gems.locked"] {
            let lock_path = lockfile_dir.join(lock_name);
            if let Ok(content) = std::fs::read_to_string(&lock_path) {
                if parse_gem_version_from_lockfile(&content, "railties").is_some() {
                    railties_in_lockfile = true;
                    break;
                }
            }
        }
    }

    // Parse rack gem version from lockfile for HttpStatusNameConsistency cops.
    // RuboCop uses `requires_gem 'rack', '>= 3.1.0'` to gate these cops.
    let mut rack_version: Option<f64> = None;
    for lock_name in &["Gemfile.lock", "gems.locked"] {
        let lock_path = lockfile_dir.join(lock_name);
        if let Ok(content) = std::fs::read_to_string(&lock_path) {
            if let Some(ver) = parse_gem_version_from_lockfile(&content, "rack") {
                rack_version = Some(ver);
                break;
            }
        }
    }

    // Discover and parse nested .rubocop.yml files for per-directory config layers.
    let dir_overrides = load_dir_overrides(&config_dir);

    Ok(ResolvedConfig {
        cop_configs: base.cop_configs,
        department_configs: base.department_configs,
        global_excludes: base.global_excludes,
        config_dir: Some(config_dir),
        new_cops: match base.new_cops.as_deref() {
            Some("enable") => NewCopsPolicy::Enable,
            _ => NewCopsPolicy::Disable,
        },
        disabled_by_default,
        require_known_cops: base.require_known_cops,
        require_departments: base.require_departments,
        // Default to 2.7 if no TargetRubyVersion resolved — matches RuboCop's default.
        target_ruby_version: target_ruby_version.or(Some(2.7)),
        target_rails_version,
        active_support_extensions_enabled: base.active_support_extensions_enabled.unwrap_or(false),
        rubocop_known_cops,
        project_mentioned_cops,
        project_mentioned_depts,
        project_enabled_depts,
        dir_overrides,
        railties_in_lockfile,
        rack_version,
        base_dir: Some(base_dir),
        migrated_schema_version: base.migrated_schema_version,
    })
}

/// Try to load rubocop's own `config/default.yml` as the base config layer.
///
/// This provides correct default Enabled states (52 cops disabled by default),
/// EnforcedStyle values, and other option defaults for all cops. Returns an
/// empty layer if the rubocop gem is not installed or the file can't be parsed.
///
/// Also returns the set of all cop names found in the installed gem's config,
/// used for core cop version awareness (cops not in the installed gem don't exist).
/// RuboCop's built-in AllCops.Exclude defaults. Used as a fallback when the
/// rubocop gem can't be found (e.g., corpus repos without their own bundle).
/// These match the patterns in rubocop's `config/default.yml`.
/// Minimal set of patterns matching rubocop's config/default.yml AllCops.Exclude.
fn fallback_default_excludes() -> ConfigLayer {
    let mut layer = ConfigLayer::empty();
    layer.global_excludes = vec![
        "node_modules/**/*".to_string(),
        "tmp/**/*".to_string(),
        "vendor/**/*".to_string(),
        ".git/**/*".to_string(),
    ];
    layer
}

fn try_load_rubocop_defaults(
    working_dir: &Path,
    gem_cache: Option<&HashMap<String, PathBuf>>,
) -> (ConfigLayer, HashSet<String>) {
    let gem_root = if let Some(path) = gem_cache.and_then(|c| c.get("rubocop")) {
        path.clone()
    } else {
        match gem_path::resolve_gem_path("rubocop", working_dir) {
            Ok(p) => p,
            Err(_) => return (fallback_default_excludes(), HashSet::new()),
        }
    };

    let default_config = gem_root.join("config").join("default.yml");
    if !default_config.exists() {
        return (fallback_default_excludes(), HashSet::new());
    }

    let contents = match std::fs::read_to_string(&default_config) {
        Ok(c) => c,
        Err(_) => return (fallback_default_excludes(), HashSet::new()),
    };

    // Strip Ruby-specific YAML tags (e.g., !ruby/regexp) that serde_yml can't handle
    let contents = contents.replace("!ruby/regexp ", "");

    let raw: Value = match serde_yml::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "warning: failed to parse rubocop default config {}: {e}",
                default_config.display()
            );
            return (fallback_default_excludes(), HashSet::new());
        }
    };

    // Collect all cop names (keys containing '/') from the config.
    let known_cops: HashSet<String> = if let Value::Mapping(ref map) = raw {
        map.keys()
            .filter_map(|k| k.as_str())
            .filter(|k| k.contains('/'))
            .map(|k| k.to_string())
            .collect()
    } else {
        HashSet::new()
    };

    (parse_config_layer(&raw), known_cops)
}

/// Recursively load a config file and all its inherited configs.
///
/// `working_dir` is the top-level config directory used for gem path resolution
/// (where `Gemfile.lock` typically lives).
/// `visited` tracks absolute paths to detect circular inheritance.
/// `override_contents` — if Some, use this YAML string instead of reading from disk
/// (used for synthetic configs generated from `.standard.yml`).
fn load_config_recursive(
    config_path: &Path,
    working_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    gem_cache: Option<&HashMap<String, PathBuf>>,
) -> Result<ConfigLayer> {
    load_config_recursive_inner(config_path, working_dir, visited, gem_cache, None)
}

fn load_config_recursive_inner(
    config_path: &Path,
    working_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    gem_cache: Option<&HashMap<String, PathBuf>>,
    override_contents: Option<&str>,
) -> Result<ConfigLayer> {
    let abs_path = if config_path.is_absolute() {
        config_path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(config_path)
    };

    // Diamond dependency detection: if this file was already loaded via a different
    // inheritance path, return an empty layer. This handles cases like standard's
    // base.yml being referenced both directly (inherit_gem: standard: config/base.yml)
    // and indirectly (ruby-3.3.yml -> inherit_from: ./base.yml). True circular
    // inheritance can't happen because we return early before recursing.
    if !visited.insert(abs_path.clone()) {
        return Ok(ConfigLayer::empty());
    }

    let contents = if let Some(s) = override_contents {
        s.to_string()
    } else {
        let raw = std::fs::read_to_string(config_path)
            .with_context(|| format!("failed to read config {}", config_path.display()))?;
        // Strip Ruby-specific YAML tags (e.g., !ruby/regexp) that serde_yml can't handle
        raw.replace("!ruby/regexp ", "")
    };
    let raw: Value = serde_yml::from_str(&contents)
        .with_context(|| format!("failed to parse {}", config_path.display()))?;

    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    // Collect inherited layers in priority order (lowest first):
    // require: gem defaults < inherit_gem < inherit_from < local
    let mut base_layer = ConfigLayer::empty();

    if let Value::Mapping(ref map) = raw {
        // Peek at AllCops.TargetRubyVersion for version-aware standard gem config selection.
        // Needed before processing require: to select the right version-specific config file.
        let local_ruby_version: Option<f64> = map
            .get(Value::String("AllCops".to_string()))
            .and_then(|ac| {
                if let Value::Mapping(ac_map) = ac {
                    ac_map
                        .get(Value::String("TargetRubyVersion".to_string()))
                        .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
                } else {
                    None
                }
            });

        // 0. Process require: AND plugins: — load plugin default configs (lowest priority).
        //    A project may have both keys (e.g., `plugins: [rubocop-rspec]` and
        //    `require: [./custom_cop.rb]`), so we must process both.
        let mut gems = Vec::new();
        for key in &["plugins", "require"] {
            if let Some(val) = map.get(Value::String(key.to_string())) {
                match val {
                    Value::String(s) => gems.push(s.clone()),
                    Value::Sequence(seq) => {
                        for v in seq {
                            if let Some(s) = v.as_str() {
                                gems.push(s.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // Deduplicate (a gem may appear in both require: and plugins:)
        gems.dedup();

        // When standard-* wrapper gems are present, inject the underlying
        // rubocop-* gems so we also load their Include/Exclude patterns.
        // The standard-rails gem config sets cop enabled/disabled states but
        // doesn't include rubocop-rails' per-cop Include patterns (e.g.,
        // Rails/Exit only applies to app/config/lib directories).
        // IMPORTANT: Insert rubocop-* BEFORE its standard-* counterpart so
        // that the standard gem's Enabled overrides take priority (later
        // merges win).
        {
            let mut injected = Vec::new();
            for (i, gem) in gems.iter().enumerate() {
                if gem == "standard-rails"
                    && !gems.iter().any(|g| g == "rubocop-rails")
                    && !injected
                        .iter()
                        .any(|(_, g): &(usize, String)| g == "rubocop-rails")
                {
                    injected.push((i, "rubocop-rails".to_string()));
                }
                if gem == "standard-performance"
                    && !gems.iter().any(|g| g == "rubocop-performance")
                    && !injected
                        .iter()
                        .any(|(_, g): &(usize, String)| g == "rubocop-performance")
                {
                    injected.push((i, "rubocop-performance".to_string()));
                }
            }
            // Insert in reverse order to maintain correct indices
            for (i, gem) in injected.into_iter().rev() {
                gems.insert(i, gem);
            }
        }

        // Save visited state before require: processing. After recording
        // require_enabled_cops, we restore it so that inherit_gem: can
        // re-load the same config files. This is needed because standard-
        // family gems may resolve require: to the same config/base.yml
        // that inherit_gem: explicitly references. Without this, the
        // inherit_gem load is skipped (file already visited), and cops
        // from that config stay in require_enabled_cops — incorrectly
        // causing them to be disabled under DisabledByDefault.
        let visited_before_require = visited.clone();

        if !gems.is_empty() {
            for gem_name in &gems {
                // Determine what config file to load for this gem.
                // rubocop-* gems use config/default.yml.
                // standard-family gems use version-specific or base config.
                // Other gems (custom cops, Ruby files, etc.) are skipped.
                let config_rel_path: Option<String> = if gem_name.starts_with("rubocop-") {
                    Some("config/default.yml".into())
                } else {
                    standard_gem_config_path(gem_name, local_ruby_version).map(|path| path.into())
                };
                let Some(config_rel_path) = config_rel_path else {
                    continue;
                };

                let gem_root = if let Some(path) = gem_cache.and_then(|c| c.get(gem_name)) {
                    path.clone()
                } else {
                    match gem_path::resolve_gem_path(gem_name, working_dir) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("warning: require '{}': {e:#}", gem_name);
                            continue;
                        }
                    }
                };
                let config_file = gem_root.join(&config_rel_path);
                if !config_file.exists() {
                    // For standard-family gems, fall back to config/base.yml if the
                    // version-specific file doesn't exist (older gem version).
                    if !gem_name.starts_with("rubocop-") {
                        let fallback = gem_root.join("config").join("base.yml");
                        if fallback.exists() {
                            match load_config_recursive(&fallback, working_dir, visited, gem_cache)
                            {
                                Ok(layer) => merge_layer_into(&mut base_layer, &layer, None),
                                Err(e) => {
                                    eprintln!(
                                        "warning: failed to load default config for {}: {e:#}",
                                        gem_name
                                    );
                                }
                            }
                        }
                    }
                    continue;
                }
                match load_config_recursive(&config_file, working_dir, visited, gem_cache) {
                    // WARNING: Do NOT make gem Exclude patterns absolute relative to
                    // the gem's config directory. Gem default configs (e.g., rubocop's
                    // config/default.yml) contain Exclude patterns like `spec/**/*`
                    // that are intended to match relative to the PROJECT root, not the
                    // gem installation path. Making them absolute (e.g.,
                    // `/gems/rubocop-1.84/config/spec/**/*`) causes them to never
                    // match, breaking exclusions for cops like Metrics/BlockLength
                    // (113k+ false positives in corpus). This mistake has been made
                    // twice — see git history for reverts. RuboCop's own
                    // make_excludes_absolute resolves relative to the project root,
                    // not the gem root. The current behavior of keeping patterns
                    // relative is correct — they match at the project level.
                    Ok(mut layer) => {
                        // RuboCop 1.84+ plugins: / require: does NOT merge
                        // AllCops.Exclude from plugin gem defaults into the
                        // project config. Only per-cop settings are loaded.
                        // rubocop-rails' default.yml has AllCops.Exclude patterns
                        // (db/*schema.rb, **/app/assets/**/*) that would incorrectly
                        // suppress offenses on those files if merged.
                        if gem_name.starts_with("rubocop-") {
                            layer.global_excludes.clear();
                        }
                        merge_layer_into(&mut base_layer, &layer, None);
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: failed to load default config for {}: {e:#}",
                            gem_name
                        );
                    }
                }
            }
        }

        // Record cops/depts enabled by require: defaults (for DisabledByDefault).
        // Under DisabledByDefault, these are NOT considered "explicitly enabled".
        let require_cops: HashSet<String> = base_layer
            .cop_configs
            .iter()
            .filter(|(_, c)| c.enabled == EnabledState::True)
            .map(|(n, _)| n.clone())
            .collect();
        let require_depts: HashSet<String> = base_layer
            .department_configs
            .iter()
            .filter(|(_, c)| c.enabled == EnabledState::True)
            .map(|(n, _)| n.clone())
            .collect();
        base_layer.require_enabled_cops = require_cops;
        base_layer.require_enabled_depts = require_depts;

        // Restore visited set so inherit_gem: can re-load files that were
        // also loaded by require:. See comment above visited_before_require.
        *visited = visited_before_require;

        // Track ALL cops mentioned in require: gem configs (for version awareness).
        // Cops from plugin departments not in this set don't exist in the
        // installed gem version and should be treated as disabled.
        base_layer.require_known_cops = base_layer.cop_configs.keys().cloned().collect();
        base_layer.require_departments = base_layer.department_configs.keys().cloned().collect();

        // Also register departments from *requested* gems even if gem resolution
        // failed (e.g., `bundle` not on PATH). This ensures plugin departments
        // from requested gems are known, preventing false positives when we
        // disable unrequested plugin departments.
        for gem_name in &gems {
            for (dept, gem) in PLUGIN_GEM_DEPARTMENTS {
                if gem_name.as_str() == *gem {
                    base_layer.require_departments.insert(dept.to_string());
                }
            }
        }

        // 1. Process inherit_gem
        if let Some(Value::Mapping(gem_map)) = map.get(Value::String("inherit_gem".to_string())) {
            for (gem_key, gem_paths) in gem_map {
                if let Some(gem_name) = gem_key.as_str() {
                    match resolve_inherit_gem(gem_name, gem_paths, working_dir, visited, gem_cache)
                    {
                        Ok(gem_layers) => {
                            for layer in gem_layers {
                                // Propagate user_mentioned from the layer's recursive loading.
                                // Don't use cop_configs.keys() — that includes require: defaults.
                                base_layer
                                    .user_mentioned_cops
                                    .extend(layer.user_mentioned_cops.iter().cloned());
                                base_layer
                                    .user_mentioned_depts
                                    .extend(layer.user_mentioned_depts.iter().cloned());
                                merge_layer_into(&mut base_layer, &layer, None);
                            }
                        }
                        Err(e) => {
                            // Match RuboCop: warn and continue loading the rest
                            // of the config. The gem's config layer is skipped,
                            // but local overrides (Max, Exclude, etc.) still apply.
                            eprintln!("warning: {e:#}");
                        }
                    }
                }
            }
        }

        // 2. Process inherit_from
        if let Some(inherit_value) = map.get(Value::String("inherit_from".to_string())) {
            let paths = match inherit_value {
                Value::String(s) => vec![s.clone()],
                Value::Sequence(seq) => seq
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                _ => vec![],
            };

            for rel_path in &paths {
                let inherited_path = config_dir.join(rel_path);
                if !inherited_path.exists() {
                    eprintln!(
                        "warning: inherit_from target not found: {} (from {})",
                        inherited_path.display(),
                        config_path.display()
                    );
                    continue;
                }
                match load_config_recursive(&inherited_path, working_dir, visited, gem_cache) {
                    Ok(layer) => {
                        // Propagate user_mentioned from the layer's recursive loading.
                        // Don't use cop_configs.keys() — that includes require: defaults.
                        base_layer
                            .user_mentioned_cops
                            .extend(layer.user_mentioned_cops.iter().cloned());
                        base_layer
                            .user_mentioned_depts
                            .extend(layer.user_mentioned_depts.iter().cloned());
                        merge_layer_into(&mut base_layer, &layer, None);
                    }
                    Err(e) => {
                        eprintln!(
                            "warning: failed to load inherited config {}: {e:#}",
                            inherited_path.display()
                        );
                    }
                }
            }
        }
    }

    // 3. Parse the local config layer and merge it on top (highest priority)
    let local_layer = parse_config_layer(&raw);
    // Track cops from the local config file as user-mentioned
    base_layer
        .user_mentioned_cops
        .extend(local_layer.cop_configs.keys().cloned());
    base_layer
        .user_mentioned_depts
        .extend(local_layer.department_configs.keys().cloned());
    merge_layer_into(
        &mut base_layer,
        &local_layer,
        Some(&local_layer.inherit_mode),
    );

    Ok(base_layer)
}

/// Resolve `inherit_gem` entries. Each gem name maps to one or more YAML paths
/// relative to the gem's root directory.
///
/// Returns an error if the gem cannot be resolved — this is intentionally a hard
/// failure because `inherit_gem` configs can set critical flags like
/// `DisabledByDefault: true`. Silently skipping them leads to incorrect behavior.
fn resolve_inherit_gem(
    gem_name: &str,
    paths_value: &Value,
    working_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    gem_cache: Option<&HashMap<String, PathBuf>>,
) -> Result<Vec<ConfigLayer>> {
    let gem_root = if let Some(path) = gem_cache.and_then(|c| c.get(gem_name)) {
        path.clone()
    } else {
        gem_path::resolve_gem_path(gem_name, working_dir).with_context(|| {
            format!(
                "inherit_gem: failed to resolve gem '{gem_name}'. \
                 Run `bundle install` to install it, or remove it from inherit_gem in .rubocop.yml."
            )
        })?
    };

    let rel_paths: Vec<String> = match paths_value {
        Value::String(s) => vec![s.clone()],
        Value::Sequence(seq) => seq
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => vec![],
    };

    let mut layers = Vec::new();
    for rel_path in &rel_paths {
        let full_path = gem_root.join(rel_path);
        if !full_path.exists() {
            anyhow::bail!(
                "inherit_gem: config file not found: {} (gem '{gem_name}')",
                full_path.display(),
            );
        }
        match load_config_recursive(&full_path, working_dir, visited, gem_cache) {
            // See WARNING in load_config_recursive_inner — do NOT make excludes
            // absolute relative to the gem config dir. Patterns are project-relative.
            Ok(layer) => layers.push(layer),
            Err(e) => {
                return Err(e).with_context(|| {
                    format!(
                        "inherit_gem: failed to load config {} from gem '{gem_name}'",
                        full_path.display()
                    )
                });
            }
        }
    }
    Ok(layers)
}

/// Parse a single YAML Value into a ConfigLayer (no inheritance resolution).
fn parse_config_layer(raw: &Value) -> ConfigLayer {
    let mut cop_configs = HashMap::new();
    let mut department_configs = HashMap::new();
    let mut global_excludes = Vec::new();
    let mut new_cops = None;
    let mut disabled_by_default = None;
    let mut inherit_mode = InheritMode::default();
    let mut target_ruby_version = None;
    let mut target_rails_version = None;
    let mut active_support_extensions_enabled = None;
    let mut migrated_schema_version: Option<String> = None;

    if let Value::Mapping(map) = raw {
        for (key, value) in map {
            let key_str = match key.as_str() {
                Some(s) => s,
                None => continue,
            };

            // Skip non-cop top-level keys silently
            match key_str {
                "inherit_from" | "inherit_gem" | "require" | "plugins" => continue,
                "inherit_mode" => {
                    inherit_mode = parse_inherit_mode(value);
                    continue;
                }
                "AllCops" => {
                    if let Some(excludes) = extract_string_list(value, "Exclude") {
                        global_excludes = excludes;
                    }
                    if let Value::Mapping(ac_map) = value {
                        if let Some(nc) = ac_map.get(Value::String("NewCops".to_string())) {
                            new_cops = nc.as_str().map(String::from);
                        }
                        if let Some(dbd) =
                            ac_map.get(Value::String("DisabledByDefault".to_string()))
                        {
                            disabled_by_default = dbd.as_bool();
                        }
                        if let Some(trv) =
                            ac_map.get(Value::String("TargetRubyVersion".to_string()))
                        {
                            target_ruby_version = trv
                                .as_f64()
                                .or_else(|| trv.as_u64().map(|u| u as f64))
                                .or_else(|| trv.as_str().and_then(|s| s.parse::<f64>().ok()));
                        }
                        if let Some(trv) =
                            ac_map.get(Value::String("TargetRailsVersion".to_string()))
                        {
                            target_rails_version = trv
                                .as_f64()
                                .or_else(|| trv.as_u64().map(|u| u as f64))
                                .or_else(|| trv.as_str().and_then(|s| s.parse::<f64>().ok()));
                        }
                        if let Some(ase) =
                            ac_map.get(Value::String("ActiveSupportExtensionsEnabled".to_string()))
                        {
                            active_support_extensions_enabled = ase.as_bool();
                        }
                        if let Some(msv) =
                            ac_map.get(Value::String("MigratedSchemaVersion".to_string()))
                        {
                            migrated_schema_version = msv
                                .as_str()
                                .map(String::from)
                                .or_else(|| msv.as_u64().map(|u| u.to_string()))
                                .or_else(|| msv.as_i64().map(|i| i.to_string()));
                        }
                    }
                    continue;
                }
                _ => {}
            }

            if key_str.contains('/') {
                // Cop-level config (e.g. "Style/FrozenStringLiteralComment")
                let cop_config = parse_cop_config(value);
                cop_configs.insert(key_str.to_string(), cop_config);
            } else {
                // Department-level config (e.g. "RSpec", "Rails")
                let dept_config = parse_department_config(value);
                department_configs.insert(key_str.to_string(), dept_config);
            }
        }
    }

    ConfigLayer {
        cop_configs,
        department_configs,
        global_excludes,
        new_cops,
        disabled_by_default,
        inherit_mode,
        require_enabled_cops: HashSet::new(),
        require_enabled_depts: HashSet::new(),
        require_known_cops: HashSet::new(),
        require_departments: HashSet::new(),
        user_mentioned_cops: HashSet::new(),
        user_mentioned_depts: HashSet::new(),
        target_ruby_version,
        target_rails_version,
        active_support_extensions_enabled,
        migrated_schema_version,
    }
}

/// Merge an overlay layer into a base layer using RuboCop merge rules:
/// - Scalar values (Enabled, Severity, options): last writer wins
/// - Exclude arrays: appended (union) by default, replaced if `inherit_mode: override`
/// - Include arrays: replaced (override) by default, appended if `inherit_mode: merge`
/// - Global excludes (AllCops.Exclude): replaced by default, appended if inherit_mode
///   includes Exclude in merge (same as cop-level Exclude). When inherit_mode is None
///   (inherited config layers), always append.
/// - NewCops / DisabledByDefault: last writer wins
fn merge_layer_into(
    base: &mut ConfigLayer,
    overlay: &ConfigLayer,
    inherit_mode: Option<&InheritMode>,
) {
    // Merge global excludes (AllCops.Exclude).
    // RuboCop ALWAYS merges AllCops.Exclude (union), unlike per-cop Exclude
    // which replaces by default. The only exception is when inherit_mode
    // explicitly sets `override: [Exclude]`.
    if !overlay.global_excludes.is_empty() {
        let should_replace = match inherit_mode {
            None => false,
            Some(mode) => mode.override_keys.contains("Exclude"),
        };
        if should_replace {
            base.global_excludes.clone_from(&overlay.global_excludes);
        } else {
            for exc in &overlay.global_excludes {
                if !base.global_excludes.contains(exc) {
                    base.global_excludes.push(exc.clone());
                }
            }
        }
    }

    // NewCops: last writer wins
    if overlay.new_cops.is_some() {
        base.new_cops.clone_from(&overlay.new_cops);
    }

    // DisabledByDefault: last writer wins
    if overlay.disabled_by_default.is_some() {
        base.disabled_by_default = overlay.disabled_by_default;
    }

    // TargetRubyVersion: last writer wins
    if overlay.target_ruby_version.is_some() {
        base.target_ruby_version = overlay.target_ruby_version;
    }

    // TargetRailsVersion: last writer wins
    if overlay.target_rails_version.is_some() {
        base.target_rails_version = overlay.target_rails_version;
    }

    // ActiveSupportExtensionsEnabled: last writer wins
    if overlay.active_support_extensions_enabled.is_some() {
        base.active_support_extensions_enabled = overlay.active_support_extensions_enabled;
    }

    // MigratedSchemaVersion: last writer wins
    if overlay.migrated_schema_version.is_some() {
        base.migrated_schema_version
            .clone_from(&overlay.migrated_schema_version);
    }

    // Merge department configs
    for (dept_name, overlay_dept) in &overlay.department_configs {
        match base.department_configs.get_mut(dept_name) {
            Some(base_dept) => {
                merge_department_config(base_dept, overlay_dept, inherit_mode);
            }
            None => {
                base.department_configs
                    .insert(dept_name.clone(), overlay_dept.clone());
            }
        }
    }

    // Merge per-cop configs
    for (cop_name, overlay_config) in &overlay.cop_configs {
        match base.cop_configs.get_mut(cop_name) {
            Some(base_config) => {
                merge_cop_config(base_config, overlay_config, inherit_mode);
            }
            None => {
                base.cop_configs
                    .insert(cop_name.clone(), overlay_config.clone());
            }
        }
        // Track require-originated enabled state through merges.
        if overlay.require_enabled_cops.contains(cop_name) {
            base.require_enabled_cops.insert(cop_name.clone());
        } else if overlay_config.enabled != EnabledState::Unset {
            base.require_enabled_cops.remove(cop_name);
        }
    }

    // Same for departments
    for (dept_name, overlay_dept) in &overlay.department_configs {
        if overlay.require_enabled_depts.contains(dept_name) {
            base.require_enabled_depts.insert(dept_name.clone());
        } else if overlay_dept.enabled != EnabledState::Unset {
            base.require_enabled_depts.remove(dept_name);
        }
    }

    // Propagate require-known cops and departments (union — once known, always known)
    for cop in &overlay.require_known_cops {
        base.require_known_cops.insert(cop.clone());
    }
    for dept in &overlay.require_departments {
        base.require_departments.insert(dept.clone());
    }
}

/// Merge a single department's overlay config into its base config.
fn merge_department_config(
    base: &mut DepartmentConfig,
    overlay: &DepartmentConfig,
    inherit_mode: Option<&InheritMode>,
) {
    // Enabled: last writer wins (only if overlay explicitly set it)
    if overlay.enabled != EnabledState::Unset {
        base.enabled = overlay.enabled;
    }

    let should_merge_include = inherit_mode
        .map(|im| im.merge.contains("Include"))
        .unwrap_or(false);
    let should_override_exclude = inherit_mode
        .map(|im| im.override_keys.contains("Exclude"))
        .unwrap_or(false);

    // Exclude: append by default, replace if inherit_mode says override
    if should_override_exclude {
        if !overlay.exclude.is_empty() {
            base.exclude.clone_from(&overlay.exclude);
        }
    } else {
        for exc in &overlay.exclude {
            if !base.exclude.contains(exc) {
                base.exclude.push(exc.clone());
            }
        }
    }

    // Include: replace by default, append if inherit_mode says merge
    if !overlay.include.is_empty() {
        if should_merge_include {
            for inc in &overlay.include {
                if !base.include.contains(inc) {
                    base.include.push(inc.clone());
                }
            }
        } else {
            base.include.clone_from(&overlay.include);
        }
    }
}

/// Merge a single cop's overlay config into its base config.
fn merge_cop_config(base: &mut CopConfig, overlay: &CopConfig, inherit_mode: Option<&InheritMode>) {
    // Enabled: last writer wins (only if overlay explicitly set it)
    if overlay.enabled != EnabledState::Unset {
        base.enabled = overlay.enabled;
    }

    // Severity: last writer wins (if overlay has one)
    if overlay.severity.is_some() {
        base.severity = overlay.severity;
    }

    let should_merge_include = inherit_mode
        .map(|im| im.merge.contains("Include"))
        .unwrap_or(false);
    let should_override_exclude = inherit_mode
        .map(|im| im.override_keys.contains("Exclude"))
        .unwrap_or(false);

    // Exclude: append (union) by default, replace if inherit_mode says override
    if should_override_exclude {
        if !overlay.exclude.is_empty() {
            base.exclude.clone_from(&overlay.exclude);
        }
    } else {
        for exc in &overlay.exclude {
            if !base.exclude.contains(exc) {
                base.exclude.push(exc.clone());
            }
        }
    }

    // Include: replace (override) by default, append if inherit_mode says merge
    if !overlay.include.is_empty() {
        if should_merge_include {
            for inc in &overlay.include {
                if !base.include.contains(inc) {
                    base.include.push(inc.clone());
                }
            }
        } else {
            base.include.clone_from(&overlay.include);
        }
    }

    // Check for cop-level inherit_mode in overlay options.
    // When a cop config contains `inherit_mode: { merge: [AllowedMethods] }`,
    // array options listed in `merge` should be appended instead of replaced.
    let cop_inherit_mode = overlay
        .options
        .get("inherit_mode")
        .and_then(|v| v.as_mapping())
        .map(|m| {
            let merge_keys: HashSet<String> = m
                .get(Value::String("merge".to_string()))
                .and_then(|v| v.as_sequence())
                .map(|seq| {
                    seq.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            merge_keys
        })
        .unwrap_or_default();

    // Options: merge (last writer wins per key, deep-merge for Mapping values
    // to match RuboCop's behavior where Hash cop options are merged, not replaced)
    for (key, value) in &overlay.options {
        // Skip inherit_mode itself — it's a merge directive, not a cop option
        if key == "inherit_mode" {
            continue;
        }

        if let (Some(Value::Mapping(base_map)), Value::Mapping(overlay_map)) =
            (base.options.get(key), value)
        {
            let mut merged = base_map.clone();
            for (k, v) in overlay_map {
                merged.insert(k.clone(), v.clone());
            }
            base.options.insert(key.clone(), Value::Mapping(merged));
        } else if cop_inherit_mode.contains(key) {
            // Cop-level inherit_mode says merge this key — append arrays
            if let (Some(Value::Sequence(base_seq)), Value::Sequence(overlay_seq)) =
                (base.options.get(key), value)
            {
                let mut merged = base_seq.clone();
                for item in overlay_seq {
                    if !merged.contains(item) {
                        merged.push(item.clone());
                    }
                }
                base.options.insert(key.clone(), Value::Sequence(merged));
            } else {
                base.options.insert(key.clone(), value.clone());
            }
        } else {
            base.options.insert(key.clone(), value.clone());
        }
    }
}

impl ResolvedConfig {
    /// Check if a cop is enabled for the given file path.
    ///
    /// Evaluates in order:
    /// 1. Determine enabled state from cop config > department config > defaults.
    ///    - `False` → disabled
    ///    - `Pending` → disabled unless `AllCops.NewCops: enable`
    ///    - `Unset` → disabled if `AllCops.DisabledByDefault: true`
    ///    - `True` → enabled
    /// 2. If global excludes match the path, return false.
    /// 3. Merge cop's default_include/default_exclude with user/department overrides.
    /// 4. If effective Include is non-empty, path must match at least one pattern.
    /// 5. If effective Exclude is non-empty, path must NOT match any pattern.
    pub fn is_cop_enabled(
        &self,
        name: &str,
        path: &Path,
        default_include: &[&str],
        default_exclude: &[&str],
    ) -> bool {
        let config = self.cop_configs.get(name);
        let dept = name.split('/').next().unwrap_or("");
        let dept_config = self.department_configs.get(dept);

        // 1. Determine enabled state following RuboCop's enable_cop? logic.
        let cop_enabled_state = config.map(|c| c.enabled).unwrap_or(EnabledState::Unset);
        let dept_enabled_state = dept_config
            .map(|dc| dc.enabled)
            .unwrap_or(EnabledState::Unset);

        let enabled_state = if cop_enabled_state == EnabledState::True {
            // Department-level Enabled:false overrides cop-level Enabled:true
            // when the cop's True came from defaults (not user config).
            if !self.disabled_by_default
                && dept_enabled_state == EnabledState::False
                && !self.project_mentioned_cops.contains(name)
            {
                EnabledState::False
            } else {
                EnabledState::True
            }
        } else if cop_enabled_state != EnabledState::Unset {
            cop_enabled_state
        } else if dept_enabled_state == EnabledState::False {
            EnabledState::False
        } else if dept_enabled_state == EnabledState::True {
            if self.disabled_by_default && self.project_enabled_depts.contains(dept) {
                // DisabledByDefault + department explicitly enabled by user
                // → restore to True (matching RuboCop's handle_disabled_by_default)
                EnabledState::True
            } else if !self.disabled_by_default {
                EnabledState::True
            } else {
                EnabledState::Unset
            }
        } else {
            EnabledState::Unset
        };

        match enabled_state {
            EnabledState::False => return false,
            EnabledState::Pending => {
                if self.new_cops != NewCopsPolicy::Enable {
                    return false;
                }
            }
            EnabledState::Unset => {
                if self.disabled_by_default {
                    return false;
                }
            }
            EnabledState::True => {}
        }

        // Plugin department awareness: cops from plugin departments (Rails, RSpec,
        // Performance, Migration, etc.) should only run if the corresponding gem was
        // loaded via `require:` or `plugins:`. If the project doesn't load the gem,
        // these cops must be disabled regardless of their default Enabled state.
        if is_plugin_department(dept) && !self.require_departments.contains(dept) {
            // The department wasn't loaded — cop should not fire unless the user
            // explicitly set it to Enabled:true in their project config.
            // Enabled:pending (from rubocop defaults) does not count.
            if config.is_none_or(|c| c.enabled != EnabledState::True) {
                return false;
            }
        }

        // Plugin version awareness: cop from require: department but not in gem config.
        // Only apply when the gem's config was actually loaded (has known cops for this dept).
        let dept_has_known_cops = self
            .require_known_cops
            .iter()
            .any(|c| c.starts_with(dept) && c.as_bytes().get(dept.len()) == Some(&b'/'));
        if dept_has_known_cops
            && self.require_departments.contains(dept)
            && !self.require_known_cops.contains(name)
            && config.is_none_or(|c| c.enabled == EnabledState::Unset)
        {
            return false;
        }

        // Core cop version awareness: if the installed rubocop gem's config was
        // loaded and this core cop isn't mentioned, it doesn't exist in that version.
        if !self.rubocop_known_cops.is_empty()
            && !is_plugin_department(dept)
            && !self.rubocop_known_cops.contains(name)
            && config.is_none_or(|c| c.enabled == EnabledState::Unset)
        {
            return false;
        }

        // Cross-cop dependency: Style/RedundantConstantBase is disabled
        // when Lint/ConstantResolution is enabled (conflicting rules).
        if name == "Style/RedundantConstantBase" {
            let lcr_enabled = self
                .cop_configs
                .get("Lint/ConstantResolution")
                .is_some_and(|c| c.enabled == EnabledState::True);
            if lcr_enabled {
                return false;
            }
        }

        // 2. Global excludes
        for pattern in &self.global_excludes {
            if glob_matches(pattern, path) {
                return false;
            }
        }

        // 3. Build effective include/exclude lists.
        //    Precedence: cop config > department config > defaults.
        let effective_include: Vec<&str> = match config {
            Some(c) if !c.include.is_empty() => c.include.iter().map(|s| s.as_str()).collect(),
            _ => match dept_config {
                Some(dc) if !dc.include.is_empty() => {
                    dc.include.iter().map(|s| s.as_str()).collect()
                }
                _ => default_include.to_vec(),
            },
        };
        let effective_exclude: Vec<&str> = match config {
            Some(c) if !c.exclude.is_empty() => c.exclude.iter().map(|s| s.as_str()).collect(),
            _ => match dept_config {
                Some(dc) if !dc.exclude.is_empty() => {
                    dc.exclude.iter().map(|s| s.as_str()).collect()
                }
                _ => default_exclude.to_vec(),
            },
        };
        // 4. Include filter: path must match at least one
        if !effective_include.is_empty()
            && !effective_include.iter().any(|pat| glob_matches(pat, path))
        {
            return false;
        }

        // 5. Exclude filter: path must NOT match any
        if effective_exclude.iter().any(|pat| glob_matches(pat, path)) {
            return false;
        }

        true
    }

    /// Get the resolved config for a specific cop.
    ///
    /// Injects global AllCops settings (like TargetRubyVersion) into the
    /// cop's options so individual cops can access them without special plumbing.
    pub fn cop_config(&self, name: &str) -> CopConfig {
        let mut config = self.cop_configs.get(name).cloned().unwrap_or_default();
        // Inject TargetRubyVersion from AllCops into cop options
        // (only if the cop doesn't already have it set explicitly)
        if let Some(version) = self.target_ruby_version {
            config
                .options
                .entry("TargetRubyVersion".to_string())
                .or_insert_with(|| Value::Number(serde_yml::Number::from(version)));
        }
        // Inject TargetRailsVersion from AllCops into cop options
        if let Some(version) = self.target_rails_version {
            config
                .options
                .entry("TargetRailsVersion".to_string())
                .or_insert_with(|| Value::Number(serde_yml::Number::from(version)));
        }
        // Inject railties_in_lockfile flag so cops can check requires_gem('railties')
        config
            .options
            .entry("__RailtiesInLockfile".to_string())
            .or_insert_with(|| Value::Bool(self.railties_in_lockfile));
        // Inject rack version for HttpStatusNameConsistency cops
        // (requires_gem 'rack', '>= 3.1.0')
        if matches!(
            name,
            "Rails/HttpStatusNameConsistency" | "RSpecRails/HttpStatusNameConsistency"
        ) {
            if let Some(ver) = self.rack_version {
                config
                    .options
                    .entry("__RackVersion".to_string())
                    .or_insert_with(|| Value::Number(serde_yml::Number::from(ver)));
            }
        }
        // Inject MaxLineLength and LineLengthEnabled from Layout/LineLength into
        // cops that need it (mirrors RuboCop's `config.for_cop('Layout/LineLength')`).
        // When Layout/LineLength is disabled, max_line_length returns nil in RuboCop,
        // which causes modifier_fits_on_single_line? to return true (no length limit).
        if matches!(
            name,
            "Style/IfUnlessModifier"
                | "Style/WhileUntilModifier"
                | "Style/GuardClause"
                | "Style/SoleNestedConditional"
                | "Style/MultilineMethodSignature"
                | "Layout/RedundantLineBreak"
        ) {
            let line_length_config = self.cop_configs.get("Layout/LineLength");
            if !config.options.contains_key("MaxLineLength") {
                let max = line_length_config
                    .and_then(|cc| cc.options.get("Max"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(120);
                config.options.insert(
                    "MaxLineLength".to_string(),
                    Value::Number(serde_yml::Number::from(max)),
                );
            }
            if !config.options.contains_key("LineLengthEnabled") {
                let enabled = line_length_config
                    .map(|cc| !matches!(cc.enabled, crate::cop::EnabledState::False))
                    .unwrap_or(true);
                config
                    .options
                    .insert("LineLengthEnabled".to_string(), Value::Bool(enabled));
            }
        }
        // Inject ActiveSupportExtensionsEnabled from AllCops for cops that need it
        if name == "Lint/DuplicateMethods"
            || name == "Style/ArrayIntersect"
            || name == "Style/CollectionQuerying"
            || name == "Style/RedundantFilterChain"
        {
            config
                .options
                .entry("ActiveSupportExtensionsEnabled".to_string())
                .or_insert_with(|| Value::Bool(self.active_support_extensions_enabled));
        }
        // Inject Layout/ArgumentAlignment EnforcedStyle into Layout/HashAlignment
        // (mirrors RuboCop's `config.for_enabled_cop('Layout/ArgumentAlignment')` lookup
        // used by autocorrect_incompatible_with_other_cops?)
        if name == "Layout/HashAlignment" {
            let aa_config = self.cop_configs.get("Layout/ArgumentAlignment");
            let aa_style = aa_config
                .and_then(|cc| cc.options.get("EnforcedStyle"))
                .and_then(|v| v.as_str())
                .unwrap_or("with_first_argument");
            config
                .options
                .entry("ArgumentAlignmentStyle".to_string())
                .or_insert_with(|| Value::String(aa_style.to_string()));
        }
        // Inject sibling Layout cop styles that other cops consult at runtime.
        if name == "Layout/FirstHashElementIndentation" {
            let hash_alignment_config = self.cop_configs.get("Layout/HashAlignment");
            for (key, default) in [
                ("EnforcedColonStyle", "key"),
                ("EnforcedHashRocketStyle", "key"),
            ] {
                let style = hash_alignment_config
                    .and_then(|cc| cc.options.get(key))
                    .cloned()
                    .unwrap_or_else(|| Value::String(default.to_string()));
                config.options.entry(key.to_string()).or_insert(style);
            }
        }
        if name == "Layout/ElseAlignment" || name == "Layout/IndentationWidth" {
            let end_config = self.cop_configs.get("Layout/EndAlignment");
            let end_style = end_config
                .and_then(|cc| cc.options.get("EnforcedStyleAlignWith"))
                .and_then(|v| v.as_str())
                .unwrap_or("keyword");
            config
                .options
                .entry("EndAlignmentStyle".to_string())
                .or_insert_with(|| Value::String(end_style.to_string()));
        }
        if name == "Layout/IndentationWidth" {
            let consistency_config = self.cop_configs.get("Layout/IndentationConsistency");
            let consistency_style = consistency_config
                .and_then(|cc| cc.options.get("EnforcedStyle"))
                .and_then(|v| v.as_str())
                .unwrap_or("normal");
            config
                .options
                .entry("IndentationConsistencyStyle".to_string())
                .or_insert_with(|| Value::String(consistency_style.to_string()));

            let access_modifier_config = self.cop_configs.get("Layout/AccessModifierIndentation");
            let access_modifier_style = access_modifier_config
                .and_then(|cc| cc.options.get("EnforcedStyle"))
                .and_then(|v| v.as_str())
                .unwrap_or("indent");
            config
                .options
                .entry("AccessModifierIndentationStyle".to_string())
                .or_insert_with(|| Value::String(access_modifier_style.to_string()));

            let indentation_style_config = self.cop_configs.get("Layout/IndentationStyle");
            let indentation_style = indentation_style_config
                .and_then(|cc| cc.options.get("EnforcedStyle"))
                .and_then(|v| v.as_str())
                .unwrap_or("spaces");
            config
                .options
                .entry("IndentationStyleEnforced".to_string())
                .or_insert_with(|| Value::String(indentation_style.to_string()));
        }
        // Inject Layout/SpaceInsideHashLiteralBraces EnforcedStyle for Layout/SpaceAfterComma
        // (mirrors RuboCop's `space_forbidden_before_rcurly?` which reads the sibling cop's style)
        if name == "Layout/SpaceAfterComma" {
            let braces_config = self.cop_configs.get("Layout/SpaceInsideHashLiteralBraces");
            let braces_style = braces_config
                .and_then(|cc| cc.options.get("EnforcedStyle"))
                .and_then(|v| v.as_str())
                .unwrap_or("space");
            config
                .options
                .entry("__SpaceInsideHashBracesStyle".to_string())
                .or_insert_with(|| Value::String(braces_style.to_string()));
        }
        // Inject cross-cop config for Style/MissingElse:
        // - Style/UnlessElse.Enabled → UnlessElseEnabled (controls whether unless is flagged)
        // - Style/EmptyElse.EnforcedStyle → EmptyElseStyle (controls message wording)
        if name == "Style/MissingElse" {
            let unless_else_config = self.cop_configs.get("Style/UnlessElse");
            let unless_else_enabled = unless_else_config
                .map(|cc| !matches!(cc.enabled, crate::cop::EnabledState::False))
                .unwrap_or(true);
            config
                .options
                .entry("UnlessElseEnabled".to_string())
                .or_insert_with(|| Value::Bool(unless_else_enabled));

            let empty_else_config = self.cop_configs.get("Style/EmptyElse");
            let empty_else_style = empty_else_config
                .and_then(|cc| cc.options.get("EnforcedStyle"))
                .and_then(|v| v.as_str())
                .unwrap_or("both");
            config
                .options
                .entry("EmptyElseStyle".to_string())
                .or_insert_with(|| Value::String(empty_else_style.to_string()));
        }
        // Inject Style/StringLiterals EnforcedStyle for Style/QuotedSymbols
        // (mirrors RuboCop's `config.for_cop('Style/StringLiterals')` lookup)
        if name == "Style/QuotedSymbols" {
            let sl_config = self.cop_configs.get("Style/StringLiterals");
            let sl_style = sl_config
                .and_then(|cc| cc.options.get("EnforcedStyle"))
                .and_then(|v| v.as_str())
                .unwrap_or("single_quotes");
            config
                .options
                .entry("StringLiteralsEnforcedStyle".to_string())
                .or_insert_with(|| Value::String(sl_style.to_string()));
        }
        config
    }

    /// Get the resolved config for a cop, applying directory-specific overrides
    /// based on the file path.
    ///
    /// Finds the nearest nested `.rubocop.yml` in a parent directory of
    /// `file_path` (up to the project root) and merges its local settings on
    /// top of the root config.
    pub fn cop_config_for_file(&self, name: &str, file_path: &Path) -> CopConfig {
        self.effective_config_for_file(file_path)
            .map_or_else(|| self.cop_config(name), |config| config.cop_config(name))
    }

    /// Whether this config has any directory-specific overrides (nested .rubocop.yml files).
    pub fn has_dir_overrides(&self) -> bool {
        !self.dir_overrides.is_empty()
    }

    /// Pre-compute base CopConfig for each cop in the registry (indexed by cop index).
    /// This avoids repeated HashMap lookups and cloning in the per-file hot loop.
    pub fn precompute_cop_configs(&self, registry: &CopRegistry) -> Vec<CopConfig> {
        registry
            .cops()
            .iter()
            .map(|cop| self.cop_config(cop.name()))
            .collect()
    }

    /// Build the effective config for a file under a nested `.rubocop.yml`.
    ///
    /// This reapplies RuboCop's DisabledByDefault normalization after merging
    /// the nearest local config layer so files in sub-config directories see
    /// the same enabled/disabled cop set as RuboCop.
    pub fn effective_config_for_file(&self, file_path: &Path) -> Option<Self> {
        let layer = self.find_dir_layer_for_file(file_path)?;

        let mut effective = self.clone();
        let mut merged = ConfigLayer {
            cop_configs: effective.cop_configs.clone(),
            department_configs: effective.department_configs.clone(),
            global_excludes: effective.global_excludes.clone(),
            new_cops: Some(match effective.new_cops {
                NewCopsPolicy::Enable => "enable".to_string(),
                NewCopsPolicy::Disable => "disable".to_string(),
            }),
            disabled_by_default: Some(effective.disabled_by_default),
            inherit_mode: InheritMode::default(),
            require_enabled_cops: HashSet::new(),
            require_enabled_depts: HashSet::new(),
            require_known_cops: effective.require_known_cops.clone(),
            require_departments: effective.require_departments.clone(),
            user_mentioned_cops: effective.project_mentioned_cops.clone(),
            user_mentioned_depts: effective.project_mentioned_depts.clone(),
            target_ruby_version: effective.target_ruby_version,
            target_rails_version: effective.target_rails_version,
            active_support_extensions_enabled: Some(effective.active_support_extensions_enabled),
            migrated_schema_version: effective.migrated_schema_version.clone(),
        };
        merge_layer_into(&mut merged, layer, Some(&layer.inherit_mode));

        effective.cop_configs = merged.cop_configs;
        effective.department_configs = merged.department_configs;
        effective.global_excludes = merged.global_excludes;
        effective.new_cops = match merged.new_cops.as_deref() {
            Some("enable") => NewCopsPolicy::Enable,
            _ => NewCopsPolicy::Disable,
        };
        effective.disabled_by_default = merged
            .disabled_by_default
            .unwrap_or(effective.disabled_by_default);
        effective.require_known_cops = merged.require_known_cops;
        effective.require_departments = merged.require_departments;
        effective.target_ruby_version = merged.target_ruby_version;
        effective.target_rails_version = merged.target_rails_version;
        effective.active_support_extensions_enabled = merged
            .active_support_extensions_enabled
            .unwrap_or(effective.active_support_extensions_enabled);
        effective.migrated_schema_version = merged.migrated_schema_version;

        effective
            .project_mentioned_cops
            .extend(layer.cop_configs.keys().cloned());
        effective
            .project_mentioned_depts
            .extend(layer.department_configs.keys().cloned());
        for (dept_name, dept_cfg) in &layer.department_configs {
            if dept_cfg.enabled == EnabledState::True {
                effective.project_enabled_depts.insert(dept_name.clone());
            } else if dept_cfg.enabled != EnabledState::Unset {
                effective.project_enabled_depts.remove(dept_name);
            }
        }

        if effective.disabled_by_default {
            for (cop_name, cop_cfg) in &mut effective.cop_configs {
                if cop_cfg.enabled == EnabledState::True
                    && !effective.project_mentioned_cops.contains(cop_name)
                {
                    cop_cfg.enabled = EnabledState::Unset;
                }
            }
            for (dept_name, dept_cfg) in &mut effective.department_configs {
                if dept_cfg.enabled == EnabledState::True
                    && !effective.project_mentioned_depts.contains(dept_name)
                {
                    dept_cfg.enabled = EnabledState::Unset;
                }
            }
        }

        Some(effective)
    }

    /// Find the nearest directory-specific config layer, if any.
    /// Checks both the original file path and the path relativized to config_dir.
    fn find_dir_layer_for_file(&self, file_path: &Path) -> Option<&ConfigLayer> {
        if self.dir_overrides.is_empty() {
            return None;
        }

        // Try direct path match first (both paths in same form).
        // dir_overrides is sorted deepest-first, so first match is most specific.
        for (dir, layer) in &self.dir_overrides {
            if file_path.starts_with(dir) {
                return Some(layer);
            }
        }

        // Try matching with path relativized to config_dir
        // (handles running from outside the project root, e.g. bench/repos/mastodon/...)
        if let Some(ref config_dir) = self.config_dir {
            if let Ok(rel_path) = file_path.strip_prefix(config_dir) {
                for (dir, layer) in &self.dir_overrides {
                    if let Ok(rel_dir) = dir.strip_prefix(config_dir) {
                        if rel_path.starts_with(rel_dir) {
                            return Some(layer);
                        }
                    }
                }
            }
        }

        None
    }

    /// Global exclude patterns from AllCops.Exclude.
    pub fn global_excludes(&self) -> &[String] {
        &self.global_excludes
    }

    /// Directory containing the resolved config file.
    pub fn config_dir(&self) -> Option<&Path> {
        self.config_dir.as_deref()
    }

    /// Base directory for resolving Include/Exclude path patterns.
    /// Falls back to `config_dir` if not set.
    pub fn base_dir(&self) -> Option<&Path> {
        self.base_dir.as_deref().or(self.config_dir.as_deref())
    }

    /// Build pre-compiled cop filters for fast per-file enablement checks.
    ///
    /// This resolves all enabled states, include/exclude patterns, and global
    /// excludes into compiled `GlobSet` matchers. Call once at startup, then
    /// share across rayon workers.
    pub fn build_cop_filters(
        &self,
        registry: &CopRegistry,
        tier_map: &crate::cop::tiers::TierMap,
        preview: bool,
    ) -> CopFilterSet {
        // Build global exclude set (globs + regexes)
        let global_exclude_pats: Vec<&str> =
            self.global_excludes.iter().map(|s| s.as_str()).collect();
        let global_exclude = build_glob_set(&global_exclude_pats).unwrap_or_else(GlobSet::empty);
        let global_exclude_patterns = self
            .global_excludes
            .iter()
            .filter(|pattern| extract_ruby_regexp(pattern).is_none())
            .cloned()
            .collect();
        let global_exclude_re = build_regex_set(&global_exclude_pats);

        // Cross-cop dependency: Style/RedundantConstantBase disables itself when
        // Lint/ConstantResolution is enabled (they have conflicting requirements).
        let lint_constant_resolution_enabled = self
            .cop_configs
            .get("Lint/ConstantResolution")
            .is_some_and(|c| c.enabled == EnabledState::True);

        let filters: Vec<CopFilter> = registry
            .cops()
            .iter()
            .map(|cop| {
                let name = cop.name();
                let config = self.cop_configs.get(name);
                let dept = name.split('/').next().unwrap_or("");
                let dept_config = self.department_configs.get(dept);

                // Determine enabled state following RuboCop's enable_cop? logic
                // (vendor/rubocop/lib/rubocop/config.rb:380-390):
                //   1. Enabled: true on cop → enabled (any source)
                //   2. Department Enabled: false → disabled (unless cop has explicit Enabled: true)
                //   3. No Enabled key → use DisabledByDefault to decide
                let cop_enabled_state = config.map(|c| c.enabled).unwrap_or(EnabledState::Unset);
                let dept_enabled_state = dept_config
                    .map(|dc| dc.enabled)
                    .unwrap_or(EnabledState::Unset);

                let enabled_state = if cop_enabled_state == EnabledState::True {
                    // Department-level Enabled:false overrides cop-level Enabled:true
                    // when the cop's True came from defaults (not user config).
                    if !self.disabled_by_default
                        && dept_enabled_state == EnabledState::False
                        && !self.project_mentioned_cops.contains(name)
                    {
                        EnabledState::False
                    } else {
                        EnabledState::True
                    }
                } else if cop_enabled_state != EnabledState::Unset {
                    // Cop has explicit False or Pending
                    cop_enabled_state
                } else if dept_enabled_state == EnabledState::False {
                    // Department explicitly disabled, cop has no explicit setting
                    EnabledState::False
                } else {
                    // No explicit cop setting; department may be True/Unset.
                    // Without DisabledByDefault, department True promotes cop.
                    // With DisabledByDefault + department explicitly enabled by user
                    // (i.e., `Security: Enabled: true`, not just `Performance: Exclude:`),
                    // restore the cop's default Enabled value. This matches RuboCop's
                    // handle_disabled_by_default which re-enables default-enabled cops
                    // in explicitly-enabled departments.
                    if dept_enabled_state == EnabledState::True {
                        if !self.disabled_by_default
                            || self.project_enabled_depts.contains(dept) && cop.default_enabled()
                        {
                            EnabledState::True
                        } else {
                            EnabledState::Unset
                        }
                    } else {
                        EnabledState::Unset
                    }
                };

                let mut enabled = match enabled_state {
                    EnabledState::False => false,
                    EnabledState::Pending => self.new_cops == NewCopsPolicy::Enable,
                    EnabledState::Unset => !self.disabled_by_default && cop.default_enabled(),
                    EnabledState::True => true,
                };

                // Plugin department awareness: cops from plugin departments should
                // only run if the corresponding gem was loaded via require:/plugins:.
                if enabled
                    && is_plugin_department(dept)
                    && !self.require_departments.contains(dept)
                    && config.is_none_or(|c| c.enabled != EnabledState::True)
                {
                    enabled = false;
                }

                // Plugin version awareness: if this cop's department comes from a
                // `require:` gem but the cop itself is NOT mentioned in the installed
                // gem's config/default.yml, the cop doesn't exist in that gem version.
                // Disable it unless the user explicitly configured it.
                // Only apply this check when the gem's config was actually loaded
                // (i.e., require_known_cops contains at least one cop from this dept).
                let dept_has_known_cops = self
                    .require_known_cops
                    .iter()
                    .any(|c| c.starts_with(dept) && c.as_bytes().get(dept.len()) == Some(&b'/'));
                if enabled
                    && dept_has_known_cops
                    && self.require_departments.contains(dept)
                    && !self.require_known_cops.contains(name)
                    && config.is_none_or(|c| c.enabled != EnabledState::True)
                {
                    enabled = false;
                }

                // Core cop version awareness: if the installed rubocop gem's
                // config/default.yml was loaded (rubocop_known_cops is non-empty)
                // and this cop is from a core department but NOT mentioned in that
                // config, the cop doesn't exist in the project's rubocop version.
                // Disable it unless the user explicitly configured it.
                if enabled
                    && !self.rubocop_known_cops.is_empty()
                    && !is_plugin_department(dept)
                    && !self.rubocop_known_cops.contains(name)
                    && config.is_none_or(|c| c.enabled != EnabledState::True)
                {
                    enabled = false;
                }

                // Preview tier gating: preview cops are disabled unless --preview
                if enabled
                    && !preview
                    && tier_map.tier_for(name) == crate::cop::tiers::Tier::Preview
                {
                    enabled = false;
                }

                // Cross-cop dependency: Style/RedundantConstantBase is disabled
                // when Lint/ConstantResolution is enabled (conflicting rules).
                if enabled
                    && name == "Style/RedundantConstantBase"
                    && lint_constant_resolution_enabled
                {
                    enabled = false;
                }

                if !enabled {
                    return CopFilter {
                        enabled: false,
                        include_set: None,
                        exclude_set: None,
                        include_re: None,
                        exclude_re: None,
                    };
                }

                // Build effective include patterns (cop config > dept config > defaults)
                let include_patterns: Vec<&str> = match config {
                    Some(c) if !c.include.is_empty() => {
                        c.include.iter().map(|s| s.as_str()).collect()
                    }
                    _ => match dept_config {
                        Some(dc) if !dc.include.is_empty() => {
                            dc.include.iter().map(|s| s.as_str()).collect()
                        }
                        _ => cop.default_include().to_vec(),
                    },
                };

                // Build effective exclude patterns (cop config > dept config > defaults)
                let exclude_patterns: Vec<&str> = match config {
                    Some(c) if !c.exclude.is_empty() => {
                        c.exclude.iter().map(|s| s.as_str()).collect()
                    }
                    _ => match dept_config {
                        Some(dc) if !dc.exclude.is_empty() => {
                            dc.exclude.iter().map(|s| s.as_str()).collect()
                        }
                        _ => cop.default_exclude().to_vec(),
                    },
                };

                CopFilter {
                    enabled: true,
                    include_set: build_glob_set(&include_patterns),
                    exclude_set: build_glob_set(&exclude_patterns),
                    include_re: build_regex_set(&include_patterns),
                    exclude_re: build_regex_set(&exclude_patterns),
                }
            })
            .collect();

        // Discover sub-directory .rubocop.yml files for per-directory path relativity.
        // Reuse directories from dir_overrides when available to avoid rediscovery.
        let sub_config_dirs = if self.dir_overrides.is_empty() {
            self.config_dir
                .as_ref()
                .map(|cd| discover_sub_config_dirs(cd))
                .unwrap_or_default()
        } else {
            self.dir_overrides
                .iter()
                .map(|(dir, _)| dir.clone())
                .collect()
        };

        // Pre-compute universal vs pattern cop index lists.
        // Universal cops (enabled, no Include/Exclude) skip per-file glob matching.
        let mut universal_cop_indices = Vec::new();
        let mut pattern_cop_indices = Vec::new();
        for (i, filter) in filters.iter().enumerate() {
            if filter.is_universal() {
                universal_cop_indices.push(i);
            } else if filter.enabled {
                pattern_cop_indices.push(i);
            }
            // disabled cops go in neither list
        }

        CopFilterSet {
            global_exclude,
            global_exclude_patterns,
            global_exclude_re,
            filters,
            config_dir: self.config_dir.clone(),
            base_dir: self.base_dir.clone(),
            scan_root: None, // Set by the caller via set_scan_root()
            sub_config_dirs,
            universal_cop_indices,
            pattern_cop_indices,
            migrated_schema_version: self.migrated_schema_version.clone(),
        }
    }

    /// Return all cop names from the config that would be enabled given
    /// the current NewCops/DisabledByDefault settings.
    pub fn enabled_cop_names(&self) -> Vec<String> {
        self.cop_configs
            .iter()
            .filter(|(_name, config)| match config.enabled {
                EnabledState::True => true,
                EnabledState::Unset => !self.disabled_by_default,
                EnabledState::Pending => self.new_cops == NewCopsPolicy::Enable,
                EnabledState::False => false,
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Compute which cops are enabled by config but will not run, grouped by reason.
    pub fn compute_skip_summary(
        &self,
        registry: &CopRegistry,
        tier_map: &crate::cop::tiers::TierMap,
        preview: bool,
    ) -> crate::cop::tiers::SkipSummary {
        use std::collections::HashSet;

        let registry_names: HashSet<&str> = registry.cops().iter().map(|c| c.name()).collect();
        let baseline: HashSet<&str> = self
            .rubocop_known_cops
            .iter()
            .map(|s| s.as_str())
            .chain(self.require_known_cops.iter().map(|s| s.as_str()))
            .collect();

        let mut summary = crate::cop::tiers::SkipSummary::default();

        for name in self.enabled_cop_names() {
            if registry_names.contains(name.as_str()) {
                // Implemented — check if preview-gated
                if !preview && tier_map.tier_for(&name) == crate::cop::tiers::Tier::Preview {
                    summary.preview_gated.push(name);
                }
            } else if baseline.contains(name.as_str()) {
                // In vendor baseline but not implemented
                summary.unimplemented.push(name);
            } else {
                // Not in vendor baseline at all
                summary.outside_baseline.push(name);
            }
        }

        // Sort each bucket for deterministic output
        summary.preview_gated.sort();
        summary.unimplemented.sort();
        summary.outside_baseline.sort();

        summary
    }
}

fn parse_cop_config(value: &Value) -> CopConfig {
    let mut config = CopConfig::default();

    if let Value::Mapping(map) = value {
        for (k, v) in map {
            let key = match k.as_str() {
                Some(s) => s,
                None => continue,
            };
            match key {
                "Enabled" => {
                    if let Some(b) = v.as_bool() {
                        config.enabled = if b {
                            EnabledState::True
                        } else {
                            EnabledState::False
                        };
                    } else if v.as_str() == Some("pending") {
                        config.enabled = EnabledState::Pending;
                    }
                }
                "Severity" => {
                    if let Some(s) = v.as_str() {
                        config.severity = Severity::from_str(s);
                    }
                }
                "Exclude" => {
                    if let Some(list) = value_to_string_list(v) {
                        config.exclude = list;
                    }
                }
                "Include" => {
                    if let Some(list) = value_to_string_list(v) {
                        config.include = list;
                    }
                }
                _ => {
                    config.options.insert(key.to_string(), v.clone());
                }
            }
        }
    }

    config
}

/// Parse a department-level config (e.g. `RSpec:` or `Rails:`).
fn parse_department_config(value: &Value) -> DepartmentConfig {
    let mut config = DepartmentConfig::default();

    if let Value::Mapping(map) = value {
        for (k, v) in map {
            match k.as_str() {
                Some("Enabled") => {
                    if let Some(b) = v.as_bool() {
                        config.enabled = if b {
                            EnabledState::True
                        } else {
                            EnabledState::False
                        };
                    } else if v.as_str() == Some("pending") {
                        config.enabled = EnabledState::Pending;
                    }
                }
                Some("Include") => {
                    if let Some(list) = value_to_string_list(v) {
                        config.include = list;
                    }
                }
                Some("Exclude") => {
                    if let Some(list) = value_to_string_list(v) {
                        config.exclude = list;
                    }
                }
                _ => {}
            }
        }
    }

    config
}

/// Parse the `inherit_mode` key from a config file.
fn parse_inherit_mode(value: &Value) -> InheritMode {
    let mut mode = InheritMode::default();

    if let Value::Mapping(map) = value {
        if let Some(merge_value) = map.get(Value::String("merge".to_string())) {
            if let Some(list) = value_to_string_list(merge_value) {
                mode.merge = list.into_iter().collect();
            }
        }
        if let Some(override_value) = map.get(Value::String("override".to_string())) {
            if let Some(list) = value_to_string_list(override_value) {
                mode.override_keys = list.into_iter().collect();
            }
        }
    }

    mode
}

fn extract_string_list(value: &Value, key: &str) -> Option<Vec<String>> {
    value
        .as_mapping()?
        .get(Value::String(key.to_string()))?
        .as_sequence()
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
}

fn value_to_string_list(value: &Value) -> Option<Vec<String>> {
    value.as_sequence().map(|seq| {
        seq.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    })
}

/// Match a RuboCop-style glob pattern against a file path.
///
/// Mapping from plugin department names to the gem that provides them.
/// Used to register departments from requested gems even when gem resolution fails.
/// Includes standard-family wrapper gems that wrap rubocop plugin gems.
const PLUGIN_GEM_DEPARTMENTS: &[(&str, &str)] = &[
    ("Rails", "rubocop-rails"),
    ("Migration", "rubocop-rails"),
    ("RSpec", "rubocop-rspec"),
    ("RSpecRails", "rubocop-rspec_rails"),
    ("FactoryBot", "rubocop-factory_bot"),
    ("Capybara", "rubocop-capybara"),
    ("Rake", "rubocop-rake"),
    ("Performance", "rubocop-performance"),
    // standard-family wrapper gems
    ("Rails", "standard-rails"),
    ("Migration", "standard-rails"),
    ("Performance", "standard-performance"),
];

/// Select config file for the `standard` gem based on target ruby version.
/// Mirrors Standard::Base::Plugin — each ruby-X.Y.yml inherits from
/// the next version up, chaining back to base.yml.
fn standard_version_config(ruby_version: f64) -> &'static str {
    if ruby_version < 1.9 {
        "config/ruby-1.8.yml"
    } else if ruby_version < 2.0 {
        "config/ruby-1.9.yml"
    } else if ruby_version < 2.1 {
        "config/ruby-2.0.yml"
    } else if ruby_version < 2.2 {
        "config/ruby-2.1.yml"
    } else if ruby_version < 2.3 {
        "config/ruby-2.2.yml"
    } else if ruby_version < 2.4 {
        "config/ruby-2.3.yml"
    } else if ruby_version < 2.5 {
        "config/ruby-2.4.yml"
    } else if ruby_version < 2.6 {
        "config/ruby-2.5.yml"
    } else if ruby_version < 2.7 {
        "config/ruby-2.6.yml"
    } else if ruby_version < 3.0 {
        "config/ruby-2.7.yml"
    } else if ruby_version < 3.1 {
        "config/ruby-3.0.yml"
    } else if ruby_version < 3.2 {
        "config/ruby-3.1.yml"
    } else if ruby_version < 3.3 {
        "config/ruby-3.2.yml"
    } else if ruby_version < 3.4 {
        "config/ruby-3.3.yml"
    } else {
        "config/base.yml"
    }
}

/// Select config file for the `standard-performance` gem based on target ruby version.
/// Mirrors Standard::Performance::DeterminesYamlPath.
fn standard_perf_version_config(ruby_version: f64) -> &'static str {
    if ruby_version < 1.9 {
        "config/ruby-1.8.yml"
    } else if ruby_version < 2.0 {
        "config/ruby-1.9.yml"
    } else if ruby_version < 2.1 {
        "config/ruby-2.0.yml"
    } else if ruby_version < 2.2 {
        "config/ruby-2.1.yml"
    } else if ruby_version < 2.3 {
        "config/ruby-2.2.yml"
    } else {
        "config/base.yml"
    }
}

/// Map a standard-family gem name to its config file path.
/// Returns None if the gem is not a recognized standard-family gem.
fn standard_gem_config_path(gem_name: &str, ruby_version: Option<f64>) -> Option<&'static str> {
    match gem_name {
        "standard" => Some(standard_version_config(ruby_version.unwrap_or(3.4))),
        "standard-performance" => Some(standard_perf_version_config(ruby_version.unwrap_or(3.4))),
        "standard-rails" | "standard-custom" => Some("config/base.yml"),
        _ => None,
    }
}

/// Returns true if the department belongs to a RuboCop plugin gem and should
/// only run when the corresponding gem is loaded via `require:` or `plugins:`.
///
/// Core departments (Layout, Lint, Style, Metrics, Naming, Security, Bundler,
/// Gemspec) are always available. Plugin departments need their gem loaded.
fn is_plugin_department(dept: &str) -> bool {
    PLUGIN_GEM_DEPARTMENTS.iter().any(|(d, _)| *d == dept)
}

/// Resolve TargetRubyVersion from a gemspec's `required_ruby_version` constraint.
/// Mirrors RuboCop's `TargetRuby::GemspecFile` source which finds the minimum known
/// Ruby version that satisfies the constraint.
fn resolve_ruby_version_from_gemspec(config_dir: &Path) -> Option<f64> {
    // Known Ruby versions (same as RuboCop's KNOWN_RUBIES)
    const KNOWN_RUBIES: &[f64] = &[
        2.0, 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 3.0, 3.1, 3.2, 3.3, 3.4, 4.0, 4.1,
    ];

    // Find a single .gemspec file in config_dir (Bundler convention)
    let entries: Vec<_> = std::fs::read_dir(config_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "gemspec"))
        .collect();
    if entries.len() != 1 {
        return None; // Must be exactly one gemspec
    }
    let content = std::fs::read_to_string(entries[0].path()).ok()?;

    // Find required_ruby_version assignment and extract the constraint string.
    // Handles patterns like: spec.required_ruby_version = ">= 3.2.0"
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if !trimmed.contains(".required_ruby_version") {
            continue;
        }
        let after = trimmed.split(".required_ruby_version").nth(1)?;
        let after = after.trim_start();
        if !after.starts_with('=') {
            continue;
        }
        // Find the first quoted string
        let quote_start = after.find(['\'', '"'])?;
        let qc = after.as_bytes()[quote_start] as char;
        let rest = &after[quote_start + 1..];
        let quote_end = rest.find(qc)?;
        let constraint = &rest[..quote_end];

        // Parse the constraint: extract operator and version digits
        let version_part = constraint.trim_start_matches(|c: char| !c.is_ascii_digit());
        let digits: Vec<&str> = version_part.split('.').collect();
        if digits.len() < 2 {
            return None;
        }
        let major: u64 = digits[0].parse().ok()?;
        let minor: u64 = digits[1].parse().ok()?;
        let min_version = major as f64 + minor as f64 / 10.0;

        // Find the minimum known Ruby that satisfies the constraint.
        // For ">= X.Y", this is X.Y if it's in KNOWN_RUBIES.
        // For "~> X.Y", this is X.Y (pessimistic: >= X.Y, < (X+1).0).
        return KNOWN_RUBIES.iter().copied().find(|&v| v >= min_version);
    }
    None
}

/// Parse a gem's major.minor version from a Gemfile.lock/gems.locked file.
/// Returns the version as a float (e.g. 7.1 for "7.1.3.4").
fn parse_gem_version_from_lockfile(content: &str, gem_name: &str) -> Option<f64> {
    // Gemfile.lock format has gems indented with 4 spaces in the GEM/specs section:
    //     railties (7.1.3.4)
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(gem_name) {
            if let Some(ver_str) = rest.strip_prefix(" (") {
                if let Some(ver_str) = ver_str.strip_suffix(')') {
                    let parts: Vec<&str> = ver_str.split('.').collect();
                    if parts.len() >= 2 {
                        if let (Ok(major), Ok(minor)) =
                            (parts[0].parse::<u64>(), parts[1].parse::<u64>())
                        {
                            return Some(major as f64 + minor as f64 / 10.0);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Patterns like `db/migrate/**/*.rb` or `**/*_spec.rb` are matched against
/// the path. We try matching against both the full path and just the relative
/// components to handle RuboCop's convention of patterns relative to project root.
fn glob_matches(pattern: &str, path: &Path) -> bool {
    // Check if this is a Ruby regexp pattern (from !ruby/regexp /pattern/)
    if let Some(re_pattern) = extract_ruby_regexp(pattern) {
        if let Ok(re) = regex::Regex::new(re_pattern) {
            let path_str = path.to_string_lossy();
            return re.is_match(&path_str);
        }
        return false;
    }
    let glob = match GlobBuilder::new(pattern).literal_separator(false).build() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let matcher = glob.compile_matcher();
    // Try matching against the path as given
    if matcher.is_match(path) {
        return true;
    }
    // Also try matching against just the path string (handles both relative and absolute)
    let path_str = path.to_string_lossy();
    matcher.is_match(path_str.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn write_config(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join(".rubocop.yml");
        fs::write(&path, content).unwrap();
        path
    }

    fn write_yaml(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn missing_config_returns_empty() {
        let config = load_config(Some(Path::new("/nonexistent/.rubocop.yml")), None, None).unwrap();
        assert!(config.global_excludes().is_empty());
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));
    }

    #[test]
    fn allcops_exclude() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_exclude");
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(
            &dir,
            "AllCops:\n  Exclude:\n    - 'vendor/**'\n    - 'tmp/**'\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        let excludes = config.global_excludes();
        // User patterns must be present (merged with rubocop defaults / fallback)
        assert!(
            excludes.iter().any(|e| e == "vendor/**"),
            "vendor/** missing: {excludes:?}"
        );
        assert!(
            excludes.iter().any(|e| e == "tmp/**"),
            "tmp/** missing: {excludes:?}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cop_enabled_false() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_disabled");
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(&dir, "Style/Foo:\n  Enabled: false\n");
        let config = load_config(Some(&path), None, None).unwrap();
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));
        // Unknown cops default to enabled
        assert!(config.is_cop_enabled("Style/Bar", Path::new("a.rb"), &[], &[]));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cop_severity_override() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_severity");
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(&dir, "Style/Foo:\n  Severity: error\n");
        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Style/Foo");
        assert_eq!(cc.severity, Some(Severity::Error));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cop_exclude_include_patterns() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_patterns");
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(
            &dir,
            "Style/Foo:\n  Exclude:\n    - 'spec/**'\n  Include:\n    - '**/*.rake'\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Style/Foo");
        assert_eq!(cc.exclude, vec!["spec/**".to_string()]);
        assert_eq!(cc.include, vec!["**/*.rake".to_string()]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cop_custom_options() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_options");
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(&dir, "Layout/LineLength:\n  Max: 120\n");
        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Layout/LineLength");
        assert_eq!(cc.options.get("Max").and_then(|v| v.as_u64()), Some(120));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn first_hash_element_indentation_inherits_hash_alignment_styles() {
        let temp_dir = tempfile::tempdir().unwrap();
        let dir = temp_dir.path();
        let path = write_config(
            dir,
            "Layout/HashAlignment:\n  EnforcedColonStyle: separator\n  EnforcedHashRocketStyle:\n    - key\n    - separator\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Layout/FirstHashElementIndentation");
        assert_eq!(
            cc.options
                .get("EnforcedColonStyle")
                .and_then(|v| v.as_str()),
            Some("separator")
        );
        let rocket_styles = cc
            .options
            .get("EnforcedHashRocketStyle")
            .and_then(|v| v.as_sequence())
            .expect("EnforcedHashRocketStyle should be inherited as a sequence");
        assert_eq!(rocket_styles.len(), 2);
        assert_eq!(rocket_styles[0].as_str(), Some("key"));
        assert_eq!(rocket_styles[1].as_str(), Some("separator"));
    }

    #[test]
    fn non_cop_keys_ignored() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_noncop");
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(
            &dir,
            "AllCops:\n  Exclude: []\nrequire:\n  - rubocop-rspec\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        // "require" has no "/" so should not be treated as a cop
        assert!(config.is_cop_enabled("require", Path::new("a.rb"), &[], &[]));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_cop_config() {
        let config = load_config(Some(Path::new("/nonexistent/.rubocop.yml")), None, None).unwrap();
        let cc = config.cop_config("Style/Whatever");
        assert_eq!(cc.enabled, EnabledState::Unset);
        assert!(cc.severity.is_none());
        assert!(cc.exclude.is_empty());
        // Only injected internal keys should be present (e.g. __RailtiesInLockfile)
        let user_keys: Vec<_> = cc.options.keys().filter(|k| !k.starts_with("__")).collect();
        assert!(
            user_keys.is_empty(),
            "unexpected user options: {user_keys:?}"
        );
    }

    // ---- Path-based Include/Exclude tests ----

    #[test]
    fn default_include_filters_files() {
        let config = load_config(Some(Path::new("/nonexistent/.rubocop.yml")), None, None).unwrap();
        // With default_include set, only matching files pass
        // Use a core department (Style) so plugin department filtering doesn't apply.
        let inc = &["db/migrate/**/*.rb"];
        assert!(config.is_cop_enabled(
            "Style/Foo",
            Path::new("db/migrate/001_create.rb"),
            inc,
            &[]
        ));
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("app/models/user.rb"), inc, &[]));
    }

    #[test]
    fn default_exclude_filters_files() {
        let config = load_config(Some(Path::new("/nonexistent/.rubocop.yml")), None, None).unwrap();
        let exc = &["spec/**/*.rb"];
        assert!(config.is_cop_enabled("Style/Foo", Path::new("app/models/user.rb"), &[], exc));
        assert!(!config.is_cop_enabled(
            "Style/Foo",
            Path::new("spec/models/user_spec.rb"),
            &[],
            exc
        ));
    }

    #[test]
    fn user_include_overrides_default() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_inc_override");
        fs::create_dir_all(&dir).unwrap();
        // Use a core department cop (Style/) so plugin department filtering doesn't apply
        let path = write_config(&dir, "Style/Migration:\n  Include:\n    - 'db/**/*.rb'\n");
        let config = load_config(Some(&path), None, None).unwrap();
        // Default include is narrower but user config overrides
        let default_inc = &["db/migrate/**/*.rb"];
        assert!(config.is_cop_enabled(
            "Style/Migration",
            Path::new("db/seeds.rb"),
            default_inc,
            &[]
        ));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn global_excludes_applied() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_global_exc");
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(&dir, "AllCops:\n  Exclude:\n    - 'vendor/**'\n");
        let config = load_config(Some(&path), None, None).unwrap();
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("vendor/gems/foo.rb"), &[], &[]));
        assert!(config.is_cop_enabled("Style/Foo", Path::new("app/models/user.rb"), &[], &[]));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn glob_matches_basic() {
        assert!(glob_matches("**/*.rb", Path::new("app/models/user.rb")));
        assert!(glob_matches(
            "db/migrate/**/*.rb",
            Path::new("db/migrate/001_create.rb")
        ));
        assert!(!glob_matches(
            "db/migrate/**/*.rb",
            Path::new("app/models/user.rb")
        ));
        assert!(glob_matches(
            "spec/**",
            Path::new("spec/models/user_spec.rb")
        ));
    }

    // ---- Ruby regexp tests ----

    #[test]
    fn extract_ruby_regexp_basic() {
        assert_eq!(extract_ruby_regexp("/vendor/"), Some("vendor"));
        assert_eq!(
            extract_ruby_regexp("/(vendor|bundle|bin)($|\\/.*)/"),
            Some("(vendor|bundle|bin)($|\\/.*)")
        );
        // Regex with flags
        assert_eq!(extract_ruby_regexp("/pattern/i"), Some("pattern"));
        assert_eq!(extract_ruby_regexp("/pattern/ixm"), Some("pattern"));
        // Not a regexp — just a regular string
        assert_eq!(extract_ruby_regexp("vendor/**"), None);
        assert_eq!(extract_ruby_regexp(""), None);
        // Single slash only — not a valid regexp
        assert_eq!(extract_ruby_regexp("/"), None);
        // Absolute filesystem paths must NOT be treated as regexps
        assert_eq!(extract_ruby_regexp("/fake/repo/cookbooks/**/*"), None);
        assert_eq!(extract_ruby_regexp("/tmp/foo.rb"), None);
        assert_eq!(extract_ruby_regexp("/home/user/vendor/**/*"), None);
        assert_eq!(extract_ruby_regexp("/absolute/path/to/file.rb"), None);
    }

    #[test]
    fn glob_matches_handles_ruby_regexp() {
        // Regex pattern: matches paths containing "vendor"
        assert!(glob_matches(
            "/(vendor|bundle|bin)($|\\/.*)/",
            Path::new("vendor/gems/foo.rb")
        ));
        assert!(glob_matches(
            "/(vendor|bundle|bin)($|\\/.*)/",
            Path::new("bundle/config")
        ));
        assert!(glob_matches(
            "/(vendor|bundle|bin)($|\\/.*)/",
            Path::new("bin/rails")
        ));
        // Should NOT match unrelated paths
        assert!(!glob_matches(
            "/(vendor|bundle|bin)($|\\/.*)/",
            Path::new("app/models/user.rb")
        ));
    }

    #[test]
    fn ruby_regexp_in_global_excludes() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_ruby_regexp");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Config with !ruby/regexp in Exclude alongside regular string patterns.
        // Use a simple regex that clearly matches vendor/*, tmp/*, etc.
        let path = write_config(
            &dir,
            "AllCops:\n  Exclude:\n    - 'config/initializers/forbidden_yaml.rb'\n    - !ruby/regexp /(vendor|bundle|tmp|server)($|\\/.*)/\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();

        // Verify the regexp pattern is stored in global_excludes (as a plain /.../ string)
        let re_count = config
            .global_excludes()
            .iter()
            .filter(|s| extract_ruby_regexp(s).is_some())
            .count();
        assert!(
            re_count > 0,
            "Expected at least one regexp pattern in global_excludes, got: {:?}",
            config.global_excludes()
        );

        // Regular string pattern should still work
        assert!(!config.is_cop_enabled(
            "Style/Foo",
            Path::new("config/initializers/forbidden_yaml.rb"),
            &[],
            &[]
        ));
        // Regexp pattern should match vendor paths
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("vendor/bundle/foo.rb"), &[], &[]));
        // Regexp pattern should match tmp paths
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("tmp/cache/something.rb"), &[], &[]));
        // Regexp pattern should match bare "vendor" (end-of-string)
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("vendor"), &[], &[]));
        // Non-matching paths should still work
        assert!(config.is_cop_enabled("Style/Foo", Path::new("app/models/user.rb"), &[], &[]));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ruby_regexp_in_cop_filter_set_global_excludes() {
        let dir = std::env::temp_dir().join("nitrocop_test_config_ruby_regexp_filter");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = write_config(
            &dir,
            "AllCops:\n  Exclude:\n    - 'tmp/**'\n    - !ruby/regexp /(vendor|bundle)($|\\/.*)/\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        // Build a minimal filter set to test is_globally_excluded
        let pats: Vec<&str> = config
            .global_excludes()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let global_exclude = build_glob_set(&pats).unwrap_or_else(GlobSet::empty);
        let global_exclude_re = build_regex_set(&pats);
        let filter_set = CopFilterSet {
            global_exclude,
            global_exclude_patterns: pats.iter().map(|pat| (*pat).to_string()).collect(),
            global_exclude_re,
            filters: Vec::new(),
            config_dir: config.config_dir().map(|p| p.to_path_buf()),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        // Glob pattern should work
        assert!(
            filter_set
                .is_globally_excluded(Path::new(&format!("{}/tmp/cache/foo.rb", dir.display())))
        );
        // Regex pattern should work
        assert!(
            filter_set
                .is_globally_excluded(Path::new(&format!("{}/vendor/gems/bar.rb", dir.display())))
        );
        assert!(
            filter_set.is_globally_excluded(Path::new(&format!("{}/bundle/config", dir.display())))
        );
        // Non-matching should not be excluded
        assert!(
            !filter_set
                .is_globally_excluded(Path::new(&format!("{}/app/models/user.rb", dir.display())))
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corpus_overlay_config_absolute_excludes_load_correctly() {
        // Integration test: mimics the overlay config that gen_repo_config.py
        // produces. The overlay lives in a temp dir (config_dir != base_dir)
        // and adds absolute-path Exclude entries. This catches regressions
        // where absolute paths are silently misclassified (e.g., treated as
        // Ruby regexps) and dropped from the GlobSet.
        let dir = std::env::temp_dir().join("nitrocop_test_corpus_overlay_config");
        let _ = fs::remove_dir_all(&dir);
        let fake_repo = dir.join("repo");
        let overlay_dir = dir.join("overlays");
        fs::create_dir_all(&fake_repo).unwrap();
        fs::create_dir_all(&overlay_dir).unwrap();

        // Write a minimal baseline config
        let baseline = fake_repo.join("baseline.yml");
        fs::write(
            &baseline,
            "AllCops:\n  Exclude:\n    - 'node_modules/**/*'\n",
        )
        .unwrap();

        // Write overlay config with absolute-path Exclude entries (like gen_repo_config.py)
        let overlay = overlay_dir.join("corpus_config_test.yml");
        fs::write(
            &overlay,
            format!(
                "inherit_from: {baseline}\n\nAllCops:\n  Exclude:\n    - \"{repo}/vendor/**/*\"\n    - \"{repo}/cookbooks/**/*\"\n",
                baseline = baseline.display(),
                repo = fake_repo.display(),
            ),
        )
        .unwrap();

        let config = load_config(Some(&overlay), None, None).unwrap();
        let excludes = config.global_excludes();

        // The absolute patterns must be present in the resolved excludes
        let vendor_pat = format!("{}/vendor/**/*", fake_repo.display());
        let cookbooks_pat = format!("{}/cookbooks/**/*", fake_repo.display());
        assert!(
            excludes.contains(&vendor_pat),
            "absolute vendor pattern should be in global_excludes, got: {excludes:?}"
        );
        assert!(
            excludes.contains(&cookbooks_pat),
            "absolute cookbooks pattern should be in global_excludes, got: {excludes:?}"
        );

        // Build filter set and verify matching works end-to-end
        let pats: Vec<&str> = excludes.iter().map(|s| s.as_str()).collect();
        let global_exclude = build_glob_set(&pats).unwrap_or_else(GlobSet::empty);
        let filter_set = CopFilterSet {
            global_exclude,
            global_exclude_patterns: pats.iter().map(|p| p.to_string()).collect(),
            global_exclude_re: build_regex_set(&pats),
            filters: Vec::new(),
            config_dir: Some(overlay_dir.clone()),
            base_dir: Some(fake_repo.clone()),
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };

        assert!(
            filter_set.is_globally_excluded(&fake_repo.join("vendor/gems/foo.rb")),
            "absolute vendor pattern should exclude vendor/ files"
        );
        assert!(
            filter_set.is_globally_excluded(&fake_repo.join("cookbooks/helpers.rb")),
            "absolute cookbooks pattern should exclude cookbooks/ files"
        );
        assert!(
            !filter_set.is_globally_excluded(&fake_repo.join("cookbooks-override/helpers.rb")),
            "cookbooks pattern must NOT exclude cookbooks-override/"
        );
        assert!(
            !filter_set.is_globally_excluded(&fake_repo.join("app/models/user.rb")),
            "non-matching paths should not be excluded"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn global_exclude_does_not_match_prefixed_nested_repo_path() {
        let pats = vec!["bin/**/*"];
        let global_exclude = build_glob_set(&pats).unwrap_or_else(GlobSet::empty);
        let filter_set = CopFilterSet {
            global_exclude,
            global_exclude_patterns: pats.iter().map(|pat| (*pat).to_string()).collect(),
            global_exclude_re: None,
            filters: Vec::new(),
            config_dir: Some(PathBuf::from(".")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };

        assert!(
            !filter_set.is_globally_excluded(Path::new("corpus/sample_repo/bin/BadFile.rb")),
            "bin/**/* should not exclude nested repo paths that only contain bin as a component"
        );
        assert!(
            filter_set.is_globally_excluded(Path::new("bin/BadFile.rb")),
            "bin/**/* should still exclude project-root bin paths"
        );
    }

    #[test]
    fn file_name_filter_still_matches_prefixed_nested_repo_path() {
        let dir = std::env::temp_dir().join("nitrocop_test_file_name_prefixed_repo_path");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("corpus/sample_repo/bin")).unwrap();
        fs::write(
            dir.join(".rubocop.yml"),
            "AllCops:\n  Exclude:\n    - 'bin/**/*'\n",
        )
        .unwrap();

        let config = load_config(Some(&dir.join(".rubocop.yml")), None, None).unwrap();
        let registry = crate::cop::registry::CopRegistry::default_registry();
        let tier_map = crate::cop::tiers::TierMap::load();
        let filters = config.build_cop_filters(&registry, &tier_map, true);
        let index = registry
            .cops()
            .iter()
            .position(|cop| cop.name() == "Naming/FileName")
            .expect("Naming/FileName should be registered");
        let cop_config = config.cop_config("Naming/FileName");
        let filter = filters.cop_filter(index);
        let tier = tier_map.tier_for("Naming/FileName");

        assert!(
            filter.is_enabled(),
            "Naming/FileName should be enabled under --preview, config: {cop_config:?}, disabled_by_default: {}, rubocop_known: {}, tier: {tier:?}",
            config.disabled_by_default,
            config.rubocop_known_cops.contains("Naming/FileName")
        );

        assert!(
            filters.is_cop_match(index, Path::new("corpus/sample_repo/bin/BadFile.rb")),
            "Naming/FileName should still run on nested repo files that only contain bin as a component; config: {cop_config:?}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_regex_set_filters_correctly() {
        // Mix of glob and regex patterns
        let patterns = vec!["vendor/**", "/(tmp|cache)($|\\/.*)/", "**/*.log"];
        let re = build_regex_set(&patterns);
        assert!(
            re.is_some(),
            "Should build a regex set with 1 regex pattern"
        );
        let re = re.unwrap();
        assert!(re.is_match("tmp/foo.rb"));
        assert!(re.is_match("cache/bar.rb"));
        assert!(!re.is_match("vendor/foo.rb")); // This is a glob, not a regex

        // Glob set should NOT include the regex pattern
        let gs = build_glob_set(&patterns);
        assert!(gs.is_some());
        let gs = gs.unwrap();
        assert!(gs.is_match(Path::new("vendor/foo.rb")));
        assert!(gs.is_match(Path::new("something.log")));
        // The regex pattern was skipped in glob set, so /tmp/... won't be a glob match
    }

    // ---- Inheritance tests ----

    #[test]
    fn inherit_from_single_file() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_single");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base.yml",
            "Layout/LineLength:\n  Max: 100\nStyle/Foo:\n  Enabled: true\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\nLayout/LineLength:\n  Max: 120\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        // Child overrides base's Max
        let cc = config.cop_config("Layout/LineLength");
        assert_eq!(cc.options.get("Max").and_then(|v| v.as_u64()), Some(120));
        // Base's Style/Foo is still present
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inherit_from_array() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_array");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base1.yml",
            "AllCops:\n  Exclude:\n    - 'vendor/**'\nStyle/Foo:\n  Enabled: false\n",
        );
        write_yaml(
            &dir,
            "base2.yml",
            "AllCops:\n  Exclude:\n    - 'tmp/**'\nStyle/Foo:\n  Enabled: true\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from:\n  - base1.yml\n  - base2.yml\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        // Global excludes are appended from both bases
        assert!(config.global_excludes().contains(&"vendor/**".to_string()));
        assert!(config.global_excludes().contains(&"tmp/**".to_string()));
        // Style/Foo: base2 overrides base1 (last writer wins)
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inherit_from_child_overrides_base() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_override");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(&dir, "base.yml", "Style/Foo:\n  Enabled: true\n");
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\nStyle/Foo:\n  Enabled: false\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inherit_from_exclude_appends() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_exclude");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base.yml",
            "Style/Foo:\n  Exclude:\n    - 'vendor/**'\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\nStyle/Foo:\n  Exclude:\n    - 'tmp/**'\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Style/Foo");
        assert!(cc.exclude.contains(&"vendor/**".to_string()));
        assert!(cc.exclude.contains(&"tmp/**".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn allcops_exclude_merges_with_inherited() {
        // RuboCop always merges AllCops.Exclude (union), unlike per-cop Exclude.
        // A child config's AllCops.Exclude should append to the parent's, not replace.
        let dir = std::env::temp_dir().join("nitrocop_test_allcops_exclude_merge");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base.yml",
            "AllCops:\n  Exclude:\n    - 'vendor/**/*'\n    - 'tmp/**/*'\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\nAllCops:\n  Exclude:\n    - 'coverage/**/*'\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let excludes = config.global_excludes();
        assert!(
            excludes.iter().any(|e| e == "vendor/**/*"),
            "inherited vendor exclude missing: {excludes:?}"
        );
        assert!(
            excludes.iter().any(|e| e == "tmp/**/*"),
            "inherited tmp exclude missing: {excludes:?}"
        );
        assert!(
            excludes.iter().any(|e| e == "coverage/**/*"),
            "local coverage exclude missing: {excludes:?}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn allcops_exclude_override_replaces() {
        // When inherit_mode explicitly overrides Exclude, AllCops.Exclude
        // from inherit_from is replaced (not merged). However, rubocop's own
        // built-in defaults (vendor/**/*, etc.) are always present — the
        // override only affects the inherit chain, not the built-in layer.
        let dir = std::env::temp_dir().join("nitrocop_test_allcops_exclude_override");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // base.yml has a CUSTOM pattern (spec/fixtures/**/*)
        write_yaml(
            &dir,
            "base.yml",
            "AllCops:\n  Exclude:\n    - 'spec/fixtures/**/*'\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\ninherit_mode:\n  override:\n    - Exclude\nAllCops:\n  Exclude:\n    - 'coverage/**/*'\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let excludes = config.global_excludes();
        // The inherit_from base's custom pattern should be overridden
        assert!(
            !excludes.iter().any(|e| e == "spec/fixtures/**/*"),
            "inherited custom exclude should be replaced: {excludes:?}"
        );
        // The local override pattern should be present
        assert!(
            excludes.iter().any(|e| e == "coverage/**/*"),
            "local coverage exclude missing: {excludes:?}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inherit_from_include_replaces() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_include");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base.yml",
            "Style/Foo:\n  Include:\n    - '**/*.rb'\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\nStyle/Foo:\n  Include:\n    - 'app/**'\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Style/Foo");
        // Include is replaced, not appended
        assert_eq!(cc.include, vec!["app/**".to_string()]);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inherit_from_missing_warns_but_succeeds() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_missing");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: nonexistent.yml\nStyle/Foo:\n  Enabled: false\n",
        );

        // Should succeed (prints a warning to stderr)
        let config = load_config(Some(&path), None, None).unwrap();
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn circular_inherit_from_breaks_cycle() {
        // A→B→A is a true cycle but it's safely broken: the second visit to
        // a.yml returns an empty layer instead of recursing. No infinite loop.
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_circular");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(&dir, "a.yml", "inherit_from: b.yml\n");
        write_yaml(&dir, "b.yml", "inherit_from: a.yml\n");

        let path = dir.join("a.yml");
        let result = load_config(Some(&path), None, None);
        assert!(
            result.is_ok(),
            "Expected cycle to be safely broken, got: {result:?}"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn require_key_silently_ignored() {
        let dir = std::env::temp_dir().join("nitrocop_test_require_ignored");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "require:\n  - rubocop-rspec\n  - rubocop-rails\nplugins:\n  - rubocop-performance\nStyle/Foo:\n  Enabled: false\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn standard_version_config_selects_correct_file() {
        assert_eq!(standard_version_config(1.8), "config/ruby-1.8.yml");
        assert_eq!(standard_version_config(1.9), "config/ruby-1.9.yml");
        assert_eq!(standard_version_config(2.0), "config/ruby-2.0.yml");
        assert_eq!(standard_version_config(2.7), "config/ruby-2.7.yml");
        assert_eq!(standard_version_config(3.0), "config/ruby-3.0.yml");
        assert_eq!(standard_version_config(3.1), "config/ruby-3.1.yml");
        assert_eq!(standard_version_config(3.2), "config/ruby-3.2.yml");
        assert_eq!(standard_version_config(3.3), "config/ruby-3.3.yml");
        // 3.4+ uses base.yml (latest, no overrides needed)
        assert_eq!(standard_version_config(3.4), "config/base.yml");
        assert_eq!(standard_version_config(3.5), "config/base.yml");
    }

    #[test]
    fn standard_perf_version_config_selects_correct_file() {
        assert_eq!(standard_perf_version_config(1.8), "config/ruby-1.8.yml");
        assert_eq!(standard_perf_version_config(2.2), "config/ruby-2.2.yml");
        // 2.3+ uses base.yml
        assert_eq!(standard_perf_version_config(2.3), "config/base.yml");
        assert_eq!(standard_perf_version_config(3.1), "config/base.yml");
    }

    #[test]
    fn standard_gem_config_path_recognizes_family() {
        // standard gem: version-specific
        assert_eq!(
            standard_gem_config_path("standard", Some(3.1)),
            Some("config/ruby-3.1.yml")
        );
        assert_eq!(
            standard_gem_config_path("standard", None),
            Some("config/base.yml") // defaults to 3.4 → base.yml
        );

        // standard-rails: always base
        assert_eq!(
            standard_gem_config_path("standard-rails", Some(3.1)),
            Some("config/base.yml")
        );

        // standard-custom: always base
        assert_eq!(
            standard_gem_config_path("standard-custom", None),
            Some("config/base.yml")
        );

        // standard-performance: version-specific for old Ruby
        assert_eq!(
            standard_gem_config_path("standard-performance", Some(2.0)),
            Some("config/ruby-2.0.yml")
        );
        assert_eq!(
            standard_gem_config_path("standard-performance", Some(3.1)),
            Some("config/base.yml")
        );

        // Unknown gems: None
        assert_eq!(standard_gem_config_path("rubocop-rspec", None), None);
        assert_eq!(standard_gem_config_path("some-other-gem", None), None);
    }

    #[test]
    fn deep_merge_cop_options() {
        let dir = std::env::temp_dir().join("nitrocop_test_deep_merge_opts");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base.yml",
            "Style/Foo:\n  Max: 100\n  EnforcedStyle: compact\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\nStyle/Foo:\n  Max: 120\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Style/Foo");
        // Max overridden by child
        assert_eq!(cc.options.get("Max").and_then(|v| v.as_u64()), Some(120));
        // EnforcedStyle preserved from base
        assert_eq!(
            cc.options.get("EnforcedStyle").and_then(|v| v.as_str()),
            Some("compact")
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn enabled_cop_names_returns_enabled_only() {
        let dir = std::env::temp_dir().join("nitrocop_test_enabled_names");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "Style/Foo:\n  Enabled: true\nStyle/Bar:\n  Enabled: false\nLint/Baz:\n  Max: 10\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let names = config.enabled_cop_names();
        assert!(names.contains(&"Style/Foo".to_string()));
        assert!(!names.contains(&"Style/Bar".to_string()));
        // Lint/Baz has no explicit Enabled, defaults to true
        assert!(names.contains(&"Lint/Baz".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    // ---- Merge logic unit tests ----

    #[test]
    fn merge_layer_scalars_last_writer_wins() {
        let mut base = ConfigLayer::empty();
        base.cop_configs.insert(
            "Style/Foo".to_string(),
            CopConfig {
                enabled: EnabledState::True,
                ..CopConfig::default()
            },
        );

        let mut overlay = ConfigLayer::empty();
        overlay.cop_configs.insert(
            "Style/Foo".to_string(),
            CopConfig {
                enabled: EnabledState::False,
                ..CopConfig::default()
            },
        );

        merge_layer_into(&mut base, &overlay, None);
        assert_eq!(base.cop_configs["Style/Foo"].enabled, EnabledState::False);
    }

    #[test]
    fn merge_layer_global_excludes_appended() {
        let mut base = ConfigLayer {
            global_excludes: vec!["vendor/**".to_string()],
            ..ConfigLayer::empty()
        };
        let overlay = ConfigLayer {
            global_excludes: vec!["tmp/**".to_string()],
            ..ConfigLayer::empty()
        };
        merge_layer_into(&mut base, &overlay, None);
        assert_eq!(base.global_excludes.len(), 2);
        assert!(base.global_excludes.contains(&"vendor/**".to_string()));
        assert!(base.global_excludes.contains(&"tmp/**".to_string()));
    }

    #[test]
    fn merge_layer_no_duplicate_excludes() {
        let mut base = ConfigLayer {
            global_excludes: vec!["vendor/**".to_string()],
            ..ConfigLayer::empty()
        };
        let overlay = ConfigLayer {
            global_excludes: vec!["vendor/**".to_string()],
            ..ConfigLayer::empty()
        };
        merge_layer_into(&mut base, &overlay, None);
        assert_eq!(base.global_excludes.len(), 1);
    }

    // ---- Auto-discovery tests ----

    #[test]
    fn auto_discover_config_from_target_dir() {
        let dir = std::env::temp_dir().join("nitrocop_test_autodiscover");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_config(&dir, "Style/Foo:\n  Enabled: false\n");

        // Auto-discover from target_dir
        let config = load_config(None, Some(&dir), None).unwrap();
        assert!(!config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));
        assert!(config.config_dir().is_some());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn auto_discover_walks_up_parent() {
        let parent = std::env::temp_dir().join("nitrocop_test_autodiscover_parent");
        let child = parent.join("app").join("models");
        let _ = fs::remove_dir_all(&parent);
        fs::create_dir_all(&child).unwrap();

        write_config(&parent, "Style/Bar:\n  Enabled: false\n");

        // Target is a subdirectory — should find config in parent
        let config = load_config(None, Some(&child), None).unwrap();
        assert!(!config.is_cop_enabled("Style/Bar", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&parent).ok();
    }

    #[test]
    fn no_config_found_applies_default_excludes() {
        let dir = std::env::temp_dir().join("nitrocop_test_no_config");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let config = load_config(None, Some(&dir), None).unwrap();
        // Even without a config file, RuboCop's default AllCops.Exclude
        // patterns are applied (vendor/**/* etc.), matching RuboCop's behavior.
        assert!(
            !config.global_excludes().is_empty(),
            "default excludes should be present even without config file"
        );
        assert!(
            config
                .global_excludes()
                .iter()
                .any(|e| e.contains("vendor")),
            "vendor/**/* should be in default excludes"
        );
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_config_explicit_file_uses_cwd_for_lockfile_resolution() {
        let repo = std::env::temp_dir().join("nitrocop_test_no_config_explicit_file");
        let nested = repo.join("test").join("functional").join("cms");
        let file = nested.join("content_controller_test.rb");
        let old_cwd = std::env::current_dir().unwrap();
        let _ = fs::remove_dir_all(&repo);
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            repo.join("Gemfile.lock"),
            "GEM\n  specs:\n    railties (3.2.22.5)\n",
        )
        .unwrap();
        fs::write(&file, "get :show, :path => 'about'\n").unwrap();

        std::env::set_current_dir(&repo).unwrap();
        let config = load_config(None, Some(&file), None).unwrap();
        std::env::set_current_dir(old_cwd).unwrap();

        assert_eq!(config.base_dir(), Some(repo.as_path()));
        assert_eq!(config.target_rails_version, Some(3.2));

        fs::remove_dir_all(&repo).ok();
    }

    // ---- EnabledState / Pending / NewCops tests ----

    #[test]
    fn enabled_pending_disabled_by_default() {
        let dir = std::env::temp_dir().join("nitrocop_test_pending_default");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_config(&dir, "Rails/Foo:\n  Enabled: pending\n");
        let config = load_config(Some(&path), None, None).unwrap();
        // Pending is disabled by default (no NewCops: enable)
        assert!(!config.is_cop_enabled("Rails/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn enabled_pending_with_new_cops_enable() {
        let dir = std::env::temp_dir().join("nitrocop_test_pending_enable");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Use a core department to test pending behavior without plugin filtering
        let path = write_config(
            &dir,
            "AllCops:\n  NewCops: enable\nLint/Foo:\n  Enabled: pending\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        // Pending is enabled when NewCops: enable
        assert!(config.is_cop_enabled("Lint/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    // ---- Cross-cop dependency tests ----

    #[test]
    fn redundant_constant_base_disabled_when_constant_resolution_enabled() {
        let dir = std::env::temp_dir().join("nitrocop_test_redundant_constant_base_cross_cop");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // When Lint/ConstantResolution is enabled, Style/RedundantConstantBase
        // must be disabled (conflicting rules, per RuboCop behavior).
        let path = write_config(
            &dir,
            "AllCops:\n  NewCops: enable\nLint/ConstantResolution:\n  Enabled: true\nStyle/RedundantConstantBase:\n  Enabled: pending\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        assert!(!config.is_cop_enabled("Style/RedundantConstantBase", Path::new("a.rb"), &[], &[]));
        // ConstantResolution itself should still be enabled
        assert!(config.is_cop_enabled("Lint/ConstantResolution", Path::new("a.rb"), &[], &[]));

        // Also verify through build_cop_filters (the production path)
        let registry = crate::cop::registry::CopRegistry::default_registry();
        let tier_map = crate::cop::tiers::TierMap::load();
        let filters = config.build_cop_filters(&registry, &tier_map, false);
        let rcb_idx = registry
            .cops()
            .iter()
            .position(|c| c.name() == "Style/RedundantConstantBase")
            .unwrap();
        assert!(
            !filters.cop_filter(rcb_idx).is_enabled(),
            "Style/RedundantConstantBase should be disabled in build_cop_filters when Lint/ConstantResolution is enabled"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn redundant_constant_base_enabled_when_constant_resolution_disabled() {
        let dir = std::env::temp_dir().join("nitrocop_test_redundant_constant_base_no_conflict");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // When Lint/ConstantResolution is NOT enabled (default),
        // Style/RedundantConstantBase should be enabled normally.
        let path = write_config(
            &dir,
            "AllCops:\n  NewCops: enable\nStyle/RedundantConstantBase:\n  Enabled: pending\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        assert!(config.is_cop_enabled("Style/RedundantConstantBase", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    // ---- DisabledByDefault tests ----

    #[test]
    fn disabled_by_default_disables_unset_cops() {
        let dir = std::env::temp_dir().join("nitrocop_test_disabled_by_default");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_config(
            &dir,
            "AllCops:\n  DisabledByDefault: true\nStyle/Foo:\n  Enabled: true\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        // Explicitly enabled cop is still enabled
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));
        // Unmentioned cop is disabled
        assert!(!config.is_cop_enabled("Style/Bar", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disabled_by_default_via_inherit_gem() {
        // Simulates the discourse scenario: inherit_gem loads a config that
        // sets DisabledByDefault: true. Only explicitly enabled cops should run.
        let dir = std::env::temp_dir().join("nitrocop_test_disabled_by_default_inherit_gem");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create a fake gem directory with a config file that sets DisabledByDefault
        let fake_gem_dir = dir.join("fake-rubocop-plugin");
        fs::create_dir_all(&fake_gem_dir).unwrap();
        write_yaml(
            &fake_gem_dir,
            "custom.yml",
            "AllCops:\n  DisabledByDefault: true\n",
        );

        // Pre-populate gem cache so we don't need `bundle`
        let mut gem_cache = HashMap::new();
        gem_cache.insert("fake-rubocop-plugin".to_string(), fake_gem_dir.clone());

        // Project config inherits from the fake gem and enables one cop
        let path = write_config(
            &dir,
            "inherit_gem:\n  fake-rubocop-plugin: custom.yml\nStyle/Foo:\n  Enabled: true\n",
        );
        let config = load_config(Some(&path), None, Some(&gem_cache)).unwrap();

        // Explicitly enabled cop is still enabled
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));
        // Unmentioned cops are disabled (DisabledByDefault from inherited gem)
        assert!(!config.is_cop_enabled("Style/Bar", Path::new("a.rb"), &[], &[]));
        assert!(!config.is_cop_enabled("Lint/SomeOtherCop", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disabled_by_default_with_require_cops_enabled() {
        // Simulates the discourse/my-company-style pattern: a gem chain with
        // DisabledByDefault: true, require: loading a plugin gem (adds to
        // defaults), and the style gem's config explicitly enabling some cops.
        // Per RuboCop's handle_disabled_by_default, cops only from require:
        // defaults are treated as defaults (disabled), while cops explicitly
        // mentioned in user config files (inherit_gem/inherit_from/local) are
        // kept enabled.
        let dir = std::env::temp_dir().join("nitrocop_test_disabled_by_default_require_cops");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Fake plugin gem: config/default.yml enables several cops
        let plugin_dir = dir.join("rubocop-fakeplugin");
        fs::create_dir_all(plugin_dir.join("config")).unwrap();
        write_yaml(
            &plugin_dir.join("config"),
            "default.yml",
            "FakePlugin:\n  Enabled: true\n\
             FakePlugin/CopA:\n  Enabled: true\n\
             FakePlugin/CopB:\n  Enabled: true\n\
             FakePlugin/CopC:\n  Enabled: true\n",
        );

        // Fake style gem: sets DisabledByDefault, requires plugin, and
        // explicitly enables only CopA in the core config
        let style_dir = dir.join("my-company-style");
        fs::create_dir_all(&style_dir).unwrap();
        write_yaml(&style_dir, "default.yml", "inherit_from: core.yml\n");
        write_yaml(
            &style_dir,
            "core.yml",
            "require:\n  - rubocop-fakeplugin\n\
             AllCops:\n  DisabledByDefault: true\n\
             FakePlugin/CopA:\n  Enabled: true\n",
        );

        let mut gem_cache = HashMap::new();
        gem_cache.insert("rubocop-fakeplugin".to_string(), plugin_dir);
        gem_cache.insert("my-company-style".to_string(), style_dir);

        let path = write_config(&dir, "inherit_gem:\n  my-company-style: default.yml\n");
        let config = load_config(Some(&path), None, Some(&gem_cache)).unwrap();

        // CopA: explicitly mentioned in core.yml (user config) → enabled
        assert!(
            config.is_cop_enabled("FakePlugin/CopA", Path::new("a.rb"), &[], &[]),
            "CopA should be enabled (explicitly in user config)"
        );

        // CopB: only in plugin defaults (require:), not in user config → disabled
        assert!(
            !config.is_cop_enabled("FakePlugin/CopB", Path::new("a.rb"), &[], &[]),
            "CopB should be disabled (only from require: defaults)"
        );

        // CopC: only in plugin defaults (require:), not in user config → disabled
        assert!(
            !config.is_cop_enabled("FakePlugin/CopC", Path::new("a.rb"), &[], &[]),
            "CopC should be disabled (only from require: defaults)"
        );

        // CopD: not mentioned anywhere → disabled (DisabledByDefault)
        assert!(
            !config.is_cop_enabled("FakePlugin/CopD", Path::new("a.rb"), &[], &[]),
            "CopD should be disabled (no Enabled key, DisabledByDefault)"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inherit_gem_missing_gem_warns_and_continues() {
        // When inherit_gem references a gem that can't be resolved, load_config
        // should warn and continue (matching RuboCop), not fail hard.
        // The rest of the config (local overrides) should still apply.
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_gem_missing");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_config(
            &dir,
            "inherit_gem:\n  nonexistent-gem-xyz: config.yml\nMetrics/MethodLength:\n  Max: 20\n",
        );
        // Pass empty gem cache — gem won't be found
        let gem_cache = HashMap::new();
        let result = load_config(Some(&path), Some(&dir), Some(&gem_cache));
        assert!(
            result.is_ok(),
            "Missing inherit_gem should warn, not error: {:?}",
            result.unwrap_err()
        );
        // Local config (Max: 20) should still be applied
        let config = result.unwrap();
        let ml_config = config.cop_config("Metrics/MethodLength");
        assert_eq!(
            ml_config.options.get("Max").and_then(|v| v.as_u64()),
            Some(20),
            "Local Max override should survive missing inherit_gem"
        );

        fs::remove_dir_all(&dir).ok();
    }

    // ---- Department-level config tests ----

    #[test]
    fn department_include_filters_cops() {
        let dir = std::env::temp_dir().join("nitrocop_test_dept_include");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Use a core department (Lint) to test department-level Include filtering
        // without being affected by plugin department detection.
        let path = write_config(
            &dir,
            "Lint:\n  Include:\n    - '**/*_spec.rb'\n    - '**/spec/**/*'\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        // Lint cop should match spec files via department include
        assert!(config.is_cop_enabled(
            "Lint/ExampleLength",
            Path::new("spec/models/user_spec.rb"),
            &[],
            &[]
        ));
        // Lint cop should NOT match non-spec files
        assert!(!config.is_cop_enabled(
            "Lint/ExampleLength",
            Path::new("app/models/user.rb"),
            &[],
            &[]
        ));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn department_enabled_false_disables_all_cops() {
        let dir = std::env::temp_dir().join("nitrocop_test_dept_disabled");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Use core department (Lint) for testing Enabled:false since plugin
        // departments are already disabled without require:/plugins: loading
        let path = write_config(&dir, "Lint:\n  Enabled: false\n");
        let config = load_config(Some(&path), None, None).unwrap();
        assert!(!config.is_cop_enabled("Lint/FindBy", Path::new("a.rb"), &[], &[]));
        assert!(!config.is_cop_enabled("Lint/HttpStatus", Path::new("a.rb"), &[], &[]));
        // Other departments unaffected
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn plugin_department_disabled_without_require() {
        // Plugin departments (Rails, RSpec, Performance, etc.) should be disabled
        // when the corresponding gem is not loaded via require:/plugins:
        let config = load_config(Some(Path::new("/nonexistent/.rubocop.yml")), None, None).unwrap();
        assert!(!config.is_cop_enabled("Rails/Output", Path::new("a.rb"), &[], &[]));
        assert!(!config.is_cop_enabled("RSpec/ExampleLength", Path::new("a.rb"), &[], &[]));
        assert!(!config.is_cop_enabled("Performance/Count", Path::new("a.rb"), &[], &[]));
        // Core departments still work
        assert!(config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]));
        assert!(config.is_cop_enabled("Lint/Foo", Path::new("a.rb"), &[], &[]));
    }

    #[test]
    fn register_departments_from_only_enables_requested_plugin_department() {
        let mut config =
            load_config(Some(Path::new("/nonexistent/.rubocop.yml")), None, None).unwrap();
        assert!(!config.is_cop_enabled("Rails/Output", Path::new("a.rb"), &[], &[]));

        config.register_departments_from_only(&["Rails/Output".to_string()]);

        assert!(config.is_cop_enabled("Rails/Output", Path::new("a.rb"), &[], &[]));
    }

    #[test]
    fn cop_config_overrides_department() {
        let dir = std::env::temp_dir().join("nitrocop_test_cop_over_dept");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_config(
            &dir,
            "Rails:\n  Enabled: false\nRails/FindBy:\n  Enabled: true\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        // Department says disabled, but cop says enabled — cop wins
        assert!(config.is_cop_enabled("Rails/FindBy", Path::new("a.rb"), &[], &[]));
        // Other Rails cops still disabled
        assert!(!config.is_cop_enabled("Rails/HttpStatus", Path::new("a.rb"), &[], &[]));

        fs::remove_dir_all(&dir).ok();
    }

    // ---- inherit_mode tests ----

    #[test]
    fn inherit_mode_merge_include() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_mode_merge");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base.yml",
            "Style/Foo:\n  Include:\n    - '**/*.rb'\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\ninherit_mode:\n  merge:\n    - Include\nStyle/Foo:\n  Include:\n    - '**/*.rake'\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Style/Foo");
        // With merge mode, Include is appended instead of replaced
        assert!(cc.include.contains(&"**/*.rb".to_string()));
        assert!(cc.include.contains(&"**/*.rake".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inherit_mode_override_exclude() {
        let dir = std::env::temp_dir().join("nitrocop_test_inherit_mode_override");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_yaml(
            &dir,
            "base.yml",
            "Style/Foo:\n  Exclude:\n    - 'vendor/**'\n",
        );
        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "inherit_from: base.yml\ninherit_mode:\n  override:\n    - Exclude\nStyle/Foo:\n  Exclude:\n    - 'tmp/**'\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let cc = config.cop_config("Style/Foo");
        // With override mode, Exclude is replaced instead of appended
        assert!(!cc.exclude.contains(&"vendor/**".to_string()));
        assert!(cc.exclude.contains(&"tmp/**".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    // ---- enabled_cop_names with pending/disabled_by_default ----

    #[test]
    fn enabled_cop_names_respects_pending_and_disabled_by_default() {
        let dir = std::env::temp_dir().join("nitrocop_test_names_pending");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_yaml(
            &dir,
            ".rubocop.yml",
            "AllCops:\n  NewCops: enable\n  DisabledByDefault: true\nStyle/Foo:\n  Enabled: true\nStyle/Bar:\n  Enabled: pending\nStyle/Baz:\n  Max: 10\n",
        );

        let config = load_config(Some(&path), None, None).unwrap();
        let names = config.enabled_cop_names();
        // Explicitly enabled
        assert!(names.contains(&"Style/Foo".to_string()));
        // Pending + NewCops: enable → enabled
        assert!(names.contains(&"Style/Bar".to_string()));
        // Unset + DisabledByDefault → disabled
        assert!(!names.contains(&"Style/Baz".to_string()));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn glob_set_anchoring_with_literal_separator() {
        // Verify that build_glob_set (literal_separator=true) anchors patterns to the
        // start of the path. app/controllers/**/*.rb must NOT match admin/app/controllers/foo.rb.
        let filter = make_filter(true, &["app/controllers/**/*.rb"], &[]);
        assert!(
            filter.is_included(Path::new("app/controllers/foo_controller.rb")),
            "standard app/controllers path should match"
        );
        assert!(
            !filter.is_included(Path::new("admin/app/controllers/foo_controller.rb")),
            "admin/app/controllers/ should NOT match app/controllers/ pattern"
        );
        assert!(
            !filter.is_included(Path::new("api/app/controllers/foo_controller.rb")),
            "api/app/controllers/ should NOT match app/controllers/ pattern"
        );
    }

    // --- is_cop_match tests ---
    // These test the Include-OR / Exclude-OR logic that handles running
    // from outside the project root where file paths have a prefix.

    fn make_filter(enabled: bool, include: &[&str], exclude: &[&str]) -> CopFilter {
        CopFilter {
            enabled,
            include_set: build_glob_set(include),
            exclude_set: build_glob_set(exclude),
            include_re: build_regex_set(include),
            exclude_re: build_regex_set(exclude),
        }
    }

    #[test]
    fn is_cop_match_exclude_works_on_relativized_path() {
        // Simulates running `nitrocop bench/repos/mastodon` where file paths
        // have a prefix but Exclude patterns are project-relative.
        let filter = make_filter(true, &[], &["lib/tasks/*.rake"]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("bench/repos/mastodon")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        let path = Path::new("bench/repos/mastodon/lib/tasks/emojis.rake");
        assert!(
            !filter_set.is_cop_match(0, path),
            "Exclude lib/tasks/*.rake should match relativized path"
        );
    }

    #[test]
    fn is_cop_match_include_works_with_absolute_patterns() {
        // Integration tests use absolute Include patterns like /tmp/test/db/migrate/**/*.rb
        let filter = make_filter(true, &["/tmp/test/db/migrate/**/*.rb"], &[]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("/tmp/test")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        let path = Path::new("/tmp/test/db/migrate/001_create_users.rb");
        assert!(
            filter_set.is_cop_match(0, path),
            "Absolute Include pattern should match full path"
        );
    }

    #[test]
    fn is_cop_match_include_works_with_relative_patterns() {
        // Relative Include pattern (e.g., spec/**/*_spec.rb) should match
        // both direct and prefixed paths.
        let filter = make_filter(true, &["**/spec/**/*_spec.rb"], &[]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("bench/repos/discourse")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        let path = Path::new("bench/repos/discourse/spec/models/user_spec.rb");
        assert!(
            filter_set.is_cop_match(0, path),
            "Relative Include with ** prefix should match prefixed path"
        );
    }

    #[test]
    fn is_cop_match_exclude_on_relativized_path_overrides_include() {
        // RSpec/EmptyExampleGroup scenario: Include matches via ** prefix,
        // but project-relative Exclude should still block the file.
        let filter = make_filter(true, &["**/spec/**/*_spec.rb"], &["spec/requests/api/*"]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("bench/repos/discourse")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        let path = Path::new("bench/repos/discourse/spec/requests/api/invites_spec.rb");
        assert!(
            !filter_set.is_cop_match(0, path),
            "Exclude spec/requests/api/* should block even when Include matches via **"
        );
    }

    #[test]
    fn is_cop_match_no_config_dir_uses_original_path() {
        // When config_dir is None, only the original path is checked.
        let filter = make_filter(true, &["**/*.rb"], &["vendor/**"]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: None,
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        assert!(filter_set.is_cop_match(0, Path::new("app/models/user.rb")));
        assert!(!filter_set.is_cop_match(0, Path::new("vendor/gems/foo.rb")));
    }

    #[test]
    fn is_cop_match_disabled_filter_returns_false() {
        let filter = make_filter(false, &[], &[]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: None,
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        assert!(!filter_set.is_cop_match(0, Path::new("anything.rb")));
    }

    #[test]
    fn corpus_overlay_global_exclude_does_not_overmatch_sibling_dirs() {
        // Reproduces the corpus overlay scenario where gen_repo_config.py
        // generates absolute Exclude patterns. The overlay config lives at
        // /tmp/nitrocop_corpus_configs/ (config_dir) while base_dir is the
        // repo directory. Tests that /repo/cookbooks/**/* does NOT exclude
        // /repo/cookbooks-override/... (sibling directory).
        let pats: Vec<&str> = vec![
            // Relative patterns from baseline config
            "bin/**/*",
            "tmp/**/*",
            "node_modules/**/*",
            // Absolute patterns from overlay (gen_repo_config.py)
            "/fake/repo/vendor/**/*",
            "/fake/repo/vendor*/**/*",
            "/fake/repo/_vendor/**/*",
            "/fake/repo/cookbooks/**/*",
        ];
        let global_exclude = build_glob_set(&pats).unwrap_or_else(GlobSet::empty);
        let filter_set = CopFilterSet {
            global_exclude,
            global_exclude_patterns: pats.iter().map(|p| p.to_string()).collect(),
            global_exclude_re: None,
            filters: Vec::new(),
            config_dir: Some(PathBuf::from("/tmp/nitrocop_corpus_configs")),
            base_dir: Some(PathBuf::from("/fake/repo")),
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };

        // Positive: files under cookbooks/ should be excluded
        assert!(
            filter_set.is_globally_excluded(Path::new("/fake/repo/cookbooks/foo.rb")),
            "cookbooks/**/* should exclude files under cookbooks/"
        );
        assert!(
            filter_set.is_globally_excluded(Path::new("/fake/repo/cookbooks/ariadne/helpers.rb")),
            "cookbooks/**/* should exclude nested files under cookbooks/"
        );

        // Bug claim: cookbooks-override/ should NOT be excluded
        assert!(
            !filter_set.is_globally_excluded(Path::new(
                "/fake/repo/cookbooks-override/ariadne/libraries/helpers.rb"
            )),
            "cookbooks/**/* must NOT exclude cookbooks-override/ (sibling directory)"
        );

        // Positive: vendor patterns should work
        assert!(
            filter_set.is_globally_excluded(Path::new("/fake/repo/vendor/gems/foo.rb")),
            "vendor/**/* should exclude files under vendor/"
        );
        // vendor*/**/* should match vendored_gems/ etc.
        assert!(
            filter_set.is_globally_excluded(Path::new("/fake/repo/vendored_gems/foo.rb")),
            "vendor*/**/* should exclude vendored_gems/"
        );
    }

    #[test]
    fn corpus_overlay_cop_include_works_with_absolute_excludes() {
        // Reproduces the EnvironmentVariableAccess scenario: cop has Include
        // pattern **/lib/**/*.rb and AllCops.Exclude contains absolute paths
        // from the corpus overlay. The cop should still match files under lib/.
        let pats: Vec<&str> = vec![
            "bin/**/*",
            "tmp/**/*",
            // Absolute parse-error exclusions from overlay
            "/fake/repo/bad_syntax.rb",
            "/fake/repo/vendor/**/*",
        ];
        let global_exclude = build_glob_set(&pats).unwrap_or_else(GlobSet::empty);
        let filter = make_filter(true, &["**/lib/**/*.rb"], &[]);
        let filter_set = CopFilterSet {
            global_exclude,
            global_exclude_patterns: pats.iter().map(|p| p.to_string()).collect(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("/tmp/nitrocop_corpus_configs")),
            base_dir: Some(PathBuf::from("/fake/repo")),
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: vec![0],
            migrated_schema_version: None,
        };

        // Cop should match files under lib/ even with absolute excludes present
        assert!(
            filter_set.is_cop_match(0, Path::new("/fake/repo/lib/helpers.rb")),
            "Include **/lib/**/*.rb should match /fake/repo/lib/helpers.rb"
        );
        assert!(
            filter_set.is_cop_match(0, Path::new("/fake/repo/app/lib/concerns/helper.rb")),
            "Include **/lib/**/*.rb should match nested lib/ path"
        );

        // Files not matching Include should still be excluded
        assert!(
            !filter_set.is_cop_match(0, Path::new("/fake/repo/app/models/user.rb")),
            "Include **/lib/**/*.rb should NOT match app/models/"
        );
    }

    #[test]
    fn is_cop_excluded_checks_exclude_only() {
        // is_cop_excluded only checks Exclude patterns, not Include.
        // A cop with Include that doesn't match should NOT be reported as excluded.
        let filter = make_filter(true, &["db/migrate/**/*.rb"], &[]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("/project")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        // File doesn't match Include, but is_cop_excluded only checks Exclude
        assert!(
            !filter_set.is_cop_excluded(0, Path::new("/project/app/models/user.rb")),
            "Include mismatch should not count as excluded"
        );
    }

    #[test]
    fn is_cop_excluded_detects_exclude_pattern() {
        let filter = make_filter(true, &[], &["**/app/controllers/**/*.rb"]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("/project")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        assert!(
            filter_set.is_cop_excluded(0, Path::new("/project/app/controllers/test.rb")),
            "File matching Exclude should be detected"
        );
        assert!(
            !filter_set.is_cop_excluded(0, Path::new("/project/app/models/test.rb")),
            "File not matching Exclude should not be detected"
        );
    }

    #[test]
    fn is_cop_excluded_with_sub_config_dir() {
        // When a file is in a sub-config directory (e.g., db/migrate/),
        // Exclude patterns relative to the root should still match.
        let filter = make_filter(true, &[], &["**/app/controllers/**/*.rb"]);
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: vec![filter],
            config_dir: Some(PathBuf::from("bench/repos/mastodon")),
            base_dir: None,
            scan_root: None,
            sub_config_dirs: vec![PathBuf::from("bench/repos/mastodon/app/controllers")],
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        // File in sub-config dir: nearest_config_dir is the sub-dir,
        // but root-relative path should still match the Exclude pattern.
        assert!(
            filter_set.is_cop_excluded(
                0,
                Path::new("bench/repos/mastodon/app/controllers/auth/test.rb")
            ),
            "Exclude should match via root-relative path even in sub-config dir"
        );
    }

    #[test]
    fn migrated_file_skippable() {
        use std::path::Path;
        let filter_set = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: Vec::new(),
            config_dir: None,
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: Some("19700101000000".to_string()),
        };
        // SHA hash containing 14-digit run <= 19700101000000 → migrated
        assert!(filter_set.is_migrated_file(Path::new(
            "repos/one_gadget/spec/data/89cc3bb19674621757594b0d0da0c2d3.rb"
        )));
        // Normal migration file with timestamp > epoch → not migrated
        assert!(
            !filter_set.is_migrated_file(Path::new("db/migrate/20240315120000_create_users.rb"))
        );
        // No digits in filename → not migrated
        assert!(!filter_set.is_migrated_file(Path::new("app/models/user.rb")));
        // Fewer than 14 digits → not migrated
        assert!(!filter_set.is_migrated_file(Path::new("test_1234567890.rb")));
        // Exactly 14 digits at epoch → migrated
        assert!(filter_set.is_migrated_file(Path::new("19700101000000_init.rb")));
        // No MigratedSchemaVersion set → never migrated
        let no_version = CopFilterSet {
            global_exclude: GlobSet::empty(),
            global_exclude_patterns: Vec::new(),
            global_exclude_re: None,
            filters: Vec::new(),
            config_dir: None,
            base_dir: None,
            scan_root: None,
            sub_config_dirs: Vec::new(),
            universal_cop_indices: Vec::new(),
            pattern_cop_indices: Vec::new(),
            migrated_schema_version: None,
        };
        assert!(!no_version.is_migrated_file(Path::new("19700101000000_init.rb")));
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        fn enabled_state_strategy() -> impl Strategy<Value = EnabledState> {
            prop::sample::select(vec![
                EnabledState::True,
                EnabledState::False,
                EnabledState::Pending,
                EnabledState::Unset,
            ])
        }

        fn string_list_strategy() -> impl Strategy<Value = Vec<String>> {
            prop::collection::vec("[a-z]{1,8}", 0..5)
        }

        fn cop_config_strategy() -> impl Strategy<Value = CopConfig> {
            (
                enabled_state_strategy(),
                prop::option::of(prop::sample::select(vec![
                    Severity::Convention,
                    Severity::Warning,
                    Severity::Error,
                ])),
                string_list_strategy(),
                string_list_strategy(),
            )
                .prop_map(|(enabled, severity, exclude, include)| CopConfig {
                    enabled,
                    severity,
                    exclude,
                    include,
                    options: HashMap::new(),
                })
        }

        fn inherit_mode_strategy() -> impl Strategy<Value = Option<InheritMode>> {
            prop_oneof![
                Just(None),
                Just(Some(InheritMode::default())),
                Just(Some(InheritMode {
                    merge: HashSet::from(["Include".to_string()]),
                    override_keys: HashSet::new(),
                })),
                Just(Some(InheritMode {
                    merge: HashSet::new(),
                    override_keys: HashSet::from(["Exclude".to_string()]),
                })),
                Just(Some(InheritMode {
                    merge: HashSet::from(["Include".to_string()]),
                    override_keys: HashSet::from(["Exclude".to_string()]),
                })),
            ]
        }

        proptest! {
            #[test]
            fn merge_enabled_last_writer_wins(
                base in cop_config_strategy(),
                overlay in cop_config_strategy(),
                inherit_mode in inherit_mode_strategy(),
            ) {
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, inherit_mode.as_ref());
                if overlay.enabled != EnabledState::Unset {
                    prop_assert_eq!(merged.enabled, overlay.enabled,
                        "overlay enabled {:?} should override base {:?}",
                        overlay.enabled, base.enabled);
                } else {
                    prop_assert_eq!(merged.enabled, base.enabled,
                        "base enabled should persist when overlay is Unset");
                }
            }

            #[test]
            fn merge_severity_last_writer_wins(
                base in cop_config_strategy(),
                overlay in cop_config_strategy(),
                inherit_mode in inherit_mode_strategy(),
            ) {
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, inherit_mode.as_ref());
                if overlay.severity.is_some() {
                    prop_assert_eq!(merged.severity, overlay.severity);
                } else {
                    prop_assert_eq!(merged.severity, base.severity);
                }
            }

            #[test]
            fn merge_exclude_default_appends(
                base in cop_config_strategy(),
                overlay in cop_config_strategy(),
            ) {
                // No inherit_mode and no override -> Exclude appends (union)
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, None);
                // All base excludes should still be present
                for exc in &base.exclude {
                    prop_assert!(merged.exclude.contains(exc),
                        "base exclude '{}' lost after merge", exc);
                }
                // All overlay excludes should be present
                for exc in &overlay.exclude {
                    prop_assert!(merged.exclude.contains(exc),
                        "overlay exclude '{}' not added", exc);
                }
            }

            #[test]
            fn merge_exclude_override_replaces(
                base in cop_config_strategy(),
                overlay in cop_config_strategy(),
            ) {
                let im = InheritMode {
                    merge: HashSet::new(),
                    override_keys: HashSet::from(["Exclude".to_string()]),
                };
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, Some(&im));
                if !overlay.exclude.is_empty() {
                    prop_assert_eq!(merged.exclude, overlay.exclude,
                        "override should replace exclude entirely");
                } else {
                    // Empty overlay doesn't replace
                    prop_assert_eq!(merged.exclude, base.exclude);
                }
            }

            #[test]
            fn merge_include_default_replaces(
                base in cop_config_strategy(),
                overlay in cop_config_strategy(),
            ) {
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, None);
                if !overlay.include.is_empty() {
                    prop_assert_eq!(merged.include, overlay.include,
                        "default should replace include");
                } else {
                    prop_assert_eq!(merged.include, base.include,
                        "empty overlay include should leave base unchanged");
                }
            }

            #[test]
            fn merge_include_with_merge_mode_appends(
                base in cop_config_strategy(),
                overlay in cop_config_strategy(),
            ) {
                let im = InheritMode {
                    merge: HashSet::from(["Include".to_string()]),
                    override_keys: HashSet::new(),
                };
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, Some(&im));
                if !overlay.include.is_empty() {
                    // All base includes should be preserved
                    for inc in &base.include {
                        prop_assert!(merged.include.contains(inc),
                            "base include '{}' lost in merge mode", inc);
                    }
                    // All overlay includes should be present
                    for inc in &overlay.include {
                        prop_assert!(merged.include.contains(inc),
                            "overlay include '{}' not appended", inc);
                    }
                }
            }

            #[test]
            fn merge_does_not_introduce_new_duplicate_excludes(
                base in cop_config_strategy(),
                overlay in cop_config_strategy(),
            ) {
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, None);
                // Each overlay exclude should appear at most once more than in base
                for exc in &overlay.exclude {
                    let count_in_merged = merged.exclude.iter().filter(|e| *e == exc).count();
                    let count_in_base = base.exclude.iter().filter(|e| *e == exc).count();
                    prop_assert!(count_in_merged <= count_in_base.max(1),
                        "overlay exclude '{}' duplicated: {} in merged vs {} in base",
                        exc, count_in_merged, count_in_base);
                }
            }

            #[test]
            fn merge_preserves_base_when_overlay_empty(base in cop_config_strategy()) {
                let overlay = CopConfig::default();
                let mut merged = base.clone();
                merge_cop_config(&mut merged, &overlay, None);
                prop_assert_eq!(merged.enabled, base.enabled);
                prop_assert_eq!(merged.severity, base.severity);
                prop_assert_eq!(merged.exclude, base.exclude);
                prop_assert_eq!(merged.include, base.include);
            }
        }
    }

    // --- .standard.yml autodiscovery tests ---

    #[test]
    fn auto_discover_standard_yml() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_discover");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let standard_path = dir.join(".standard.yml");
        fs::write(&standard_path, "# empty standard config\n").unwrap();

        let found = find_config(&dir);
        assert_eq!(found, Some(standard_path));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rubocop_yml_preferred_over_standard_yml() {
        let dir = std::env::temp_dir().join("nitrocop_test_prefer_rubocop");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".rubocop.yml"), "# rubocop\n").unwrap();
        fs::write(dir.join(".standard.yml"), "# standard\n").unwrap();

        let found = find_config(&dir);
        assert_eq!(found, Some(dir.join(".rubocop.yml")));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_plugins() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_plugins");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(
            &path,
            "plugins:\n  - standard-performance\n  - standard-rails\n",
        )
        .unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(
            yaml.contains("- standard\n"),
            "should always include 'standard'"
        );
        assert!(yaml.contains("- standard-performance"));
        assert!(yaml.contains("- standard-rails"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_ruby_version() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_ruby_ver");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(&path, "ruby_version: 3.2\n").unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(yaml.contains("TargetRubyVersion: 3.2"), "got: {yaml}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_ignore_simple_patterns() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_ignore");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(&path, "ignore:\n  - 'db/migrate/201*'\n  - 'vendor/**'\n").unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(yaml.contains("AllCops:"), "got: {yaml}");
        assert!(yaml.contains("Exclude:"), "got: {yaml}");
        assert!(yaml.contains("db/migrate/201*"), "got: {yaml}");
        assert!(yaml.contains("vendor/**"), "got: {yaml}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_ignore_cop_disable() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_cop_disable");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(
            &path,
            "ignore:\n  - '**/*':\n    - Lint/UselessAssignment\n",
        )
        .unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(
            yaml.contains("Lint/UselessAssignment:\n  Enabled: false"),
            "got: {yaml}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_ignore_cop_exclude() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_cop_exclude");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(
            &path,
            "ignore:\n  - 'test/**':\n    - Style/StringLiterals\n",
        )
        .unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(
            yaml.contains("Style/StringLiterals:\n  Exclude:\n    - 'test/**'"),
            "got: {yaml}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_extend_config() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_extend");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(&path, "extend_config:\n  - .custom_cops.yml\n").unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(yaml.contains("inherit_from:"), "got: {yaml}");
        assert!(yaml.contains("- .custom_cops.yml"), "got: {yaml}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_default_ignores() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_default_ignores");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        // No default_ignores key → defaults to true
        fs::write(&path, "# empty config\n").unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(
            yaml.contains("bin/*"),
            "default ignores should include bin/*; got: {yaml}"
        );
        assert!(
            yaml.contains("db/schema.rb"),
            "default ignores should include db/schema.rb; got: {yaml}"
        );
        assert!(
            yaml.contains("inherit_mode:"),
            "should emit inherit_mode to merge Exclude; got: {yaml}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn convert_standard_yml_default_ignores_disabled() {
        let dir = std::env::temp_dir().join("nitrocop_test_standard_no_default_ignores");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(&path, "default_ignores: false\n").unwrap();

        let yaml = convert_standard_yml(&path).unwrap();
        assert!(
            !yaml.contains("bin/*"),
            "default ignores should be suppressed when default_ignores: false; got: {yaml}"
        );
        assert!(
            !yaml.contains("db/schema.rb"),
            "default ignores should be suppressed when default_ignores: false; got: {yaml}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn standard_yml_loads_via_load_config() {
        // Test the full pipeline: .standard.yml → convert → load_config
        let dir = std::env::temp_dir().join("nitrocop_test_standard_load");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".standard.yml");
        fs::write(
            &path,
            "ignore:\n  - '**/*':\n    - Lint/UselessAssignment\n",
        )
        .unwrap();

        let config = load_config(Some(&path), None, None).unwrap();
        assert!(
            !config.is_cop_enabled("Lint/UselessAssignment", Path::new("a.rb"), &[], &[]),
            "Lint/UselessAssignment should be disabled via .standard.yml ignore"
        );
        fs::remove_dir_all(&dir).ok();
    }

    // ---- DisabledByDefault + department-level Enabled tests ----
    // These tests cover the discourse-style scenario (DisabledByDefault: true
    // with department-level `Enabled: true`) and the rails-style scenario
    // (plugin gem sets `Enabled: true` on its department via defaults, which
    // should NOT count as user-enabled).

    #[test]
    fn disabled_by_default_with_dept_enabled_restores_cops() {
        // Discourse scenario: DisabledByDefault: true + Security: Enabled: true
        // Cops in the Security department that are default-enabled should be
        // restored to enabled, matching RuboCop's handle_disabled_by_default.
        let dir = std::env::temp_dir().join("nitrocop_test_dbd_dept_enabled");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_config(
            &dir,
            "AllCops:\n  DisabledByDefault: true\n\
             Security:\n  Enabled: true\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();

        // Security/Eval is a default-enabled cop in a user-enabled department
        // → should be enabled
        assert!(
            config.is_cop_enabled("Security/Eval", Path::new("a.rb"), &[], &[]),
            "Security/Eval should be enabled (dept explicitly enabled by user + default-enabled cop)"
        );
        // Security/IoMethods is also default-enabled in Security
        assert!(
            config.is_cop_enabled("Security/IoMethods", Path::new("a.rb"), &[], &[]),
            "Security/IoMethods should be enabled (dept explicitly enabled by user)"
        );
        // Style cops should still be disabled (DisabledByDefault, no dept enable)
        assert!(
            !config.is_cop_enabled("Style/StringLiterals", Path::new("a.rb"), &[], &[]),
            "Style/StringLiterals should be disabled (DisabledByDefault, dept not enabled)"
        );
        // Lint cops should still be disabled
        assert!(
            !config.is_cop_enabled("Lint/Void", Path::new("a.rb"), &[], &[]),
            "Lint/Void should be disabled (DisabledByDefault, dept not enabled)"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disabled_by_default_dept_mentioned_without_enabled_stays_disabled() {
        // Rails scenario: DisabledByDefault: true + department mentioned only
        // via Exclude (e.g., `Performance: Exclude: [...]`). Since the user
        // didn't write `Enabled: true` on the department, cops should stay
        // disabled. This prevents false positives.
        let dir = std::env::temp_dir().join("nitrocop_test_dbd_dept_no_enable");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_config(
            &dir,
            "AllCops:\n  DisabledByDefault: true\n\
             Lint:\n  Exclude:\n    - 'test/**'\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();

        // Lint department is mentioned but NOT with Enabled: true
        // → cops should stay disabled under DisabledByDefault
        assert!(
            !config.is_cop_enabled("Lint/Void", Path::new("a.rb"), &[], &[]),
            "Lint/Void should be disabled (dept mentioned via Exclude only, not Enabled: true)"
        );
        // Explicitly enabling a cop should still work
        let path2 = write_config(
            &dir,
            "AllCops:\n  DisabledByDefault: true\n\
             Lint:\n  Exclude:\n    - 'test/**'\n\
             Lint/Void:\n  Enabled: true\n",
        );
        let config2 = load_config(Some(&path2), None, None).unwrap();
        assert!(
            config2.is_cop_enabled("Lint/Void", Path::new("a.rb"), &[], &[]),
            "Lint/Void should be enabled when explicitly set"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disabled_by_default_require_dept_enabled_not_user_enabled() {
        // Rails + rubocop-performance scenario: require: rubocop-performance
        // loads gem defaults that set `Performance: Enabled: true`. But the
        // USER didn't write that — it came from the gem. Under DisabledByDefault,
        // Performance cops should stay disabled.
        let dir = std::env::temp_dir().join("nitrocop_test_dbd_require_dept");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Fake plugin gem: config/default.yml enables department + cops
        let plugin_dir = dir.join("rubocop-fakeperf");
        fs::create_dir_all(plugin_dir.join("config")).unwrap();
        write_yaml(
            &plugin_dir.join("config"),
            "default.yml",
            "FakePerf:\n  Enabled: true\n\
             FakePerf/CopA:\n  Enabled: true\n\
             FakePerf/CopB:\n  Enabled: true\n",
        );

        // Style gem: DisabledByDefault + require: the plugin
        let style_dir = dir.join("custom-style");
        fs::create_dir_all(&style_dir).unwrap();
        write_yaml(
            &style_dir,
            "core.yml",
            "require:\n  - rubocop-fakeperf\n\
             AllCops:\n  DisabledByDefault: true\n",
        );

        let mut gem_cache = HashMap::new();
        gem_cache.insert("rubocop-fakeperf".to_string(), plugin_dir);
        gem_cache.insert("custom-style".to_string(), style_dir);

        // Project config inherits from style gem (gets DisabledByDefault + require)
        let path = write_config(&dir, "inherit_gem:\n  custom-style: core.yml\n");
        let config = load_config(Some(&path), None, Some(&gem_cache)).unwrap();

        // FakePerf cops should be DISABLED: Enabled: true came from require:
        // gem defaults, not from user config
        assert!(
            !config.is_cop_enabled("FakePerf/CopA", Path::new("a.rb"), &[], &[]),
            "FakePerf/CopA should be disabled (dept Enabled: true from require: defaults, not user)"
        );
        assert!(
            !config.is_cop_enabled("FakePerf/CopB", Path::new("a.rb"), &[], &[]),
            "FakePerf/CopB should be disabled (dept Enabled: true from require: defaults, not user)"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disabled_by_default_user_dept_enabled_overrides_require() {
        // Combined scenario: require: loads plugin gem defaults with
        // FakePerf: Enabled: true, AND the user config also writes
        // FakePerf: Enabled: true. The user-explicit enable should win,
        // restoring default-enabled cops in that department.
        let dir = std::env::temp_dir().join("nitrocop_test_dbd_user_overrides_require");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Fake plugin gem with department + cops
        let plugin_dir = dir.join("rubocop-fakeperf2");
        fs::create_dir_all(plugin_dir.join("config")).unwrap();
        write_yaml(
            &plugin_dir.join("config"),
            "default.yml",
            "FakePerf2:\n  Enabled: true\n\
             FakePerf2/CopA:\n  Enabled: true\n\
             FakePerf2/CopB:\n  Enabled: true\n",
        );

        // Style gem: DisabledByDefault + require: the plugin
        let style_dir = dir.join("custom-style2");
        fs::create_dir_all(&style_dir).unwrap();
        write_yaml(
            &style_dir,
            "core.yml",
            "require:\n  - rubocop-fakeperf2\n\
             AllCops:\n  DisabledByDefault: true\n",
        );

        let mut gem_cache = HashMap::new();
        gem_cache.insert("rubocop-fakeperf2".to_string(), plugin_dir);
        gem_cache.insert("custom-style2".to_string(), style_dir);

        // Project config inherits from style gem AND explicitly enables
        // the FakePerf2 department
        let path = write_config(
            &dir,
            "inherit_gem:\n  custom-style2: core.yml\n\
             FakePerf2:\n  Enabled: true\n",
        );
        let config = load_config(Some(&path), None, Some(&gem_cache)).unwrap();

        // FakePerf2 cops should be ENABLED: user explicitly wrote
        // FakePerf2: Enabled: true in project config
        assert!(
            config.is_cop_enabled("FakePerf2/CopA", Path::new("a.rb"), &[], &[]),
            "FakePerf2/CopA should be enabled (user explicitly enabled dept)"
        );
        assert!(
            config.is_cop_enabled("FakePerf2/CopB", Path::new("a.rb"), &[], &[]),
            "FakePerf2/CopB should be enabled (user explicitly enabled dept)"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn include_without_base_dir_match_does_not_fire() {
        // When the config is external (base_dir/config_dir don't cover the
        // target files), non-**-prefixed Include patterns like `spec/**/*.rb`
        // or bare `Rakefile` should NOT match. This matches RuboCop's behavior:
        // it resolves Include patterns against base_dir (CWD for non-.rubocop
        // configs), with no scan-root fallback.
        //
        // In normal usage (CWD = project root), the match happens via rel_path
        // or rel_to_base. The external-config scenario only matters for corpus
        // oracle comparisons.
        let dir = std::env::temp_dir().join("nitrocop_test_no_scan_root_include");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Config with Include patterns that don't start with **
        let path = write_config(
            &dir,
            "Style/FrozenStringLiteralComment:\n  Include:\n    - 'spec/**/*.rb'\n    - 'Rakefile'\n    - 'db/**/*.rb'\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        let registry = crate::cop::registry::CopRegistry::default_registry();
        let tier_map = crate::cop::tiers::TierMap::load();

        let filters = config.build_cop_filters(&registry, &tier_map, true);
        let idx = registry
            .cops()
            .iter()
            .position(|c| c.name() == "Style/FrozenStringLiteralComment")
            .unwrap();

        // Files under an external path should NOT match non-**-prefixed patterns
        assert!(
            !filters.is_cop_match(idx, Path::new("/fake/repo/spec/foo.rb")),
            "spec/foo.rb should not match when base_dir != repo root"
        );
        assert!(
            !filters.is_cop_match(idx, Path::new("/fake/repo/Rakefile")),
            "Rakefile should not match when base_dir != repo root"
        );
        assert!(
            !filters.is_cop_match(idx, Path::new("/fake/repo/db/seeds.rb")),
            "db/seeds.rb should not match when base_dir != repo root"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn include_star_star_prefix_matches_any_path() {
        // Include patterns starting with ** (e.g., `**/app/**/*.rb`) should
        // match files regardless of path prefix — they work through glob
        // matching on the absolute path.
        let dir = std::env::temp_dir().join("nitrocop_test_star_star_include");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = write_config(
            &dir,
            "Style/FrozenStringLiteralComment:\n  Include:\n    - '**/spec/**/*.rb'\n    - '**/*.rake'\n",
        );
        let config = load_config(Some(&path), None, None).unwrap();
        let registry = crate::cop::registry::CopRegistry::default_registry();
        let tier_map = crate::cop::tiers::TierMap::load();

        let filters = config.build_cop_filters(&registry, &tier_map, true);
        let idx = registry
            .cops()
            .iter()
            .position(|c| c.name() == "Style/FrozenStringLiteralComment")
            .unwrap();

        // **-prefixed patterns match through absolute path glob matching
        assert!(
            filters.is_cop_match(idx, Path::new("/fake/repo/spec/foo.rb")),
            "**/spec/**/*.rb should match via absolute path glob"
        );
        assert!(
            filters.is_cop_match(idx, Path::new("/fake/repo/lib/tasks/foo.rake")),
            "**/*.rake should match via absolute path glob"
        );
        // Non-matching patterns should still reject
        assert!(
            !filters.is_cop_match(idx, Path::new("/fake/repo/app/foo.rb")),
            "app/foo.rb should not match **/spec/**/*.rb or **/*.rake"
        );

        fs::remove_dir_all(&dir).ok();
    }
}
