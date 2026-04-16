pub mod autocorrect_allowlist;
pub mod bundler;
pub mod factory_bot;
pub mod gemspec;
pub mod layout;
pub mod lint;
pub mod metrics;
pub mod migration;
pub mod naming;
pub mod performance;
pub mod rails;
pub mod rake;
pub mod registry;
pub mod rspec;
pub mod rspec_rails;
pub mod security;
pub mod shared;
pub mod style;
pub mod tiers;
pub mod variable_force;
pub mod walker;

use std::collections::HashMap;

use serde::Serialize;

use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;

/// Tri-state for cop Enabled field, matching RuboCop semantics.
///
/// - `True` / `False` — explicitly set in config
/// - `Pending` — set by plugin defaults (e.g. `rubocop-rails`); disabled
///   unless `AllCops.NewCops: enable`
/// - `Unset` — no explicit setting; inherits from defaults (enabled unless
///   `AllCops.DisabledByDefault: true`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub enum EnabledState {
    True,
    False,
    Pending,
    #[default]
    Unset,
}

/// Per-cop configuration extracted from .rubocop.yml.
#[derive(Debug, Clone, Serialize)]
pub struct CopConfig {
    pub enabled: EnabledState,
    pub severity: Option<Severity>,
    pub exclude: Vec<String>,
    pub include: Vec<String>,
    pub options: HashMap<String, serde_yml::Value>,
}

impl Default for CopConfig {
    fn default() -> Self {
        Self {
            enabled: EnabledState::Unset,
            severity: None,
            exclude: Vec::new(),
            include: Vec::new(),
            options: HashMap::new(),
        }
    }
}

