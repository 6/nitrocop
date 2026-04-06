use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rayon::prelude::*;
use ruby_prism::Visit;

use crate::cache::ResultCache;
use crate::cli::Args;
use crate::config::{CopFilterSet, ResolvedConfig};
use crate::cop::lint::redundant_cop_disable_directive::allow_redundant_disable_flagging_for_known_gap_cop;
use crate::cop::registry::CopRegistry;
use crate::cop::tiers::{SkipSummary, TierMap};
use crate::cop::walker::BatchedCopWalker;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::fs::DiscoveredFiles;
use crate::parse::codemap::CodeMap;
use crate::parse::directives::DisableDirective;
use crate::parse::source::SourceFile;

/// Thread-safe phase timing counters (nanoseconds) for profiling.
pub(crate) struct PhaseTimers {
    file_io_ns: AtomicU64,
    parse_ns: AtomicU64,
    codemap_ns: AtomicU64,
    cop_exec_ns: AtomicU64,
    cop_filter_ns: AtomicU64,
    cop_ast_ns: AtomicU64,
    disable_ns: AtomicU64,
}

impl PhaseTimers {
    fn new() -> Self {
        Self {
            file_io_ns: AtomicU64::new(0),
            parse_ns: AtomicU64::new(0),
            codemap_ns: AtomicU64::new(0),
            cop_exec_ns: AtomicU64::new(0),
            cop_filter_ns: AtomicU64::new(0),
            cop_ast_ns: AtomicU64::new(0),
            disable_ns: AtomicU64::new(0),
        }
    }

    fn print_summary(&self, total: std::time::Duration, file_count: usize) {
        let file_io = std::time::Duration::from_nanos(self.file_io_ns.load(Ordering::Relaxed));
        let parse = std::time::Duration::from_nanos(self.parse_ns.load(Ordering::Relaxed));
        let codemap = std::time::Duration::from_nanos(self.codemap_ns.load(Ordering::Relaxed));
        let cop_exec = std::time::Duration::from_nanos(self.cop_exec_ns.load(Ordering::Relaxed));
        let disable = std::time::Duration::from_nanos(self.disable_ns.load(Ordering::Relaxed));
        let accounted = file_io + parse + codemap + cop_exec + disable;

        eprintln!("debug: --- linter phase breakdown ({file_count} files) ---");
        eprintln!("debug:   file I/O:       {file_io:.0?} (cumulative across threads)");
        eprintln!("debug:   prism parse:    {parse:.0?}");
        eprintln!("debug:   codemap build:  {codemap:.0?}");
        let cop_filter =
            std::time::Duration::from_nanos(self.cop_filter_ns.load(Ordering::Relaxed));
        let cop_ast = std::time::Duration::from_nanos(self.cop_ast_ns.load(Ordering::Relaxed));
        eprintln!("debug:   cop execution:  {cop_exec:.0?}");
        eprintln!("debug:     filter+config:  {cop_filter:.0?}");
        eprintln!("debug:     AST walk:       {cop_ast:.0?}");
        eprintln!("debug:   disable filter: {disable:.0?}");
        eprintln!("debug:   accounted:      {accounted:.0?} (sum of per-thread time)");
        eprintln!("debug:   wall clock:     {total:.0?}");
    }
}

/// Renamed cops snapshot from src/resources/renamed_cops.yml.
/// Maps old cop name -> new cop name (e.g., "Naming/PredicateName" -> "Naming/PredicatePrefix").
pub(crate) static RENAMED_COPS: LazyLock<HashMap<String, String>> =
    LazyLock::new(|| parse_renamed_cops(include_str!("resources/renamed_cops.yml")));

/// Parse the `renamed:` section from obsoletion.yml content.
///
/// The YAML format supports two styles:
///   - Simple:   `OldName: NewName`
///   - Extended:  `OldName:\n  new_name: NewName\n  severity: warning`
fn parse_renamed_cops(yaml_content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    let raw: serde_yml::Value = match serde_yml::from_str(yaml_content) {
        Ok(v) => v,
        Err(_) => return map,
    };

    let Some(renamed) = raw.get("renamed") else {
        return map;
    };
    let Some(mapping) = renamed.as_mapping() else {
        return map;
    };

    for (key, value) in mapping {
        let Some(old_name) = key.as_str() else {
            continue;
        };

        let new_name = if let Some(s) = value.as_str() {
            // Simple format: OldName: NewName
            s.to_string()
        } else if let Some(inner_map) = value.as_mapping() {
            // Extended format: OldName: { new_name: NewName, severity: ... }
            inner_map
                .get(serde_yml::Value::String("new_name".to_string()))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default()
        } else {
            continue;
        };

        if !new_name.is_empty() {
            map.insert(old_name.to_string(), new_name);
        }
    }

    map
}

pub struct LintResult {
    pub diagnostics: Vec<Diagnostic>,
    pub file_count: usize,
    pub corrected_count: usize,
    pub skip_summary: SkipSummary,
}

/// Lint a single SourceFile (already loaded into memory). Used for --stdin mode.
pub fn lint_source(
    source: &SourceFile,
    config: &ResolvedConfig,
    registry: &CopRegistry,
    args: &Args,
    tier_map: &TierMap,
    allowlist: &crate::cop::autocorrect_allowlist::AutocorrectAllowlist,
) -> LintResult {
    let cop_filters = config.build_cop_filters(registry, tier_map, args.preview);
    let base_configs = config.precompute_cop_configs(registry);
    let has_dir_overrides = config.has_dir_overrides();
    let (diagnostics, _corrected_bytes, corrected_count) = lint_source_inner(
        source,
        config,
        registry,
        args,
        tier_map,
        &cop_filters,
        &base_configs,
        has_dir_overrides,
        None,
        allowlist,
    );
    let mut sorted = diagnostics;
    sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    let skip_summary = config.compute_skip_summary(registry, tier_map, args.preview);
    LintResult {
        diagnostics: sorted,
        file_count: 1,
        corrected_count,
        skip_summary,
    }
}

