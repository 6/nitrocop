pub mod cache;
pub mod cli;
pub mod config;
pub mod cop;
pub mod correction;
pub mod diagnostic;
pub mod doctor;
pub mod formatter;
pub mod fs;
pub mod linter;
pub mod migrate;
pub mod node_pattern;
pub mod parse;
pub mod rules;
pub mod schema;
pub mod verify;

#[cfg(test)]
pub mod testutil;

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

use anyhow::Result;

use cli::{Args, StrictScope};
use config::load_config;
use cop::registry::CopRegistry;
use cop::tiers::{SkipSummary, TierMap};
use formatter::create_formatter;
use fs::{DiscoveredFiles, discover_files};
use linter::{lint_source, run_linter};
use parse::source::SourceFile;

/// Check whether the skip summary violates the given strict scope.
/// Returns `true` if the strict check fails (i.e., exit 2 should be used).
fn strict_check_fails(scope: StrictScope, summary: &SkipSummary) -> bool {
    match scope {
        StrictScope::Coverage | StrictScope::ImplementedOnly => !summary.preview_gated.is_empty(),
        StrictScope::All => !summary.is_empty(),
    }
}

/// Print a strict-mode warning to stderr.
fn print_strict_warning(scope: StrictScope, summary: &SkipSummary) {
    let scope_name = match scope {
        StrictScope::Coverage => "coverage",
        StrictScope::ImplementedOnly => "implemented-only",
        StrictScope::All => "all",
    };
    let total = match scope {
        StrictScope::Coverage | StrictScope::ImplementedOnly => summary.preview_gated.len(),
        StrictScope::All => summary.total(),
    };
    let detail = match scope {
        StrictScope::Coverage | StrictScope::ImplementedOnly => {
            format!("{} preview-gated", summary.preview_gated.len())
        }
        StrictScope::All => {
            let mut parts = Vec::new();
            if !summary.preview_gated.is_empty() {
                parts.push(format!("{} preview-gated", summary.preview_gated.len()));
            }
            if !summary.unimplemented.is_empty() {
                parts.push(format!("{} unimplemented", summary.unimplemented.len()));
            }
            if !summary.outside_baseline.is_empty() {
                parts.push(format!(
                    "{} outside baseline",
                    summary.outside_baseline.len()
                ));
            }
            parts.join(", ")
        }
    };
    let hint = if !summary.preview_gated.is_empty() {
        " Use --preview to run them."
    } else {
        ""
    };
    eprintln!(
        "warning: --strict={scope_name}: {total} skipped cops violate {scope_name} ({detail}).{hint}"
    );
}

/// Batch corpus check: lint each subdirectory of `corpus_dir` as a separate repo.
/// Outputs JSON with per-repo offense counts (deduplicated by path+line+cop).
fn collect_corpus_check_results(
    corpus_dir: &std::path::Path,
    config: &config::ResolvedConfig,
    registry: &cop::registry::CopRegistry,
    args: &cli::Args,
    tier_map: &cop::tiers::TierMap,
    allowlist: &cop::autocorrect_allowlist::AutocorrectAllowlist,
) -> Result<HashMap<String, usize>> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Initialize schema once (same config for all repos)
    schema::init(config.config_dir());

    // Precompute cop filters and configs once
    let cop_filters = config.build_cop_filters(registry, tier_map, args.preview);
    let base_configs = config.precompute_cop_configs(registry);
    let has_dir_overrides = config.has_dir_overrides();

    // List subdirectories (each is a corpus repo)
    let mut repos: Vec<_> = std::fs::read_dir(corpus_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|ft| ft.is_dir()))
        .map(|e| e.path())
        .collect();
    repos.sort();

    let total = repos.len();
    let done = AtomicUsize::new(0);

    // Process repos in parallel
    let results: Vec<(String, usize)> = repos
        .par_iter()
        .map(|repo_path| {
            let repo_id = repo_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            // Discover .rb files in this repo
            let discovered = match fs::discover_files(std::slice::from_ref(repo_path), config) {
                Ok(d) => d,
                Err(_) => return (repo_id, 0),
            };

            // Lint each file and collect diagnostics
            let mut seen = HashSet::new();
            for path in &discovered.files {
                let source = match parse::source::SourceFile::from_path(path) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let (diags, _, _) = linter::lint_source_inner(
                    &source,
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
                // Deduplicate by (path, line, cop_name) to match corpus oracle
                for d in &diags {
                    seen.insert((d.path.clone(), d.location.line, d.cop_name.clone()));
                }
            }

            let count = seen.len();
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 100 == 0 {
                eprintln!("  [{n}/{total}]...");
            }
            (repo_id, count)
        })
        .collect();

    Ok(results.into_iter().collect())
}