impl CopConfig {
    /// Get a string option with a default value.
    pub fn get_str<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.options
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
    }

    /// Get an unsigned integer option with a default value.
    pub fn get_usize(&self, key: &str, default: usize) -> usize {
        self.options
            .get(key)
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(default)
    }

    /// Get a boolean option with a default value.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        self.options
            .get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    }

    /// Get a string array option. Returns None if the key is absent.
    pub fn get_string_array(&self, key: &str) -> Option<Vec<String>> {
        self.options.get(key).and_then(|v| {
            v.as_sequence().map(|seq| {
                seq.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
        })
    }

    /// Get a string→string hash option. Returns None if the key is absent.
    /// Integer and float values are automatically converted to strings (e.g.,
    /// `Methods: inject: 2` yields `{"inject": "2"}`).
    pub fn get_string_hash(&self, key: &str) -> Option<HashMap<String, String>> {
        self.options.get(key).and_then(|v| {
            v.as_mapping().map(|m| {
                m.iter()
                    .filter_map(|(k, v)| {
                        let ks = k.as_str()?;
                        // Try string first, then integer/float for numeric YAML values
                        let vs = if let Some(s) = v.as_str() {
                            s.to_string()
                        } else if let Some(n) = v.as_u64() {
                            n.to_string()
                        } else if let Some(n) = v.as_i64() {
                            n.to_string()
                        } else if let Some(n) = v.as_f64() {
                            n.to_string()
                        } else {
                            return None;
                        };
                        Some((ks.to_string(), vs))
                    })
                    .collect()
            })
        })
    }

    /// Get all string values from a config key that is either:
    /// - A flat array of strings
    /// - A hash of group_name → array of strings (like DebuggerMethods)
    ///
    /// Returns the flattened list of all strings. None if the key is absent.
    pub fn get_flat_string_values(&self, key: &str) -> Option<Vec<String>> {
        let v = self.options.get(key)?;
        let mut result = Vec::new();
        if let Some(mapping) = v.as_mapping() {
            for (_, group_val) in mapping.iter() {
                if let Some(seq) = group_val.as_sequence() {
                    for item in seq {
                        if let Some(s) = item.as_str() {
                            result.push(s.to_string());
                        }
                    }
                }
                if let Some(s) = group_val.as_str() {
                    result.push(s.to_string());
                }
            }
        }
        if let Some(seq) = v.as_sequence() {
            for item in seq {
                if let Some(s) = item.as_str() {
                    result.push(s.to_string());
                }
            }
        }
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Whether the project has a known Rails version AND `railties` is in the lockfile.
    /// Non-Rails projects won't have `railties` in their lockfile.
    ///
    /// Rails cops that use `minimum_target_rails_version` in RuboCop should call this
    /// to skip non-Rails projects — matching RuboCop's `requires_gem 'railties'` gate.
    pub fn has_target_rails_version(&self) -> bool {
        self.options.contains_key("TargetRailsVersion") && self.railties_in_lockfile()
    }

    /// Get the target Rails version (defaults to 5.0 if set but unparseable).
    /// Returns `None` if `TargetRailsVersion` is not present (non-Rails project).
    pub fn target_rails_version(&self) -> Option<f64> {
        self.options.get("TargetRailsVersion").map(|v| {
            v.as_f64()
                .or_else(|| v.as_u64().map(|u| u as f64))
                .unwrap_or(5.0)
        })
    }

    /// Check that the target Rails version meets a minimum requirement.
    /// Returns `false` if `TargetRailsVersion` is not set (non-Rails project) or
    /// is below the specified minimum.
    ///
    /// Also returns `false` if `railties` was not found in the project's Gemfile.lock,
    /// matching RuboCop 1.84+'s `requires_gem 'railties'` gate. This ensures cops
    /// with `minimum_target_rails_version` are disabled when the project doesn't
    /// actually use Rails, even if `TargetRailsVersion` is set in config.
    pub fn rails_version_at_least(&self, minimum: f64) -> bool {
        // RuboCop 1.84+ requires_gem check: railties must be in lockfile
        if !self.railties_in_lockfile() {
            return false;
        }
        self.target_rails_version().is_some_and(|v| v >= minimum)
    }

    /// Get the `rack` gem version from the project's Gemfile.lock.
    /// Returns `None` if `rack` is not in the lockfile.
    /// Used by `HttpStatusNameConsistency` cops that require `rack >= 3.1.0`.
    pub fn rack_version(&self) -> Option<f64> {
        self.options
            .get("__RackVersion")
            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
    }

    /// Get the actual `railties` gem version from the project's Gemfile.lock.
    /// Returns `None` if `railties` was not found in the lockfile.
    pub fn railties_version(&self) -> Option<f64> {
        self.options
            .get("__RailtiesVersion")
            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
    }

    /// Whether `railties` was found in the project's Gemfile.lock.
    /// Mirrors RuboCop 1.84+'s `requires_gem 'railties'` API.
    fn railties_in_lockfile(&self) -> bool {
        self.options
            .get("__RailtiesInLockfile")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Whether the cop itself is considered safe (default: true).
    pub fn is_safe(&self) -> bool {
        self.get_bool("Safe", true)
    }

    /// Whether the cop's autocorrect is considered safe (default: true).
    pub fn is_safe_autocorrect(&self) -> bool {
        self.get_bool("SafeAutoCorrect", true)
    }

    /// Read the `AutoCorrect` config key. RuboCop supports both boolean and
    /// string values ("always", "contextual", "disabled"). Default: "always".
    pub fn autocorrect_setting(&self) -> &str {
        if let Some(v) = self.options.get("AutoCorrect") {
            // Boolean true -> "always", false -> "disabled"
            if let Some(b) = v.as_bool() {
                return if b { "always" } else { "disabled" };
            }
            if let Some(s) = v.as_str() {
                return s;
            }
        }
        "always"
    }

    /// Whether this cop should autocorrect given the current CLI mode.
    pub fn should_autocorrect(&self, mode: crate::cli::AutocorrectMode) -> bool {
        use crate::cli::AutocorrectMode;
        match mode {
            AutocorrectMode::Off => false,
            AutocorrectMode::Safe => {
                self.is_safe()
                    && self.is_safe_autocorrect()
                    && self.autocorrect_setting() != "disabled"
            }
            AutocorrectMode::All => self.autocorrect_setting() != "disabled",
        }
    }
}

/// A lint rule. Implementations must be Send + Sync so they can be shared
/// across rayon worker threads.
pub trait Cop: Send + Sync {
    /// The fully-qualified cop name, e.g. "Style/FrozenStringLiteralComment".
    fn name(&self) -> &'static str;

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    /// Default Include patterns for this cop. If non-empty, the cop only runs
    /// on files matching at least one pattern. User config overrides these.
    fn default_include(&self) -> &'static [&'static str] {
        &[]
    }

    /// Default Exclude patterns for this cop. If non-empty, the cop is skipped
    /// on files matching any pattern. User config overrides these.
    fn default_exclude(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether the cop is enabled by default.
    ///
    /// Matches the `Enabled` value from vendor `config/default.yml`.
    /// Cops that have `Enabled: false` in the vendor config should override
    /// this to return `false`. This ensures they stay disabled even when no
    /// `.rubocop.yml` is present (and vendor defaults are not loaded).
    fn default_enabled(&self) -> bool {
        true
    }

    /// Create a Diagnostic with standard fields filled in.
    fn diagnostic(
        &self,
        source: &SourceFile,
        line: usize,
        column: usize,
        message: String,
    ) -> Diagnostic {
        Diagnostic {
            path: source.path_str().to_string(),
            location: Location { line, column },
            severity: self.default_severity(),
            cop_name: self.name().to_string(),
            message,
            corrected: false,
        }
    }

    /// Whether this cop can produce autocorrections.
    fn supports_autocorrect(&self) -> bool {
        false
    }

    /// Whether this cop's autocorrections are safe (won't change semantics).
    fn safe_autocorrect(&self) -> bool {
        true
    }

    /// Line-based check — runs before AST traversal.
    #[allow(unused_variables)]
    fn check_lines(
        &self,
        source: &SourceFile,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
    }

    /// Source-based check — runs once per file with full parse context and CodeMap.
    ///
    /// Use this for cops that scan raw source bytes while needing to skip
    /// non-code regions (strings, comments, regexps).
    #[allow(unused_variables)]
    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
    }

    /// Node types this cop handles in `check_node`.
    /// Return a non-empty slice to opt into selective dispatch (only called for
    /// matching node types). Return `&[]` to be called for every node (default).
    fn interested_node_types(&self) -> &'static [u8] {
        &[]
    }

    /// Node-based check — called for every AST node during traversal.
    #[allow(unused_variables)]
    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
    }

    /// Return `Some(self)` if this cop consumes VariableForce analysis.
    /// Override this to opt into the shared variable dataflow engine instead
    /// of implementing your own AST visitor for variable tracking.
    fn as_variable_force_consumer(&self) -> Option<&dyn variable_force::VariableForceConsumer> {
        None
    }
}