pub fn run_linter(
    discovered: &DiscoveredFiles,
    config: &ResolvedConfig,
    registry: &CopRegistry,
    args: &Args,
    tier_map: &TierMap,
    allowlist: &crate::cop::autocorrect_allowlist::AutocorrectAllowlist,
) -> LintResult {
    let files = &discovered.files;
    let wall_start = std::time::Instant::now();

    // Initialize schema (db/schema.rb) for schema-aware cops
    crate::schema::init(config.config_dir());

    // Build cop filters once before the parallel loop
    let cop_filters = config.build_cop_filters(registry, tier_map, args.preview);
    // Pre-compute base cop configs once (avoids HashMap clone per cop per file)
    let base_configs = config.precompute_cop_configs(registry);
    let has_dir_overrides = config.has_dir_overrides();

    // Result cache: enabled by default, disable with --no-cache, --cache false,
    // or autocorrect.
    let cache_enabled = args.cache == "true"
        && !args.no_cache
        && args.stdin.is_none()
        && args.autocorrect_mode() == crate::cli::AutocorrectMode::Off;
    let cache_enabled = cache_enabled && !has_dir_overrides;
    let cache = if cache_enabled {
        let c = ResultCache::new(env!("CARGO_PKG_VERSION"), &base_configs, args);
        if args.debug {
            eprintln!("debug: result cache enabled");
        }
        c
    } else {
        if args.debug && args.no_cache {
            eprintln!("debug: result cache disabled (--no-cache)");
        } else if args.debug && args.cache != "true" {
            eprintln!("debug: result cache disabled (--cache false)");
        } else if args.debug && has_dir_overrides {
            eprintln!("debug: result cache disabled (directory-specific configs)");
        }
        ResultCache::disabled()
    };

    let timers = if args.debug {
        Some(PhaseTimers::new())
    } else {
        None
    };

    let cache_stat_hits = std::sync::atomic::AtomicUsize::new(0);
    let cache_content_hits = std::sync::atomic::AtomicUsize::new(0);
    let cache_misses = std::sync::atomic::AtomicUsize::new(0);
    let found_offense = AtomicBool::new(false);
    let total_corrected = std::sync::atomic::AtomicUsize::new(0);

    let diagnostics: Vec<Diagnostic> = files
        .par_iter()
        .flat_map(|path| {
            // --fail-fast: skip remaining files once an offense is found
            if args.fail_fast && found_offense.load(Ordering::Relaxed) {
                return Vec::new();
            }
            let result = lint_file(
                path,
                config,
                registry,
                args,
                tier_map,
                &cop_filters,
                &base_configs,
                has_dir_overrides,
                timers.as_ref(),
                &cache,
                &cache_stat_hits,
                &cache_content_hits,
                &cache_misses,
                &discovered.explicit,
                &total_corrected,
                allowlist,
            );
            if args.fail_fast && !result.is_empty() {
                found_offense.store(true, Ordering::Relaxed);
            }
            result
        })
        .collect();

    let mut sorted = diagnostics;
    sorted.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    if let Some(ref t) = timers {
        t.print_summary(wall_start.elapsed(), files.len());
    }

    if args.debug && cache.is_enabled() {
        let stat_hits = cache_stat_hits.load(std::sync::atomic::Ordering::Relaxed);
        let content_hits = cache_content_hits.load(std::sync::atomic::Ordering::Relaxed);
        let misses = cache_misses.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "debug: cache stat_hits: {stat_hits}, content_hits: {content_hits}, misses: {misses}"
        );
    }

    // Flush in-memory cache to disk, then run eviction (best-effort)
    if cache.is_enabled() {
        cache.flush();
        cache.evict(50);
    }

    // Per-cop timing: enabled by NITROCOP_COP_PROFILE=1
    if std::env::var("NITROCOP_COP_PROFILE").is_ok() {
        use std::sync::Mutex;
        let cop_timings: Vec<Mutex<(u64, u64, u64)>> = (0..registry.cops().len())
            .map(|_| Mutex::new((0u64, 0u64, 0u64)))
            .collect();
        // Re-run single-threaded for profiling
        for path in files {
            if cop_filters.is_globally_excluded(path) {
                continue;
            }
            let source = match SourceFile::from_path(path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let parse_result = crate::parse::parse_source(source.as_bytes());
            let code_map = CodeMap::from_parse_result(source.as_bytes(), &parse_result);
            for (i, cop) in registry.cops().iter().enumerate() {
                if !cop_filters.is_cop_match(i, &source.path) {
                    continue;
                }
                let cop_config = &base_configs[i];
                let t0 = std::time::Instant::now();
                let mut d = Vec::new();
                cop.check_lines(&source, cop_config, &mut d, None);
                let lines_ns = t0.elapsed().as_nanos() as u64;
                let t1 = std::time::Instant::now();
                cop.check_source(&source, &parse_result, &code_map, cop_config, &mut d, None);
                let source_ns = t1.elapsed().as_nanos() as u64;
                let t2 = std::time::Instant::now();
                // check_node via single-cop walker
                if !cop.interested_node_types().is_empty() || cop.name().contains('/') {
                    let ast_cops: Vec<(&dyn Cop, &CopConfig)> = vec![(&**cop, cop_config)];
                    let mut walker = BatchedCopWalker::new(ast_cops, &source, &parse_result);
                    walker.visit(&parse_result.node());
                }
                let ast_ns = t2.elapsed().as_nanos() as u64;
                let mut m = cop_timings[i].lock().unwrap();
                m.0 += lines_ns;
                m.1 += source_ns;
                m.2 += ast_ns;
            }
        }
        let mut entries: Vec<(String, u64, u64, u64)> = registry
            .cops()
            .iter()
            .enumerate()
            .map(|(i, cop)| {
                let m = cop_timings[i].lock().unwrap();
                (cop.name().to_string(), m.0, m.1, m.2)
            })
            .filter(|(_, l, s, a)| *l + *s + *a > 0)
            .collect();
        entries.sort_by(|a, b| (b.1 + b.2 + b.3).cmp(&(a.1 + a.2 + a.3)));
        eprintln!("\n=== Per-cop timing (top 30) ===");
        eprintln!(
            "{:<50} {:>10} {:>10} {:>10} {:>10}",
            "Cop", "lines", "source", "ast", "total"
        );
        for (name, l, s, a) in entries.iter().take(30) {
            let lms = *l as f64 / 1_000_000.0;
            let sms = *s as f64 / 1_000_000.0;
            let ams = *a as f64 / 1_000_000.0;
            let total = lms + sms + ams;
            eprintln!(
                "{:<50} {:>9.1}ms {:>9.1}ms {:>9.1}ms {:>9.1}ms",
                name, lms, sms, ams, total
            );
        }
        let total_all: u64 = entries.iter().map(|(_, l, s, a)| l + s + a).sum();
        eprintln!(
            "{:<50} {:>10} {:>10} {:>10} {:>9.1}ms",
            "TOTAL",
            "",
            "",
            "",
            total_all as f64 / 1_000_000.0
        );
    }

    let corrected_count = total_corrected.load(std::sync::atomic::Ordering::Relaxed);
    let skip_summary = config.compute_skip_summary(registry, tier_map, args.preview);
    LintResult {
        diagnostics: sorted,
        file_count: files.len(),
        corrected_count,
        skip_summary,
    }
}

