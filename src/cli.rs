use std::path::PathBuf;

use clap::Parser;

/// Known flag names that users commonly pass as bare positional arguments
/// (e.g. `nitrocop migrate` instead of `nitrocop --migrate`).
const FLAG_LIKE_NAMES: &[&str] = &["migrate", "init", "doctor", "rules", "verify"];

/// Clap value parser that rejects bare command names with a helpful hint.
fn parse_path(s: &str) -> Result<PathBuf, String> {
    if FLAG_LIKE_NAMES.contains(&s) {
        return Err(format!(
            "unknown path '{s}'. Did you mean `nitrocop --{s}`?"
        ));
    }
    Ok(PathBuf::from(s))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutocorrectMode {
    Off,
    /// `-a` / `--autocorrect`: safe corrections only.
    Safe,
    /// `-A` / `--autocorrect-all`: all corrections including unsafe.
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrictScope {
    /// Preview-gated cops cause failure (nitrocop implements them but didn't run them).
    Coverage,
    /// Same as coverage (preview-gated only; unimplemented/outside-baseline are ignored).
    ImplementedOnly,
    /// Any skipped cop (preview-gated + unimplemented + outside-baseline) causes failure.
    All,
}

#[derive(Parser, Debug)]
#[command(name = "nitrocop", version, about = "A fast Ruby linter")]
pub struct Args {
    /// Files or directories to lint
    #[arg(default_value = ".", value_parser = parse_path)]
    pub paths: Vec<PathBuf>,

    /// Path to configuration file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Output format
    #[arg(short, long, default_value = "progress", value_parser = ["progress", "text", "json", "github", "pacman", "quiet", "files", "emacs", "simple"])]
    pub format: String,

    /// Run only the specified cops (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub only: Vec<String>,

    /// Exclude the specified cops (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub except: Vec<String>,

    /// Disable color output
    #[arg(long)]
    pub no_color: bool,

    /// Enable debug output
    #[arg(long)]
    pub debug: bool,

    /// Print comma-separated list of cops not covered by nitrocop, then exit
    #[arg(long)]
    pub rubocop_only: bool,

    /// List all registered cop names, one per line, then exit
    #[arg(long)]
    pub list_cops: bool,

    /// List cops that support autocorrect, one per line, then exit
    #[arg(long)]
    pub list_autocorrectable_cops: bool,

    /// Analyze config and report cop coverage (no linting), then exit
    #[arg(long)]
    pub migrate: bool,

    /// Print debug/support information (baseline versions, config, gem mismatches), then exit
    #[arg(long)]
    pub doctor: bool,

    /// List all cops with tier, implementation status, and baseline presence, then exit
    #[arg(long)]
    pub rules: bool,

    /// Filter --rules output by tier
    #[arg(long, value_name = "TIER", value_parser = ["stable", "preview"])]
    pub tier: Option<String>,

    /// Read source from stdin, use PATH for display and config matching
    #[arg(long, value_name = "PATH")]
    pub stdin: Option<PathBuf>,

    /// Resolve gem paths and write lockfile to cache directory, then exit
    #[arg(long)]
    pub init: bool,

    /// Bypass the lockfile and disable result-cache reads/writes
    #[arg(long)]
    pub no_cache: bool,

    /// Enable/disable file-level result caching [default: true]
    #[arg(long, default_value = "true", hide_default_value = true)]
    pub cache: String,

    /// Clear the result cache and exit
    #[arg(long)]
    pub cache_clear: bool,

    /// Minimum severity for a non-zero exit code (convention, warning, error, fatal, or C/W/E/F)
    #[arg(long, value_name = "SEVERITY", default_value = "convention")]
    pub fail_level: String,

    /// Stop after first file with offenses
    #[arg(short = 'F', long)]
    pub fail_fast: bool,

    /// Apply AllCops.Exclude to explicitly-passed files (by default, explicit files bypass exclusion)
    #[arg(long)]
    pub force_exclusion: bool,

    /// Print files that would be linted, then exit
    #[arg(short = 'L', long)]
    pub list_target_files: bool,

    /// Display cop names in offense output (accepted for RuboCop compatibility; always enabled)
    #[arg(short = 'D', long)]
    pub display_cop_names: bool,

    /// Use parallel processing (accepted for RuboCop compatibility; always enabled)
    #[arg(short = 'P', long)]
    pub parallel: bool,

    /// Load additional Ruby files (accepted for RuboCop compatibility; ignored)
    #[arg(short = 'r', long = "require")]
    pub require_libs: Vec<String>,

    /// Ignore all `# rubocop:disable` inline comments
    #[arg(long)]
    pub ignore_disable_comments: bool,

    /// Ignore all config files and use built-in defaults only
    #[arg(long)]
    pub force_default_config: bool,

    /// Autocorrect offenses (safe cops only)
    #[arg(short = 'a', long = "autocorrect")]
    pub autocorrect: bool,

    /// Autocorrect offenses (all cops, including unsafe)
    #[arg(short = 'A', long = "autocorrect-all")]
    pub autocorrect_all: bool,

    /// Enable preview-tier cops (unstable, may have false positives)
    #[arg(long)]
    pub preview: bool,

    /// Suppress the skip summary notice at the end of a run
    #[arg(long)]
    pub quiet_skips: bool,

    /// Fail with exit code 2 if skipped cops violate the strict scope
    #[arg(long, value_name = "SCOPE", default_missing_value = "coverage", num_args = 0..=1)]
    pub strict: Option<String>,

    /// Compare nitrocop output against RuboCop (requires Ruby), then exit
    #[arg(long)]
    pub verify: bool,

    /// Override RuboCop command for --verify (default: "bundle exec rubocop")
    #[arg(long, value_name = "CMD", default_value = "bundle exec rubocop")]
    pub rubocop_cmd: String,

    /// Batch corpus check: lint each subdirectory as a separate repo, output per-repo JSON
    #[arg(long, value_name = "DIR")]
    pub corpus_check: Option<PathBuf>,
}

impl Args {
    /// Resolve the autocorrect mode from CLI flags.
    /// `-A` takes precedence over `-a` (matching RuboCop behavior).
    pub fn autocorrect_mode(&self) -> AutocorrectMode {
        if self.autocorrect_all {
            AutocorrectMode::All
        } else if self.autocorrect {
            AutocorrectMode::Safe
        } else {
            AutocorrectMode::Off
        }
    }

    /// Parse the `--strict` value into a `StrictScope`.
    /// Returns `None` if `--strict` was not passed or the value is invalid.
    pub fn strict_scope(&self) -> Option<StrictScope> {
        self.strict.as_deref().and_then(|s| match s {
            "coverage" => Some(StrictScope::Coverage),
            "implemented-only" => Some(StrictScope::ImplementedOnly),
            "all" => Some(StrictScope::All),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with_strict(val: Option<&str>) -> Args {
        Args {
            paths: vec![],
            config: None,
            format: "text".to_string(),
            only: vec![],
            except: vec![],
            no_color: false,
            debug: false,
            rubocop_only: false,
            list_cops: false,
            list_autocorrectable_cops: false,
            migrate: false,
            doctor: false,
            rules: false,
            tier: None,
            stdin: None,
            init: false,
            no_cache: false,
            cache: "true".to_string(),
            cache_clear: false,
            fail_level: "convention".to_string(),
            fail_fast: false,
            force_exclusion: false,
            list_target_files: false,
            display_cop_names: false,
            parallel: false,
            require_libs: vec![],
            ignore_disable_comments: false,
            force_default_config: false,
            autocorrect: false,
            autocorrect_all: false,
            preview: false,
            quiet_skips: false,
            strict: val.map(|s| s.to_string()),
            verify: false,
            rubocop_cmd: "bundle exec rubocop".to_string(),
            corpus_check: None,
        }
    }

    #[test]
    fn strict_scope_parsing() {
        assert_eq!(args_with_strict(None).strict_scope(), None);
        assert_eq!(
            args_with_strict(Some("coverage")).strict_scope(),
            Some(StrictScope::Coverage)
        );
        assert_eq!(
            args_with_strict(Some("implemented-only")).strict_scope(),
            Some(StrictScope::ImplementedOnly)
        );
        assert_eq!(
            args_with_strict(Some("all")).strict_scope(),
            Some(StrictScope::All)
        );
    }

    #[test]
    fn strict_scope_invalid() {
        assert_eq!(args_with_strict(Some("bogus")).strict_scope(), None);
        assert_eq!(args_with_strict(Some("")).strict_scope(), None);
    }

    #[test]
    fn parse_path_rejects_flag_names() {
        for name in FLAG_LIKE_NAMES {
            let result = parse_path(name);
            assert!(result.is_err(), "'{name}' should be rejected");
            let err = result.unwrap_err();
            assert!(
                err.contains(&format!("--{name}")),
                "error for '{name}' should suggest --{name}, got: {err}"
            );
        }
    }

    #[test]
    fn parse_path_accepts_normal_paths() {
        assert_eq!(parse_path(".").unwrap(), PathBuf::from("."));
        assert_eq!(parse_path("src").unwrap(), PathBuf::from("src"));
        assert_eq!(
            parse_path("lib/foo.rb").unwrap(),
            PathBuf::from("lib/foo.rb")
        );
    }

    #[test]
    fn clap_rejects_bare_migrate() {
        let result = Args::try_parse_from(["nitrocop", "migrate"]);
        assert!(result.is_err(), "bare 'migrate' should fail clap parsing");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("--migrate"),
            "clap error should suggest --migrate, got: {err}"
        );
    }

    #[test]
    fn clap_accepts_flag_migrate() {
        let result = Args::try_parse_from(["nitrocop", "--migrate", "."]);
        assert!(
            result.is_ok(),
            "--migrate should parse: {}",
            result.unwrap_err()
        );
        assert!(result.unwrap().migrate);
    }
}