/// Generate standard offense/no_offense fixture tests for a cop.
///
/// Usage:
/// ```ignore
/// #[cfg(test)]
/// mod tests {
///     use super::*;
///     crate::cop_fixture_tests!(CopStruct, "cops/dept/cop_name");
///     // additional tests...
/// }
/// ```
#[macro_export]
macro_rules! cop_fixture_tests {
    ($cop:expr, $path:literal) => {
        #[test]
        fn offense_fixture() {
            $crate::testutil::assert_cop_offenses_full(
                &$cop,
                include_bytes!(concat!("../../../tests/fixtures/", $path, "/offense.rb")),
            );
        }

        #[test]
        fn no_offense_fixture() {
            $crate::testutil::assert_cop_no_offenses_full(
                &$cop,
                include_bytes!(concat!("../../../tests/fixtures/", $path, "/no_offense.rb")),
            );
        }

        #[test]
        fn no_orphaned_fixtures() {
            $crate::testutil::assert_no_orphaned_fixtures($path, file!());
        }
    };
}

/// Generate scenario-based fixture tests for cops that need multiple offense files.
///
/// Use when a cop fires at most once per file (e.g., InitialIndentation,
/// LeadingEmptyLines) or when offenses can't be annotated with `^` markers
/// (e.g., TrailingEmptyLines). Each scenario file is a separate `.rb` file
/// in an `offense/` directory.
///
/// Usage:
/// ```ignore
/// #[cfg(test)]
/// mod tests {
///     use super::*;
///     crate::cop_scenario_fixture_tests!(
///         CopStruct, "cops/dept/cop_name",
///         scenario_one = "scenario_one.rb",
///         scenario_two = "scenario_two.rb",
///     );
/// }
/// ```
#[macro_export]
macro_rules! cop_scenario_fixture_tests {
    ($cop:expr, $path:literal, $($name:ident = $file:literal),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                $crate::testutil::assert_cop_offenses_full(
                    &$cop,
                    include_bytes!(concat!("../../../tests/fixtures/", $path, "/offense/", $file)),
                );
            }
        )+

        #[test]
        fn no_offense_fixture() {
            $crate::testutil::assert_cop_no_offenses_full(
                &$cop,
                include_bytes!(concat!("../../../tests/fixtures/", $path, "/no_offense.rb")),
            );
        }
    };
}