#[allow(clippy::too_many_arguments)] // orchestration entry point threading shared state
fn lint_file(
    path: &Path,
    config: &ResolvedConfig,
    registry: &CopRegistry,
    args: &Args,
    tier_map: &TierMap,
    cop_filters: &CopFilterSet,
    base_configs: &[CopConfig],
    has_dir_overrides: bool,
    timers: Option<&PhaseTimers>,
    cache: &ResultCache,
    cache_stat_hits: &std::sync::atomic::AtomicUsize,
    cache_content_hits: &std::sync::atomic::AtomicUsize,
    cache_misses: &std::sync::atomic::AtomicUsize,
    explicit_files: &HashSet<std::path::PathBuf>,
    total_corrected: &std::sync::atomic::AtomicUsize,
    allowlist: &crate::cop::autocorrect_allowlist::AutocorrectAllowlist,
) -> Vec<Diagnostic> {
    use crate::cache::CacheLookup;

    // Check global excludes once per file.
    // Explicitly-passed files bypass AllCops.Exclude (matching RuboCop default)
    // unless --force-exclusion is set.
    if cop_filters.is_globally_excluded(path) {
        let is_explicit = explicit_files.contains(path)
            || path
                .canonicalize()
                .ok()
                .is_some_and(|c| explicit_files.contains(&c));
        if args.force_exclusion || !is_explicit {
            return Vec::new();
        }
    }

    // rubocop-rails MigrationFileSkippable: suppress all offenses on files
    // whose basename contains a 14-digit "timestamp" <= MigratedSchemaVersion.
    if cop_filters.is_migrated_file(path) {
        return Vec::new();
    }

    // Tier 1: stat check (mtime + size) — no file read needed
    if cache.is_enabled() {
        if let CacheLookup::StatHit(cached) = cache.get_by_stat(path) {
            cache_stat_hits.fetch_add(1, Ordering::Relaxed);
            return cached;
        }
    }

    // File read needed (either cache disabled, stat miss, or no entry)
    let io_start = std::time::Instant::now();
    let source = match SourceFile::from_path(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e:#}");
            return Vec::new();
        }
    };
    if let Some(t) = timers {
        t.file_io_ns
            .fetch_add(io_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    // Files with invalid UTF-8 that lack an encoding magic comment are
    // treated by RuboCop as a fatal Lint/Syntax error ("Invalid byte
    // sequence in utf-8.") and no other cops run. Files WITH an encoding
    // magic comment (e.g., `# encoding: iso-8859-1`) are processed normally.
    if std::str::from_utf8(source.as_bytes()).is_err()
        && !has_encoding_magic_comment(source.as_bytes())
    {
        let result = emit_invalid_utf8_diagnostic(
            &source,
            config,
            registry,
            cop_filters,
            has_dir_overrides,
            tier_map,
            args,
        );
        if cache.is_enabled() {
            cache.put(path, source.as_bytes(), &result);
        }
        return result;
    }

    // Tier 2: content hash check — file was read, mtime didn't match
    if cache.is_enabled() {
        if let CacheLookup::ContentHit(cached) = cache.get_by_content(path, source.as_bytes()) {
            cache_content_hits.fetch_add(1, Ordering::Relaxed);
            return cached;
        }
        cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    let (result, corrected_bytes, corrected_count) = lint_source_inner(
        &source,
        config,
        registry,
        args,
        tier_map,
        cop_filters,
        base_configs,
        has_dir_overrides,
        timers,
        allowlist,
    );
    if corrected_count > 0 {
        total_corrected.fetch_add(corrected_count, Ordering::Relaxed);
    }

    // Write corrected bytes to disk if autocorrect produced changes
    if let Some(bytes) = corrected_bytes {
        if let Err(e) = std::fs::write(path, &bytes) {
            eprintln!(
                "error: failed to write corrected file {}: {e}",
                path.display()
            );
        }
    }

    // Store result in cache
    if cache.is_enabled() {
        cache.put(path, source.as_bytes(), &result);
    }

    result
}

/// Check whether the first two lines of the file contain a Ruby encoding
/// magic comment (e.g., `# encoding: iso-8859-1`, `# -*- coding: euc-jp -*-`,
/// `# vim: fileencoding=shift_jis`). The check is byte-level (no UTF-8
/// decoding required) and only looks at the first two lines (or three if
/// the first line is a shebang).
fn has_encoding_magic_comment(bytes: &[u8]) -> bool {
    // Scan up to 3 lines (shebang + encoding comment + safety margin)
    let mut start = 0;
    for _line_idx in 0..3 {
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| start + p)
            .unwrap_or(bytes.len());
        let line = &bytes[start..end];
        // Trim leading whitespace and optional BOM on first line
        let trimmed = line
            .iter()
            .copied()
            .skip_while(|&b| b == b' ' || b == b'\t' || b == 0xEF || b == 0xBB || b == 0xBF)
            .collect::<Vec<u8>>();
        if trimmed.starts_with(b"#") {
            let lower: Vec<u8> = trimmed.iter().map(|b| b.to_ascii_lowercase()).collect();
            // Look for encoding/coding keywords in the comment
            if contains_subsequence(&lower, b"encoding") || contains_subsequence(&lower, b"coding")
            {
                return true;
            }
        }
        start = end + 1;
        if start >= bytes.len() {
            break;
        }
    }
    false
}

fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Name of the redundant cop disable directive cop.
const REDUNDANT_DISABLE_COP: &str = "Lint/RedundantCopDisableDirective";

/// Cops with known detection gaps (FN > 0 in corpus) that cause false
/// positives when flagging unused disable directives. When `all_cops_ran`
/// is true, unused directives for these cops are NOT flagged — the cop may
/// have missed an offense that the directive legitimately suppresses.
///
/// Generated from corpus oracle data. A cop belongs here if and only if it
/// has FN > 0 in the corpus baseline. As cop detection improves, entries
/// should be removed. Cops not in the registry (plugin-only) are handled
/// separately via the "unknown cop" path.
const REDUNDANT_DISABLE_SKIP_COPS: &[&str] = &[
    // Layout
    "Layout/ExtraSpacing",                   // FN=20
    "Layout/IndentationConsistency",         // FN=47
    "Layout/IndentationWidth",               // FN=739
    "Layout/LineLength",                     // FN=90
    "Layout/MultilineMethodCallIndentation", // FN=7992
    "Layout/MultilineOperationIndentation",  // FN=5590
    "Layout/RedundantLineBreak",             // FN=22466
    "Layout/SpaceAroundOperators",           // FN=2280
    // Lint
    "Lint/ShadowingOuterLocalVariable", // FN=3
    "Lint/Syntax",                      // FN=4
    "Lint/UnusedMethodArgument",        // FN=8
    "Lint/UselessAssignment",           // FN=523
    // Rails
    "Rails/AddColumnIndex",                      // FN=4
    "Rails/BulkChangeTable",                     // FN=2469
    "Rails/CreateTableWithTimestamps",           // FN=1462
    "Rails/EnumSyntax",                          // FN=3
    "Rails/HttpPositionalArguments",             // FN=977
    "Rails/NotNullColumn",                       // FN=129
    "Rails/RedundantTravelBack",                 // FN=7
    "Rails/ReversibleMigration",                 // FN=6
    "Rails/ReversibleMigrationMethodDefinition", // FN=2350
    "Rails/SaveBang",                            // FN=241
    "Rails/ThreeStateBooleanColumn",             // FN=25
    "Rails/TimeZoneAssignment",                  // FN=3
    "Rails/UniqueValidationWithoutIndex",        // FN=22
    "Rails/UnusedIgnoredColumns",                // FN=29
    // Rake
    "Rake/DuplicateNamespace", // FN=4
    // Security
    "Security/YAMLLoad", // stub (never fires)
    // Style
    "Style/AccessModifierDeclarations",   // FN=16
    "Style/ConditionalAssignment",        // FN=5951
    "Style/Documentation",                // FN=22
    "Style/DocumentationMethod",          // FN=63
    "Style/FrozenStringLiteralComment",   // FN=13
    "Style/IdenticalConditionalBranches", // FN=9
    "Style/IfUnlessModifier",             // FN=1216
    "Style/NonNilCheck",                  // FN=2
    "Style/RedundantLineContinuation",    // FN=87
    "Style/RedundantParentheses",         // FN=811
    "Style/RedundantSelf",                // FN=228
    "Style/SafeNavigation",               // FN=205
    "Style/Semicolon",                    // FN=60
];

fn redundant_disable_target_ruby_version(config: &CopConfig) -> f64 {
    config
        .options
        .get("TargetRubyVersion")
        .and_then(|value| value.as_f64())
        .unwrap_or(2.7)
}

struct RedundantDirectiveCheck<'a> {
    registry: &'a CopRegistry,
    base_configs: &'a [CopConfig],
    cop_filters: &'a CopFilterSet,
    path: &'a Path,
    has_only_filter: bool,
    redundant_disable_explicitly_selected: bool,
    all_cops_ran: bool,
}