/// Batch corpus check: lint each subdirectory of `corpus_dir` as a separate repo.
/// Outputs JSON with per-repo offense counts (deduplicated by path+line+cop).
fn run_corpus_check(
    corpus_dir: &std::path::Path,
    config: &config::ResolvedConfig,
    registry: &cop::registry::CopRegistry,
    args: &cli::Args,
    tier_map: &cop::tiers::TierMap,
    allowlist: &cop::autocorrect_allowlist::AutocorrectAllowlist,
) -> Result<i32> {
    let repos_map =
        collect_corpus_check_results(corpus_dir, config, registry, args, tier_map, allowlist)?;
    let output = serde_json::json!({ "repos": repos_map });
    println!("{}", serde_json::to_string(&output)?);

    Ok(0)
}

/// Run the linter. Returns the exit code: 0 = clean, 1 = offenses, 2 = strict failure, 3 = error.
pub fn run(args: Args) -> Result<i32> {
    // Warn about unsupported --require flag
    if !args.require_libs.is_empty() {
        eprintln!("warning: --require is not supported; use `require:` in .rubocop.yml instead");
    }

    // Validate --fail-level early
    let fail_level = diagnostic::Severity::from_str(&args.fail_level).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid --fail-level '{}'. Expected: convention, warning, error, fatal (or C, W, E, F)",
            args.fail_level
        )
    })?;

    // Validate --strict early
    if let Some(ref val) = args.strict {
        if args.strict_scope().is_none() {
            anyhow::bail!(
                "invalid --strict value '{val}'. Expected: coverage, implemented-only, all"
            );
        }
    }

    let target_dir = args.paths.first().map(|p| {
        if p.is_file() {
            p.parent().unwrap_or(p)
        } else {
            p.as_path()
        }
    });

    let registry = CopRegistry::default_registry();
    let tier_map = TierMap::load();
    let allowlist = cop::autocorrect_allowlist::AutocorrectAllowlist::load();

    // --list-cops: print all registered cop names and exit (no config needed)
    if args.list_cops {
        let mut names: Vec<&str> = registry.cops().iter().map(|c| c.name()).collect();
        names.sort();
        for name in names {
            println!("{name}");
        }
        return Ok(0);
    }

    // --list-autocorrectable-cops: print cops that support autocorrect and exit
    if args.list_autocorrectable_cops {
        let mut names: Vec<&str> = registry
            .cops()
            .iter()
            .filter(|c| c.supports_autocorrect())
            .map(|c| c.name())
            .collect();
        names.sort();
        for name in names {
            println!("{name}");
        }
        return Ok(0);
    }

    // --rules: list all cops with tier, implementation status, baseline presence
    if args.rules {
        let rule_list = rules::build_rules(&registry, &tier_map, args.tier.as_deref());
        if args.format == "json" {
            rules::print_json(&rule_list);
        } else {
            rules::print_table(&rule_list);
        }
        return Ok(0);
    }

    // --cache-clear: remove result cache directory and exit
    if args.cache_clear {
        match cache::clear_cache() {
            Ok(()) => {
                eprintln!("Result cache cleared.");
                return Ok(0);
            }
            Err(e) => {
                anyhow::bail!("Failed to clear result cache: {e}");
            }
        }
    }

    // --init: resolve gem paths and write lockfile
    if args.init {
        let config_start = std::time::Instant::now();
        let _config = load_config(args.config.as_deref(), target_dir, None)?;
        let config_elapsed = config_start.elapsed();

        let gem_paths = config::gem_path::drain_resolved_paths();
        // Use target_dir (CLI path), not config_dir(), so read_lock() can find
        // the lockfile without loading config first (chicken-and-egg).
        let lock_dir = target_dir.unwrap_or(std::path::Path::new("."));
        config::lockfile::write_lock(&gem_paths, lock_dir)?;

        let lockfile_location = config::lockfile::lockfile_path(lock_dir);
        eprintln!(
            "Created lockfile ({} gems cached in {config_elapsed:.0?})",
            gem_paths.len()
        );
        eprintln!("  location: {}", lockfile_location.display());
        for (name, path) in &gem_paths {
            eprintln!("  {name}: {}", path.display());
        }
        return Ok(0);
    }

    // Determine whether to use lockfile:
    // --no-lock, --rubocop-only, --list-target-files, --force-default-config, and --stdin
    // bypass the lockfile requirement
    let use_cache = !args.no_cache
        && !args.rubocop_only
        && !args.list_target_files
        && !args.force_default_config
        && args.stdin.is_none();

    // Load config — use lockfile if available
    let config_start = std::time::Instant::now();
    let config = if args.force_default_config {
        let mut cfg = config::ResolvedConfig::empty();
        // When --only targets plugin-department cops (RSpec, Rails, etc.),
        // register those departments so the cops aren't silently disabled.
        // Without this, --force-default-config creates an empty config with
        // no require_departments, causing all plugin cops to be skipped.
        cfg.register_departments_from_only(&args.only);
        cfg
    } else if use_cache {
        // Try to find config dir for lockfile lookup
        let lock_dir = target_dir.unwrap_or(std::path::Path::new("."));
        match config::lockfile::read_lock(lock_dir) {
            Ok(lock) => {
                config::lockfile::check_freshness(&lock, lock_dir)?;
                if args.debug {
                    eprintln!("debug: using lockfile ({} cached gems)", lock.gems.len());
                }
                load_config(args.config.as_deref(), target_dir, Some(&lock.gems))?
            }
            Err(e) => {
                // If lockfile is missing, fail with helpful message
                anyhow::bail!("{e}");
            }
        }
    } else {
        load_config(args.config.as_deref(), target_dir, None)?
    };
    let config_elapsed = config_start.elapsed();

    if args.debug {
        eprintln!("debug: config loading total: {config_elapsed:.0?}");

        if let Some(dir) = config.config_dir() {
            eprintln!("debug: config loaded from: {}", dir.display());
        } else {
            eprintln!("debug: no config file found");
        }
        eprintln!("debug: global excludes: {:?}", config.global_excludes());
    }

    // --rubocop-only: print uncovered cops and exit
    if args.rubocop_only {
        let covered: HashSet<&str> = registry.cops().iter().map(|c| c.name()).collect();
        let mut remaining: Vec<String> = config
            .enabled_cop_names()
            .into_iter()
            .filter(|name| !covered.contains(name.as_str()))
            .collect();
        remaining.sort();
        println!("{}", remaining.join(","));
        return Ok(0);
    }

    // --migrate: config analysis, no linting
    if args.migrate {
        let report = migrate::build_report(&config, &registry, &tier_map);
        if args.format == "json" {
            migrate::print_json(&report);
        } else {
            migrate::print_text(&report, &args);
        }
        return Ok(0);
    }

    // --doctor: debug/support output
    if args.doctor {
        doctor::run_doctor(&config, &registry, &tier_map, target_dir);
        return Ok(0);
    }

    // --verify: compare nitrocop output against RuboCop
    if args.verify {
        let result = verify::run_verify(&args, &config, &registry, &tier_map, &allowlist)?;
        if args.format == "json" {
            verify::print_json(&result);
        } else {
            verify::print_text(&result);
        }
        return Ok(if result.false_positives + result.false_negatives > 0 {
            1
        } else {
            0
        });
    }

    // --corpus-check: batch lint each subdirectory as a separate repo
    if let Some(ref corpus_dir) = args.corpus_check {
        return run_corpus_check(corpus_dir, &config, &registry, &args, &tier_map, &allowlist);
    }

    if args.debug {
        eprintln!("debug: autocorrect mode: {:?}", args.autocorrect_mode());
    }

    // --stdin + autocorrect: not yet supported
    if args.stdin.is_some() && args.autocorrect_mode() != cli::AutocorrectMode::Off {
        eprintln!("warning: autocorrect is not supported with --stdin, ignoring");
    }

    // --stdin: read from stdin and lint a single file
    if let Some(ref display_path) = args.stdin {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        let source = SourceFile::from_string(display_path.clone(), input);
        let result = lint_source(&source, &config, &registry, &args, &tier_map, &allowlist);
        let mut formatter = create_formatter(&args.format);
        formatter.set_skip_summary(result.skip_summary.clone());
        formatter.print(&result.diagnostics, std::slice::from_ref(display_path));
        let has_lint_failure = result.diagnostics.iter().any(|d| d.severity >= fail_level);
        let strict_failure = args.strict_scope().is_some_and(|scope| {
            let fails = strict_check_fails(scope, &result.skip_summary);
            if fails {
                print_strict_warning(scope, &result.skip_summary);
            }
            fails
        });
        return if has_lint_failure {
            Ok(1)
        } else if strict_failure {
            Ok(2)
        } else {
            Ok(0)
        };
    }

    let discovered = discover_files(&args.paths, &config)?;

    // Build cop filters once (reused for --list-target-files, file filtering, and linting).
    let mut cop_filters = config.build_cop_filters(&registry, &tier_map, args.preview);

    // Set scan root so Include patterns like `db/**/*.rb` resolve relative to
    // the target directory, not just the config file's dir or process CWD.
    if let Some(target) = args.paths.first() {
        let target_path = std::path::Path::new(target);
        if target_path.is_dir() {
            if let Ok(abs) = target_path.canonicalize() {
                cop_filters.set_scan_root(abs);
            }
        }
    }

    // Filter globally-excluded files so they are not counted in "N files inspected"
    // or given progress dots.  Explicit CLI files bypass AllCops.Exclude unless
    // --force-exclusion is set (matching RuboCop behavior).
    let effective_files: Vec<PathBuf> = discovered
        .files
        .iter()
        .filter(|file| {
            if cop_filters.is_globally_excluded(file) {
                let is_explicit = discovered.explicit.contains(file.as_path())
                    || file
                        .canonicalize()
                        .ok()
                        .is_some_and(|c| discovered.explicit.contains(&c));
                if args.force_exclusion || !is_explicit {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();

    // --list-target-files (-L): print files that would be linted, then exit
    if args.list_target_files {
        for file in &effective_files {
            println!("{}", file.display());
        }
        return Ok(0);
    }

    if args.debug {
        eprintln!("debug: {} files to lint", effective_files.len());
        eprintln!("debug: {} cops registered", registry.len());
    }

    let effective_discovered = DiscoveredFiles {
        files: effective_files,
        explicit: discovered.explicit,
    };

    let result = run_linter(
        &effective_discovered,
        &config,
        &registry,
        &args,
        &tier_map,
        &allowlist,
    );

    // Print skip summary to stderr unless suppressed
    if !args.quiet_skips && !result.skip_summary.is_empty() {
        let s = &result.skip_summary;
        let mut parts = Vec::new();
        if !s.preview_gated.is_empty() {
            parts.push(format!("{} preview-gated", s.preview_gated.len()));
        }
        if !s.unimplemented.is_empty() {
            parts.push(format!("{} unimplemented", s.unimplemented.len()));
        }
        if !s.outside_baseline.is_empty() {
            parts.push(format!("{} outside baseline", s.outside_baseline.len()));
        }
        eprintln!(
            "Skipped {} cops ({}). Run `nitrocop --migrate` for details.",
            s.total(),
            parts.join(", "),
        );
    }

    let skip_summary = result.skip_summary.clone();
    let mut formatter = create_formatter(&args.format);
    formatter.set_skip_summary(result.skip_summary);
    formatter.print(&result.diagnostics, &effective_discovered.files);

    let has_lint_failure = result.diagnostics.iter().any(|d| d.severity >= fail_level);
    let strict_failure = args.strict_scope().is_some_and(|scope| {
        let fails = strict_check_fails(scope, &skip_summary);
        if fails {
            print_strict_warning(scope, &skip_summary);
        }
        fails
    });

    if has_lint_failure {
        Ok(1)
    } else if strict_failure {
        Ok(2)
    } else {
        Ok(0)
    }
}