/// Generate variant fixture tests for a cop.
///
/// For each variant name, generates tests that load `offense.<variant>.rb` and/or
/// `no_offense.<variant>.rb` from the fixture directory. Each fixture must start
/// with a `# nitrocop-config:` directive specifying the variant config.
///
/// Unlike `cop_fixture_tests!`, both offense and no_offense files are optional —
/// only existing files are tested. This allows pre-populating just the examples
/// available from the corpus.
///
/// Usage:
/// ```ignore
/// #[cfg(test)]
/// mod tests {
///     use super::*;
///     crate::cop_fixture_tests!(CopStruct, "cops/dept/cop_name");
///     crate::cop_variant_fixture_tests!(CopStruct, "cops/dept/cop_name",
///         semantic, always_braces, braces_for_chaining,
///     );
/// }
/// ```
#[macro_export]
macro_rules! cop_variant_fixture_tests {
    ($cop:expr, $path:literal, $($variant:ident),+ $(,)?) => {
        $(
            $crate::cop_variant_fixture_tests!(@maybe_offense $cop, $path, $variant);
            $crate::cop_variant_fixture_tests!(@maybe_no_offense $cop, $path, $variant);
        )+
    };

    // Internal: generate offense test if fixture file exists
    (@maybe_offense $cop:expr, $path:literal, $variant:ident) => {
        paste::paste! {
            #[test]
            fn [<offense_ $variant _variant_fixture>]() {
                let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
                let fixture_path = manifest
                    .join("tests/fixtures")
                    .join($path)
                    .join(concat!("offense.", stringify!($variant), ".rb"));
                if !fixture_path.exists() {
                    return; // skip — fixture not present
                }
                let bytes = std::fs::read(&fixture_path)
                    .unwrap_or_else(|e| panic!("Failed to read {}: {e}", fixture_path.display()));
                let (config, source) = $crate::testutil::parse_variant_fixture(&bytes);
                $crate::testutil::assert_cop_offenses_full_with_config(&$cop, &source, config);
            }
        }
    };

    // Internal: generate no_offense test if fixture file exists
    (@maybe_no_offense $cop:expr, $path:literal, $variant:ident) => {
        paste::paste! {
            #[test]
            fn [<no_offense_ $variant _variant_fixture>]() {
                let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
                let fixture_path = manifest
                    .join("tests/fixtures")
                    .join($path)
                    .join(concat!("no_offense.", stringify!($variant), ".rb"));
                if !fixture_path.exists() {
                    return; // skip — fixture not present
                }
                let bytes = std::fs::read(&fixture_path)
                    .unwrap_or_else(|e| panic!("Failed to read {}: {e}", fixture_path.display()));
                let (config, source) = $crate::testutil::parse_variant_fixture(&bytes);
                $crate::testutil::assert_cop_no_offenses_full_with_config(&$cop, &source, config);
            }
        }
    };
}

/// Generate autocorrect fixture tests for a cop.
///
/// If `testdata/<path>/corrected.rb` exists, this generates a test that:
/// 1. Strips annotations from `offense.rb` to get the input source
/// 2. Runs the cop with corrections enabled
/// 3. Applies corrections to produce corrected source
/// 4. Asserts the output matches `corrected.rb` byte-for-byte
///
/// Usage:
/// ```ignore
/// #[cfg(test)]
/// mod tests {
///     use super::*;
///     crate::cop_fixture_tests!(CopStruct, "cops/dept/cop_name");
///     crate::cop_autocorrect_fixture_tests!(CopStruct, "cops/dept/cop_name");
/// }
/// ```
#[macro_export]
macro_rules! cop_autocorrect_fixture_tests {
    ($cop:expr, $path:literal) => {
        #[test]
        fn autocorrect_fixture() {
            $crate::testutil::assert_cop_autocorrect(
                &$cop,
                include_bytes!(concat!("../../../tests/fixtures/", $path, "/offense.rb")),
                include_bytes!(concat!("../../../tests/fixtures/", $path, "/corrected.rb")),
            );
        }
    };
}