/// Determine if a disable directive should be flagged as redundant.
///
/// Returns `Some(suffix)` if the directive IS redundant (should be reported),
/// where `suffix` is appended to the message (e.g. `" (unknown cop)"`).
/// Returns `None` if we should skip it.
///
/// The logic is conservative to avoid false positives:
///   - "all" or department-only directives: never flag (too broad to check)
///   - Known cop that is explicitly disabled (Enabled: false): flag as redundant
///   - Known cop that is enabled + all_cops_ran + file matched + NOT in skip
///     list: flag as redundant (cop ran and didn't fire)
///   - Known cop in `REDUNDANT_DISABLE_SKIP_COPS`: never flag (detection gaps)
///   - Renamed cop: flag, unless new-name cop is in skip list
///   - Completely unknown cop: flag with "(unknown cop)" suffix, except during
///     unrelated `--only` runs where it would leak into another cop's results
fn is_directive_redundant(
    directive: &DisableDirective,
    context: &RedundantDirectiveCheck<'_>,
) -> Option<&'static str> {
    let cop_name = directive.cop_name.as_str();

    // "all" is a wildcard — never flag (too broad to determine redundancy)
    if cop_name == "all" {
        return None;
    }

    // Department-only name (no '/') — never flag (too broad to check)
    if !cop_name.contains('/') {
        return None;
    }

    // Self-referential: disabling RedundantCopDisableDirective itself is
    // legitimate (used in RuboCop's own source, for example).
    if cop_name == REDUNDANT_DISABLE_COP {
        return None;
    }

    // Fully qualified cop name — check if it's in the registry
    let cop_entry = context
        .registry
        .cops()
        .iter()
        .enumerate()
        .find(|(_, c)| c.name() == cop_name);

    if let Some((idx, _)) = cop_entry {
        // Cop IS in the registry.
        let filter = context.cop_filters.cop_filter(idx);
        if !filter.is_enabled() {
            // Cop is explicitly disabled — the disable directive is redundant.
            return Some("");
        }
        // Cop is enabled. Check if it's explicitly excluded from this file
        // by Exclude patterns (e.g., Lint/UselessMethodDefinition excluded
        // from app/controllers/**). Only check Exclude, NOT Include — Include
        // mismatches can arise from sub-config directory path issues and are
        // not reliable indicators of redundancy.
        if context.cop_filters.is_cop_excluded(idx, context.path) {
            return Some("");
        }
        // Cop is enabled and not explicitly excluded.
        if context.all_cops_ran && context.cop_filters.is_cop_match(idx, context.path) {
            let target_ruby_version =
                redundant_disable_target_ruby_version(&context.base_configs[idx]);
            // All cops ran AND this specific cop matched the file (Include
            // patterns matched), so we have reliable usage data — if the
            // directive is still unused, the cop genuinely didn't fire and
            // the directive is redundant.
            //
            // Skip cops with known detection gaps vs RuboCop, where
            // nitrocop may miss an offense that the directive legitimately
            // suppresses.
            if !REDUNDANT_DISABLE_SKIP_COPS.contains(&cop_name)
                || allow_redundant_disable_flagging_for_known_gap_cop(
                    cop_name,
                    target_ruby_version,
                    directive.is_inline,
                )
            {
                return Some("");
            }
        }
        // Conservative: in --only mode for other cops, the target cop may
        // not have run, so we can't tell if the directive is needed.
        None
    } else {
        // Cop is NOT in the registry. Check if it's a renamed cop whose new
        // name IS in the registry and is enabled. RuboCop treats disable
        // directives for renamed cops as redundant since the old name no
        // longer exists.
        if let Some(new_name) = RENAMED_COPS.get(cop_name) {
            // The cop was renamed. Look up the new-name cop's state to mirror
            // the logic applied to registered cops above.
            let new_entry = context
                .registry
                .cops()
                .iter()
                .enumerate()
                .find(|(_, c)| c.name() == new_name.as_str());
            if let Some((new_idx, _)) = new_entry {
                // In --only mode the new-name cop may not have run, so check
                // its config: if it's enabled, the old-name directive might be
                // suppressing its offenses — skip.
                if context.has_only_filter
                    && !context.all_cops_ran
                    && context.cop_filters.cop_filter(new_idx).is_enabled()
                    && !context.cop_filters.is_cop_excluded(new_idx, context.path)
                {
                    return None;
                }
                // In run_all_for_redundant mode, the new-name cop ran. If it
                // has known detection gaps (in REDUNDANT_DISABLE_SKIP_COPS),
                // the old-name directive might legitimately suppress an offense
                // that nitrocop missed — don't flag it.
                if context.all_cops_ran
                    && context.cop_filters.is_cop_match(new_idx, context.path)
                    && REDUNDANT_DISABLE_SKIP_COPS.contains(&new_name.as_str())
                    && !allow_redundant_disable_flagging_for_known_gap_cop(
                        new_name,
                        redundant_disable_target_ruby_version(&context.base_configs[new_idx]),
                        directive.is_inline,
                    )
                {
                    return None;
                }
                // Even without all_cops_ran (e.g. --except mode), don't flag
                // if the new-name cop has known detection gaps — the directive
                // may legitimately suppress an offense nitrocop misses.
                if !context.all_cops_ran && REDUNDANT_DISABLE_SKIP_COPS.contains(&new_name.as_str())
                {
                    return None;
                }
            }
            return Some("");
        }

        // Cop is completely unknown — not in registry, not renamed. RuboCop
        // flags these with "(unknown cop)" since they can never suppress an
        // offense. However, during --only runs for an unrelated cop, suppress
        // these to avoid leaking diagnostics into that cop's corpus results.
        if context.has_only_filter && !context.redundant_disable_explicitly_selected {
            return None;
        }
        Some(" (unknown cop)")
    }
}

/// Validate that corrected bytes are still valid Ruby by re-parsing with Prism.
/// Returns `None` (discarding corrections) if parse errors are found.
fn validate_corrected_bytes(
    original_bytes: &[u8],
    current_bytes: Vec<u8>,
    path: &std::path::Path,
) -> Option<Vec<u8>> {
    if current_bytes == original_bytes {
        return None;
    }
    // Scope the parse_result so its borrow of current_bytes is dropped before we return.
    let has_errors = {
        let parse_result = crate::parse::parse_source(&current_bytes);
        parse_result.errors().count() > 0
    };
    if has_errors {
        eprintln!(
            "warning: autocorrect produced invalid syntax for {}, skipping corrections",
            path.display()
        );
        return None;
    }
    Some(current_bytes)
}

/// Returns (diagnostics, corrected_bytes, corrected_count).
/// corrected_count is the total number of offenses corrected across all iterations.
#[allow(clippy::too_many_arguments)] // internal lint pipeline threading shared state
pub(crate) fn lint_source_inner(
    source: &SourceFile,
    config: &ResolvedConfig,
    registry: &CopRegistry,
    args: &Args,
    tier_map: &TierMap,
    cop_filters: &CopFilterSet,
    base_configs: &[CopConfig],
    has_dir_overrides: bool,
    timers: Option<&PhaseTimers>,
    allowlist: &crate::cop::autocorrect_allowlist::AutocorrectAllowlist,
) -> (Vec<Diagnostic>, Option<Vec<u8>>, usize) {
    let autocorrect_mode = args.autocorrect_mode();

    if autocorrect_mode == crate::cli::AutocorrectMode::Off {
        // Fast path: no autocorrect, run once
        let (diags, _) = lint_source_once(
            source,
            config,
            registry,
            args,
            tier_map,
            cop_filters,
            base_configs,
            has_dir_overrides,
            timers,
            autocorrect_mode,
            allowlist,
        );
        return (diags, None, 0);
    }

    // Autocorrect iteration loop
    let original_bytes = source.as_bytes();
    let mut current_bytes = original_bytes.to_vec();
    let path = source.path.clone();
    let mut corrected_diags: Vec<Diagnostic> = Vec::new();

    const MAX_ITERATIONS: usize = 200;

    for _iteration in 0..MAX_ITERATIONS {
        let iter_source = SourceFile::from_vec(path.clone(), current_bytes.clone());
        let (diags, corrections) = lint_source_once(
            &iter_source,
            config,
            registry,
            args,
            tier_map,
            cop_filters,
            base_configs,
            has_dir_overrides,
            timers,
            autocorrect_mode,
            allowlist,
        );

        if corrections.is_empty() {
            // Converged — no more corrections. Merge corrected diagnostics from
            // earlier iterations with the remaining diagnostics from this pass.
            let mut all_diags = corrected_diags;
            all_diags.extend(diags);
            let total_corrected = all_diags.iter().filter(|d| d.corrected).count();
            let corrected_bytes = validate_corrected_bytes(original_bytes, current_bytes, &path);
            return (all_diags, corrected_bytes, total_corrected);
        }

        // Collect corrected diagnostics from this iteration
        corrected_diags.extend(diags.into_iter().filter(|d| d.corrected));

        let correction_set = crate::correction::CorrectionSet::from_vec(corrections);
        let new_bytes = correction_set.apply(&current_bytes);

        if new_bytes == current_bytes {
            // Source unchanged despite corrections — bail to avoid infinite loop.
            let total_corrected = corrected_diags.iter().filter(|d| d.corrected).count();
            return (corrected_diags, None, total_corrected);
        }

        current_bytes = new_bytes;
    }

    // Hit max iterations — run one final pass without corrections to get clean diagnostics
    let final_source = SourceFile::from_vec(path.clone(), current_bytes.clone());
    let (diags, _) = lint_source_once(
        &final_source,
        config,
        registry,
        args,
        tier_map,
        cop_filters,
        base_configs,
        has_dir_overrides,
        timers,
        crate::cli::AutocorrectMode::Off,
        allowlist,
    );
    let mut all_diags = corrected_diags;
    all_diags.extend(diags);
    let total_corrected = all_diags.iter().filter(|d| d.corrected).count();
    let corrected_bytes = validate_corrected_bytes(original_bytes, current_bytes, &path);
    (all_diags, corrected_bytes, total_corrected)
}

/// Check if a Prism parse error is "semantic" — meaning the AST structure is still
/// valid despite the error. Prism reports certain construct-context violations
/// (break/next/redo outside loops, retry outside rescue, yield outside methods)
/// as parse errors, but the Parser gem (used by RuboCop) accepts them as valid
/// syntax. Skipping files with only these errors causes false negatives.
fn is_semantic_parse_error(message: &str) -> bool {
    // PM_ERR_INVALID_BLOCK_EXIT: "Invalid break", "Invalid next", "Invalid redo"
    // PM_ERR_INVALID_RETRY_*: "Invalid retry ..."
    // PM_ERR_INVALID_YIELD: "Invalid yield"
    // PM_ERR_RETURN_INVALID: "Invalid return in class/module body"
    message.starts_with("Invalid break")
        || message.starts_with("Invalid next")
        || message.starts_with("Invalid redo")
        || message.starts_with("Invalid retry")
        || message == "Invalid yield"
        || message.starts_with("Invalid return in class/module body")
}