/// Generate standard offense/no_offense fixture tests for a Rails cop that
/// requires `TargetRailsVersion` to be set (matching RuboCop's
/// `minimum_target_rails_version` / `requires_gem 'railties'` gates).
///
/// The `$min_version` parameter is the minimum Rails version the cop requires
/// (e.g., `5.0`, `6.0`, `7.0`). It is injected into the test config so the
/// cop's `config.rails_version_at_least(...)` check passes.
///
/// Usage:
/// ```ignore
/// #[cfg(test)]
/// mod tests {
///     use super::*;
///     crate::cop_rails_fixture_tests!(CopStruct, "cops/rails/cop_name", 5.0);
/// }
/// ```
#[macro_export]
macro_rules! cop_rails_fixture_tests {
    ($cop:expr, $path:literal, $min_version:expr) => {
        fn rails_config() -> $crate::cop::CopConfig {
            let mut options = std::collections::HashMap::new();
            options.insert(
                "TargetRailsVersion".to_string(),
                serde_yml::Value::Number(serde_yml::value::Number::from($min_version as f64)),
            );
            options.insert(
                "__RailtiesInLockfile".to_string(),
                serde_yml::Value::Bool(true),
            );
            $crate::cop::CopConfig {
                options,
                ..$crate::cop::CopConfig::default()
            }
        }

        #[test]
        fn offense_fixture() {
            $crate::testutil::assert_cop_offenses_full_with_config(
                &$cop,
                include_bytes!(concat!("../../../tests/fixtures/", $path, "/offense.rb")),
                rails_config(),
            );
        }

        #[test]
        fn no_offense_fixture() {
            $crate::testutil::assert_cop_no_offenses_full_with_config(
                &$cop,
                include_bytes!(concat!("../../../tests/fixtures/", $path, "/no_offense.rb")),
                rails_config(),
            );
        }

        #[test]
        fn skipped_when_no_target_rails_version() {
            // Non-Rails projects have no TargetRailsVersion — cop should not fire.
            let source = include_bytes!(concat!("../../../tests/fixtures/", $path, "/offense.rb"));
            let parsed = $crate::testutil::parse_fixture(source);
            let diagnostics = $crate::testutil::run_cop_full_internal(
                &$cop,
                &parsed.source,
                $crate::cop::CopConfig::default(),
                "test.rb",
            );
            assert!(
                diagnostics.is_empty(),
                "Should not fire when TargetRailsVersion is not set (non-Rails project), but got {} offenses",
                diagnostics.len()
            );
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(options: HashMap<String, serde_yml::Value>) -> CopConfig {
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    // --- should_autocorrect / autocorrect_setting unit tests ---

    #[test]
    fn should_autocorrect_off_always_false() {
        use crate::cli::AutocorrectMode;
        let cfg = config_with(HashMap::new());
        assert!(!cfg.should_autocorrect(AutocorrectMode::Off));
        // Even with everything explicitly enabled
        let cfg = config_with(HashMap::from([
            ("Safe".into(), serde_yml::Value::Bool(true)),
            ("SafeAutoCorrect".into(), serde_yml::Value::Bool(true)),
            ("AutoCorrect".into(), serde_yml::Value::Bool(true)),
        ]));
        assert!(!cfg.should_autocorrect(AutocorrectMode::Off));
    }

    #[test]
    fn should_autocorrect_safe_requires_both_safe_flags() {
        use crate::cli::AutocorrectMode;
        // Default config (both Safe and SafeAutoCorrect default to true)
        let cfg = config_with(HashMap::new());
        assert!(cfg.should_autocorrect(AutocorrectMode::Safe));
    }

    #[test]
    fn should_autocorrect_safe_blocked_by_unsafe_cop() {
        use crate::cli::AutocorrectMode;
        let cfg = config_with(HashMap::from([(
            "Safe".into(),
            serde_yml::Value::Bool(false),
        )]));
        assert!(!cfg.should_autocorrect(AutocorrectMode::Safe));
    }

    #[test]
    fn should_autocorrect_safe_blocked_by_unsafe_autocorrect() {
        use crate::cli::AutocorrectMode;
        let cfg = config_with(HashMap::from([(
            "SafeAutoCorrect".into(),
            serde_yml::Value::Bool(false),
        )]));
        assert!(!cfg.should_autocorrect(AutocorrectMode::Safe));
    }

    #[test]
    fn should_autocorrect_all_ignores_safe_flags() {
        use crate::cli::AutocorrectMode;
        let cfg = config_with(HashMap::from([
            ("Safe".into(), serde_yml::Value::Bool(false)),
            ("SafeAutoCorrect".into(), serde_yml::Value::Bool(false)),
        ]));
        assert!(cfg.should_autocorrect(AutocorrectMode::All));
    }

    #[test]
    fn should_autocorrect_disabled_blocks_all_modes() {
        use crate::cli::AutocorrectMode;
        let cfg = config_with(HashMap::from([(
            "AutoCorrect".into(),
            serde_yml::Value::Bool(false),
        )]));
        assert!(!cfg.should_autocorrect(AutocorrectMode::Safe));
        assert!(!cfg.should_autocorrect(AutocorrectMode::All));
    }

    #[test]
    fn autocorrect_setting_bool_true_is_always() {
        let cfg = config_with(HashMap::from([(
            "AutoCorrect".into(),
            serde_yml::Value::Bool(true),
        )]));
        assert_eq!(cfg.autocorrect_setting(), "always");
    }

    #[test]
    fn autocorrect_setting_bool_false_is_disabled() {
        let cfg = config_with(HashMap::from([(
            "AutoCorrect".into(),
            serde_yml::Value::Bool(false),
        )]));
        assert_eq!(cfg.autocorrect_setting(), "disabled");
    }

    #[test]
    fn autocorrect_setting_string_passthrough() {
        let cfg = config_with(HashMap::from([(
            "AutoCorrect".into(),
            serde_yml::Value::String("contextual".into()),
        )]));
        assert_eq!(cfg.autocorrect_setting(), "contextual");
    }

    #[test]
    fn autocorrect_setting_missing_is_always() {
        let cfg = config_with(HashMap::new());
        assert_eq!(cfg.autocorrect_setting(), "always");
    }

    #[test]
    fn get_string_hash_converts_integer_values() {
        use serde_yml::Value;
        let mut methods = serde_yml::Mapping::new();
        methods.insert(Value::String("inject".into()), Value::Number(2.into()));
        methods.insert(Value::String("reduce".into()), Value::Number(2.into()));
        methods.insert(Value::String("max_by".into()), Value::Number(1.into()));
        let cfg = config_with(HashMap::from([("Methods".into(), Value::Mapping(methods))]));
        let hash = cfg.get_string_hash("Methods").unwrap();
        assert_eq!(hash.len(), 3);
        assert_eq!(hash.get("inject").unwrap(), "2");
        assert_eq!(hash.get("reduce").unwrap(), "2");
        assert_eq!(hash.get("max_by").unwrap(), "1");
    }

    #[test]
    fn get_string_hash_handles_string_values() {
        use serde_yml::Value;
        let mut mapping = serde_yml::Mapping::new();
        mapping.insert(Value::String("collect".into()), Value::String("map".into()));
        let cfg = config_with(HashMap::from([(
            "PreferredMethods".into(),
            Value::Mapping(mapping),
        )]));
        let hash = cfg.get_string_hash("PreferredMethods").unwrap();
        assert_eq!(hash.get("collect").unwrap(), "map");
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;
        use serde_yml::Value;

        /// Strategy for serde_yml::Value that exercises the type branches.
        fn yaml_value_strategy() -> impl Strategy<Value = Value> {
            prop_oneof![
                any::<bool>().prop_map(Value::Bool),
                any::<u64>().prop_map(|n| Value::Number(serde_yml::Number::from(n))),
                "[a-z]{0,20}".prop_map(Value::String),
                Just(Value::Null),
                prop::collection::vec("[a-z]{1,10}".prop_map(Value::String), 0..5)
                    .prop_map(Value::Sequence),
            ]
        }

        proptest! {
            #[test]
            fn get_str_missing_key_returns_default(default in "[a-z]{1,10}") {
                let cfg = config_with(HashMap::new());
                prop_assert_eq!(cfg.get_str("NoSuchKey", &default), default.as_str());
            }

            #[test]
            fn get_str_present_string(key in "[A-Z][a-z]{1,8}", val in "[a-z]{1,20}") {
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::String(val.clone())),
                ]));
                prop_assert_eq!(cfg.get_str(&key, "fallback"), val.as_str());
            }

            #[test]
            fn get_str_wrong_type_returns_default(key in "[A-Z][a-z]{1,8}", n in any::<u64>()) {
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::Number(serde_yml::Number::from(n))),
                ]));
                prop_assert_eq!(cfg.get_str(&key, "default"), "default");
            }

            #[test]
            fn get_usize_missing_key_returns_default(default in any::<usize>()) {
                let cfg = config_with(HashMap::new());
                prop_assert_eq!(cfg.get_usize("NoSuchKey", default), default);
            }

            #[test]
            fn get_usize_present_number(key in "[A-Z][a-z]{1,8}", n in 0u64..1_000_000) {
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::Number(serde_yml::Number::from(n))),
                ]));
                prop_assert_eq!(cfg.get_usize(&key, 999), n as usize);
            }

            #[test]
            fn get_usize_wrong_type_returns_default(key in "[A-Z][a-z]{1,8}", s in "[a-z]{1,10}") {
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::String(s)),
                ]));
                prop_assert_eq!(cfg.get_usize(&key, 42), 42);
            }

            #[test]
            fn get_bool_missing_key_returns_default(default in any::<bool>()) {
                let cfg = config_with(HashMap::new());
                prop_assert_eq!(cfg.get_bool("NoSuchKey", default), default);
            }

            #[test]
            fn get_bool_present_bool(key in "[A-Z][a-z]{1,8}", val in any::<bool>()) {
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::Bool(val)),
                ]));
                prop_assert_eq!(cfg.get_bool(&key, !val), val);
            }

            #[test]
            fn get_bool_wrong_type_returns_default(key in "[A-Z][a-z]{1,8}", s in "[a-z]{1,10}") {
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::String(s)),
                ]));
                prop_assert_eq!(cfg.get_bool(&key, true), true);
            }

            #[test]
            fn get_string_array_missing_returns_none(key in "[A-Z][a-z]{1,8}") {
                let cfg = config_with(HashMap::new());
                prop_assert!(cfg.get_string_array(&key).is_none());
            }

            #[test]
            fn get_string_array_present(
                key in "[A-Z][a-z]{1,8}",
                items in prop::collection::vec("[a-z]{1,10}", 0..10),
            ) {
                let seq: Vec<Value> = items.iter().map(|s| Value::String(s.clone())).collect();
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::Sequence(seq)),
                ]));
                let result = cfg.get_string_array(&key).unwrap();
                prop_assert_eq!(result, items);
            }

            #[test]
            fn get_string_array_filters_non_strings(key in "[A-Z][a-z]{1,8}") {
                let seq = vec![
                    Value::String("keep".to_string()),
                    Value::Bool(true),
                    Value::Number(serde_yml::Number::from(42u64)),
                    Value::String("also_keep".to_string()),
                ];
                let cfg = config_with(HashMap::from([
                    (key.clone(), Value::Sequence(seq)),
                ]));
                let result = cfg.get_string_array(&key).unwrap();
                prop_assert_eq!(result, vec!["keep".to_string(), "also_keep".to_string()]);
            }

            #[test]
            fn no_panic_on_arbitrary_values(
                key in "[A-Z][a-z]{1,8}",
                val in yaml_value_strategy(),
            ) {
                let cfg = config_with(HashMap::from([(key.clone(), val)]));
                // None of these should panic regardless of value type
                let _ = cfg.get_str(&key, "default");
                let _ = cfg.get_usize(&key, 0);
                let _ = cfg.get_bool(&key, false);
                let _ = cfg.get_string_array(&key);
                let _ = cfg.get_string_hash(&key);
                let _ = cfg.get_flat_string_values(&key);
            }
        }
    }
}