/// Emit Lint/Syntax diagnostics for structural parse errors.
/// RuboCop reports parser errors as Lint/Syntax offenses. We emit one diagnostic
/// per structural parse error, skipping semantic-only errors (break/next/retry/yield
/// outside proper context) which Prism reports but the Parser gem does not.
#[allow(clippy::too_many_arguments)]
fn emit_syntax_diagnostics(
    source: &SourceFile,
    parse_result: &ruby_prism::ParseResult<'_>,
    registry: &CopRegistry,
    cop_filters: &CopFilterSet,
    has_dir_overrides: bool,
    config: &ResolvedConfig,
    tier_map: &TierMap,
    args: &Args,
) -> Vec<Diagnostic> {
    const SYNTAX_COP: &str = "Lint/Syntax";

    // Check if Lint/Syntax is enabled for this file
    let cops = registry.cops();
    let syntax_idx = cops.iter().position(|c| c.name() == SYNTAX_COP);
    let syntax_idx = match syntax_idx {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    // Apply per-file config overrides if needed
    let effective_config = if has_dir_overrides {
        config.effective_config_for_file(&source.path)
    } else {
        None
    };
    let owned_filters;
    let active_filters = if let Some(ref file_config) = effective_config {
        owned_filters = file_config.build_cop_filters(registry, tier_map, args.preview);
        &owned_filters
    } else {
        cop_filters
    };

    let filter = active_filters.cop_filter(syntax_idx);
    if !filter.is_enabled() {
        return Vec::new();
    }
    if active_filters.is_cop_excluded(syntax_idx, &source.path) {
        return Vec::new();
    }
    if !args.only.is_empty() && !args.only.iter().any(|o| o == SYNTAX_COP) {
        return Vec::new();
    }
    if args.except.iter().any(|e| e == SYNTAX_COP) {
        return Vec::new();
    }
    let src_bytes = source.as_bytes();
    let src_len = src_bytes.len();
    let ends_with_newline = src_bytes.last() == Some(&b'\n');

    let mut diagnostics = Vec::new();
    for err in parse_result.errors() {
        // Skip most semantic parse errors (break/next/redo/yield outside proper
        // context) — the Parser gem doesn't report those. But "Invalid retry
        // without rescue" and "Invalid return in class/module body" ARE reported
        // by RuboCop as Lint/Syntax offenses even when the file has structural
        // errors, so let those through.
        if is_semantic_parse_error(err.message())
            && !crate::cop::lint::syntax::is_rubocop_reported_semantic_error(err.message())
        {
            continue;
        }
        let loc = err.location();
        let offset = loc.start_offset();
        let (mut line, mut column) = source.offset_to_line_col(offset);
        // Prism reports errors at one-past-the-end (offset == file length) as
        // being on the line AFTER the last newline. Our offset_to_line_col maps
        // that offset to the last content line instead. Adjust to match Prism's
        // (and RuboCop's) line numbering.
        if offset >= src_len && ends_with_newline {
            line += 1;
            column = 0;
        }
        diagnostics.push(Diagnostic {
            path: source.path.display().to_string(),
            location: Location { line, column },
            severity: Severity::Fatal,
            cop_name: SYNTAX_COP.to_string(),
            message: err.message().to_string(),
            corrected: false,
        });
    }
    diagnostics
}

/// Emit a single Lint/Syntax diagnostic for files with invalid UTF-8 bytes.
///
/// RuboCop treats files with invalid byte sequences (and no encoding magic
/// comment) as a fatal Lint/Syntax error with message "Invalid byte sequence
/// in utf-8." reported at line 1. This matches `add_offense_from_error` in
/// RuboCop's `Lint::Syntax` cop.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_invalid_utf8_diagnostic(
    source: &SourceFile,
    config: &ResolvedConfig,
    registry: &CopRegistry,
    cop_filters: &CopFilterSet,
    has_dir_overrides: bool,
    tier_map: &TierMap,
    args: &Args,
) -> Vec<Diagnostic> {
    const SYNTAX_COP: &str = "Lint/Syntax";

    let cops = registry.cops();
    let syntax_idx = match cops.iter().position(|c| c.name() == SYNTAX_COP) {
        Some(idx) => idx,
        None => return Vec::new(),
    };

    let effective_config = if has_dir_overrides {
        config.effective_config_for_file(&source.path)
    } else {
        None
    };
    let owned_filters;
    let active_filters = if let Some(ref file_config) = effective_config {
        owned_filters = file_config.build_cop_filters(registry, tier_map, args.preview);
        &owned_filters
    } else {
        cop_filters
    };

    let filter = active_filters.cop_filter(syntax_idx);
    if !filter.is_enabled() {
        return Vec::new();
    }
    if active_filters.is_cop_excluded(syntax_idx, &source.path) {
        return Vec::new();
    }
    if !args.only.is_empty() && !args.only.iter().any(|o| o == SYNTAX_COP) {
        return Vec::new();
    }
    if args.except.iter().any(|e| e == SYNTAX_COP) {
        return Vec::new();
    }

    vec![Diagnostic {
        path: source.path.display().to_string(),
        location: Location { line: 1, column: 0 },
        severity: Severity::Fatal,
        cop_name: SYNTAX_COP.to_string(),
        message: "Invalid byte sequence in utf-8.".to_string(),
        corrected: false,
    }]
}

/// Run all enabled cops once on a source file. Returns (diagnostics, corrections).
#[allow(clippy::too_many_arguments)] // internal lint pipeline threading shared state
fn lint_source_once(
    source: &SourceFile,
    config: &ResolvedConfig,
    registry: &CopRegistry,
    args: &Args,
    tier_map: &TierMap,
    cop_filters: &CopFilterSet,
    base_configs: &[CopConfig],
    has_dir_overrides: bool,
    timers: Option<&PhaseTimers>,
    autocorrect_mode: crate::cli::AutocorrectMode,
    allowlist: &crate::cop::autocorrect_allowlist::AutocorrectAllowlist,
) -> (Vec<Diagnostic>, Vec<crate::correction::Correction>) {
    // Parse on this thread (ParseResult is !Send)
    let parse_start = std::time::Instant::now();
    let parse_result = crate::parse::parse_source(source.as_bytes());
    if let Some(t) = timers {
        t.parse_ns
            .fetch_add(parse_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    // Skip cops on files with structural parse errors — the AST from error
    // recovery is unreliable and produces false positives (e.g., Procfile.spec
    // parsed as Ruby).
    //
    // However, Prism reports some "semantic" errors (like `break` outside a
    // loop, `retry` outside rescue, `yield` outside a method) that don't affect
    // AST structure. RuboCop (using the Parser gem) doesn't skip files with
    // these errors, so we must also process them to avoid false negatives.
    let has_structural_errors = parse_result
        .errors()
        .any(|err| !is_semantic_parse_error(err.message()));
    if has_structural_errors {
        if let Some(t) = timers {
            t.codemap_ns.fetch_add(0, Ordering::Relaxed);
        }

        // Emit Lint/Syntax diagnostics for each structural parse error,
        // matching RuboCop which reports parser errors as Lint/Syntax offenses.
        let syntax_diagnostics = emit_syntax_diagnostics(
            source,
            &parse_result,
            registry,
            cop_filters,
            has_dir_overrides,
            config,
            tier_map,
            args,
        );
        return (syntax_diagnostics, Vec::new());
    }

    // Non-UTF-8 files with encoding magic comments (e.g., `# encoding: iso-8859-7`)
    // are parsed successfully by Prism. We build the CodeMap and run all cop phases
    // (check_source, check_node) on them, matching RuboCop's behavior. Files without
    // encoding comments are already skipped above (returned empty Vec).
    let codemap_start = std::time::Instant::now();
    let code_map = CodeMap::from_parse_result(source.as_bytes(), &parse_result);
    if let Some(t) = timers {
        t.codemap_ns
            .fetch_add(codemap_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    let mut diagnostics = Vec::new();
    let mut corrections: Vec<crate::correction::Correction> = Vec::new();

    let cop_start = std::time::Instant::now();
    let filter_start = std::time::Instant::now();

    let effective_config = if has_dir_overrides {
        config.effective_config_for_file(&source.path)
    } else {
        None
    };
    let owned_filters;
    let owned_base_configs;
    let (active_filters, active_base_configs) = if let Some(ref file_config) = effective_config {
        owned_filters = file_config.build_cop_filters(registry, tier_map, args.preview);
        owned_base_configs = file_config.precompute_cop_configs(registry);
        (&owned_filters, owned_base_configs.as_slice())
    } else {
        (cop_filters, base_configs)
    };

    let mut ast_cop_indices: Vec<usize> = Vec::new();

    let cops = registry.cops();
    let has_only = !args.only.is_empty();

    // When Lint/RedundantCopDisableDirective is the sole --only target, run ALL
    // cops so that disable directives which truly suppress offenses get marked
    // as used. Without this, --only mode never runs other cops, so every
    // directive appears unused and we can't distinguish truly redundant ones.
    let run_all_for_redundant =
        has_only && args.only.len() == 1 && args.only[0] == REDUNDANT_DISABLE_COP;

    // All cops ran if: (a) run_all_for_redundant mode, or (b) normal mode
    // with no --only/--except filters. In these cases, every enabled cop that
    // matched the file executed, so unused disable directives are reliable
    // indicators of redundancy (modulo REDUNDANT_DISABLE_SKIP_COPS).
    let all_cops_ran = run_all_for_redundant || (!has_only && args.except.is_empty());

    // Pass 1: Universal cops
    for &i in active_filters.universal_cop_indices() {
        let cop = &cops[i];
        let name = cop.name();
        if name == REDUNDANT_DISABLE_COP {
            continue;
        }
        if has_only && !run_all_for_redundant && !args.only.iter().any(|o| o == name) {
            continue;
        }
        if args.except.iter().any(|e| e == name) {
            continue;
        }

        let cop_config = &active_base_configs[i];

        let should_correct = autocorrect_mode != crate::cli::AutocorrectMode::Off
            && cop.supports_autocorrect()
            && cop_config.should_autocorrect(autocorrect_mode)
            && (autocorrect_mode == crate::cli::AutocorrectMode::All || allowlist.contains(name));

        if should_correct {
            cop.check_lines(source, cop_config, &mut diagnostics, Some(&mut corrections));
            cop.check_source(
                source,
                &parse_result,
                &code_map,
                cop_config,
                &mut diagnostics,
                Some(&mut corrections),
            );
        } else {
            cop.check_lines(source, cop_config, &mut diagnostics, None);
            cop.check_source(
                source,
                &parse_result,
                &code_map,
                cop_config,
                &mut diagnostics,
                None,
            );
        }
        ast_cop_indices.push(i);
    }

    // Pass 2: Pattern cops
    for &i in active_filters.pattern_cop_indices() {
        let cop = &cops[i];
        let name = cop.name();
        if name == REDUNDANT_DISABLE_COP {
            continue;
        }
        if has_only && !run_all_for_redundant && !args.only.iter().any(|o| o == name) {
            continue;
        }
        if args.except.iter().any(|e| e == name) {
            continue;
        }

        if !active_filters.is_cop_match(i, &source.path) {
            continue;
        }

        let cop_config = &active_base_configs[i];

        let should_correct = autocorrect_mode != crate::cli::AutocorrectMode::Off
            && cop.supports_autocorrect()
            && cop_config.should_autocorrect(autocorrect_mode)
            && (autocorrect_mode == crate::cli::AutocorrectMode::All || allowlist.contains(name));

        if should_correct {
            cop.check_lines(source, cop_config, &mut diagnostics, Some(&mut corrections));
            cop.check_source(
                source,
                &parse_result,
                &code_map,
                cop_config,
                &mut diagnostics,
                Some(&mut corrections),
            );
        } else {
            cop.check_lines(source, cop_config, &mut diagnostics, None);
            cop.check_source(
                source,
                &parse_result,
                &code_map,
                cop_config,
                &mut diagnostics,
                None,
            );
        }
        ast_cop_indices.push(i);
    }

    if let Some(t) = timers {
        t.cop_filter_ns
            .fetch_add(filter_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    // ── VariableForce engine ───────────────────────────────────────────
    // Run the shared variable analysis engine if any active cop is a
    // VariableForceConsumer. This performs a single AST walk that replaces
    // up to 10 separate per-cop variable-tracking visitors.
    {
        let vf_consumers: Vec<crate::cop::variable_force::engine::RegisteredConsumer<'_>> =
            ast_cop_indices
                .iter()
                .filter_map(|&i| {
                    let cop = &*registry.cops()[i];
                    cop.as_variable_force_consumer().map(|consumer| {
                        crate::cop::variable_force::engine::RegisteredConsumer {
                            consumer,
                            config: &active_base_configs[i],
                        }
                    })
                })
                .collect();
        if !vf_consumers.is_empty() {
            let mut engine = crate::cop::variable_force::engine::Engine::new(source, &vf_consumers);
            engine.run(&parse_result);
            diagnostics.extend(engine.into_diagnostics());
        }
    }

    let ast_start = std::time::Instant::now();
    if !ast_cop_indices.is_empty() {
        let ast_cops: Vec<(&dyn Cop, &CopConfig)> = ast_cop_indices
            .iter()
            .map(|&i| (&*registry.cops()[i] as &dyn Cop, &active_base_configs[i]))
            .collect();
        let mut walker = BatchedCopWalker::new(ast_cops, source, &parse_result);
        if autocorrect_mode != crate::cli::AutocorrectMode::Off {
            walker = walker.with_corrections();
        }
        walker.visit(&parse_result.node());
        let (walker_diags, walker_corrections) = walker.into_results();
        diagnostics.extend(walker_diags);
        if let Some(wc) = walker_corrections {
            if autocorrect_mode == crate::cli::AutocorrectMode::Safe {
                corrections.extend(wc.into_iter().filter(|c| allowlist.contains(c.cop_name)));
            } else {
                corrections.extend(wc);
            }
        }
    }
    if let Some(t) = timers {
        t.cop_ast_ns
            .fetch_add(ast_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        t.cop_exec_ns
            .fetch_add(cop_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    // Filter out diagnostics suppressed by inline disable comments,
    // and detect redundant disable directives.
    let disable_start = std::time::Instant::now();
    let mut disabled =
        crate::parse::directives::DisabledRanges::from_comments(source, &parse_result, registry);

    if !args.ignore_disable_comments && !disabled.is_empty() {
        diagnostics.retain(|d| !disabled.check_and_mark_used(&d.cop_name, d.location.line));
    }

    // In run_all_for_redundant mode, we ran all cops just to mark directives as
    // used. Now discard their diagnostics — only RedundantCopDisableDirective
    // output should be emitted.
    if run_all_for_redundant {
        diagnostics.clear();
    }

    if !args.ignore_disable_comments
        && disabled.has_directives()
        && !args.except.iter().any(|e| e == REDUNDANT_DISABLE_COP)
    {
        let cop_enabled = registry
            .cops()
            .iter()
            .enumerate()
            .find(|(_, c)| c.name() == REDUNDANT_DISABLE_COP)
            .is_some_and(|(idx, _)| active_filters.is_cop_match(idx, &source.path));

        if cop_enabled {
            let redundant_disable_explicitly_selected =
                args.only.iter().any(|o| o == REDUNDANT_DISABLE_COP);
            let redundant_check = RedundantDirectiveCheck {
                registry,
                base_configs: active_base_configs,
                cop_filters: active_filters,
                path: &source.path,
                has_only_filter: !args.only.is_empty(),
                redundant_disable_explicitly_selected,
                all_cops_ran,
            };
            for directive in disabled.unused_directives() {
                // If this line is within an explicit `# rubocop:disable
                // Lint/RedundantCopDisableDirective` region, suppress the
                // offense — the user explicitly silenced this cop here.
                // Only check the exact cop name, not department ("Lint") or
                // "all" wildcards, to match the conservative existing behavior
                // for broad disables.
                if disabled.check_ranges(REDUNDANT_DISABLE_COP, directive.line) {
                    continue;
                }

                let suffix = match is_directive_redundant(directive, &redundant_check) {
                    Some(s) => s,
                    None => continue,
                };

                let message = format!(
                    "Unnecessary disabling of `{}`{}.",
                    directive.cop_name, suffix
                );
                diagnostics.push(Diagnostic {
                    path: source.path_str().to_string(),
                    location: Location {
                        line: directive.line,
                        column: directive.column,
                    },
                    severity: Severity::Warning,
                    cop_name: REDUNDANT_DISABLE_COP.to_string(),
                    message,
                    corrected: false,
                });
            }
        }
    }
    if let Some(t) = timers {
        t.disable_ns
            .fetch_add(disable_start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    (diagnostics, corrections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- validate_corrected_bytes ---

    #[test]
    fn validate_corrected_bytes_rejects_invalid_syntax() {
        let original = b"puts 'hello'";
        let invalid = b"def (".to_vec();
        let result = validate_corrected_bytes(original, invalid, &PathBuf::from("test.rb"));
        assert!(result.is_none(), "should reject invalid Ruby syntax");
    }

    #[test]
    fn validate_corrected_bytes_accepts_valid_syntax() {
        let original = b"puts 'hello'";
        let valid = b"puts 'world'".to_vec();
        let result = validate_corrected_bytes(original, valid, &PathBuf::from("test.rb"));
        assert!(result.is_some(), "should accept valid Ruby syntax");
        assert_eq!(result.unwrap(), b"puts 'world'");
    }

    #[test]
    fn validate_corrected_bytes_returns_none_when_unchanged() {
        let original = b"puts 'hello'";
        let unchanged = b"puts 'hello'".to_vec();
        let result = validate_corrected_bytes(original, unchanged, &PathBuf::from("test.rb"));
        assert!(result.is_none(), "should return None when source unchanged");
    }

    // --- parse_renamed_cops ---

    #[test]
    fn parse_renamed_cops_simple_format() {
        let yaml = "renamed:\n  Old/Cop: New/Cop\n  Another/Old: Another/New\n";
        let map = parse_renamed_cops(yaml);
        assert_eq!(map.get("Old/Cop").unwrap(), "New/Cop");
        assert_eq!(map.get("Another/Old").unwrap(), "Another/New");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parse_renamed_cops_extended_format() {
        let yaml = "renamed:\n  Naming/PredicateName:\n    new_name: Naming/PredicatePrefix\n    severity: warning\n";
        let map = parse_renamed_cops(yaml);
        assert_eq!(
            map.get("Naming/PredicateName").unwrap(),
            "Naming/PredicatePrefix"
        );
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_renamed_cops_mixed_formats() {
        let yaml = "\
renamed:
  Simple/Rename: Target/Cop
  Extended/Rename:
    new_name: Target/Extended
    severity: warning
  Another/Simple: Another/Target
";
        let map = parse_renamed_cops(yaml);
        assert_eq!(map.get("Simple/Rename").unwrap(), "Target/Cop");
        assert_eq!(map.get("Extended/Rename").unwrap(), "Target/Extended");
        assert_eq!(map.get("Another/Simple").unwrap(), "Another/Target");
        assert_eq!(map.len(), 3);
    }

    #[test]
    fn parse_renamed_cops_missing_renamed_section() {
        let yaml = "removed:\n  Some/Cop: true\n";
        let map = parse_renamed_cops(yaml);
        assert!(map.is_empty());
    }

    #[test]
    fn parse_renamed_cops_empty_yaml() {
        let map = parse_renamed_cops("");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_renamed_cops_invalid_yaml() {
        let map = parse_renamed_cops("{{invalid yaml::");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_renamed_cops_extended_missing_new_name_key() {
        // Extended format but without the new_name key — should be skipped
        let yaml = "renamed:\n  Bad/Entry:\n    severity: warning\n";
        let map = parse_renamed_cops(yaml);
        assert!(
            map.is_empty(),
            "entry with empty new_name should be skipped"
        );
    }

    #[test]
    fn parse_renamed_cops_bundled_snapshot() {
        // Verify the bundled renamed-cops snapshot parses correctly.
        let map = parse_renamed_cops(include_str!("resources/renamed_cops.yml"));
        // Spot-check a few known renames
        assert_eq!(map.get("Layout/Tab").unwrap(), "Layout/IndentationStyle");
        assert_eq!(map.get("Lint/Eval").unwrap(), "Security/Eval");
        assert_eq!(
            map.get("Naming/PredicateName").unwrap(),
            "Naming/PredicatePrefix"
        );
        assert_eq!(
            map.get("Style/PredicateName").unwrap(),
            "Naming/PredicatePrefix"
        );
        // Should have a reasonable number of entries
        assert!(
            map.len() >= 30,
            "expected at least 30 renames, got {}",
            map.len()
        );
    }

    // --- PhaseTimers ---

    #[test]
    fn phase_timers_initializes_to_zero() {
        let t = PhaseTimers::new();
        assert_eq!(t.file_io_ns.load(Ordering::Relaxed), 0);
        assert_eq!(t.parse_ns.load(Ordering::Relaxed), 0);
        assert_eq!(t.codemap_ns.load(Ordering::Relaxed), 0);
        assert_eq!(t.cop_exec_ns.load(Ordering::Relaxed), 0);
        assert_eq!(t.cop_filter_ns.load(Ordering::Relaxed), 0);
        assert_eq!(t.cop_ast_ns.load(Ordering::Relaxed), 0);
        assert_eq!(t.disable_ns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn phase_timers_accumulates_across_fetches() {
        let t = PhaseTimers::new();
        t.parse_ns.fetch_add(100, Ordering::Relaxed);
        t.parse_ns.fetch_add(200, Ordering::Relaxed);
        assert_eq!(t.parse_ns.load(Ordering::Relaxed), 300);

        t.cop_exec_ns.fetch_add(500, Ordering::Relaxed);
        t.file_io_ns.fetch_add(50, Ordering::Relaxed);
        assert_eq!(t.cop_exec_ns.load(Ordering::Relaxed), 500);
        assert_eq!(t.file_io_ns.load(Ordering::Relaxed), 50);
    }

    // --- RENAMED_COPS static ---

    #[test]
    fn renamed_cops_static_is_populated() {
        // The LazyLock should parse the bundled snapshot on first access.
        assert!(!RENAMED_COPS.is_empty(), "RENAMED_COPS should not be empty");
        assert!(RENAMED_COPS.contains_key("Layout/Tab"));
    }

    // --- has_encoding_magic_comment ---

    #[test]
    fn encoding_comment_standard() {
        assert!(has_encoding_magic_comment(
            b"# encoding: iso-8859-1\nx = 1\n"
        ));
    }

    #[test]
    fn encoding_comment_coding() {
        assert!(has_encoding_magic_comment(b"# coding: euc-jp\nx = 1\n"));
    }

    #[test]
    fn encoding_comment_emacs_style() {
        assert!(has_encoding_magic_comment(
            b"# -*- encoding: iso-8859-9 -*-\nx = 1\n"
        ));
    }

    #[test]
    fn encoding_comment_vim_style() {
        assert!(has_encoding_magic_comment(
            b"# vim: fileencoding=shift_jis\nx = 1\n"
        ));
    }

    #[test]
    fn encoding_comment_after_shebang() {
        assert!(has_encoding_magic_comment(
            b"#!/usr/bin/env ruby\n# encoding: EUC-JP\nx = 1\n"
        ));
    }

    #[test]
    fn no_encoding_comment() {
        assert!(!has_encoding_magic_comment(b"# A normal comment\nx = 1\n"));
    }

    #[test]
    fn no_encoding_comment_empty_file() {
        assert!(!has_encoding_magic_comment(b""));
    }

    #[test]
    fn encoding_comment_utf8_still_detected() {
        assert!(has_encoding_magic_comment(b"# encoding: utf-8\nx = 1\n"));
    }
}
