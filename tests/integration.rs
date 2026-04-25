//! Integration tests for the nitrocop linting pipeline.
//!
//! These tests exercise the full linter: file reading, config loading,
//! cop registry, cop execution, and diagnostic collection. They write
//! real files to a temp directory and invoke `run_linter` directly.

use std::fs;
use std::path::{Path, PathBuf};

use nitrocop::cli::Args;
use nitrocop::config::load_config;
use nitrocop::cop::autocorrect_allowlist::AutocorrectAllowlist;
use nitrocop::cop::registry::CopRegistry;
use nitrocop::cop::tiers::TierMap;
use nitrocop::fs::DiscoveredFiles;
use nitrocop::linter::run_linter;

/// Create a temporary directory with a unique name for each test.
fn temp_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nitrocop_integration_{test_name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a tiers.json to `dir` that marks the given cop as preview (everything else stable).
/// Returns the path for use with `NITROCOP_TIERS_FILE` env var.
fn write_preview_tiers(dir: &Path, cop_name: &str) -> PathBuf {
    let path = dir.join("tiers.json");
    fs::write(
        &path,
        format!(r#"{{"schema":1,"default_tier":"stable","overrides":{{"{cop_name}":"preview"}}}}"#),
    )
    .unwrap();
    path
}

fn write_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
    path
}

fn default_args() -> Args {
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
        preview: true,
        quiet_skips: false,
        strict: None,
        verify: false,
        rubocop_cmd: "bundle exec rubocop".to_string(),
        corpus_check: None,
    }
}

/// Wrap file paths as DiscoveredFiles with no explicit files (directory-discovered).
fn discovered(files: &[PathBuf]) -> DiscoveredFiles {
    DiscoveredFiles {
        files: files.to_vec(),
        explicit: std::collections::HashSet::new(),
    }
}

// ---------- Full pipeline tests ----------

#[test]
fn lint_clean_file_no_offenses() {
    let dir = temp_dir("clean_file");
    let file = write_file(
        &dir,
        "clean.rb",
        b"# frozen_string_literal: true\n\nx = 1\ny = 2\nputs(x + y)\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.file_count, 1);
    assert!(
        result.diagnostics.is_empty(),
        "Expected no offenses on clean file, got: {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| format!("{d}"))
            .collect::<Vec<_>>()
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn lint_file_with_multiple_offenses() {
    let dir = temp_dir("multi_offense");
    // Missing frozen_string_literal + trailing whitespace
    let file = write_file(&dir, "bad.rb", b"x = 1  \ny = 2\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.file_count, 1);

    let cop_names: Vec<&str> = result
        .diagnostics
        .iter()
        .map(|d| d.cop_name.as_str())
        .collect();
    assert!(
        cop_names.contains(&"Style/FrozenStringLiteralComment"),
        "Expected FrozenStringLiteralComment offense"
    );
    assert!(
        cop_names.contains(&"Layout/TrailingWhitespace"),
        "Expected TrailingWhitespace offense"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn lint_multiple_files() {
    let dir = temp_dir("multi_file");
    let f1 = write_file(
        &dir,
        "a.rb",
        b"# frozen_string_literal: true\n\nx = 1\nputs(x)\n",
    );
    let f2 = write_file(&dir, "b.rb", b"y = 2  \n");

    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[f1, f2]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.file_count, 2);

    // a.rb should be clean, b.rb should have offenses
    let a_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.path.contains("a.rb"))
        .collect();
    let b_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.path.contains("b.rb"))
        .collect();
    assert!(a_offenses.is_empty(), "a.rb should be clean");
    assert!(!b_offenses.is_empty(), "b.rb should have offenses");

    fs::remove_dir_all(&dir).ok();
}

// ---------- Filtering tests ----------

#[test]
fn only_filter_limits_cops() {
    let dir = temp_dir("only_filter");
    // Missing frozen_string_literal + trailing whitespace
    let file = write_file(&dir, "test.rb", b"x = 1  \n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // Only TrailingWhitespace should fire
    for d in &result.diagnostics {
        assert_eq!(
            d.cop_name, "Layout/TrailingWhitespace",
            "Only TrailingWhitespace should fire with --only filter, got: {}",
            d.cop_name,
        );
    }
    assert!(
        !result.diagnostics.is_empty(),
        "TrailingWhitespace should still fire"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn except_filter_excludes_cops() {
    let dir = temp_dir("except_filter");
    let file = write_file(&dir, "test.rb", b"x = 1  \n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        except: vec![
            "Style/FrozenStringLiteralComment".to_string(),
            "Layout/TrailingWhitespace".to_string(),
        ],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let excluded_cops = [
        "Style/FrozenStringLiteralComment",
        "Layout/TrailingWhitespace",
    ];
    for d in &result.diagnostics {
        assert!(
            !excluded_cops.contains(&d.cop_name.as_str()),
            "Excluded cop {} should not fire",
            d.cop_name,
        );
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn only_with_single_cop_on_clean_file() {
    let dir = temp_dir("only_clean");
    let file = write_file(&dir, "test.rb", b"x = 1\ny = 2\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(result.diagnostics.is_empty());

    fs::remove_dir_all(&dir).ok();
}

// ---------- Config override tests ----------

#[test]
fn config_disables_cop() {
    let dir = temp_dir("config_disable");
    let file = write_file(&dir, "test.rb", b"x = 1  \n");
    let config_path = write_file(
        &dir,
        ".rubocop.yml",
        b"Layout/TrailingWhitespace:\n  Enabled: false\nStyle/FrozenStringLiteralComment:\n  Enabled: false\n",
    );
    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let disabled_cops = [
        "Layout/TrailingWhitespace",
        "Style/FrozenStringLiteralComment",
    ];
    for d in &result.diagnostics {
        assert!(
            !disabled_cops.contains(&d.cop_name.as_str()),
            "Disabled cop {} should not fire",
            d.cop_name,
        );
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn nested_config_disables_cop_for_subdir() {
    let dir = temp_dir("nested_disable_cop");
    write_file(&dir, ".rubocop.yml", b"# root config\n");
    write_file(
        &dir,
        "spec/ruby/.rubocop.yml",
        b"Style/BlockComments:\n  Enabled: false\n",
    );
    let file = write_file(&dir, "spec/ruby/fixture.rb", b"=begin\ncomment\n=end\n");
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    assert!(
        result
            .diagnostics
            .iter()
            .all(|d| d.cop_name != "Style/BlockComments"),
        "Nested config should disable Style/BlockComments, got: {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| d.cop_name.as_str())
            .collect::<Vec<_>>()
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn nested_disabled_by_default_only_runs_explicitly_enabled_cops() {
    let dir = temp_dir("nested_disabled_by_default");
    write_file(&dir, ".rubocop.yml", b"# root config\n");
    write_file(
        &dir,
        "spec/ruby/.rubocop.yml",
        b"AllCops:\n  DisabledByDefault: true\nStyle/BlockComments:\n  Enabled: true\n",
    );
    let file = write_file(
        &dir,
        "spec/ruby/fixture.rb",
        b"module Example\n  extend self\nend\n\n=begin\ncomment\n=end\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let cop_names: Vec<_> = result
        .diagnostics
        .iter()
        .map(|d| d.cop_name.as_str())
        .collect();
    assert!(
        cop_names.contains(&"Style/BlockComments"),
        "Explicitly enabled nested cop should still run, got: {:?}",
        cop_names
    );
    assert!(
        !cop_names.contains(&"Style/ModuleFunction"),
        "DisabledByDefault sub-config should suppress unmentioned cops, got: {:?}",
        cop_names
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn nested_config_without_root_still_applies_disabled_by_default() {
    let dir = temp_dir("nested_no_root_disabled_by_default");
    write_file(
        &dir,
        "spec/ruby/.rubocop.yml",
        b"AllCops:\n  DisabledByDefault: true\nStyle/BlockComments:\n  Enabled: true\n",
    );
    let file = write_file(
        &dir,
        "spec/ruby/fixture.rb",
        b"module Example\n  extend self\nend\n\n=begin\ncomment\n=end\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let cop_names: Vec<_> = result
        .diagnostics
        .iter()
        .map(|d| d.cop_name.as_str())
        .collect();
    assert!(
        cop_names.contains(&"Style/BlockComments"),
        "Explicitly enabled nested cop should still run without a root config, got: {:?}",
        cop_names
    );
    assert!(
        !cop_names.contains(&"Style/ModuleFunction"),
        "DisabledByDefault nested config should suppress unmentioned cops without a root config, got: {:?}",
        cop_names
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_line_length_max_override() {
    let dir = temp_dir("config_max");
    // Line is 20 chars — under default 120 but over Max:10
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\ntwenty_char_line = 1\n",
    );
    let config_path = write_file(&dir, ".rubocop.yml", b"Layout/LineLength:\n  Max: 10\n");
    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/LineLength".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    assert!(
        !result.diagnostics.is_empty(),
        "LineLength should fire with Max:10 on a 20-char line"
    );
    for d in &result.diagnostics {
        assert_eq!(d.cop_name, "Layout/LineLength");
        assert!(
            d.message.contains("/10]"),
            "Message should reference Max:10"
        );
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn default_line_length_allows_120() {
    let dir = temp_dir("default_max");
    // 120 chars exactly — should NOT trigger
    let line = format!("# frozen_string_literal: true\n\n{}\n", "x".repeat(120));
    let file = write_file(&dir, "test.rb", line.as_bytes());
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/LineLength".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.diagnostics.is_empty(),
        "120-char line should not trigger default LineLength"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- Edge case tests ----------

#[test]
fn empty_file_no_crash() {
    let dir = temp_dir("empty");
    let file = write_file(&dir, "empty.rb", b"");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.file_count, 1);
    // Should not panic; may or may not have offenses (FrozenStringLiteralComment fires)

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn file_with_syntax_errors_still_lints() {
    let dir = temp_dir("syntax_error");
    // Invalid Ruby syntax — Prism parses with errors but still returns a tree
    let file = write_file(&dir, "bad_syntax.rb", b"def foo(\n  x = 1\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    // Should not panic
    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.file_count, 1);
    // Line-based cops should still find offenses (at minimum FrozenStringLiteralComment)

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn binary_content_no_crash() {
    let dir = temp_dir("binary");
    // Binary content with null bytes
    let file = write_file(&dir, "binary.rb", b"\x00\x01\x02\xff\xfe");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    // Should not panic
    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.file_count, 1);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn crlf_line_endings_detected() {
    let dir = temp_dir("crlf");
    let file = write_file(
        &dir,
        "crlf.rb",
        b"# frozen_string_literal: true\r\n\r\nx = 1\r\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/EndOfLine".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        !result.diagnostics.is_empty(),
        "EndOfLine should detect CRLF"
    );
    for d in &result.diagnostics {
        assert_eq!(d.cop_name, "Layout/EndOfLine");
        assert_eq!(d.message, "Carriage return character detected.");
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn diagnostics_are_sorted_by_path_then_location() {
    let dir = temp_dir("sort_order");
    let f1 = write_file(&dir, "b.rb", b"x = 1  \n");
    let f2 = write_file(&dir, "a.rb", b"y = 2  \n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[f1, f2]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.diagnostics.len(), 2);
    // Diagnostics should be sorted: a.rb before b.rb
    assert!(
        result.diagnostics[0].path < result.diagnostics[1].path
            || (result.diagnostics[0].path == result.diagnostics[1].path
                && result.diagnostics[0].location.line <= result.diagnostics[1].location.line),
        "Diagnostics should be sorted by path then location"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- All 8 cops fire correctly ----------

#[test]
fn all_registered_cops_can_fire() {
    let dir = temp_dir("all_cops");
    // This file triggers multiple cops:
    // - Missing frozen_string_literal
    // - Trailing whitespace on line 1
    // - Tab on line 2
    let file = write_file(&dir, "test.rb", b"x = 1  \n\ty = 2\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let cop_names: Vec<&str> = result
        .diagnostics
        .iter()
        .map(|d| d.cop_name.as_str())
        .collect();
    assert!(
        cop_names.contains(&"Style/FrozenStringLiteralComment"),
        "FrozenStringLiteralComment should fire"
    );
    assert!(
        cop_names.contains(&"Layout/TrailingWhitespace"),
        "TrailingWhitespace should fire"
    );
    assert!(
        cop_names.contains(&"Layout/IndentationStyle"),
        "IndentationStyle should fire"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn registry_has_expected_cop_count() {
    let registry = CopRegistry::default_registry();
    // 915 supported + 5 no-ops (obsolete on Ruby 3.4+)
    assert_eq!(registry.len(), 915 + 5);

    let names = registry.names();
    let expected = [
        // Bundler (7)
        "Bundler/DuplicatedGem",
        "Bundler/DuplicatedGroup",
        "Bundler/GemComment",
        "Bundler/GemFilename",
        "Bundler/GemVersion",
        "Bundler/InsecureProtocolSource",
        "Bundler/OrderedGems",
        // FactoryBot (11)
        "FactoryBot/AssociationStyle",
        "FactoryBot/AttributeDefinedStatically",
        "FactoryBot/ConsistentParenthesesStyle",
        "FactoryBot/CreateList",
        "FactoryBot/ExcessiveCreateList",
        "FactoryBot/FactoryAssociationWithStrategy",
        "FactoryBot/FactoryClassName",
        "FactoryBot/FactoryNameStyle",
        "FactoryBot/IdSequence",
        "FactoryBot/RedundantFactoryOption",
        "FactoryBot/SyntaxMethods",
        // Gemspec (10)
        "Gemspec/AddRuntimeDependency",
        "Gemspec/AttributeAssignment",
        "Gemspec/DependencyVersion",
        "Gemspec/DeprecatedAttributeAssignment",
        "Gemspec/DevelopmentDependencies",
        "Gemspec/DuplicatedAssignment",
        "Gemspec/OrderedDependencies",
        "Gemspec/RequireMFA",
        "Gemspec/RequiredRubyVersion",
        "Gemspec/RubyVersionGlobalsUsage",
        // Layout (100)
        "Layout/AccessModifierIndentation",
        "Layout/ArgumentAlignment",
        "Layout/ArrayAlignment",
        "Layout/AssignmentIndentation",
        "Layout/BeginEndAlignment",
        "Layout/BlockAlignment",
        "Layout/BlockEndNewline",
        "Layout/CaseIndentation",
        "Layout/ClassStructure",
        "Layout/ClosingHeredocIndentation",
        "Layout/ClosingParenthesisIndentation",
        "Layout/CommentIndentation",
        "Layout/ConditionPosition",
        "Layout/DefEndAlignment",
        "Layout/DotPosition",
        "Layout/ElseAlignment",
        "Layout/EmptyComment",
        "Layout/EmptyLineAfterGuardClause",
        "Layout/EmptyLineAfterMagicComment",
        "Layout/EmptyLineAfterMultilineCondition",
        "Layout/EmptyLineBetweenDefs",
        "Layout/EmptyLines",
        "Layout/EmptyLinesAfterModuleInclusion",
        "Layout/EmptyLinesAroundAccessModifier",
        "Layout/EmptyLinesAroundArguments",
        "Layout/EmptyLinesAroundAttributeAccessor",
        "Layout/EmptyLinesAroundBeginBody",
        "Layout/EmptyLinesAroundBlockBody",
        "Layout/EmptyLinesAroundClassBody",
        "Layout/EmptyLinesAroundExceptionHandlingKeywords",
        "Layout/EmptyLinesAroundMethodBody",
        "Layout/EmptyLinesAroundModuleBody",
        "Layout/EndAlignment",
        "Layout/EndOfLine",
        "Layout/ExtraSpacing",
        "Layout/FirstArgumentIndentation",
        "Layout/FirstArrayElementIndentation",
        "Layout/FirstArrayElementLineBreak",
        "Layout/FirstHashElementIndentation",
        "Layout/FirstHashElementLineBreak",
        "Layout/FirstMethodArgumentLineBreak",
        "Layout/FirstMethodParameterLineBreak",
        "Layout/FirstParameterIndentation",
        "Layout/HashAlignment",
        "Layout/HeredocArgumentClosingParenthesis",
        "Layout/HeredocIndentation",
        "Layout/IndentationConsistency",
        "Layout/IndentationStyle",
        "Layout/IndentationWidth",
        "Layout/InitialIndentation",
        "Layout/LeadingCommentSpace",
        "Layout/LeadingEmptyLines",
        "Layout/LineContinuationLeadingSpace",
        "Layout/LineContinuationSpacing",
        "Layout/LineEndStringConcatenationIndentation",
        "Layout/LineLength",
        "Layout/MultilineArrayBraceLayout",
        "Layout/MultilineArrayLineBreaks",
        "Layout/MultilineAssignmentLayout",
        "Layout/MultilineBlockLayout",
        "Layout/MultilineHashBraceLayout",
        "Layout/MultilineHashKeyLineBreaks",
        "Layout/MultilineMethodArgumentLineBreaks",
        "Layout/MultilineMethodCallBraceLayout",
        "Layout/MultilineMethodCallIndentation",
        "Layout/MultilineMethodDefinitionBraceLayout",
        "Layout/MultilineMethodParameterLineBreaks",
        "Layout/MultilineOperationIndentation",
        "Layout/ParameterAlignment",
        "Layout/RedundantLineBreak",
        "Layout/RescueEnsureAlignment",
        "Layout/SingleLineBlockChain",
        "Layout/SpaceAfterColon",
        "Layout/SpaceAfterComma",
        "Layout/SpaceAfterMethodName",
        "Layout/SpaceAfterNot",
        "Layout/SpaceAfterSemicolon",
        "Layout/SpaceAroundBlockParameters",
        "Layout/SpaceAroundEqualsInParameterDefault",
        "Layout/SpaceAroundKeyword",
        "Layout/SpaceAroundMethodCallOperator",
        "Layout/SpaceAroundOperators",
        "Layout/SpaceBeforeBlockBraces",
        "Layout/SpaceBeforeBrackets",
        "Layout/SpaceBeforeComma",
        "Layout/SpaceBeforeComment",
        "Layout/SpaceBeforeFirstArg",
        "Layout/SpaceBeforeSemicolon",
        "Layout/SpaceInLambdaLiteral",
        "Layout/SpaceInsideArrayLiteralBrackets",
        "Layout/SpaceInsideArrayPercentLiteral",
        "Layout/SpaceInsideBlockBraces",
        "Layout/SpaceInsideHashLiteralBraces",
        "Layout/SpaceInsideParens",
        "Layout/SpaceInsidePercentLiteralDelimiters",
        "Layout/SpaceInsideRangeLiteral",
        "Layout/SpaceInsideReferenceBrackets",
        "Layout/SpaceInsideStringInterpolation",
        "Layout/TrailingEmptyLines",
        "Layout/TrailingWhitespace",
        // Lint (116)
        "Lint/AmbiguousAssignment",
        "Lint/AmbiguousOperatorPrecedence",
        "Lint/AmbiguousRange",
        "Lint/AssignmentInCondition",
        "Lint/BigDecimalNew",
        "Lint/BinaryOperatorWithIdenticalOperands",
        "Lint/BooleanSymbol",
        "Lint/CircularArgumentReference",
        "Lint/CopDirectiveSyntax",
        "Lint/ConstantDefinitionInBlock",
        "Lint/ConstantOverwrittenInRescue",
        "Lint/ConstantReassignment",
        "Lint/Debugger",
        "Lint/DeprecatedClassMethods",
        "Lint/DeprecatedConstants",
        "Lint/DeprecatedOpenSSLConstant",
        "Lint/DisjunctiveAssignmentInConstructor",
        "Lint/DuplicateBranch",
        "Lint/DuplicateCaseCondition",
        "Lint/DuplicateElsifCondition",
        "Lint/DuplicateHashKey",
        "Lint/DuplicateMagicComment",
        "Lint/DuplicateMethods",
        "Lint/DuplicateRequire",
        "Lint/DuplicateRescueException",
        "Lint/EachWithObjectArgument",
        "Lint/ElseLayout",
        "Lint/EmptyBlock",
        "Lint/EmptyClass",
        "Lint/EmptyConditionalBody",
        "Lint/EmptyEnsure",
        "Lint/EmptyExpression",
        "Lint/EmptyFile",
        "Lint/EmptyInPattern",
        "Lint/EmptyInterpolation",
        "Lint/EmptyWhen",
        "Lint/EnsureReturn",
        "Lint/FlipFlop",
        "Lint/FloatComparison",
        "Lint/FloatOutOfRange",
        "Lint/FormatParameterMismatch",
        "Lint/HashCompareByIdentity",
        "Lint/IdentityComparison",
        "Lint/ImplicitStringConcatenation",
        "Lint/IneffectiveAccessModifier",
        "Lint/InheritException",
        "Lint/InterpolationCheck",
        "Lint/LiteralAsCondition",
        "Lint/LiteralAssignmentInCondition",
        "Lint/LiteralInInterpolation",
        "Lint/Loop",
        "Lint/MissingCopEnableDirective",
        "Lint/MissingSuper",
        "Lint/MultipleComparison",
        "Lint/NestedMethodDefinition",
        "Lint/NestedPercentLiteral",
        "Lint/NextWithoutAccumulator",
        "Lint/NoReturnInBeginEndBlocks",
        "Lint/NonAtomicFileOperation",
        "Lint/NonLocalExitFromIterator",
        "Lint/NumberedParameterAssignment",
        "Lint/OrAssignmentToConstant",
        "Lint/OrderedMagicComments",
        "Lint/OutOfRangeRegexpRef",
        "Lint/PercentStringArray",
        "Lint/PercentSymbolArray",
        "Lint/RaiseException",
        "Lint/RandOne",
        "Lint/RedundantCopDisableDirective",
        "Lint/RedundantCopEnableDirective",
        "Lint/RedundantDirGlobSort",
        "Lint/RedundantRegexpQuantifiers",
        "Lint/RedundantRequireStatement",
        "Lint/RedundantSafeNavigation",
        "Lint/RedundantSplatExpansion",
        "Lint/RedundantStringCoercion",
        "Lint/RedundantWithIndex",
        "Lint/RedundantWithObject",
        "Lint/RegexpAsCondition",
        "Lint/RequireParentheses",
        "Lint/RequireRangeParentheses",
        "Lint/RescueException",
        "Lint/RescueType",
        "Lint/ReturnInVoidContext",
        "Lint/SafeNavigationChain",
        "Lint/SafeNavigationConsistency",
        "Lint/ScriptPermission",
        "Lint/SelfAssignment",
        "Lint/SendWithMixinArgument",
        "Lint/ShadowedArgument",
        "Lint/ShadowedException",
        "Lint/ShadowingOuterLocalVariable",
        "Lint/StructNewOverride",
        "Lint/SuppressedException",
        "Lint/SymbolConversion",
        "Lint/Syntax",
        "Lint/ToEnumArguments",
        "Lint/ToJSON",
        "Lint/TopLevelReturnWithArgument",
        "Lint/TrailingCommaInAttributeDeclaration",
        "Lint/TripleQuotes",
        "Lint/UnescapedBracketInRegexp",
        "Lint/UnifiedInteger",
        "Lint/UnmodifiedReduceAccumulator",
        "Lint/UnreachableCode",
        "Lint/UnreachableLoop",
        "Lint/UnusedBlockArgument",
        "Lint/UnusedMethodArgument",
        "Lint/UriEscapeUnescape",
        "Lint/UriRegexp",
        "Lint/UselessAccessModifier",
        "Lint/UselessAssignment",
        "Lint/UselessElseWithoutRescue",
        "Lint/UselessMethodDefinition",
        "Lint/UselessSetterCall",
        "Lint/Void",
        // Metrics (10)
        "Metrics/AbcSize",
        "Metrics/BlockLength",
        "Metrics/BlockNesting",
        "Metrics/ClassLength",
        "Metrics/CollectionLiteralLength",
        "Metrics/CyclomaticComplexity",
        "Metrics/MethodLength",
        "Metrics/ModuleLength",
        "Metrics/ParameterLists",
        "Metrics/PerceivedComplexity",
        // Migration (1)
        "Migration/DepartmentName",
        // Naming (19)
        "Naming/AccessorMethodName",
        "Naming/AsciiIdentifiers",
        "Naming/BinaryOperatorParameterName",
        "Naming/BlockForwarding",
        "Naming/BlockParameterName",
        "Naming/ClassAndModuleCamelCase",
        "Naming/ConstantName",
        "Naming/FileName",
        "Naming/HeredocDelimiterCase",
        "Naming/HeredocDelimiterNaming",
        "Naming/InclusiveLanguage",
        "Naming/MemoizedInstanceVariableName",
        "Naming/MethodName",
        "Naming/MethodParameterName",
        "Naming/PredicateMethod",
        "Naming/PredicatePrefix",
        "Naming/RescuedExceptionsVariableName",
        "Naming/VariableName",
        "Naming/VariableNumber",
        // Performance (47)
        "Performance/AncestorsInclude",
        "Performance/ArraySemiInfiniteRangeSlice",
        "Performance/BigDecimalWithNumericArgument",
        "Performance/BindCall",
        "Performance/BlockGivenWithExplicitBlock",
        "Performance/Caller",
        "Performance/CaseWhenSplat",
        "Performance/Casecmp",
        "Performance/ChainArrayAllocation",
        "Performance/CompareWithBlock",
        "Performance/ConcurrentMonotonicTime",
        "Performance/Count",
        "Performance/DeletePrefix",
        "Performance/DeleteSuffix",
        "Performance/Detect",
        "Performance/DoubleStartEndWith",
        "Performance/EndWith",
        "Performance/FlatMap",
        "Performance/InefficientHashSearch",
        "Performance/IoReadlines",
        "Performance/MapCompact",
        "Performance/MapMethodChain",
        "Performance/MethodObjectAsBlock",
        "Performance/OpenStruct",
        "Performance/RangeInclude",
        "Performance/RedundantBlockCall",
        "Performance/RedundantEqualityComparisonBlock",
        "Performance/RedundantMatch",
        "Performance/RedundantMerge",
        "Performance/RedundantSortBlock",
        "Performance/RedundantSplitRegexpArgument",
        "Performance/RedundantStringChars",
        "Performance/RegexpMatch",
        "Performance/ReverseEach",
        "Performance/ReverseFirst",
        "Performance/SelectMap",
        "Performance/Size",
        "Performance/SortReverse",
        "Performance/Squeeze",
        "Performance/StartWith",
        "Performance/StringIdentifierArgument",
        "Performance/StringInclude",
        "Performance/StringReplacement",
        "Performance/Sum",
        "Performance/TimesMap",
        "Performance/UnfreezeString",
        "Performance/UriDefaultParser",
        // RSpec (113)
        "RSpec/AlignLeftLetBrace",
        "RSpec/AlignRightLetBrace",
        "RSpec/AnyInstance",
        "RSpec/AroundBlock",
        "RSpec/Be",
        "RSpec/BeEmpty",
        "RSpec/BeEq",
        "RSpec/BeEql",
        "RSpec/BeNil",
        "RSpec/BeforeAfterAll",
        "RSpec/ChangeByZero",
        "RSpec/ClassCheck",
        "RSpec/ContainExactly",
        "RSpec/ContextMethod",
        "RSpec/ContextWording",
        "RSpec/DescribeClass",
        "RSpec/DescribeMethod",
        "RSpec/DescribeSymbol",
        "RSpec/DescribedClass",
        "RSpec/DescribedClassModuleWrapping",
        "RSpec/Dialect",
        "RSpec/DuplicatedMetadata",
        "RSpec/EmptyExampleGroup",
        "RSpec/EmptyHook",
        "RSpec/EmptyLineAfterExample",
        "RSpec/EmptyLineAfterExampleGroup",
        "RSpec/EmptyLineAfterFinalLet",
        "RSpec/EmptyLineAfterHook",
        "RSpec/EmptyLineAfterSubject",
        "RSpec/EmptyMetadata",
        "RSpec/EmptyOutput",
        "RSpec/Eq",
        "RSpec/ExampleLength",
        "RSpec/ExampleWithoutDescription",
        "RSpec/ExampleWording",
        "RSpec/ExcessiveDocstringSpacing",
        "RSpec/ExpectActual",
        "RSpec/ExpectChange",
        "RSpec/ExpectInHook",
        "RSpec/ExpectInLet",
        "RSpec/ExpectOutput",
        "RSpec/Focus",
        "RSpec/HookArgument",
        "RSpec/HooksBeforeExamples",
        "RSpec/IdenticalEqualityAssertion",
        "RSpec/ImplicitBlockExpectation",
        "RSpec/ImplicitExpect",
        "RSpec/ImplicitSubject",
        "RSpec/IncludeExamples",
        "RSpec/IndexedLet",
        "RSpec/InstanceSpy",
        "RSpec/InstanceVariable",
        "RSpec/IsExpectedSpecify",
        "RSpec/ItBehavesLike",
        "RSpec/IteratedExpectation",
        "RSpec/LeadingSubject",
        "RSpec/LeakyConstantDeclaration",
        "RSpec/LeakyLocalVariable",
        "RSpec/LetBeforeExamples",
        "RSpec/LetSetup",
        "RSpec/MatchArray",
        "RSpec/MessageChain",
        "RSpec/MessageExpectation",
        "RSpec/MessageSpies",
        "RSpec/MetadataStyle",
        "RSpec/MissingExampleGroupArgument",
        "RSpec/MissingExpectationTargetMethod",
        "RSpec/MultipleDescribes",
        "RSpec/MultipleExpectations",
        "RSpec/MultipleMemoizedHelpers",
        "RSpec/MultipleSubjects",
        "RSpec/NamedSubject",
        "RSpec/NestedGroups",
        "RSpec/NoExpectationExample",
        "RSpec/NotToNot",
        "RSpec/Output",
        "RSpec/OverwritingSetup",
        "RSpec/Pending",
        "RSpec/PendingWithoutReason",
        "RSpec/PredicateMatcher",
        "RSpec/ReceiveCounts",
        "RSpec/ReceiveMessages",
        "RSpec/ReceiveNever",
        "RSpec/RedundantAround",
        "RSpec/RedundantPredicateMatcher",
        "RSpec/RemoveConst",
        "RSpec/RepeatedDescription",
        "RSpec/RepeatedExample",
        "RSpec/RepeatedExampleGroupBody",
        "RSpec/RepeatedExampleGroupDescription",
        "RSpec/RepeatedIncludeExample",
        "RSpec/RepeatedSubjectCall",
        "RSpec/ReturnFromStub",
        "RSpec/ScatteredLet",
        "RSpec/ScatteredSetup",
        "RSpec/SharedContext",
        "RSpec/SharedExamples",
        "RSpec/SingleArgumentMessageChain",
        "RSpec/SkipBlockInsideExample",
        "RSpec/SortMetadata",
        "RSpec/SpecFilePathFormat",
        "RSpec/SpecFilePathSuffix",
        "RSpec/StubbedMock",
        "RSpec/SubjectDeclaration",
        "RSpec/SubjectStub",
        "RSpec/UndescriptiveLiteralsDescription",
        "RSpec/UnspecifiedException",
        "RSpec/VariableDefinition",
        "RSpec/VariableName",
        "RSpec/VerifiedDoubleReference",
        "RSpec/VerifiedDoubles",
        "RSpec/VoidExpect",
        "RSpec/Yield",
        // Rails (110)
        "Rails/ActionControllerFlashBeforeRender",
        "Rails/ActionControllerTestCase",
        "Rails/ActionOrder",
        "Rails/ActiveRecordCallbacksOrder",
        "Rails/ActiveSupportAliases",
        "Rails/ActiveSupportOnLoad",
        "Rails/AddColumnIndex",
        "Rails/AfterCommitOverride",
        "Rails/ApplicationController",
        "Rails/ApplicationJob",
        "Rails/ApplicationMailer",
        "Rails/ApplicationRecord",
        "Rails/AttributeDefaultBlockValue",
        "Rails/Blank",
        "Rails/BulkChangeTable",
        "Rails/CompactBlank",
        "Rails/ContentTag",
        "Rails/CreateTableWithTimestamps",
        "Rails/DangerousColumnNames",
        "Rails/Date",
        "Rails/Delegate",
        "Rails/DelegateAllowBlank",
        "Rails/DeprecatedActiveModelErrorsMethods",
        "Rails/DotSeparatedKeys",
        "Rails/DuplicateAssociation",
        "Rails/DuplicateScope",
        "Rails/DurationArithmetic",
        "Rails/DynamicFindBy",
        "Rails/EnumHash",
        "Rails/EnumSyntax",
        "Rails/EnumUniqueness",
        "Rails/Env",
        "Rails/EnvLocal",
        "Rails/EnvironmentComparison",
        "Rails/EnvironmentVariableAccess",
        "Rails/Exit",
        "Rails/ExpandedDateRange",
        "Rails/FilePath",
        "Rails/FindBy",
        "Rails/FindByOrAssignmentMemoization",
        "Rails/FindEach",
        "Rails/FreezeTime",
        "Rails/HasAndBelongsToMany",
        "Rails/HasManyOrHasOneDependent",
        "Rails/HelperInstanceVariable",
        "Rails/HttpPositionalArguments",
        "Rails/HttpStatus",
        "Rails/HttpStatusNameConsistency",
        "Rails/I18nLazyLookup",
        "Rails/I18nLocaleAssignment",
        "Rails/I18nLocaleTexts",
        "Rails/IndexBy",
        "Rails/Inquiry",
        "Rails/InverseOf",
        "Rails/LexicallyScopedActionFilter",
        "Rails/MigrationClassName",
        "Rails/NegateInclude",
        "Rails/NotNullColumn",
        "Rails/Output",
        "Rails/OutputSafety",
        "Rails/Pick",
        "Rails/Pluck",
        "Rails/PluckId",
        "Rails/PluckInWhere",
        "Rails/Present",
        "Rails/RakeEnvironment",
        "Rails/ReadWriteAttribute",
        "Rails/RedundantActiveRecordAllMethod",
        "Rails/RedundantAllowNil",
        "Rails/RedundantForeignKey",
        "Rails/RedundantPresenceValidationOnBelongsTo",
        "Rails/RedundantReceiverInWithOptions",
        "Rails/RedundantTravelBack",
        "Rails/ReflectionClassName",
        "Rails/RefuteMethods",
        "Rails/RelativeDateConstant",
        "Rails/RenderInline",
        "Rails/RenderPlainText",
        "Rails/RequestReferer",
        "Rails/ResponseParsedBody",
        "Rails/ReversibleMigration",
        "Rails/ReversibleMigrationMethodDefinition",
        "Rails/RootJoinChain",
        "Rails/RootPathnameMethods",
        "Rails/RootPublicPath",
        "Rails/SafeNavigation",
        "Rails/SaveBang",
        "Rails/SchemaComment",
        "Rails/ScopeArgs",
        "Rails/SelectMap",
        "Rails/ShortI18n",
        "Rails/SkipsModelValidations",
        "Rails/SquishedSQLHeredocs",
        "Rails/StripHeredoc",
        "Rails/StrongParametersExpect",
        "Rails/TableNameAssignment",
        "Rails/ThreeStateBooleanColumn",
        "Rails/TimeZone",
        "Rails/TimeZoneAssignment",
        "Rails/ToFormattedS",
        "Rails/ToSWithArgument",
        "Rails/TransactionExitStatement",
        "Rails/UniqueValidationWithoutIndex",
        "Rails/UnknownEnv",
        "Rails/UnusedIgnoredColumns",
        "Rails/UnusedRenderContent",
        "Rails/Validation",
        "Rails/WhereExists",
        "Rails/WhereMissing",
        "Rails/WhereNot",
        "Rails/WhereRange",
        // Rake (5)
        "Rake/ClassDefinitionInTask",
        "Rake/Desc",
        "Rake/DuplicateNamespace",
        "Rake/DuplicateTask",
        "Rake/MethodDefinitionInTask",
        // Security (7)
        "Security/CompoundHash",
        "Security/Eval",
        "Security/IoMethods",
        "Security/JSONLoad",
        "Security/MarshalLoad",
        "Security/Open",
        "Security/YAMLLoad",
        // Style (80)
        "Style/AndOr",
        "Style/BlockDelimiters",
        "Style/ClassAndModuleChildren",
        "Style/ClassVars",
        "Style/ColonMethodCall",
        "Style/DefWithParentheses",
        "Style/Documentation",
        "Style/DoubleNegation",
        "Style/EmptyMethod",
        "Style/ExplicitBlockArgument",
        "Style/FrozenStringLiteralComment",
        "Style/GlobalVars",
        "Style/GuardClause",
        "Style/HashSyntax",
        "Style/HashTransformKeys",
        "Style/HashTransformValues",
        "Style/IfUnlessModifier",
        "Style/Lambda",
        "Style/MethodCallWithArgsParentheses",
        "Style/MethodDefParentheses",
        "Style/MutableConstant",
        "Style/NegatedIf",
        "Style/NegatedUnless",
        "Style/NegatedWhile",
        "Style/NilLambda",
        "Style/NonNilCheck",
        "Style/Not",
        "Style/NumericLiteralPrefix",
        "Style/NumericLiterals",
        "Style/NumericPredicate",
        "Style/ObjectThen",
        "Style/OpenStructUse",
        "Style/OptionalBooleanParameter",
        "Style/OrAssignment",
        "Style/ParenthesesAroundCondition",
        "Style/PreferredHashMethods",
        "Style/Proc",
        "Style/RaiseArgs",
        "Style/RedundantBegin",
        "Style/RedundantCondition",
        "Style/RedundantConditional",
        "Style/RedundantException",
        "Style/RedundantFileExtensionInRequire",
        "Style/RedundantFreeze",
        "Style/RedundantInterpolation",
        "Style/RedundantPercentQ",
        "Style/RedundantReturn",
        "Style/RedundantSelf",
        "Style/RedundantSort",
        "Style/RescueModifier",
        "Style/RescueStandardError",
        "Style/SafeNavigation",
        "Style/Sample",
        "Style/SelectByRegexp",
        "Style/SelfAssignment",
        "Style/Semicolon",
        "Style/SignalException",
        "Style/SingleArgumentDig",
        "Style/SingleLineMethods",
        "Style/SlicingWithRange",
        "Style/SpecialGlobalVars",
        "Style/StabbyLambdaParentheses",
        "Style/StringConcatenation",
        "Style/StringLiterals",
        "Style/Strip",
        "Style/SymbolArray",
        "Style/SymbolLiteral",
        "Style/TernaryParentheses",
        "Style/TrailingCommaInArguments",
        "Style/TrailingCommaInArrayLiteral",
        "Style/TrailingCommaInHashLiteral",
        "Style/TrivialAccessors",
        "Style/UnlessElse",
        "Style/UnpackFirst",
        "Style/WhenThen",
        "Style/WordArray",
        "Style/YodaCondition",
        "Style/ZeroLengthPredicate",
    ];
    for name in &expected {
        assert!(
            names.contains(name),
            "Registry missing expected cop: {name}"
        );
    }
}

// ---------- Performance department integration tests ----------

#[test]
fn performance_cops_fire_on_slow_patterns() {
    let dir = temp_dir("perf_cops");
    let file = write_file(
        &dir,
        "slow.rb",
        b"# frozen_string_literal: true\n\narr = [1, 2, 3]\narr.select { |x| x > 1 }.first\narr.reverse.each { |x| puts x }\narr.select { |x| x > 1 }.count\narr.flatten.map { |x| x.to_s }\n",
    );
    // Need plugins: entry so Performance department cops are enabled
    write_file(&dir, ".rubocop.yml", b"plugins:\n  - rubocop-performance\n");
    let config = load_config(None, Some(dir.as_ref()), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let perf_diags: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name.starts_with("Performance/"))
        .collect();

    assert!(
        perf_diags.len() >= 3,
        "Expected at least 3 Performance diagnostics, got {}: {:?}",
        perf_diags.len(),
        perf_diags.iter().map(|d| &d.cop_name).collect::<Vec<_>>()
    );

    let cop_names: Vec<&str> = perf_diags.iter().map(|d| d.cop_name.as_str()).collect();
    assert!(
        cop_names.contains(&"Performance/Detect"),
        "Expected Performance/Detect to fire on select.first"
    );
    assert!(
        cop_names.contains(&"Performance/ReverseEach"),
        "Expected Performance/ReverseEach to fire on reverse.each"
    );
    assert!(
        cop_names.contains(&"Performance/Count"),
        "Expected Performance/Count to fire on select.count"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- Lint department integration tests ----------

#[test]
fn lint_cops_fire_on_bad_code() {
    let dir = temp_dir("lint_cops");
    let file = write_file(
        &dir,
        "bad.rb",
        b"# frozen_string_literal: true\n\nbinding.pry\nraise Exception, \"bad\"\nx = :true\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let lint_diags: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name.starts_with("Lint/"))
        .collect();

    assert!(
        lint_diags.len() >= 3,
        "Expected at least 3 Lint diagnostics, got {}: {:?}",
        lint_diags.len(),
        lint_diags.iter().map(|d| &d.cop_name).collect::<Vec<_>>()
    );

    let cop_names: Vec<&str> = lint_diags.iter().map(|d| d.cop_name.as_str()).collect();
    assert!(
        cop_names.contains(&"Lint/Debugger"),
        "Expected Lint/Debugger to fire on binding.pry"
    );
    assert!(
        cop_names.contains(&"Lint/RaiseException"),
        "Expected Lint/RaiseException to fire on raise Exception"
    );
    assert!(
        cop_names.contains(&"Lint/BooleanSymbol"),
        "Expected Lint/BooleanSymbol to fire on :true"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- Multi-department JSON output test ----------

#[test]
fn json_formatter_includes_all_departments() {
    let dir = temp_dir("multi_dept");
    // This file triggers cops from multiple departments:
    // - Layout: trailing whitespace
    // - Style: missing frozen_string_literal
    // - Lint: binding.pry (Debugger), :true (BooleanSymbol)
    let file = write_file(&dir, "multi.rb", b"binding.pry  \nx = :true\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // Collect unique department prefixes
    let departments: std::collections::HashSet<&str> = result
        .diagnostics
        .iter()
        .filter_map(|d| d.cop_name.split('/').next())
        .collect();

    assert!(
        departments.contains("Layout"),
        "Expected Layout department diagnostics, got departments: {:?}",
        departments
    );
    assert!(
        departments.contains("Style"),
        "Expected Style department diagnostics, got departments: {:?}",
        departments
    );
    assert!(
        departments.contains("Lint"),
        "Expected Lint department diagnostics, got departments: {:?}",
        departments
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- Include/Exclude integration tests ----------
//
// These tests exercise the full linter pipeline with path-based filtering.
// Since run_linter receives absolute paths but Include/Exclude patterns are
// relative, we construct config patterns using absolute paths to match.

#[test]
fn migration_cop_filtered_by_path() {
    let dir = temp_dir("migration_path_filter");
    // Use a config that sets Include with an absolute pattern matching our temp dir.
    // This mirrors what default_include does but with absolute paths.
    // Need plugins: entry so Rails department cops are enabled.
    let dir_str = dir.display();
    let config_yaml = format!(
        "plugins:\n  - rubocop-rails\nRails/CreateTableWithTimestamps:\n  Include:\n    - '{dir_str}/db/migrate/**/*.rb'\n"
    );
    let config_path = write_file(&dir, ".rubocop.yml", config_yaml.as_bytes());
    let migration_content = b"class CreateUsers < ActiveRecord::Migration[7.0]\n  def change\n    create_table :users do |t|\n      t.string :name\n    end\n  end\nend\n";
    let migrate_file = write_file(&dir, "db/migrate/001_create_users.rb", migration_content);
    let model_file = write_file(&dir, "app/models/user.rb", migration_content);
    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Rails/CreateTableWithTimestamps".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[migrate_file, model_file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // Only the migration file should have offenses
    let migrate_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.path.contains("db/migrate"))
        .collect();
    let model_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.path.contains("app/models"))
        .collect();

    assert!(
        !migrate_offenses.is_empty(),
        "CreateTableWithTimestamps should fire on db/migrate/ files"
    );
    assert!(
        model_offenses.is_empty(),
        "CreateTableWithTimestamps should NOT fire on app/models/ files (not in Include path)"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn global_exclude_skips_file() {
    let dir = temp_dir("global_exclude");
    // Use absolute pattern in AllCops.Exclude to match temp dir paths
    let dir_str = dir.display();
    let config_yaml = format!("AllCops:\n  Exclude:\n    - '{dir_str}/vendor/**'\n");
    let config_path = write_file(&dir, ".rubocop.yml", config_yaml.as_bytes());
    // Place a file with trailing whitespace in vendor/
    let vendor_file = write_file(&dir, "vendor/foo.rb", b"x = 1  \n");
    // Place the same file outside vendor/
    let app_file = write_file(&dir, "app.rb", b"x = 1  \n");

    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[vendor_file, app_file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let vendor_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.path.contains("vendor"))
        .collect();
    let app_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.path.contains("app.rb"))
        .collect();

    assert!(
        vendor_offenses.is_empty(),
        "Global Exclude should prevent offenses on vendor/ files"
    );
    assert!(
        !app_offenses.is_empty(),
        "Non-excluded files should still have offenses"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn global_exclude_not_counted_in_file_count() {
    let dir = temp_dir("global_exclude_count");
    let dir_str = dir.display();
    let config_yaml = format!("AllCops:\n  Exclude:\n    - '{dir_str}/vendor/**'\n");
    let config_path = write_file(&dir, ".rubocop.yml", config_yaml.as_bytes());
    // Two files: one excluded, one not
    let vendor_file = write_file(&dir, "vendor/foo.rb", b"x = 1  \n");
    let app_file = write_file(&dir, "app.rb", b"x = 1  \n");

    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    // Simulate what lib.rs now does: filter before passing to run_linter
    let all_files = vec![vendor_file, app_file];
    let cop_filters = config.build_cop_filters(&registry, &TierMap::load(), args.preview);
    let effective: Vec<PathBuf> = all_files
        .iter()
        .filter(|f| !cop_filters.is_globally_excluded(f))
        .cloned()
        .collect();

    let result = run_linter(
        &discovered(&effective),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    assert_eq!(
        result.file_count, 1,
        "Globally-excluded files should not be counted; expected 1, got {}",
        result.file_count
    );
    assert!(
        !result.diagnostics.is_empty(),
        "The non-excluded file should produce offenses"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn user_include_override_widens_scope() {
    let dir = temp_dir("user_include_override");
    // CreateTableWithTimestamps defaults to Include: db/migrate/**/*.rb
    // Override to widen scope to all db/**/*.rb (using absolute path for temp dir)
    // Need plugins: entry so Rails department cops are enabled.
    let dir_str = dir.display();
    let config_yaml = format!(
        "plugins:\n  - rubocop-rails\nRails/CreateTableWithTimestamps:\n  Include:\n    - '{dir_str}/db/**/*.rb'\n"
    );
    let config_path = write_file(&dir, ".rubocop.yml", config_yaml.as_bytes());
    let migration_content = b"class CreateUsers < ActiveRecord::Migration[7.0]\n  def change\n    create_table :users do |t|\n      t.string :name\n    end\n  end\nend\n";
    // This file is in db/ but NOT in db/migrate/ — only matches the widened Include
    let seeds_file = write_file(&dir, "db/seeds.rb", migration_content);
    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Rails/CreateTableWithTimestamps".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[seeds_file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    assert!(
        !result.diagnostics.is_empty(),
        "User Include override should widen scope to db/seeds.rb"
    );
    for d in &result.diagnostics {
        assert_eq!(d.cop_name, "Rails/CreateTableWithTimestamps");
    }

    fs::remove_dir_all(&dir).ok();
}

// ---------- Test coverage guard ----------

/// Convert CamelCase to snake_case, handling runs of uppercase letters.
/// Examples: "TrailingWhitespace" -> "trailing_whitespace",
///           "AbcSize" -> "abc_size", "ABCSize" -> "abc_size",
///           "UriRegexp" -> "uri_regexp"
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                let prev = chars[i - 1];
                if prev.is_lowercase() || prev.is_ascii_digit() {
                    result.push('_');
                } else if i + 1 < chars.len() && chars[i + 1].is_lowercase() {
                    result.push('_');
                }
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// Count annotations in fixture content: both `^` markers and `# nitrocop-expect:` lines.
fn count_annotations(content: &str) -> usize {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            // Standard ^ annotations
            (trimmed.starts_with('^') && trimmed.contains(": ") && trimmed.contains('/'))
            // Explicit expect annotations
            || line.starts_with("# nitrocop-expect: ")
        })
        .count()
}

#[test]
fn all_cops_have_minimum_test_coverage() {
    let testdata = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cops");
    let registry = CopRegistry::default_registry();

    // No-op stub cops that never produce offenses by design.
    // These exist for configuration compatibility only.
    let stub_cops: &[&str] = &[
        "Lint/RedundantCopDisableDirective", // requires post-processing after all cops run
        "Lint/Syntax",                       // syntax errors reported by parser, not this cop
        // Unsupported cops — obsolete under modern Ruby, kept as no-ops for config compatibility
        "Lint/ItWithoutArgumentsInBlock", // `it` is a block parameter in Ruby 3.4+
        "Lint/NonDeterministicRequireOrder", // Dir sorts since Ruby 3.0
        "Lint/NumberedParameterAssignment", // `_1 = x` is a syntax error in Ruby 3.4+
        "Lint/UselessElseWithoutRescue",  // `else` without `rescue` is a syntax error in Ruby 3.4+
        "Security/YAMLLoad",              // YAML.load is safe since Ruby 3.1
    ];

    let mut failures = Vec::new();

    for cop_name in registry.names() {
        if stub_cops.contains(&cop_name) {
            continue;
        }
        let parts: Vec<&str> = cop_name.split('/').collect();
        let dept = parts[0].to_lowercase();
        let name = to_snake_case(parts[1]);

        let dir = testdata.join(&dept).join(&name);
        let dir_alt = testdata.join(&dept).join(format!("{name}_cop"));
        let effective_dir = if dir.exists() {
            &dir
        } else if dir_alt.exists() {
            &dir_alt
        } else {
            continue; // all_cops_have_fixture_files covers this
        };

        // Check offense annotations: either from offense.rb or offense/ directory.
        let offense_dir = effective_dir.join("offense");
        let annotation_count = if offense_dir.exists() && offense_dir.is_dir() {
            // Sum annotations across all .rb files in offense/
            let mut count = 0;
            for entry in fs::read_dir(&offense_dir).unwrap() {
                let entry = entry.unwrap();
                if entry.path().extension().map_or(false, |e| e == "rb") {
                    let content = fs::read_to_string(entry.path()).unwrap();
                    count += count_annotations(&content);
                }
            }
            count
        } else if let Ok(content) = fs::read_to_string(effective_dir.join("offense.rb")) {
            count_annotations(&content)
        } else {
            continue; // all_cops_have_fixture_files covers this
        };

        if annotation_count < 3 {
            failures.push(format!(
                "{cop_name}: only {annotation_count} offense case(s), need at least 3"
            ));
        }

        // Check no_offense.rb has at least 5 non-empty lines
        if let Ok(no_offense_content) = fs::read_to_string(effective_dir.join("no_offense.rb")) {
            let non_empty = no_offense_content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count();
            if non_empty < 5 {
                failures.push(format!(
                    "{cop_name}: only {non_empty} non-empty line(s) in no_offense.rb, need at least 5"
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Cops below minimum test coverage thresholds:\n{}",
        failures.join("\n")
    );
}

#[test]
fn all_cops_have_fixture_files() {
    let testdata = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cops");
    let registry = CopRegistry::default_registry();
    let mut missing = Vec::new();

    for cop_name in registry.names() {
        let parts: Vec<&str> = cop_name.split('/').collect();
        let dept = parts[0].to_lowercase();
        let name = to_snake_case(parts[1]);

        let dir = testdata.join(&dept).join(&name);
        // Some cops use a `_cop` suffix to avoid Rust keyword conflicts (e.g. loop -> loop_cop)
        let dir_alt = testdata.join(&dept).join(format!("{name}_cop"));

        let effective_dir = if dir.exists() {
            &dir
        } else if dir_alt.exists() {
            &dir_alt
        } else {
            missing.push(format!(
                "{cop_name}: missing directory ({} or {})",
                dir.display(),
                dir_alt.display()
            ));
            continue;
        };

        let has_offense_file = effective_dir.join("offense.rb").exists();
        let has_offense_dir = effective_dir.join("offense").is_dir();
        if !has_offense_file && !has_offense_dir {
            missing.push(format!(
                "{cop_name}: missing offense.rb or offense/ directory"
            ));
        }
        if !effective_dir.join("no_offense.rb").exists() {
            missing.push(format!("{cop_name}: missing no_offense.rb"));
        }
    }

    assert!(
        missing.is_empty(),
        "Cops missing fixture files:\n{}",
        missing.join("\n")
    );
}

/// Every top-level block in an offense.rb fixture must have at least one annotation.
///
/// Catches "dead" fixture code — Ruby code added to offense.rb without `^` annotations,
/// which passes tests vacuously (the cop doesn't detect it but no annotation expects it to).
/// This was the failure mode in PR #135 where a minimax agent added 3 unannotated describe blocks.
///
/// Splits files into regions at blank lines following a top-level `end`. Each region with
/// ≥5 source lines must contain at least one annotation.
#[test]
fn offense_fixtures_have_no_unannotated_blocks() {
    let testdata = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cops");
    let registry = CopRegistry::default_registry();

    let stub_cops: &[&str] = &[
        "Lint/RedundantCopDisableDirective",
        "Lint/Syntax",
        "Lint/ItWithoutArgumentsInBlock",
        "Lint/NonDeterministicRequireOrder",
        "Lint/NumberedParameterAssignment",
        "Lint/UselessElseWithoutRescue",
        "Security/YAMLLoad",
    ];

    let mut failures = Vec::new();

    for cop_name in registry.names() {
        if stub_cops.contains(&cop_name) {
            continue;
        }
        let parts: Vec<&str> = cop_name.split('/').collect();
        let dept = parts[0].to_lowercase();
        let name = to_snake_case(parts[1]);

        let dir = testdata.join(&dept).join(&name);
        let dir_alt = testdata.join(&dept).join(format!("{name}_cop"));
        let effective_dir = if dir.exists() {
            &dir
        } else if dir_alt.exists() {
            &dir_alt
        } else {
            continue;
        };

        let mut offense_files: Vec<PathBuf> = Vec::new();
        let offense_dir = effective_dir.join("offense");
        if offense_dir.is_dir() {
            for entry in fs::read_dir(&offense_dir).unwrap() {
                let entry = entry.unwrap();
                if entry.path().extension().map_or(false, |e| e == "rb") {
                    offense_files.push(entry.path());
                }
            }
        } else {
            let f = effective_dir.join("offense.rb");
            if f.exists() {
                offense_files.push(f);
            }
        }

        for file_path in &offense_files {
            let content = fs::read_to_string(file_path).unwrap();
            if let Some(desc) = find_unannotated_region(&content) {
                let rel = file_path.strip_prefix(&testdata).unwrap_or(file_path);
                failures.push(format!("{cop_name} ({}): {desc}", rel.display()));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Offense fixtures with unannotated code blocks (missing ^ annotations):\n{}",
        failures.join("\n")
    );
}

/// Split an offense fixture into regions at blank lines that follow a top-level `end`,
/// then check that each region with ≥5 source lines has at least one annotation.
fn find_unannotated_region(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return None;
    }

    // Split into regions: a blank line after a top-level `end` starts a new region.
    let mut regions: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Check if the last non-blank, non-annotation source line is "end" at col 0
            let last_source = current.iter().rev().find_map(|prev| {
                let pt = prev.trim();
                if !pt.is_empty() && !is_annotation_line(pt) {
                    Some(*prev)
                } else {
                    None
                }
            });
            if let Some(ls) = last_source {
                if ls.trim() == "end" && !ls.starts_with(' ') && !ls.starts_with('\t') {
                    regions.push(current);
                    current = Vec::new();
                    continue;
                }
            }
        }
        current.push(line);
    }
    if !current.is_empty() {
        regions.push(current);
    }

    // Check each region
    let mut line_offset = 0usize;
    for region in &regions {
        let mut has_annotation = false;
        let mut source_lines = 0usize;
        let mut first_source_line: Option<(usize, &str)> = None;

        for (j, line) in region.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if is_annotation_line(trimmed) {
                has_annotation = true;
            } else {
                source_lines += 1;
                if first_source_line.is_none() {
                    first_source_line = Some((line_offset + j + 1, trimmed));
                }
            }
        }

        if !has_annotation && source_lines >= 5 {
            if let Some((lineno, first)) = first_source_line {
                let display = if first.len() > 60 {
                    format!("{}...", &first[..60])
                } else {
                    first.to_string()
                };
                return Some(format!(
                    "unannotated block at line {lineno} with {source_lines} source lines ('{display}')"
                ));
            }
        }

        line_offset += region.len();
    }

    None
}

fn is_annotation_line(trimmed: &str) -> bool {
    (trimmed.starts_with('^') && trimmed.contains(": ") && trimmed.contains('/'))
        || trimmed.starts_with("# nitrocop-expect: ")
}

// ---------- M3 integration tests ----------

#[test]
fn metrics_cops_fire_on_complex_code() {
    let dir = temp_dir("metrics_complex");
    // 16-line method body exceeds default Max:10 for MethodLength
    let file = write_file(
        &dir,
        "complex.rb",
        b"# frozen_string_literal: true\n\ndef long_method\n  a = 1\n  b = 2\n  c = 3\n  d = 4\n  e = 5\n  f = 6\n  g = 7\n  h = 8\n  i = 9\n  j = 10\n  k = 11\n  l = 12\n  m = 13\n  n = 14\n  o = 15\n  p = 16\nend\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Metrics/MethodLength".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        !result.diagnostics.is_empty(),
        "Metrics/MethodLength should fire on 16-line method"
    );
    assert_eq!(result.diagnostics[0].cop_name, "Metrics/MethodLength");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn naming_cops_fire_on_bad_names() {
    let dir = temp_dir("naming_bad");
    // camelCase method name should trigger Naming/MethodName
    let file = write_file(
        &dir,
        "bad_names.rb",
        b"# frozen_string_literal: true\n\ndef myMethod\nend\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Naming/MethodName".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let cop_names: Vec<&str> = result
        .diagnostics
        .iter()
        .map(|d| d.cop_name.as_str())
        .collect();
    assert!(
        cop_names.contains(&"Naming/MethodName"),
        "Naming/MethodName should fire on camelCase method name"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_overrides_new_departments() {
    let dir = temp_dir("config_new_dept");
    // 4-line method body: under default Max:10 but over Max:3
    let file = write_file(
        &dir,
        "short_method.rb",
        b"# frozen_string_literal: true\n\ndef foo\n  a = 1\n  b = 2\n  c = 3\n  d = 4\nend\n",
    );
    let config_path = write_file(&dir, ".rubocop.yml", b"Metrics/MethodLength:\n  Max: 3\n");
    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Metrics/MethodLength".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        !result.diagnostics.is_empty(),
        "Metrics/MethodLength should fire with Max:3 on 4-line method"
    );
    assert_eq!(result.diagnostics[0].cop_name, "Metrics/MethodLength");
    assert!(
        result.diagnostics[0].message.contains("/3]"),
        "Message should reference Max:3"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- M10: Config inheritance tests ----------

#[test]
fn inherit_from_merges_configs() {
    let child_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config/inherit_from/child.yml");
    let config = load_config(Some(child_path.as_path()), None, None).unwrap();

    // Child overrides Layout/LineLength Max from 100 to 120
    let cc = config.cop_config("Layout/LineLength");
    assert_eq!(
        cc.options.get("Max").and_then(|v| v.as_u64()),
        Some(120),
        "Child should override base's Max:100 with Max:120"
    );

    // Child disables FrozenStringLiteralComment (base had it enabled)
    assert!(
        !config.is_cop_enabled(
            "Style/FrozenStringLiteralComment",
            Path::new("a.rb"),
            &[],
            &[]
        ),
        "Child should disable FrozenStringLiteralComment"
    );

    // Global excludes: RuboCop always merges AllCops.Exclude (union).
    // Child's AllCops.Exclude patterns are added to the base's patterns.
    let excludes = config.global_excludes();
    assert!(
        excludes.contains(&"vendor/**".to_string()),
        "Base's vendor/** should be preserved (AllCops.Exclude always merges)"
    );
    assert!(
        excludes.contains(&"tmp/**".to_string()),
        "Child's tmp/** exclude should be present"
    );
}

#[test]
fn circular_inherit_from_is_detected() {
    // A→B→A circular inheritance is safely broken: the second visit returns
    // an empty layer instead of recursing. This handles both true cycles and
    // diamond dependencies (e.g., standard's base.yml referenced from multiple paths).
    let circular_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config/inherit_from/circular_a.yml");
    let result = load_config(Some(circular_path.as_path()), None, None);
    assert!(
        result.is_ok(),
        "Circular inheritance should be safely broken, got: {result:?}"
    );
}

#[test]
fn diamond_dependency_does_not_error() {
    // diamond_root → diamond_left → diamond_base
    //             → diamond_right → diamond_base
    // diamond_base is visited twice from different paths. Should not error.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config/inherit_from/diamond_root.yml");
    let config = load_config(Some(path.as_path()), None, None)
        .expect("Diamond dependency should load successfully");
    // Verify the config chain loaded correctly — DisabledByDefault from base should apply
    assert!(
        !config.is_cop_enabled("Style/Foo", Path::new("a.rb"), &[], &[]),
        "DisabledByDefault from diamond_base.yml should disable unknown cops"
    );
    // But explicitly enabled cops from left/right branches should be on
    assert!(
        config.is_cop_enabled(
            "Style/FrozenStringLiteralComment",
            Path::new("a.rb"),
            &[],
            &[]
        ),
        "Style/FrozenStringLiteralComment enabled in diamond_left.yml should be on"
    );
    assert!(
        config.is_cop_enabled("Style/StringLiterals", Path::new("a.rb"), &[], &[]),
        "Style/StringLiterals enabled in diamond_right.yml should be on"
    );
}

// ---------- M10: --rubocop-only CLI tests ----------

#[test]
fn rubocop_only_outputs_uncovered_cops() {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config/rubocop_only/mixed.yml");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--rubocop-only", "--config", config_path.to_str().unwrap()])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--rubocop-only should exit 0, stderr: {stderr}"
    );

    // Should contain uncovered cops
    assert!(
        stdout.contains("Custom/MyCop"),
        "Output should contain Custom/MyCop, got: {stdout}"
    );
    assert!(
        stdout.contains("Vendor/SpecialCop"),
        "Output should contain Vendor/SpecialCop, got: {stdout}"
    );

    // Should NOT contain nitrocop-covered cops
    assert!(
        !stdout.contains("Style/FrozenStringLiteralComment"),
        "Output should NOT contain covered cop Style/FrozenStringLiteralComment"
    );
    assert!(
        !stdout.contains("Layout/TrailingWhitespace"),
        "Output should NOT contain covered cop Layout/TrailingWhitespace"
    );

    // Should NOT contain disabled cops
    assert!(
        !stdout.contains("Custom/DisabledCop"),
        "Output should NOT contain disabled cop Custom/DisabledCop"
    );
}

// ---------- M10: --stdin CLI tests ----------

#[test]
fn stdin_detects_trailing_whitespace() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--stdin",
            "test.rb",
            "--only",
            "Layout/TrailingWhitespace",
            "--preview",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start nitrocop");

    // Write source with trailing whitespace to stdin
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(b"x = 1   \n").unwrap();
    }

    let output = child
        .wait_with_output()
        .expect("Failed to wait for nitrocop");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !output.status.success(),
        "--stdin should exit 1 when offenses found"
    );
    assert!(
        stdout.contains("Layout/TrailingWhitespace"),
        "Should detect trailing whitespace via stdin, got: {stdout}"
    );
    assert!(
        stdout.contains("test.rb"),
        "Display path should be test.rb, got: {stdout}"
    );
}

#[test]
fn stdin_clean_code_exits_zero() {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--stdin",
            "clean.rb",
            "--only",
            "Layout/TrailingWhitespace",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start nitrocop");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(b"x = 1\ny = 2\n").unwrap();
    }

    let output = child
        .wait_with_output()
        .expect("Failed to wait for nitrocop");

    assert!(
        output.status.success(),
        "--stdin with clean code should exit 0"
    );
}

#[test]
fn stdin_display_path_affects_include_matching() {
    // RSpec cops should run when display path matches spec pattern.
    // Need a config dir with plugins: rubocop-rspec so the RSpec department is enabled.
    let config_dir = temp_dir("stdin_include_config");
    write_file(
        &config_dir,
        ".rubocop.yml",
        b"plugins:\n  - rubocop-rspec\n",
    );

    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--stdin",
            "spec/foo_spec.rb",
            "--only",
            "RSpec/Focus",
            "--preview",
        ])
        .current_dir(&config_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start nitrocop");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        stdin
            .write_all(b"RSpec.describe Foo, :focus do\n  it 'works' do\n    expect(1).to eq(1)\n  end\nend\n")
            .unwrap();
    }

    let output = child
        .wait_with_output()
        .expect("Failed to wait for nitrocop");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("RSpec/Focus"),
        "RSpec/Focus should fire when display path matches spec pattern, got: {stdout}"
    );

    // Same code with non-spec display path — RSpec cops should NOT run
    let mut child2 = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--stdin",
            "app/foo.rb",
            "--only",
            "RSpec/Focus",
            "--preview",
        ])
        .current_dir(&config_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to start nitrocop");

    {
        use std::io::Write;
        let stdin = child2.stdin.as_mut().unwrap();
        stdin
            .write_all(b"RSpec.describe Foo, :focus do\n  it 'works' do\n    expect(1).to eq(1)\n  end\nend\n")
            .unwrap();
    }

    let output2 = child2
        .wait_with_output()
        .expect("Failed to wait for nitrocop");
    let stdout2 = String::from_utf8_lossy(&output2.stdout);

    assert!(
        !stdout2.contains("RSpec/Focus"),
        "RSpec/Focus should NOT fire when display path is app/foo.rb, got: {stdout2}"
    );
}

// ---------- M10: Config inheritance with linter pipeline ----------

#[test]
fn inherited_config_affects_linting() {
    let dir = temp_dir("inherited_linting");
    // base.yml disables TrailingWhitespace
    write_file(
        &dir,
        "base.yml",
        b"Layout/TrailingWhitespace:\n  Enabled: false\n",
    );
    let config_path = write_file(&dir, ".rubocop.yml", b"inherit_from: base.yml\n");
    let file = write_file(&dir, "test.rb", b"x = 1   \n");

    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.diagnostics.is_empty(),
        "TrailingWhitespace should be disabled by inherited config, got {} offenses",
        result.diagnostics.len()
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn lint_source_directly() {
    use nitrocop::linter::lint_source;
    use nitrocop::parse::source::SourceFile;

    let source = SourceFile::from_string(PathBuf::from("test.rb"), "x = 1   \n".to_string());
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        preview: true,
        ..default_args()
    };

    let result = lint_source(
        &source,
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.file_count, 1);
    assert!(
        !result.diagnostics.is_empty(),
        "lint_source should detect trailing whitespace"
    );
    assert_eq!(result.diagnostics[0].cop_name, "Layout/TrailingWhitespace");
    assert_eq!(result.diagnostics[0].path, "test.rb");
}

// ---------- --list-cops CLI tests ----------

#[test]
fn list_cops_prints_all_registered_cops() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--list-cops"])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "--list-cops should exit 0, stderr: {stderr}"
    );

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        920,
        "Expected 920 cop names, got {}",
        lines.len()
    );

    // Verify sorted order
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(lines, sorted, "--list-cops output should be sorted");

    // Spot-check a few cops from different departments
    assert!(
        lines.contains(&"Layout/TrailingWhitespace"),
        "Should contain Layout/TrailingWhitespace"
    );
    assert!(
        lines.contains(&"Style/FrozenStringLiteralComment"),
        "Should contain Style/FrozenStringLiteralComment"
    );
    assert!(
        lines.contains(&"Lint/Debugger"),
        "Should contain Lint/Debugger"
    );
    assert!(
        lines.contains(&"Performance/Detect"),
        "Should contain Performance/Detect"
    );
    assert!(
        lines.contains(&"Rails/FindBy"),
        "Should contain Rails/FindBy"
    );
    assert!(lines.contains(&"RSpec/Focus"), "Should contain RSpec/Focus");
}

// ---------- Config audit ----------

#[test]
fn config_audit() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = CopRegistry::default_registry();

    // Parse each vendor default YAML
    let yaml_paths = [
        manifest.join("vendor/rubocop/config/default.yml"),
        manifest.join("vendor/rubocop-performance/config/default.yml"),
        manifest.join("vendor/rubocop-rails/config/default.yml"),
        manifest.join("vendor/rubocop-rspec/config/default.yml"),
    ];

    let infrastructure_keys: std::collections::HashSet<&str> = [
        "Enabled",
        "Description",
        "StyleGuide",
        "Reference",
        "References",
        "VersionAdded",
        "VersionChanged",
        "SafeAutoCorrect",
        "AutoCorrect",
        "SupportedStyles",
        "SupportedStylesForMultiline",
        "Exclude",
        "Include",
        "Safe",
        "DocumentationBaseURL",
        "inherit_mode",
        "Severity",
    ]
    .into_iter()
    .collect();

    // Also filter Supported* prefix keys (e.g. SupportedStylesAlignWith)
    let is_infrastructure =
        |key: &str| -> bool { infrastructure_keys.contains(key) || key.starts_with("Supported") };

    // Build a map of cop name -> YAML child keys (config options only)
    let mut yaml_cop_keys: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    for yaml_path in &yaml_paths {
        let content = fs::read_to_string(yaml_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", yaml_path.display()));
        let doc: serde_yml::Value = serde_yml::from_str(&content)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", yaml_path.display()));

        if let serde_yml::Value::Mapping(map) = doc {
            for (key, value) in &map {
                let key_str = match key.as_str() {
                    Some(s) => s,
                    None => continue,
                };
                // Only cop names contain '/'
                if !key_str.contains('/') {
                    continue;
                }
                if let serde_yml::Value::Mapping(child_map) = value {
                    let config_keys: Vec<String> = child_map
                        .keys()
                        .filter_map(|k| k.as_str())
                        .filter(|k| !is_infrastructure(k))
                        .map(|k| k.to_string())
                        .collect();
                    if !config_keys.is_empty() {
                        yaml_cop_keys.insert(key_str.to_string(), config_keys);
                    }
                }
            }
        }
    }

    // For each cop in the registry, check which YAML keys the Rust source reads
    let mut current_gaps: Vec<String> = Vec::new();

    for cop_name in registry.names() {
        let yaml_keys = match yaml_cop_keys.get(cop_name) {
            Some(keys) => keys,
            None => continue, // No YAML config for this cop
        };

        let parts: Vec<&str> = cop_name.split('/').collect();
        let dept = parts[0].to_lowercase();
        let name = to_snake_case(parts[1]);

        // Try to find the source file
        let src_path = manifest.join(format!("src/cop/{dept}/{name}.rs"));
        let src_path_alt = manifest.join(format!("src/cop/{dept}/{name}_cop.rs"));
        let source = if src_path.exists() {
            fs::read_to_string(&src_path).unwrap()
        } else if src_path_alt.exists() {
            fs::read_to_string(&src_path_alt).unwrap()
        } else {
            continue; // Source not found, skip
        };

        let mut missing: Vec<&str> = Vec::new();
        for key in yaml_keys {
            // Check if the Rust source references this config key via any get_ method
            let pattern = format!("\"{key}\"");
            if !source.contains(&pattern) {
                missing.push(key);
            }
        }

        if !missing.is_empty() {
            current_gaps.push(format!("{cop_name}: {}", missing.join(", ")));
        }
    }

    current_gaps.sort();

    assert!(
        current_gaps.is_empty(),
        "\n[config_audit] {} cop(s) have YAML config keys not referenced in Rust source:\n{}\n",
        current_gaps.len(),
        current_gaps
            .iter()
            .map(|g| format!("  {g}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------- Prism pitfalls ----------

#[test]
fn prism_pitfalls() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let registry = CopRegistry::default_registry();

    let mut current_gaps: Vec<String> = Vec::new();

    for cop_name in registry.names() {
        let parts: Vec<&str> = cop_name.split('/').collect();
        let dept = parts[0].to_lowercase();
        let name = to_snake_case(parts[1]);

        let src_path = manifest.join(format!("src/cop/{dept}/{name}.rs"));
        let src_path_alt = manifest.join(format!("src/cop/{dept}/{name}_cop.rs"));
        let source = if src_path.exists() {
            fs::read_to_string(&src_path).unwrap()
        } else if src_path_alt.exists() {
            fs::read_to_string(&src_path_alt).unwrap()
        } else {
            continue;
        };

        // Pitfall 1: KeywordHashNode gap
        if source.contains("as_hash_node") && !source.contains("keyword_hash_node") {
            current_gaps.push(format!(
                "{cop_name}: handles Hash literals (as_hash_node) but misses keyword arguments (keyword_hash_node)"
            ));
        }

        // Pitfall 2: ConstantPathNode gap
        if source.contains("as_constant_read_node") && !source.contains("constant_path_node") {
            current_gaps.push(format!(
                "{cop_name}: handles simple constants (as_constant_read_node) but misses qualified constants (constant_path_node)"
            ));
        }
    }

    current_gaps.sort();

    assert!(
        current_gaps.is_empty(),
        "\n[prism_pitfalls] {} pitfall(s) found:\n{}\n",
        current_gaps.len(),
        current_gaps
            .iter()
            .map(|g| format!("  {g}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------- File discovery sync ----------

/// Zero-tolerance test: RUBY_EXTENSIONS and RUBY_FILENAMES in fs.rs must match
/// AllCops.Include from vendor/rubocop/config/default.yml.
///
/// Prevents regressions like removing "spec" from extensions (which caused 89 cops
/// to lose exact-match status across the corpus).
#[test]
fn file_discovery_sync() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let default_yml = manifest.join("vendor/rubocop/config/default.yml");
    let fs_rs = manifest.join("src/fs.rs");

    let yml_content = fs::read_to_string(&default_yml)
        .unwrap_or_else(|e| panic!("Cannot read {}: {e}", default_yml.display()));
    let fs_source = fs::read_to_string(&fs_rs)
        .unwrap_or_else(|e| panic!("Cannot read {}: {e}", fs_rs.display()));

    // --- Parse AllCops.Include from YAML ---
    // Patterns are like: '**/*.rb', '**/.irbrc', '**/Gemfile', '**/*Fastfile'
    let mut yaml_extensions: Vec<String> = Vec::new();
    let mut yaml_filenames: Vec<String> = Vec::new();
    let mut yaml_dynamic: Vec<String> = Vec::new();

    let mut in_include = false;
    for line in yml_content.lines() {
        let trimmed = line.trim();
        if trimmed == "Include:" {
            in_include = true;
            continue;
        }
        if in_include {
            if !trimmed.starts_with("- ") {
                break; // End of Include list
            }
            // Strip `- '...'` wrapper
            let pattern = trimmed
                .trim_start_matches("- ")
                .trim_matches('\'')
                .trim_matches('"');

            if let Some(ext) = pattern.strip_prefix("**/*.") {
                yaml_extensions.push(ext.to_string());
            } else if let Some(name) = pattern.strip_prefix("**/") {
                if name.contains('*') {
                    yaml_dynamic.push(pattern.to_string());
                } else {
                    yaml_filenames.push(name.to_string());
                }
            }
        }
    }

    // --- Parse RUBY_EXTENSIONS from fs.rs ---
    let rust_extensions: Vec<String> = fs_source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('"') && trimmed.ends_with("\",") {
                Some(
                    trimmed
                        .trim_matches('"')
                        .trim_end_matches(',')
                        .trim_matches('"')
                        .to_string(),
                )
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    // Extract just the extensions section (between RUBY_EXTENSIONS and the next const)
    let ext_section_start = fs_source
        .find("const RUBY_EXTENSIONS")
        .expect("RUBY_EXTENSIONS not found in fs.rs");
    let ext_section_end = fs_source[ext_section_start..].find("];").unwrap() + ext_section_start;
    let ext_section = &fs_source[ext_section_start..ext_section_end];
    let rust_extensions: Vec<String> = ext_section
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim().trim_end_matches(',');
            if trimmed.starts_with('"') && trimmed.ends_with('"') {
                Some(trimmed.trim_matches('"').to_string())
            } else {
                None
            }
        })
        .collect();

    // --- Parse RUBY_FILENAMES from fs.rs ---
    let fn_section_start = fs_source
        .find("const RUBY_FILENAMES")
        .expect("RUBY_FILENAMES not found in fs.rs");
    let fn_section_end = fs_source[fn_section_start..].find("];").unwrap() + fn_section_start;
    let fn_section = &fs_source[fn_section_start..fn_section_end];
    let rust_filenames: Vec<String> = fn_section
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim().trim_end_matches(',');
            if trimmed.starts_with('"') && trimmed.ends_with('"') {
                Some(trimmed.trim_matches('"').to_string())
            } else {
                None
            }
        })
        .collect();

    // --- Compare ---
    let mut gaps: Vec<String> = Vec::new();

    // Extensions in YAML but missing from Rust
    for ext in &yaml_extensions {
        if !rust_extensions.iter().any(|r| r.eq_ignore_ascii_case(ext)) {
            gaps.push(format!(
                "RUBY_EXTENSIONS missing '**/*.{ext}' from AllCops.Include"
            ));
        }
    }
    // Extensions in Rust but not in YAML (extra is less critical but worth flagging)
    for ext in &rust_extensions {
        if !yaml_extensions.iter().any(|y| y.eq_ignore_ascii_case(ext)) {
            gaps.push(format!(
                "RUBY_EXTENSIONS has '{ext}' not in AllCops.Include"
            ));
        }
    }

    // Filenames in YAML but missing from Rust (skip dynamic patterns like *Fastfile)
    for name in &yaml_filenames {
        if !rust_filenames.contains(name) {
            // Check if it's covered by the dynamic *Fastfile pattern in code
            if name.ends_with("Fastfile") || name.ends_with("fastfile") {
                continue;
            }
            gaps.push(format!(
                "RUBY_FILENAMES missing '**/{}' from AllCops.Include",
                name
            ));
        }
    }
    // Filenames in Rust but not in YAML
    for name in &rust_filenames {
        if !yaml_filenames.contains(name) {
            gaps.push(format!(
                "RUBY_FILENAMES has '{name}' not in AllCops.Include"
            ));
        }
    }

    gaps.sort();

    assert!(
        gaps.is_empty(),
        "\n[file_discovery_sync] {} gap(s) between fs.rs and vendor/rubocop/config/default.yml AllCops.Include:\n{}\n",
        gaps.len(),
        gaps.iter()
            .map(|g| format!("  {g}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------- Ruby version gates ----------

/// Convert a snake_case string to CamelCase (e.g. "it_block_parameter" -> "ItBlockParameter").
fn to_camel_case(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    let mut result = c.to_uppercase().to_string();
                    result.extend(chars);
                    result
                }
                None => String::new(),
            }
        })
        .collect()
}

/// Capitalize a department name from file path form to display form.
/// e.g. "style" -> "Style", "lint" -> "Lint", "rspec" -> "RSpec"
fn capitalize_dept(dept: &str) -> &str {
    match dept {
        "bundler" => "Bundler",
        "gemspec" => "Gemspec",
        "layout" => "Layout",
        "lint" => "Lint",
        "metrics" => "Metrics",
        "migration" => "Migration",
        "naming" => "Naming",
        "performance" => "Performance",
        "rails" => "Rails",
        "rspec" => "RSpec",
        "security" => "Security",
        "style" => "Style",
        _ => dept, // fallback
    }
}

/// Extract the version number from a vendor Ruby file's version gate declaration.
/// Looks for patterns like `minimum_target_ruby_version 3.4` or `maximum_target_ruby_version 2.7`.
fn extract_ruby_version(content: &str, gate_type: &str) -> String {
    let pattern = format!("{gate_type}_target_ruby_version");
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&pattern) {
            let version = rest.trim();
            if !version.is_empty() {
                return version.to_string();
            }
        }
    }
    "?".to_string()
}

/// Recursively collect all `.rb` files under a directory.
fn collect_rb_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rb_files(&path, files);
            } else if path.extension().map_or(false, |e| e == "rb") {
                files.push(path);
            }
        }
    }
}

/// Ensure every nitrocop cop that has a `minimum_target_ruby_version` or
/// `maximum_target_ruby_version` in the vendor RuboCop source also has a
/// corresponding `TargetRubyVersion` check in its Rust implementation.
///
/// This is a zero-tolerance test like `config_audit` and `prism_pitfalls`.
/// Cops in the `KNOWN_MISSING` allowlist are temporarily exempt — the goal
/// is to empty this list over time.
#[test]
fn ruby_version_gates() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_cop_dir = manifest.join("src/cop");

    // Vendor cop directories to scan (main rubocop + plugins)
    let vendor_cop_dirs: Vec<(PathBuf, &str)> = vec![
        (
            manifest.join("vendor/rubocop/lib/rubocop/cop"),
            "vendor/rubocop",
        ),
        (
            manifest.join("vendor/rubocop-performance/lib/rubocop/cop"),
            "vendor/rubocop-performance",
        ),
        (
            manifest.join("vendor/rubocop-rails/lib/rubocop/cop"),
            "vendor/rubocop-rails",
        ),
    ];

    // Known-missing cops that don't yet have TargetRubyVersion checks in their
    // Rust implementation. This allowlist should shrink over time as cops are fixed.
    // When adding a version gate to a cop, remove it from this list.
    let known_missing: std::collections::HashSet<&str> = [
        "Layout/HeredocIndentation",
        "Lint/DuplicateMatchPattern",
        "Lint/EmptyInPattern",
        "Lint/ErbNewArguments",
        // No-op cops — obsolete under modern Ruby, kept for config compatibility
        "Lint/ItWithoutArgumentsInBlock",
        "Lint/NonDeterministicRequireOrder",
        "Lint/SafeNavigationChain",
        "Lint/SuppressedExceptionInNumberConversion",
        "Lint/UselessElseWithoutRescue",
        "Performance/ArraySemiInfiniteRangeSlice",
        "Performance/BigDecimalWithNumericArgument",
        "Performance/BindCall",
        "Performance/DeletePrefix",
        "Performance/DeleteSuffix",
        "Performance/MapCompact",
        "Performance/RedundantEqualityComparisonBlock",
        "Performance/SelectMap",
        "Performance/Sum",
        "Performance/UnfreezeString",
        "Rails/EnumSyntax",
        "Rails/SafeNavigation",
        "Rails/StripHeredoc",
        "Rails/WhereRange",
        "Style/BitwisePredicate",
        "Style/CollectionCompact",
        "Style/ComparableClamp",
        "Style/DataInheritance",
        "Style/Dir",
        "Style/DirEmpty",
        "Style/FileEmpty",
        "Style/HashExcept",
        "Style/HashFetchChain",
        "Style/HashSlice",
        "Style/HashTransformKeys",
        "Style/HashTransformValues",
        "Style/InPatternThen",
        "Style/MultilineInPatternThen",
        "Style/NumberedParameters",
        "Style/NumberedParametersLimit",
        "Style/ObjectThen",
        "Style/SafeNavigation",
        "Style/SlicingWithRange",
        "Style/SymbolArray",
        "Style/UnpackFirst",
        // No-op cop — YAML.load is safe since Ruby 3.1
        "Security/YamlLoad",
    ]
    .into_iter()
    .collect();

    let mut failures = Vec::new();
    let mut known_missing_found = std::collections::HashSet::new();

    for (vendor_cop_dir, _vendor_label) in &vendor_cop_dirs {
        if !vendor_cop_dir.exists() {
            eprintln!(
                "Skipping {}: vendor directory not found (run `git submodule update --init`)",
                vendor_cop_dir.display()
            );
            continue;
        }

        let mut rb_files = Vec::new();
        collect_rb_files(vendor_cop_dir, &mut rb_files);

        for path in &rb_files {
            // Skip the mixin definition file
            if path
                .to_str()
                .unwrap_or("")
                .contains("mixin/target_ruby_version")
            {
                continue;
            }

            let content = fs::read_to_string(path).unwrap();

            let has_min = content.contains("minimum_target_ruby_version");
            let has_max = content.contains("maximum_target_ruby_version");
            if !has_min && !has_max {
                continue;
            }

            // Extract department and cop name from path.
            // e.g. vendor/rubocop/lib/rubocop/cop/style/it_block_parameter.rb
            //   -> relative to vendor_cop_dir: style/it_block_parameter.rb
            let relative = match path.strip_prefix(vendor_cop_dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let rel_str = relative.to_str().unwrap_or("");
            let rel_no_ext = rel_str.trim_end_matches(".rb");
            let components: Vec<&str> = rel_no_ext.split('/').collect();
            if components.len() != 2 {
                continue; // Skip non-standard paths (e.g. mixin subdirectories)
            }
            let dept = components[0];
            let snake_name = components[1];

            // Build the display cop name (e.g. "Style/ItBlockParameter")
            let cop_name = format!("{}/{}", capitalize_dept(dept), to_camel_case(snake_name));

            // Find corresponding Rust file (try both plain and _cop suffix)
            let rust_file = src_cop_dir.join(dept).join(format!("{snake_name}.rs"));
            let rust_file_alt = src_cop_dir.join(dept).join(format!("{snake_name}_cop.rs"));
            let rust_path = if rust_file.exists() {
                rust_file
            } else if rust_file_alt.exists() {
                rust_file_alt
            } else {
                continue; // Cop not implemented in nitrocop yet
            };

            let rust_content = fs::read_to_string(&rust_path).unwrap();
            if !rust_content.contains("TargetRubyVersion") {
                if known_missing.contains(cop_name.as_str()) {
                    known_missing_found.insert(cop_name.clone());
                    continue; // Allowed for now
                }

                let gate_type = if has_min { "minimum" } else { "maximum" };
                let version = extract_ruby_version(&content, gate_type);

                failures.push(format!(
                    "{cop_name}: vendor has {gate_type}_target_ruby_version {version} \
                     but {} has no TargetRubyVersion check",
                    rust_path
                        .strip_prefix(&manifest)
                        .unwrap_or(&rust_path)
                        .display()
                ));
            }
        }
    }

    // Check for stale entries in the known_missing list (cops that have been fixed
    // but not removed from the allowlist).
    let mut stale: Vec<&&str> = known_missing
        .iter()
        .filter(|name| !known_missing_found.contains(**name))
        .collect();
    stale.sort();

    // Only flag stale entries if the cop's Rust file exists (otherwise it's just
    // not implemented yet, which is fine to keep in the allowlist for when it is).
    let stale_with_files: Vec<&&str> = stale
        .into_iter()
        .filter(|name| {
            let parts: Vec<&str> = name.split('/').collect();
            if parts.len() != 2 {
                return false;
            }
            let dept = parts[0].to_lowercase();
            let snake = to_snake_case(parts[1]);
            let p1 = src_cop_dir.join(&dept).join(format!("{snake}.rs"));
            let p2 = src_cop_dir.join(&dept).join(format!("{snake}_cop.rs"));
            p1.exists() || p2.exists()
        })
        .collect();

    if !stale_with_files.is_empty() {
        eprintln!(
            "\n[ruby_version_gates] {} stale known_missing entries (cop now has TargetRubyVersion — remove from allowlist):",
            stale_with_files.len()
        );
        for name in &stale_with_files {
            eprintln!("  {name}");
        }
    }

    failures.sort();

    assert!(
        failures.is_empty(),
        "\n[ruby_version_gates] {} cop(s) missing TargetRubyVersion gates:\n{}\n\n\
         Each of these cops has a minimum_target_ruby_version or maximum_target_ruby_version \
         in the vendor RuboCop source but no corresponding TargetRubyVersion check in its \
         Rust implementation. Add the check, or (temporarily) add the cop to the known_missing \
         allowlist in this test.",
        failures.len(),
        failures
            .iter()
            .map(|f| format!("  {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    );

    assert!(
        stale_with_files.is_empty(),
        "\n[ruby_version_gates] {} stale known_missing entries — these cops now have \
         TargetRubyVersion checks and should be removed from the allowlist:\n{}\n",
        stale_with_files.len(),
        stale_with_files
            .iter()
            .map(|f| format!("  {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ---------- NodePattern codegen integration tests ----------

#[test]
fn codegen_generates_output_for_vendor_file() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let vendor_file = manifest.join("vendor/rubocop-rails/lib/rubocop/cop/rails/inverse_of.rb");

    if !vendor_file.exists() {
        eprintln!("Skipping codegen integration test: vendor file not found");
        return;
    }

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_node_pattern_codegen"))
        .args(["generate", vendor_file.to_str().unwrap()])
        .output()
        .expect("Failed to execute node_pattern_codegen");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "codegen should exit 0 for a file with patterns, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // inverse_of.rb has multiple def_node_matcher patterns
    assert!(
        stdout.contains("Auto-generated by node_pattern_codegen"),
        "Output should contain header comment"
    );
    assert!(
        stdout.contains("fn "),
        "Output should contain generated Rust functions"
    );
    assert!(
        stdout.contains("as_call_node") || stdout.contains("as_"),
        "Output should contain Prism cast methods"
    );
    // Should extract several patterns
    assert!(
        stdout.matches("def_node_matcher").count() >= 3,
        "Should reference at least 3 patterns from inverse_of.rb"
    );
}

#[test]
fn codegen_handles_file_with_no_patterns() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Use a file that likely has no def_node_matcher
    let test_file = manifest.join("tests/fixtures/cops/layout/trailing_whitespace/offense.rb");

    if !test_file.exists() {
        eprintln!("Skipping codegen no-patterns test: test file not found");
        return;
    }

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_node_pattern_codegen"))
        .args(["generate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to execute node_pattern_codegen");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "codegen should exit 0 even with no patterns"
    );
    assert!(
        stdout.is_empty(),
        "No output expected for file without patterns, got: {stdout}"
    );
    assert!(
        stderr.contains("No def_node_matcher"),
        "Should print informational message to stderr, got: {stderr}"
    );
}

#[test]
fn codegen_exits_nonzero_for_missing_file() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_node_pattern_codegen"))
        .args(["generate", "/nonexistent/file.rb"])
        .output()
        .expect("Failed to execute node_pattern_codegen");

    assert!(
        !output.status.success(),
        "codegen should exit non-zero for missing file"
    );
}

#[test]
fn codegen_exits_nonzero_without_args() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_node_pattern_codegen"))
        .output()
        .expect("Failed to execute node_pattern_codegen");

    assert!(
        !output.status.success(),
        "codegen should exit non-zero without arguments"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Usage"),
        "Should print usage, got: {stderr}"
    );
}

// ---------- Lint/RedundantCopDisableDirective tests ----------
//
// This cop requires the full linter pipeline because it detects disable
// directives that didn't suppress any offense. It can't use the standard
// fixture test framework since that runs a single cop in isolation.
//
// The implementation is conservative: it only flags directives for cops
// that are UNKNOWN (not in the registry or any loaded plugin config),
// DISABLED, EXCLUDED, or RENAMED. It does NOT flag directives for cops
// that ARE running, since nitrocop might have gaps in its detection vs.
// RuboCop.

#[test]
fn redundant_disable_unknown_cop_in_known_department() {
    // Disable for a cop name that doesn't exist in a known department.
    // RuboCop flags these with "(unknown cop)".
    let dir = temp_dir("redundant_disable_known_dept");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable Style/NonexistentFakeCop\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Should flag unknown cop, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("(unknown cop)"),
        "Message should include '(unknown cop)': {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_unknown_department() {
    // Disable for a cop in a completely unknown department — flag with "(unknown cop)".
    let dir = temp_dir("redundant_disable_unknown_dept");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable CustomDept/CustomCop\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Should flag unknown department cop, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("(unknown cop)"),
        "Message should include '(unknown cop)': {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_running_cop_no_offense_flagged_in_normal_mode() {
    // In normal mode (no --only/--except), all cops run. If a cop ran and
    // didn't fire, its unused disable directive IS redundant.
    let dir = temp_dir("redundant_disable_running_clean");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable Layout/TrailingWhitespace\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Should flag unused disable for a running cop in normal mode, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("Layout/TrailingWhitespace"),
        "Message should mention the cop: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_for_stub_cop_in_normal_mode() {
    // Security/YAMLLoad is a stub cop that never fires. Its disable
    // directives should be flagged as redundant since the cop ran (as a
    // no-op) and produced no offenses.
    let dir = temp_dir("redundant_disable_stub_cop");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nYAML.load(data) # rubocop:disable Security/YAMLLoad\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Should flag disable for stub cop that never fires, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("Security/YAMLLoad"),
        "Message should mention the cop: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_block_yaml_load() {
    // Block-style disable for a stub cop should also be flagged.
    let dir = temp_dir("redundant_disable_block_yaml_load");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\n# rubocop:disable Security/YAMLLoad\nYAML.load(data)\n# rubocop:enable Security/YAMLLoad\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Should flag block disable for stub cop, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_with_except_filter() {
    // With --except, the excepted cop didn't run, so we can't tell if the
    // disable directive is needed. Should NOT flag.
    let dir = temp_dir("redundant_disable_except");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable Layout/TrailingWhitespace\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let mut args = default_args();
    args.except = vec!["Layout/TrailingWhitespace".to_string()];

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert!(
        redundant.is_empty(),
        "Should NOT flag disable when cop is in --except, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_when_offense_suppressed() {
    // Block disable around a line with trailing whitespace -> used, not redundant.
    let dir = temp_dir("redundant_disable_used");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\n# rubocop:disable Layout/TrailingWhitespace\nx = 1   \n# rubocop:enable Layout/TrailingWhitespace\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert!(
        redundant.is_empty(),
        "Should NOT report redundant disable when it actually suppresses an offense, got: {:?}",
        redundant
    );

    // Also verify the trailing whitespace offense was suppressed
    let tw: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Layout/TrailingWhitespace")
        .collect();
    assert!(
        tw.is_empty(),
        "TrailingWhitespace should be suppressed by disable directive"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_with_only_flag() {
    // Even in an unrelated --only run, a directive for a known disabled cop is
    // still redundant because config state alone proves it cannot suppress anything.
    let dir = temp_dir("redundant_disable_only");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable Style/Copyright\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Should report redundant disable for disabled cop even with --only, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn unknown_redundant_disable_does_not_leak_into_unrelated_only_run() {
    let dir = temp_dir("redundant_disable_only_unknown");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable CustomDept/CustomCop\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert!(
        redundant.is_empty(),
        "Unknown-cop redundant disable should not leak into unrelated --only runs, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_only_mode_flags_non_skipped_enabled_cop() {
    let dir = temp_dir("redundant_disable_only_non_skipped");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n# rubocop:disable Style/SymbolProc\nx = 1\n# rubocop:enable Style/SymbolProc\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Lint/RedundantCopDisableDirective".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Expected run-all redundant mode to flag Style/SymbolProc, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("Style/SymbolProc"),
        "Message should mention the redundant cop: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_only_mode_skips_known_gap_cop() {
    let dir = temp_dir("redundant_disable_only_known_gap");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\nlistener = Object.new\nif listener # rubocop:disable Style/SafeNavigation\n  listener.to_s\nend\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Lint/RedundantCopDisableDirective".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert!(
        redundant.is_empty(),
        "Known-gap cop should stay on the denylist in run-all redundant mode, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_disabled_cop() {
    // Disable for a cop that is explicitly disabled in config.
    // Style/Copyright is disabled by default (default_enabled = false).
    // A disable directive for it is redundant since the cop doesn't run.
    let dir = temp_dir("redundant_disable_disabled_cop");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable Style/Copyright\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Expected 1 redundant disable for disabled cop Style/Copyright, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("Style/Copyright"),
        "Message should mention the cop name: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_department_used() {
    // Department-level disables are never flagged (conservative)
    let dir = temp_dir("redundant_disable_dept_used");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1   # rubocop:disable Layout\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert!(
        redundant.is_empty(),
        "Department disable should NOT be redundant, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_todo_directive() {
    // rubocop:todo is treated the same as disable — disabled cop is flagged
    let dir = temp_dir("redundant_disable_todo");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:todo Style/Copyright\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "rubocop:todo for disabled cop should be flagged, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_all() {
    // `disable all` is never flagged as redundant (too broad to check)
    let dir = temp_dir("redundant_disable_all_not_flagged");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\n# rubocop:disable all\nx = 1\n# rubocop:enable all\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        except: vec!["Style/DisableCopsWithinSourceCodeDirective".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert!(
        redundant.is_empty(),
        "disable all should NOT be flagged as redundant (conservative), got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_renamed_cop_extended_format() {
    // Naming/PredicateName was renamed to Naming/PredicatePrefix (extended format
    // in obsoletion.yml with new_name key). The short name changed
    // (PredicateName -> PredicatePrefix), so RuboCop does NOT honor the old name
    // as a disable for the new cop. nitrocop should match this behavior.
    let dir = temp_dir("redundant_disable_renamed_cop");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\ndef is_foo? # rubocop:disable Naming/PredicateName\n  true\nend\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // The old name should NOT suppress the new cop (short name changed),
    // so the PredicatePrefix offense should still be reported.
    let prefix_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Naming/PredicatePrefix")
        .collect();
    assert!(
        !prefix_offenses.is_empty(),
        "Old name with changed short name should NOT suppress new cop offense"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_moved_cop_same_short_name() {
    // Lint/Eval moved to Security/Eval but kept the short name. RuboCop still
    // qualifies the legacy name in inline directives, so the offense should be
    // suppressed and the directive should not be redundant.
    let dir = temp_dir("redundant_disable_moved_cop_same_short_name");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\neval(user_input) # rubocop:disable Lint/Eval\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let eval_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Security/Eval")
        .collect();
    assert!(
        eval_offenses.is_empty(),
        "Moved legacy name should suppress Security/Eval, got: {:?}",
        eval_offenses
    );

    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();
    assert!(
        redundant.is_empty(),
        "Moved legacy directive should not be redundant, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_renamed_cop_simple_value() {
    // Layout/Tab was renamed to Layout/IndentationStyle (simple key: value format
    // in obsoletion.yml). A disable directive for the old name should be flagged.
    let dir = temp_dir("redundant_disable_renamed_simple");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\nx = 1 # rubocop:disable Layout/Tab\n",
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Expected 1 redundant disable for renamed cop Layout/Tab, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("Layout/Tab"),
        "Message should mention the old cop name: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

/// Ensure no cop still uses the old `-> Vec<Diagnostic>` return type for trait methods.
///
/// The Cop trait methods (check_lines, check_source, check_node) now take
/// `diagnostics: &mut Vec<Diagnostic>` and return `()`. This test scans all cop
/// source files to catch any new cop that accidentally uses the old signature pattern.
#[test]
fn no_cop_returns_vec_diagnostic() {
    let cop_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/cop");
    let mut failures = Vec::new();

    let skip_files = [
        "mod.rs",
        "walker.rs",
        "node_type.rs",
        "registry.rs",
        "util.rs",
    ];

    fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_rs_files(&path, files);
                } else if path.extension().map_or(false, |e| e == "rs") {
                    files.push(path);
                }
            }
        }
    }

    let mut rs_files = Vec::new();
    collect_rs_files(&cop_dir, &mut rs_files);

    for path in &rs_files {
        let filename = path.file_name().unwrap().to_str().unwrap();
        if skip_files.contains(&filename) {
            continue;
        }

        let content = fs::read_to_string(path).unwrap();

        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if (trimmed.contains("fn check_node(")
                || trimmed.contains("fn check_lines(")
                || trimmed.contains("fn check_source("))
                && !trimmed.starts_with("//")
            {
                let remaining: String = content
                    .lines()
                    .skip(i)
                    .take(10)
                    .collect::<Vec<_>>()
                    .join(" ");
                if remaining.contains("-> Vec<Diagnostic>") {
                    let rel = path.strip_prefix(&cop_dir).unwrap().display();
                    failures.push(format!(
                        "{rel}:{}: trait method still returns Vec<Diagnostic> — use `diagnostics: &mut Vec<Diagnostic>` parameter instead",
                        i + 1
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Cops using old Vec<Diagnostic> return pattern (should use &mut Vec<Diagnostic> parameter):\n{}",
        failures.join("\n")
    );
}

/// Ensure non-test source code does not include compile-time vendor file paths.
///
/// Vendor submodules are not guaranteed to exist for crate consumers, so
/// `include_str!/include_bytes!` must not point at `vendor/` in `src/**/*.rs`.
#[test]
fn no_vendor_include_macros_in_src() {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut failures = Vec::new();

    fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_rs_files(&path, files);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
    }

    let mut rs_files = Vec::new();
    collect_rs_files(&src_dir, &mut rs_files);

    let include_macro_re = regex::Regex::new(r#"include_(?:str|bytes)!\s*\((?s:.*?)\)"#).unwrap();
    for path in rs_files {
        let content = fs::read_to_string(&path).unwrap();
        for m in include_macro_re.find_iter(&content) {
            let snippet = &content[m.start()..m.end()];
            if snippet.contains("vendor/") {
                let line = content[..m.start()].bytes().filter(|&b| b == b'\n').count() + 1;
                failures.push(format!("{}:{line}", path.display()));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Found include_str!/include_bytes! macros referencing vendor/ in src code:\n{}",
        failures.join("\n")
    );
}

// ---------- Result cache integration tests ----------

#[test]
fn cache_produces_same_results_as_uncached() {
    let dir = temp_dir("cache_same_results");
    // File with offenses
    let file1 = write_file(&dir, "trailing.rb", b"x = 1 \ny = 2\n");
    // File without offenses
    let file2 = write_file(&dir, "clean.rb", b"x = 1\ny = 2\n");

    let config = load_config(None, Some(dir.as_path()), None).unwrap();
    let registry = CopRegistry::default_registry();

    // Run without cache
    let args_no_cache = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        cache: "false".to_string(),
        preview: true,
        ..default_args()
    };
    let result_no_cache = run_linter(
        &discovered(&[file1.clone(), file2.clone()]),
        &config,
        &registry,
        &args_no_cache,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // Run with cache (cold)
    let cache_dir = dir.join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    // SAFETY: test-only, set env var for cache root
    unsafe { std::env::set_var("NITROCOP_CACHE_DIR", &cache_dir) };
    let args_cached = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        cache: "true".to_string(),
        preview: true,
        ..default_args()
    };
    let result_cold = run_linter(
        &discovered(&[file1.clone(), file2.clone()]),
        &config,
        &registry,
        &args_cached,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // Run with cache (warm)
    let result_warm = run_linter(
        &discovered(&[file1.clone(), file2.clone()]),
        &config,
        &registry,
        &args_cached,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    unsafe { std::env::remove_var("NITROCOP_CACHE_DIR") };

    // All three runs should produce identical diagnostics
    assert_eq!(
        result_no_cache.diagnostics.len(),
        result_cold.diagnostics.len(),
        "Cold cache should produce same offense count as uncached"
    );
    assert_eq!(
        result_no_cache.diagnostics.len(),
        result_warm.diagnostics.len(),
        "Warm cache should produce same offense count as uncached"
    );

    // Verify actual offenses match
    for (d1, d2) in result_no_cache
        .diagnostics
        .iter()
        .zip(result_warm.diagnostics.iter())
    {
        assert_eq!(d1.cop_name, d2.cop_name);
        assert_eq!(d1.location.line, d2.location.line);
        assert_eq!(d1.location.column, d2.location.column);
        assert_eq!(d1.message, d2.message);
    }

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn no_cache_flag_disables_result_cache_writes() {
    let dir = temp_dir("no_cache_disables_result_cache");
    write_file(&dir, "trailing.rb", b"x = 1 \n");

    let cache_dir = dir.join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_CACHE_DIR", &cache_dir)
        .args([
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--format",
            "json",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    assert!(
        output.status.code().is_some_and(|code| code != 3),
        "nitrocop should not crash:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let cache_files: Vec<_> = fs::read_dir(&cache_dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    assert!(
        cache_files.is_empty(),
        "--no-cache should not create result cache files, found: {:?}",
        cache_files
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cache_invalidated_by_file_change() {
    let dir = temp_dir("cache_file_change");
    let file = write_file(&dir, "test.rb", b"x = 1 \n");

    let cache_dir = dir.join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    unsafe { std::env::set_var("NITROCOP_CACHE_DIR", &cache_dir) };

    let config = load_config(None, Some(dir.as_path()), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        cache: "true".to_string(),
        preview: true,
        ..default_args()
    };

    // First run: should detect trailing whitespace
    let result1 = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(
        result1.diagnostics.len(),
        1,
        "Should detect trailing whitespace"
    );

    // Modify file to remove the offense
    fs::write(&file, b"x = 1\n").unwrap();

    // Second run: file changed, cache should miss, no offense
    let result2 = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(
        result2.diagnostics.len(),
        0,
        "After fix, should find no offenses"
    );

    unsafe { std::env::remove_var("NITROCOP_CACHE_DIR") };
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cache_invalidated_by_config_change() {
    let dir = temp_dir("cache_config_change");
    let file = write_file(&dir, "test.rb", b"x = 1 \n");

    let cache_dir = dir.join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    unsafe { std::env::set_var("NITROCOP_CACHE_DIR", &cache_dir) };

    let config = load_config(None, Some(dir.as_path()), None).unwrap();
    let registry = CopRegistry::default_registry();

    // Run with --only TrailingWhitespace
    let args1 = Args {
        only: vec!["Layout/TrailingWhitespace".to_string()],
        cache: "true".to_string(),
        preview: true,
        ..default_args()
    };
    let result1 = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args1,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result1.diagnostics.len(), 1);

    // Run with --only a different cop — different session hash, so cache miss
    let args2 = Args {
        only: vec!["Style/FrozenStringLiteralComment".to_string()],
        cache: "true".to_string(),
        preview: true,
        ..default_args()
    };
    let result2 = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args2,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    // Should get different results (FrozenStringLiteralComment, not TrailingWhitespace)
    for d in &result2.diagnostics {
        assert_ne!(
            d.cop_name, "Layout/TrailingWhitespace",
            "Config change should use different session, not return stale cached results"
        );
    }

    unsafe { std::env::remove_var("NITROCOP_CACHE_DIR") };
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cache_preserves_all_severity_types() {
    use nitrocop::cache::{CacheLookup, ResultCache};
    use nitrocop::diagnostic::{Diagnostic, Location, Severity};

    let tmp = tempfile::tempdir().unwrap();
    let configs = vec![nitrocop::cop::CopConfig::default()];
    let args = default_args();
    let cache = ResultCache::with_root(tmp.path(), "0.1.0-test", &configs, &args);

    // Create a real file so stat() works
    let rb_file = tmp.path().join("severity_test.rb");
    let content = b"test content";
    fs::write(&rb_file, content).unwrap();

    let diagnostics = vec![
        Diagnostic {
            path: rb_file.to_string_lossy().to_string(),
            location: Location { line: 1, column: 0 },
            severity: Severity::Convention,
            cop_name: "Style/A".to_string(),
            message: "convention".to_string(),
            corrected: false,
        },
        Diagnostic {
            path: rb_file.to_string_lossy().to_string(),
            location: Location { line: 2, column: 5 },
            severity: Severity::Warning,
            cop_name: "Lint/B".to_string(),
            message: "warning".to_string(),
            corrected: false,
        },
        Diagnostic {
            path: rb_file.to_string_lossy().to_string(),
            location: Location {
                line: 3,
                column: 10,
            },
            severity: Severity::Error,
            cop_name: "Security/C".to_string(),
            message: "error".to_string(),
            corrected: false,
        },
        Diagnostic {
            path: rb_file.to_string_lossy().to_string(),
            location: Location { line: 4, column: 0 },
            severity: Severity::Fatal,
            cop_name: "Lint/D".to_string(),
            message: "fatal".to_string(),
            corrected: false,
        },
    ];

    cache.put(&rb_file, content, &diagnostics);

    // Should get a stat hit since file hasn't changed
    match cache.get_by_stat(&rb_file) {
        CacheLookup::StatHit(cached) => {
            assert_eq!(cached.len(), 4);
            assert_eq!(cached[0].severity, Severity::Convention);
            assert_eq!(cached[1].severity, Severity::Warning);
            assert_eq!(cached[1].location.column, 5);
            assert_eq!(cached[2].severity, Severity::Error);
            assert_eq!(cached[2].location.line, 3);
            assert_eq!(cached[3].severity, Severity::Fatal);
        }
        _ => panic!("Expected StatHit"),
    }
}

#[test]
fn redundant_disable_for_disabled_cop() {
    // A cop that is explicitly disabled in config has its disable directive
    // flagged as redundant — disabling an already-disabled cop is pointless.
    let dir = temp_dir("redundant_disable_disabled_cop_yml");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n\ndef edit # rubocop:disable Lint/UselessMethodDefinition\n  super\nend\n",
    );
    // Config that explicitly disables the cop
    write_file(
        &dir,
        ".rubocop.yml",
        b"Lint/UselessMethodDefinition:\n  Enabled: false\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Expected 1 redundant disable for disabled cop, got: {:?}",
        redundant
    );
    assert!(
        redundant[0]
            .message
            .contains("Lint/UselessMethodDefinition"),
        "Message should mention the disabled cop: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_excluded_cop() {
    // A cop that is enabled but excluded from this file by Exclude patterns
    // didn't execute on it. The disable directive is therefore redundant.
    let dir = temp_dir("redundant_disable_excluded_cop_v2");
    let sub = dir.join("app").join("controllers");
    fs::create_dir_all(&sub).unwrap();
    let file = write_file(
        &dir,
        "app/controllers/test_controller.rb",
        b"# frozen_string_literal: true\n\ndef edit # rubocop:disable Lint/UselessMethodDefinition\n  super\nend\n",
    );
    // Config that excludes the cop from app/controllers/**
    write_file(
        &dir,
        ".rubocop.yml",
        b"Lint/UselessMethodDefinition:\n  Exclude:\n    - 'app/controllers/**/*'\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Expected 1 redundant disable for excluded cop, got: {:?}",
        redundant
    );
    assert!(
        redundant[0]
            .message
            .contains("Lint/UselessMethodDefinition"),
        "Message should mention the excluded cop: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_include_mismatch_cop() {
    // A cop with an Include pattern that doesn't match the file. The cop is
    // enabled but won't execute (Include mismatch). We do NOT flag this as
    // redundant because Include mismatches can arise from sub-config directory
    // path resolution issues and aren't reliable indicators of redundancy.
    // Only Exclude-based exclusions are flagged.
    let dir = temp_dir("no_redundant_disable_include_mismatch");
    let file = write_file(
        &dir,
        "app/models/user.rb",
        b"# frozen_string_literal: true\n\n# rubocop:disable Rails/CreateTableWithTimestamps\nclass User < ApplicationRecord\nend\n# rubocop:enable Rails/CreateTableWithTimestamps\n",
    );
    // Config that limits the cop to only db/migrate/ files
    write_file(
        &dir,
        ".rubocop.yml",
        b"Rails/CreateTableWithTimestamps:\n  Enabled: true\n  Include:\n    - 'db/migrate/**/*.rb'\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        0,
        "Include mismatch should NOT be flagged as redundant: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn rake_duplicate_namespace_matches_rake_files_by_default() {
    let dir = temp_dir("rake_duplicate_namespace_rake_file");
    let file = write_file(
        &dir,
        "lib/tasks/test.rake",
        b"namespace :db do\nend\n\nnamespace :db do\nend\n",
    );
    let mut config = nitrocop::config::ResolvedConfig::empty();
    config.register_departments_from_only(&["Rake/DuplicateNamespace".to_string()]);
    let registry = CopRegistry::default_registry();
    let tier_map = TierMap::load();
    let mut filters = config.build_cop_filters(&registry, &tier_map, true);
    filters.set_scan_root(dir.clone());
    let duplicate_namespace_idx = registry
        .cops()
        .iter()
        .position(|cop| cop.name() == "Rake/DuplicateNamespace")
        .unwrap();

    assert_eq!(
        filters.is_cop_match(duplicate_namespace_idx, &file),
        true,
        "Expected .rake files to match DuplicateNamespace include filters"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn rake_duplicate_namespace_ignores_nested_rakefile_by_default() {
    let dir = temp_dir("rake_duplicate_namespace_nested_rakefile");
    let file = write_file(
        &dir,
        "activerecord/Rakefile",
        b"namespace :db do\nend\n\nnamespace :db do\nend\n",
    );
    let mut config = nitrocop::config::ResolvedConfig::empty();
    config.register_departments_from_only(&["Rake/DuplicateNamespace".to_string()]);
    let registry = CopRegistry::default_registry();
    let tier_map = TierMap::load();
    let mut filters = config.build_cop_filters(&registry, &tier_map, true);
    filters.set_scan_root(dir.clone());
    let duplicate_namespace_idx = registry
        .cops()
        .iter()
        .position(|cop| cop.name() == "Rake/DuplicateNamespace")
        .unwrap();

    assert_eq!(
        filters.is_cop_match(duplicate_namespace_idx, &file),
        false,
        "Expected nested subdir/Rakefile to be skipped by default"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_self_referential() {
    // Disabling Lint/RedundantCopDisableDirective itself is legitimate and
    // should never be flagged as redundant.
    let dir = temp_dir("no_redundant_disable_self_ref");
    let file = write_file(
        &dir,
        "test.rb",
        b"# rubocop:disable Lint/RedundantCopDisableDirective\nx = 1\n# rubocop:enable Lint/RedundantCopDisableDirective\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        0,
        "Self-referential disable should not be flagged, got: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_mixed_excluded_and_active() {
    // Two cops on one disable directive: one is excluded (Lint/UselessMethodDefinition),
    // one actively suppresses an offense (Layout/TrailingWhitespace). Only the
    // excluded cop's directive should be flagged as redundant.
    // Use a block disable so the trailing whitespace on the next line is suppressed.
    let dir = temp_dir("redundant_disable_mixed_excl_active");
    let sub = dir.join("app").join("controllers");
    fs::create_dir_all(&sub).unwrap();
    let file = write_file(
        &dir,
        "app/controllers/test_controller.rb",
        b"# rubocop:disable Lint/UselessMethodDefinition, Layout/TrailingWhitespace\nx = 1   \n# rubocop:enable Lint/UselessMethodDefinition, Layout/TrailingWhitespace\n",
    );
    write_file(
        &dir,
        ".rubocop.yml",
        b"Lint/UselessMethodDefinition:\n  Exclude:\n    - 'app/controllers/**/*'\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    // Only the excluded cop's directive should be flagged
    assert_eq!(
        redundant.len(),
        1,
        "Expected 1 redundant disable (excluded cop only), got: {:?}",
        redundant
    );
    assert!(
        redundant[0]
            .message
            .contains("Lint/UselessMethodDefinition"),
        "Should flag the excluded cop, not the active one: {}",
        redundant[0].message
    );
    // The trailing whitespace offense should NOT appear (it's suppressed)
    let tw_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Layout/TrailingWhitespace")
        .collect();
    assert_eq!(
        tw_offenses.len(),
        0,
        "TrailingWhitespace should be suppressed by the directive, got: {:?}",
        tw_offenses
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_executed_cop_no_offense() {
    // A cop that is enabled and executes on the file but produces no offense.
    // In normal mode (no --only/--except), all cops ran, so the unused
    // directive is correctly flagged as redundant.
    let dir = temp_dir("no_redundant_disable_exec_no_off");
    // Layout/TrailingWhitespace would fire on trailing spaces, but this
    // file is clean. The disable is unused and the cop ran.
    let file = write_file(
        &dir,
        "test.rb",
        b"x = 1 # rubocop:disable Layout/TrailingWhitespace\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Executed cop with no offense should be flagged in normal mode: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("Layout/TrailingWhitespace"),
        "Message should mention the cop: {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_for_renamed_cop() {
    // Naming/PredicateName was renamed to Naming/PredicatePrefix.
    // The short name changed (PredicateName -> PredicatePrefix), so RuboCop
    // does NOT honor the old name in disable comments for the new cop.
    // nitrocop should match this behavior: the offense is NOT suppressed.
    let dir = temp_dir("redundant_disable_for_renamed_cop");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\ndef is_valid? # rubocop:disable Naming/PredicateName\n  true\nend\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // The old name should NOT suppress the new cop (short name changed),
    // so the PredicatePrefix offense should still be reported.
    let prefix_offenses: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Naming/PredicatePrefix")
        .collect();
    assert!(
        !prefix_offenses.is_empty(),
        "Old name with changed short name should NOT suppress new cop offense"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn redundant_disable_for_unknown_cop() {
    // A disable for a completely unknown cop (not in registry, not renamed)
    // should be flagged with "(unknown cop)" — matching RuboCop behavior.
    let dir = temp_dir("redundant_disable_unknown_cop");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n# rubocop:disable Custom/MyCop\nx = 1\n# rubocop:enable Custom/MyCop\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        1,
        "Unknown cop should be flagged as redundant, got: {:?}",
        redundant
    );
    assert!(
        redundant[0].message.contains("(unknown cop)"),
        "Message should include '(unknown cop)': {}",
        redundant[0].message
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_for_department_only() {
    // A disable for a whole department (e.g., "Layout") should not be flagged.
    let dir = temp_dir("no_redundant_disable_dept");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n# rubocop:disable Layout\nx = 1\n# rubocop:enable Layout\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        0,
        "Department-only disable should NOT be flagged: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_for_all_wildcard() {
    // A disable for "all" should not be flagged.
    let dir = temp_dir("no_redundant_disable_all");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n# rubocop:disable all\nx = 1\n# rubocop:enable all\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        0,
        "Wildcard 'all' disable should NOT be flagged: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_redundant_disable_for_malformed_cop_name() {
    // Cop names that don't start with an uppercase letter (e.g. "/BlockLength")
    // are malformed and should be silently ignored, matching RuboCop behavior.
    let dir = temp_dir("no_redundant_disable_malformed");
    let file = write_file(
        &dir,
        "test.rb",
        b"# frozen_string_literal: true\n# rubocop:disable /BlockLength, Metrics/\nx = 1\n# rubocop:enable /BlockLength, Metrics/\n",
    );
    let config = load_config(None, Some(&dir), None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = default_args();

    let result = run_linter(
        &discovered(&[file]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    let redundant: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Lint/RedundantCopDisableDirective")
        .collect();

    assert_eq!(
        redundant.len(),
        0,
        "Malformed cop name should NOT be flagged: {:?}",
        redundant
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- -L / --list-target-files CLI tests ----------

#[test]
fn list_target_files_prints_discovered_files() {
    let dir = temp_dir("list_target");
    fs::write(dir.join("a.rb"), "x = 1\n").unwrap();
    fs::write(dir.join("b.rb"), "y = 2\n").unwrap();
    fs::write(dir.join("c.txt"), "not ruby\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["-L", "--no-cache", dir.to_str().unwrap()])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "-L should exit 0, stderr: {stderr}"
    );

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "Should list 2 .rb files, got: {stdout}");
    assert!(stdout.contains("a.rb"), "Should list a.rb, got: {stdout}");
    assert!(stdout.contains("b.rb"), "Should list b.rb, got: {stdout}");
    assert!(
        !stdout.contains("c.txt"),
        "Should not list c.txt, got: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_target_files_exits_without_linting() {
    let dir = temp_dir("list_target_nolint");
    fs::write(dir.join("bad.rb"), "x = 1   \n").unwrap(); // trailing whitespace

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["-L", "--no-cache", dir.to_str().unwrap()])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should exit 0 (not 1 for offenses) — it just lists files
    assert!(output.status.success(), "-L should always exit 0");
    // Should print the filename, not offense output
    assert!(stdout.contains("bad.rb"), "Should list the file");
    assert!(
        !stdout.contains("TrailingWhitespace"),
        "Should not run linting, got: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- -D / --display-cop-names CLI tests ----------

#[test]
fn display_cop_names_flag_accepted() {
    let dir = temp_dir("display_cop_names");
    fs::write(dir.join("test.rb"), "x = 1   \n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "-D",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Flag should be accepted without error (cop names are always shown)
    assert!(
        stdout.contains("Layout/TrailingWhitespace"),
        "Cop names should appear in output with -D: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- -P / --parallel CLI tests ----------

#[test]
fn parallel_flag_accepted() {
    let dir = temp_dir("parallel_flag");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--preview", "-P", "--no-cache", dir.to_str().unwrap()])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should not error — flag is accepted and ignored
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "-P should be accepted without error, stderr: {stderr}"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- -r / --require CLI tests ----------

#[test]
fn require_flag_accepted_and_ignored() {
    let dir = temp_dir("require_flag");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "-r",
            "rubocop-rspec",
            "--require",
            "rubocop-performance",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should not error — flag is accepted with a warning
    assert!(
        output.status.success() || output.status.code() == Some(1),
        "-r should be accepted without error, stderr: {stderr}"
    );
    assert!(
        stderr.contains("--require is not supported"),
        "-r should print a warning, stderr: {stderr}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn require_multiple_values_accepted() {
    // Test that multiple -r flags work (common in .rubocop files)
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["-r", "rubocop-rspec", "-r", "rubocop-rails", "--list-cops"])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Multiple -r flags should be accepted, stderr: {stderr}"
    );
}

// ---------- --fail-fast CLI tests ----------

#[test]
fn fail_fast_stops_early() {
    let dir = temp_dir("fail_fast");
    // Create many files with offenses
    for i in 0..20 {
        fs::write(dir.join(format!("file_{i:02}.rb")), "x = 1   \n").unwrap();
    }

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "-F",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert_eq!(
        output.status.code(),
        Some(1),
        "--fail-fast should exit 1 on offenses"
    );

    // With --fail-fast, we should see offenses from fewer files than the total 20
    // (exact count depends on rayon scheduling, but should be significantly less)
    let file_count: usize = stdout
        .lines()
        .filter(|l| l.contains("Layout/TrailingWhitespace"))
        .count();
    assert!(
        file_count >= 1,
        "--fail-fast should still report at least 1 offense"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- --fail-level CLI tests ----------

#[test]
fn fail_level_warning_ignores_conventions() {
    let dir = temp_dir("fail_level_w");
    // FrozenStringLiteralComment is convention severity
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--fail-level",
            "W",
            "--only",
            "Style/FrozenStringLiteralComment",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    // Convention-level offense should NOT cause exit 1 when fail-level is warning
    assert!(
        output.status.success(),
        "--fail-level W should exit 0 for convention offenses"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn fail_level_convention_catches_conventions() {
    let dir = temp_dir("fail_level_c");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--fail-level",
            "convention",
            "--only",
            "Style/FrozenStringLiteralComment",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    assert_eq!(
        output.status.code(),
        Some(1),
        "--fail-level convention should exit 1 for convention offenses"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn fail_level_invalid_value_errors() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--fail-level", "bogus", "--no-cache", "."])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "Invalid --fail-level should fail");
    assert!(
        stderr.contains("invalid --fail-level"),
        "Should show helpful error, got: {stderr}"
    );
}

// ---------- --force-exclusion CLI tests ----------

#[test]
fn force_exclusion_excludes_explicit_file() {
    let dir = temp_dir("force_excl");
    let vendor_dir = dir.join("vendor");
    fs::create_dir_all(&vendor_dir).unwrap();
    let vendor_file = vendor_dir.join("bad.rb");
    fs::write(&vendor_file, "x = 1   \n").unwrap();

    // Config that excludes vendor/**/*
    fs::write(
        dir.join(".rubocop.yml"),
        "AllCops:\n  Exclude:\n    - 'vendor/**/*'\n",
    )
    .unwrap();

    // Without --force-exclusion: explicit file should be linted
    let output_no_force = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            vendor_file.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_no_force = String::from_utf8_lossy(&output_no_force.stdout);
    assert!(
        stdout_no_force.contains("TrailingWhitespace"),
        "Without --force-exclusion, explicit file should be linted: {stdout_no_force}"
    );

    // With --force-exclusion: explicit file should be excluded
    let output_force = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--force-exclusion",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            vendor_file.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_force = String::from_utf8_lossy(&output_force.stdout);
    assert!(
        !stdout_force.contains("TrailingWhitespace"),
        "With --force-exclusion, explicit file should be excluded: {stdout_force}"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- --ignore-disable-comments CLI tests ----------

#[test]
fn ignore_disable_comments_shows_suppressed_offenses() {
    let dir = temp_dir("ignore_disable");
    // rubocop:disable on line 1 covers lines below; line 2 has trailing whitespace
    fs::write(
        dir.join("test.rb"),
        "# rubocop:disable Layout/TrailingWhitespace\nx = 1   \n# rubocop:enable Layout/TrailingWhitespace\n",
    )
    .unwrap();

    // Without flag: offense is suppressed by disable comment
    let output_normal = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_normal = String::from_utf8_lossy(&output_normal.stdout);
    assert!(
        !stdout_normal.contains("Layout/TrailingWhitespace"),
        "Without flag, offense should be suppressed: {stdout_normal}"
    );

    // With --ignore-disable-comments: offense is shown
    let output_ignore = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--ignore-disable-comments",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_ignore = String::from_utf8_lossy(&output_ignore.stdout);
    assert!(
        stdout_ignore.contains("Layout/TrailingWhitespace"),
        "With --ignore-disable-comments, offense should be shown: {stdout_ignore}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn ignore_disable_comments_skips_redundant_disable_check() {
    let dir = temp_dir("ignore_disable_redundant");
    // A disable for a cop that won't fire — normally flagged as redundant
    fs::write(
        dir.join("test.rb"),
        "# frozen_string_literal: true\nx = 1 # rubocop:disable Layout/TrailingWhitespace\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--ignore-disable-comments",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should NOT report redundant disable when ignoring disable comments
    assert!(
        !stdout.contains("RedundantCopDisableDirective"),
        "Should not report redundant disables when ignoring comments: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn three_state_boolean_column_respects_dynamic_change_column_null_in_cli() {
    let dir = temp_dir("three_state_boolean_column_dynamic_table");
    fs::create_dir_all(dir.join("db/migrate")).unwrap();
    fs::write(
        dir.join("db/migrate/20250101010101_query_flags.rb"),
        "# frozen_string_literal: true\n\
class QueryFlags < ActiveRecord::Migration[7.0]\n\
  def change\n\
    table = :queries\n\
    create_table table do |t|\n\
      t.boolean :starred\n\
    end\n\
    change_column_null table, :starred, false\n\
  end\n\
end\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--only",
            "Rails/ThreeStateBooleanColumn",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Rails/ThreeStateBooleanColumn"),
        "Dynamic table expressions with matching change_column_null should not register offenses: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- --force-default-config CLI tests ----------

#[test]
fn force_default_config_ignores_config_file() {
    let dir = temp_dir("force_default");
    // Config disables TrailingWhitespace
    fs::write(
        dir.join(".rubocop.yml"),
        "Layout/TrailingWhitespace:\n  Enabled: false\n",
    )
    .unwrap();
    fs::write(dir.join("test.rb"), "x = 1   \n").unwrap();

    // Without flag: config disables the cop, no offense
    let output_normal = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_normal = String::from_utf8_lossy(&output_normal.stdout);
    assert!(
        !stdout_normal.contains("TrailingWhitespace"),
        "Config should disable cop: {stdout_normal}"
    );

    // With --force-default-config: config is ignored, cop fires
    let output_force = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--force-default-config",
            "--only",
            "Layout/TrailingWhitespace",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_force = String::from_utf8_lossy(&output_force.stdout);
    assert!(
        stdout_force.contains("TrailingWhitespace"),
        "With --force-default-config, cop should fire: {stdout_force}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn unique_validation_without_index_only_loads_schema_relative_to_cwd() {
    // RuboCop's SchemaLoader walks up from `Pathname.pwd` to find db/schema.rb
    // and returns nil if none is found. Nitrocop must mirror that, otherwise
    // corpus runs (cwd outside the repo) flag offenses RuboCop never emits.
    let dir = temp_dir("unique_validation_schema_relative_to_cwd");
    let outside_cwd = std::env::temp_dir().join("nitrocop_uvi_outside_cwd");
    fs::create_dir_all(&outside_cwd).unwrap();
    fs::create_dir_all(dir.join("app/models")).unwrap();
    fs::create_dir_all(dir.join("db")).unwrap();
    fs::write(
        dir.join("app/models/label.rb"),
        "class Label < ApplicationRecord\n  validates :name, uniqueness: true\nend\n",
    )
    .unwrap();
    fs::write(
        dir.join("db/schema.rb"),
        "ActiveRecord::Schema.define(version: 1) do\n  create_table \"labels\", force: :cascade do |t|\n    t.string \"name\"\n  end\nend\n",
    )
    .unwrap();

    let from_outside = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--force-default-config",
            "--only",
            "Rails/UniqueValidationWithoutIndex",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .current_dir(&outside_cwd)
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_outside = String::from_utf8_lossy(&from_outside.stdout);
    assert!(
        !stdout_outside.contains("Rails/UniqueValidationWithoutIndex"),
        "Schema must not be discovered when cwd has no db/schema.rb in any ancestor: {stdout_outside}"
    );

    let from_repo = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--preview",
            "--force-default-config",
            "--only",
            "Rails/UniqueValidationWithoutIndex",
            "--no-cache",
            dir.to_str().unwrap(),
        ])
        .current_dir(&dir)
        .output()
        .expect("Failed to execute nitrocop");
    let stdout_repo = String::from_utf8_lossy(&from_repo.stdout);
    assert!(
        stdout_repo.contains("Rails/UniqueValidationWithoutIndex"),
        "When cwd is the repo root, schema is loaded and the cop fires: {stdout_repo}"
    );

    fs::remove_dir_all(&dir).ok();
    fs::remove_dir_all(&outside_cwd).ok();
}

// ---------- Autocorrect tests ----------

#[test]
fn autocorrect_fixes_file_on_disk() {
    let dir = temp_dir("autocorrect_disk");
    // File has: leading blank lines, trailing whitespace, trailing blank lines
    let file = write_file(&dir, "fixme.rb", b"\n\nx = 1  \ny = 2\n\n\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true,
        only: vec![
            "Layout/LeadingEmptyLines".to_string(),
            "Layout/TrailingWhitespace".to_string(),
            "Layout/TrailingEmptyLines".to_string(),
        ],
        preview: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.corrected_count > 0,
        "Expected corrected_count > 0, got {}",
        result.corrected_count
    );

    // Verify file was corrected on disk
    let corrected = fs::read(&file).unwrap();
    assert_eq!(
        corrected, b"x = 1\ny = 2\n",
        "File should be corrected: leading blanks removed, trailing whitespace removed, trailing blank lines removed"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_inserts_frozen_string_literal() {
    let dir = temp_dir("autocorrect_frozen");
    let file = write_file(&dir, "missing_magic.rb", b"x = 1\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    // FrozenStringLiteralComment is SafeAutoCorrect: false, so use -A
    let args = Args {
        autocorrect_all: true,
        only: vec!["Style/FrozenStringLiteralComment".to_string()],
        preview: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.corrected_count > 0,
        "Expected corrected_count > 0, got {}",
        result.corrected_count
    );

    let corrected = fs::read(&file).unwrap();
    assert!(
        corrected.starts_with(b"# frozen_string_literal: true\n"),
        "File should start with frozen_string_literal comment, got: {:?}",
        String::from_utf8_lossy(&corrected[..corrected.len().min(60)])
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_safe_mode_skips_unsafe_cops() {
    let dir = temp_dir("autocorrect_safe");
    // Configure FrozenStringLiteralComment with SafeAutoCorrect: false
    let file = write_file(&dir, "unsafe_test.rb", b"x = 1  \n");
    let config_path = write_file(
        &dir,
        ".rubocop.yml",
        b"Style/FrozenStringLiteralComment:\n  SafeAutoCorrect: false\n",
    );
    let config = load_config(Some(config_path.as_path()), None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true, // -a (safe only)
        only: vec![
            "Style/FrozenStringLiteralComment".to_string(),
            "Layout/TrailingWhitespace".to_string(),
        ],
        preview: true,
        ..default_args()
    };

    let _result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    let corrected = fs::read(&file).unwrap();
    // TrailingWhitespace should be fixed (safe autocorrect)
    assert!(
        corrected.windows(2).all(|w| w != b"  "),
        "Trailing whitespace should be removed"
    );
    // FrozenStringLiteralComment should NOT be added (SafeAutoCorrect: false with -a)
    assert!(
        !corrected.starts_with(b"# frozen_string_literal"),
        "FrozenStringLiteralComment should not be inserted with -a when SafeAutoCorrect: false"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_all_mode_includes_unsafe_cops() {
    let dir = temp_dir("autocorrect_all");
    let file = write_file(&dir, "unsafe_all.rb", b"x = 1  \n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect_all: true, // -A (all)
        only: vec![
            "Style/FrozenStringLiteralComment".to_string(),
            "Layout/TrailingWhitespace".to_string(),
        ],
        preview: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.corrected_count > 0,
        "Expected corrected_count > 0, got {}",
        result.corrected_count
    );

    let corrected = fs::read(&file).unwrap();
    // Both should be fixed with -A
    assert!(
        corrected.starts_with(b"# frozen_string_literal: true\n"),
        "FrozenStringLiteralComment should be inserted with -A"
    );
    assert!(
        corrected.windows(3).all(|w| w != b"  \n"),
        "Trailing whitespace should be removed"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_clean_file_unchanged() {
    let dir = temp_dir("autocorrect_clean");
    let content = b"# frozen_string_literal: true\n\nx = 1\ny = 2\n";
    let file = write_file(&dir, "clean.rb", content);
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect_all: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(
        result.corrected_count, 0,
        "Clean file should have no corrections"
    );

    let after = fs::read(&file).unwrap();
    assert_eq!(after, content, "Clean file should be unchanged on disk");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_multi_iteration_converges() {
    let dir = temp_dir("autocorrect_multi_iter");
    // Leading blanks + trailing whitespace — requires multiple cops to fix
    let file = write_file(&dir, "multi.rb", b"\n\nx = 1  \ny = 2\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true,
        only: vec![
            "Layout/LeadingEmptyLines".to_string(),
            "Layout/TrailingWhitespace".to_string(),
        ],
        preview: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.corrected_count >= 2,
        "Expected at least 2 corrections (leading blanks + trailing whitespace), got {}",
        result.corrected_count
    );

    let corrected = fs::read(&file).unwrap();
    assert_eq!(corrected, b"x = 1\ny = 2\n");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_corrected_count_across_iterations() {
    let dir = temp_dir("autocorrect_count");
    // Leading blanks + trailing whitespace + missing frozen_string_literal
    let file = write_file(&dir, "count.rb", b"\n\nx = 1  \n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect_all: true, // -A to include FrozenStringLiteralComment
        only: vec![
            "Layout/LeadingEmptyLines".to_string(),
            "Layout/TrailingWhitespace".to_string(),
            "Style/FrozenStringLiteralComment".to_string(),
        ],
        preview: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.corrected_count >= 3,
        "Expected at least 3 corrections, got {}",
        result.corrected_count
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_idempotent_second_run() {
    let dir = temp_dir("autocorrect_idempotent");
    let file = write_file(&dir, "idem.rb", b"\n\nx = 1  \ny = 2\n\n\n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true,
        only: vec![
            "Layout/LeadingEmptyLines".to_string(),
            "Layout/TrailingWhitespace".to_string(),
            "Layout/TrailingEmptyLines".to_string(),
        ],
        preview: true,
        ..default_args()
    };

    // First run: fix offenses
    let result1 = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(result1.corrected_count > 0);
    let after_first = fs::read(&file).unwrap();

    // Second run: should find nothing to correct
    let result2 = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(
        result2.corrected_count, 0,
        "Second run should have no corrections"
    );
    let after_second = fs::read(&file).unwrap();
    assert_eq!(
        after_first, after_second,
        "File should be unchanged after second run"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_empty_file_no_crash() {
    let dir = temp_dir("autocorrect_empty");
    let file = write_file(&dir, "empty.rb", b"");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect_all: true,
        ..default_args()
    };

    // Should not panic
    let _result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_no_write_when_no_corrections() {
    let dir = temp_dir("autocorrect_no_write");
    let content = b"x = 1\ny = 2\n";
    let file = write_file(&dir, "clean.rb", content);

    // Record mtime before
    let mtime_before = fs::metadata(&file).unwrap().modified().unwrap();

    // Small delay to ensure mtime would differ if file were rewritten
    std::thread::sleep(std::time::Duration::from_millis(50));

    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true,
        only: vec!["Layout/TrailingWhitespace".to_string()],
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert_eq!(result.corrected_count, 0);

    let mtime_after = fs::metadata(&file).unwrap().modified().unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "File mtime should be unchanged when no corrections applied"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_diagnostics_include_corrected_offenses() {
    // When nitrocop -A corrects an offense, the returned diagnostics should
    // include that offense with `corrected: true`. Previously, the autocorrect
    // loop would re-lint the corrected source and return only the remaining
    // (uncorrected) diagnostics, losing the corrected ones from the output.
    let dir = temp_dir("autocorrect_corrected_diags");
    let file = write_file(
        &dir,
        "test.rb",
        b"x = 1  \n", // trailing whitespace
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true,
        only: vec!["Layout/TrailingWhitespace".to_string()],
        preview: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    // The file should be corrected
    let content = fs::read_to_string(&file).unwrap();
    assert_eq!(
        content, "x = 1\n",
        "File should have trailing whitespace removed"
    );

    // The diagnostics should include the corrected offense
    let corrected_diags: Vec<_> = result.diagnostics.iter().filter(|d| d.corrected).collect();
    assert!(
        !corrected_diags.is_empty(),
        "Expected at least one diagnostic with corrected=true, but found none. \
         All diagnostics: {:?}",
        result
            .diagnostics
            .iter()
            .map(|d| (&d.cop_name, d.corrected))
            .collect::<Vec<_>>()
    );

    // Verify it's specifically the TrailingWhitespace offense
    assert!(
        corrected_diags
            .iter()
            .any(|d| d.cop_name == "Layout/TrailingWhitespace"),
        "Expected a corrected Layout/TrailingWhitespace diagnostic"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_json_output_marks_corrected_offenses() {
    // End-to-end test: corrected offenses should appear with corrected: true
    // alongside any remaining (uncorrected) offenses.
    let dir = temp_dir("autocorrect_json_corrected");
    let file = write_file(
        &dir,
        "test.rb",
        b"x = 1  \ny = 2\n", // trailing whitespace on line 1, clean line 2
    );
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true,
        only: vec!["Layout/TrailingWhitespace".to_string()],
        preview: true,
        ..default_args()
    };

    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );

    assert_eq!(result.corrected_count, 1, "Should have corrected 1 offense");

    // There should be exactly 1 diagnostic, and it should be marked corrected
    let tw_diags: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.cop_name == "Layout/TrailingWhitespace")
        .collect();
    assert_eq!(
        tw_diags.len(),
        1,
        "Should have exactly 1 TrailingWhitespace diagnostic"
    );
    assert!(
        tw_diags[0].corrected,
        "The diagnostic should be marked as corrected"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- Exit code contract tests ----------

#[test]
fn internal_error_exits_three() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--no-cache", "/nonexistent/path/that/does/not/exist"])
        .output()
        .expect("Failed to execute nitrocop");

    assert_eq!(
        output.status.code(),
        Some(3),
        "Internal error should exit 3, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn strict_flag_accepted() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--strict", "--list-cops"])
        .output()
        .expect("Failed to execute nitrocop");

    assert!(
        output.status.success(),
        "--strict should be accepted, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn prefixed_repo_path_does_not_apply_bin_exclude_to_nested_repo_files() {
    let dir = temp_dir("prefixed_repo_path_excludes");
    fs::create_dir_all(dir.join("corpus/sample_repo/bin")).unwrap();
    fs::write(dir.join("corpus/sample_repo/bin/BadFile.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "AllCops:\n  Exclude:\n    - 'bin/**/*'\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--no-cache",
            "--format",
            "json",
            "--only",
            "Naming/FileName",
            "--preview",
            "--config",
        ])
        .arg(dir.join(".rubocop.yml"))
        .arg(dir.join("corpus/sample_repo"))
        .output()
        .expect("Failed to execute nitrocop");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Expected Naming/FileName offense, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Should be valid JSON: {e}\n{stdout}"));
    assert_eq!(
        parsed["offenses"].as_array().map(|a| a.len()),
        Some(1),
        "bin/**/* should not exclude files inside a prefixed target repo: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn strict_coverage_exits_two_for_preview_gated() {
    // Force the cop into preview tier via NITROCOP_TIERS_FILE.
    // Enable it in config, run without --preview → it's preview-gated → --strict exits 2.
    let dir = temp_dir("strict_coverage_preview");
    let tiers = write_preview_tiers(&dir, "Performance/BigDecimalWithNumericArgument");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Performance/BigDecimalWithNumericArgument:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_TIERS_FILE", &tiers)
        .args([
            "--strict",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "--strict should exit 2 when preview-gated cops exist, stderr: {stderr}"
    );
    assert!(
        stderr.contains("--strict=coverage"),
        "Should print strict warning, stderr: {stderr}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn strict_coverage_with_preview_exits_zero() {
    // Same as above but with --preview, so the cop is no longer preview-gated → exit 0.
    let dir = temp_dir("strict_coverage_with_preview");
    let tiers = write_preview_tiers(&dir, "Performance/BigDecimalWithNumericArgument");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Performance/BigDecimalWithNumericArgument:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_TIERS_FILE", &tiers)
        .args([
            "--strict",
            "--preview",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "--strict with --preview should exit 0 when all preview cops run, stderr: {stderr}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn strict_all_exits_two_for_unimplemented() {
    // Enable a cop that doesn't exist in the registry → classified as outside-baseline.
    // --strict=all should exit 2.
    let dir = temp_dir("strict_all_unimplemented");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Custom/FakeCop:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--strict=all",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "--strict=all should exit 2 for unimplemented/unknown cops, stderr: {stderr}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn strict_implemented_only_ignores_unimplemented() {
    // Same config as above but --strict=implemented-only → unknown cops are ignored → exit 0.
    let dir = temp_dir("strict_impl_only_unknown");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Custom/FakeCop:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--strict=implemented-only",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "--strict=implemented-only should exit 0 for unknown cops, stderr: {stderr}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn lint_failure_takes_priority_over_strict() {
    // Lint offenses AND strict failure → exit 1 (lint takes priority).
    let dir = temp_dir("strict_lint_priority");
    let tiers = write_preview_tiers(&dir, "Performance/BigDecimalWithNumericArgument");
    fs::write(dir.join("test.rb"), "x = 1   \n").unwrap(); // trailing whitespace → offense
    fs::write(
        dir.join(".rubocop.yml"),
        "Performance/BigDecimalWithNumericArgument:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_TIERS_FILE", &tiers)
        .args([
            "--preview",
            "--strict",
            "--only",
            "Layout/TrailingWhitespace",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    assert_eq!(
        output.status.code(),
        Some(1),
        "Lint failure (exit 1) should take priority over strict failure (exit 2)"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn strict_invalid_value_errors() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--strict=bogus", "--no-cache", "."])
        .output()
        .expect("Failed to execute nitrocop");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(3),
        "Invalid --strict value should exit 3, stderr: {stderr}"
    );
    assert!(
        stderr.contains("invalid --strict value"),
        "Should show helpful error, got: {stderr}"
    );
}

// ---------- --migrate CLI tests ----------

#[test]
fn migrate_text_output() {
    let dir = temp_dir("migrate_text");
    let tiers = write_preview_tiers(&dir, "Performance/BigDecimalWithNumericArgument");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Performance/BigDecimalWithNumericArgument:\n  Enabled: true\nLayout/TrailingWhitespace:\n  Enabled: true\nCustom/FakeCop:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_TIERS_FILE", &tiers)
        .args([
            "--migrate",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "--migrate should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("Baseline versions:"),
        "Should show baseline: {stdout}"
    );
    assert!(
        stdout.contains("Stable:"),
        "Should show stable count: {stdout}"
    );
    assert!(
        stdout.contains("Preview:"),
        "Should show preview count: {stdout}"
    );
    assert!(
        stdout.contains("Performance/BigDecimalWithNumericArgument"),
        "Should list preview cop: {stdout}"
    );
    assert!(
        stdout.contains("Custom/FakeCop"),
        "Should list outside-baseline cop: {stdout}"
    );
    assert!(
        stdout.contains("Suggested CI command:"),
        "Should show suggested CI command: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn migrate_json_output() {
    let dir = temp_dir("migrate_json");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Layout/TrailingWhitespace:\n  Enabled: true\nCustom/FakeCop:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--migrate",
            "--format",
            "json",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "--migrate --format json should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Should be valid JSON: {e}\n{stdout}"));
    assert!(
        parsed["baseline"]["rubocop"].is_string(),
        "Should have baseline.rubocop"
    );
    assert!(
        parsed["counts"]["stable"].is_number(),
        "Should have counts.stable"
    );
    assert!(parsed["cops"].is_array(), "Should have cops array");

    // Check that Custom/FakeCop is classified as outside_baseline
    let cops = parsed["cops"].as_array().unwrap();
    let fake = cops.iter().find(|c| c["name"] == "Custom/FakeCop");
    assert!(fake.is_some(), "Should include Custom/FakeCop in cops list");
    assert_eq!(
        fake.unwrap()["status"],
        "outside_baseline",
        "Custom/FakeCop should be outside_baseline"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn migrate_clean_config_no_skips() {
    let dir = temp_dir("migrate_clean");
    let tiers = write_preview_tiers(&dir, "Performance/BigDecimalWithNumericArgument");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Layout/TrailingWhitespace:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_TIERS_FILE", &tiers)
        .args([
            "--migrate",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("All enabled cops are stable"),
        "Clean config should say no migration needed: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- bare command name detection ----------

#[test]
fn bare_migrate_suggests_flag() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["migrate"])
        .output()
        .expect("Failed to execute nitrocop");

    assert!(
        !output.status.success(),
        "bare 'migrate' should fail, but got exit 0"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Did you mean `nitrocop --migrate`?"),
        "Should suggest --migrate flag, got: {stderr}"
    );
}

#[test]
fn bare_init_suggests_flag() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["init"])
        .output()
        .expect("Failed to execute nitrocop");

    assert!(
        !output.status.success(),
        "bare 'init' should fail, but got exit 0"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Did you mean `nitrocop --init`?"),
        "Should suggest --init flag, got: {stderr}"
    );
}

#[test]
fn bare_doctor_suggests_flag() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["doctor"])
        .output()
        .expect("Failed to execute nitrocop");

    assert!(
        !output.status.success(),
        "bare 'doctor' should fail, but got exit 0"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Did you mean `nitrocop --doctor`?"),
        "Should suggest --doctor flag, got: {stderr}"
    );
}

#[test]
fn normal_path_not_rejected_as_flag_name() {
    // Paths like "src" or "." must not be confused with flag-like names.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--list-cops", "."])
        .output()
        .expect("Failed to execute nitrocop");

    assert!(
        output.status.success(),
        "normal path '.' should not be rejected, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---------- --doctor CLI tests ----------

#[test]
fn doctor_shows_baseline_and_registry() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--doctor", "--force-default-config", "."])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "--doctor should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("Baseline versions"),
        "Should show baseline versions: {stdout}"
    );
    assert!(
        stdout.contains("rubocop 1."),
        "Should show rubocop version: {stdout}"
    );
    assert!(
        stdout.contains("Registry:"),
        "Should show registry info: {stdout}"
    );
    assert!(
        stdout.contains("cops registered"),
        "Should show cop count: {stdout}"
    );
}

#[test]
fn doctor_shows_config_root() {
    let dir = temp_dir("doctor_config_root");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Layout/TrailingWhitespace:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--doctor",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("Config root:"),
        "Should show config root: {stdout}"
    );
    assert!(
        stdout.contains(&dir.to_string_lossy().to_string()),
        "Config root should contain the temp dir path: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn doctor_shows_skip_summary() {
    let dir = temp_dir("doctor_skip_summary");
    let tiers = write_preview_tiers(&dir, "Performance/BigDecimalWithNumericArgument");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Performance/BigDecimalWithNumericArgument:\n  Enabled: true\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_TIERS_FILE", &tiers)
        .args([
            "--doctor",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("Preview-gated:"),
        "Should show preview-gated cops: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn doctor_detects_gem_version_mismatch() {
    let dir = temp_dir("doctor_gem_mismatch");
    fs::write(dir.join("test.rb"), "x = 1\n").unwrap();
    fs::write(
        dir.join(".rubocop.yml"),
        "Layout/TrailingWhitespace:\n  Enabled: true\n",
    )
    .unwrap();
    // Gemfile.lock with an older rubocop version
    fs::write(
        dir.join("Gemfile.lock"),
        "GEM\n  remote: https://rubygems.org/\n  specs:\n    rubocop (1.50.0)\n",
    )
    .unwrap();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args([
            "--doctor",
            "--no-cache",
            "--config",
            dir.join(".rubocop.yml").to_str().unwrap(),
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(
        stdout.contains("MISMATCH"),
        "Should detect version mismatch: {stdout}"
    );
    assert!(
        stdout.contains("1.50.0"),
        "Should show installed version: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

// ---------- --rules CLI tests ----------

#[test]
fn rules_table_output() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--rules"])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "--rules should exit 0, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("Name"), "Should show header: {stdout}");
    assert!(
        stdout.contains("Layout/TrailingWhitespace"),
        "Should list a known cop: {stdout}"
    );
    assert!(
        stdout.contains("cops total"),
        "Should show summary: {stdout}"
    );
}

#[test]
fn rules_tier_filter_preview() {
    let dir = temp_dir("rules_tier_filter_preview");
    let tiers = write_preview_tiers(&dir, "Performance/BigDecimalWithNumericArgument");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .env("NITROCOP_TIERS_FILE", &tiers)
        .args(["--rules", "--tier", "preview"])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    // All listed cops should be preview-tier
    assert!(
        stdout.contains("preview"),
        "Should show preview cops: {stdout}"
    );
    assert!(
        stdout.contains("Performance/BigDecimalWithNumericArgument"),
        "Should show the configured preview cop: {stdout}"
    );
    // Should NOT contain stable cops.
    assert!(
        !stdout.contains("Layout/TrailingWhitespace"),
        "Should not show stable cops when filtered to preview: {stdout}"
    );

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn rules_json_output() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_nitrocop"))
        .args(["--rules", "--format", "json"])
        .output()
        .expect("Failed to execute nitrocop");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Should be valid JSON: {e}\n{}",
            &stdout[..200.min(stdout.len())]
        )
    });
    assert!(parsed.is_array(), "Should be a JSON array");
    let arr = parsed.as_array().unwrap();
    assert!(arr.len() > 900, "Should have 900+ cops, got {}", arr.len());
    // Spot-check a known cop
    let tw = arr
        .iter()
        .find(|c| c["name"] == "Layout/TrailingWhitespace");
    assert!(tw.is_some(), "Should contain Layout/TrailingWhitespace");
    let tw = tw.unwrap();
    assert_eq!(tw["implemented"], true);
    assert_eq!(tw["in_baseline"], true);
}

#[test]
fn autocorrect_safe_allowlist_permits_listed_cop() {
    let dir = temp_dir("autocorrect_allowlist_safe");
    let file = write_file(&dir, "test.rb", b"x = 1  \n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect: true, // -a (safe only)
        only: vec!["Layout/TrailingWhitespace".to_string()],
        preview: true,
        ..default_args()
    };

    // Layout/TrailingWhitespace is on the allowlist, so -a should correct it
    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.corrected_count > 0,
        "Expected -a to correct allowlisted cop, but corrected_count = {}",
        result.corrected_count,
    );
    let corrected = fs::read(&file).unwrap();
    assert_eq!(corrected, b"x = 1\n");

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn autocorrect_all_bypasses_allowlist() {
    let dir = temp_dir("autocorrect_allowlist_all");
    let file = write_file(&dir, "test.rb", b"x = 1  \n");
    let config = load_config(None, None, None).unwrap();
    let registry = CopRegistry::default_registry();
    let args = Args {
        autocorrect_all: true, // -A (all)
        only: vec!["Layout/TrailingWhitespace".to_string()],
        preview: true,
        ..default_args()
    };

    // -A should correct regardless of allowlist
    let result = run_linter(
        &discovered(&[file.clone()]),
        &config,
        &registry,
        &args,
        &TierMap::load(),
        &AutocorrectAllowlist::load(),
    );
    assert!(
        result.corrected_count > 0,
        "Expected -A to correct cop, but corrected_count = {}",
        result.corrected_count,
    );
    let corrected = fs::read(&file).unwrap();
    assert_eq!(corrected, b"x = 1\n");

    fs::remove_dir_all(&dir).ok();
}

// ── NodePattern Verifier ──────────────────────────────────────────────────

/// Verify that all patterns in the pattern database parse successfully.
///
/// This is the first layer of the verifier: ensuring the interpreter can
/// handle every pattern we've curated from vendor RuboCop source.
#[test]
fn verifier_all_patterns_parse() {
    use nitrocop::node_pattern::lexer::Lexer;
    use nitrocop::node_pattern::parser::Parser;
    use nitrocop::node_pattern::pattern_db::PATTERNS;

    let mut failures = Vec::new();

    for entry in PATTERNS {
        let mut lexer = Lexer::new(entry.pattern);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);
        if parser.parse().is_none() {
            failures.push(format!(
                "  {} — failed to parse: {:?}",
                entry.cop_name, entry.pattern
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Pattern parse failures ({}/{}):\n{}",
        failures.len(),
        PATTERNS.len(),
        failures.join("\n"),
    );

    eprintln!(
        "verifier: all {}/{} patterns parse OK",
        PATTERNS.len(),
        PATTERNS.len()
    );
}

/// Verify the interpreter produces correct results on known Ruby snippets.
///
/// Tests a matrix of (pattern, ruby_source, expected_match) triples to ensure
/// the interpreter agrees with known-correct answers derived from RuboCop's
/// behavior.
#[test]
fn verifier_known_matches() {
    use nitrocop::node_pattern::interpreter::interpret_pattern;

    struct Case {
        pattern: &'static str,
        ruby: &'static [u8],
        expected: bool,
        label: &'static str,
    }

    let cases = [
        // ── Lint/BooleanSymbol ────────────────────────────────
        Case {
            pattern: "(sym {:true :false})",
            ruby: b":true",
            expected: true,
            label: "BooleanSymbol matches :true",
        },
        Case {
            pattern: "(sym {:true :false})",
            ruby: b":false",
            expected: true,
            label: "BooleanSymbol matches :false",
        },
        Case {
            pattern: "(sym {:true :false})",
            ruby: b":foo",
            expected: false,
            label: "BooleanSymbol rejects :foo",
        },
        // ── Style/NilComparison ───────────────────────────────
        Case {
            pattern: "(send _ {:== :===} nil)",
            ruby: b"x == nil",
            expected: true,
            label: "NilComparison matches x == nil",
        },
        Case {
            pattern: "(send _ {:== :===} nil)",
            ruby: b"x === nil",
            expected: true,
            label: "NilComparison matches x === nil",
        },
        Case {
            pattern: "(send _ {:== :===} nil)",
            ruby: b"x == 1",
            expected: false,
            label: "NilComparison rejects x == 1",
        },
        Case {
            pattern: "(send _ :nil?)",
            ruby: b"x.nil?",
            expected: true,
            label: "NilComparison matches x.nil?",
        },
        // ── Style/DoubleNegation ──────────────────────────────
        Case {
            pattern: "(send (send _ :!) :!)",
            ruby: b"!!x",
            expected: true,
            label: "DoubleNegation matches !!x",
        },
        Case {
            pattern: "(send (send _ :!) :!)",
            ruby: b"!x",
            expected: false,
            label: "DoubleNegation rejects !x",
        },
        // ── Style/SymbolProc/proc_node ────────────────────────
        Case {
            pattern: "(send (const {nil? cbase} :Proc) :new)",
            ruby: b"Proc.new",
            expected: true,
            label: "SymbolProc matches Proc.new",
        },
        Case {
            pattern: "(send (const {nil? cbase} :Proc) :new)",
            ruby: b"Lambda.new",
            expected: false,
            label: "SymbolProc rejects Lambda.new",
        },
        // ── Lint/RandOne ──────────────────────────────────────
        Case {
            pattern: "(send {(const {nil? cbase} :Kernel) nil?} :rand {(int {-1 1}) (float {-1.0 1.0})})",
            ruby: b"rand(1)",
            expected: true,
            label: "RandOne matches rand(1)",
        },
        Case {
            pattern: "(send {(const {nil? cbase} :Kernel) nil?} :rand {(int {-1 1}) (float {-1.0 1.0})})",
            ruby: b"rand(5)",
            expected: false,
            label: "RandOne rejects rand(5)",
        },
        // ── Performance/ReverseEach ───────────────────────────
        Case {
            pattern: "(send (send _ :reverse) :each)",
            ruby: b"arr.reverse.each",
            expected: true,
            label: "ReverseEach matches arr.reverse.each",
        },
        Case {
            pattern: "(send (send _ :reverse) :each)",
            ruby: b"arr.sort.each",
            expected: false,
            label: "ReverseEach rejects arr.sort.each",
        },
        // ── Style/StringConcatenation ─────────────────────────
        Case {
            pattern: "{(send str? :+ _) (send _ :+ str?)}",
            ruby: b"'hello' + x",
            expected: true,
            label: "StringConcatenation matches 'hello' + x",
        },
        // ── If with no else ───────────────────────────────────
        Case {
            pattern: "(if _ _ nil?)",
            ruby: b"if x; y; end",
            expected: true,
            label: "If matches when no else clause",
        },
        Case {
            pattern: "(if _ _ nil?)",
            ruby: b"if x; y; else; z; end",
            expected: false,
            label: "If rejects when else present",
        },
        // ── Array with rest ───────────────────────────────────
        Case {
            pattern: "(array ...)",
            ruby: b"[1, 2, 3]",
            expected: true,
            label: "Array rest matches [1,2,3]",
        },
        // ── Def pattern ───────────────────────────────────────
        Case {
            pattern: "(def :initialize ...)",
            ruby: b"def initialize; end",
            expected: true,
            label: "Def matches initialize",
        },
        Case {
            pattern: "(def :initialize ...)",
            ruby: b"def other_method; end",
            expected: false,
            label: "Def rejects other_method",
        },
        // ── Boolean/nil literals ──────────────────────────────
        Case {
            pattern: "true",
            ruby: b"true",
            expected: true,
            label: "True literal matches true",
        },
        Case {
            pattern: "false",
            ruby: b"false",
            expected: true,
            label: "False literal matches false",
        },
        Case {
            pattern: "nil",
            ruby: b"nil",
            expected: true,
            label: "Nil literal matches nil",
        },
        Case {
            pattern: "true",
            ruby: b"false",
            expected: false,
            label: "True literal rejects false",
        },
    ];

    let mut failures = Vec::new();

    for case in &cases {
        let result = ruby_prism::parse(case.ruby);
        let root = result.node();
        let program = root.as_program_node().unwrap();
        let stmts = program.statements();
        let node = stmts.body().iter().next().unwrap();

        let got = interpret_pattern(case.pattern, &node);
        if got != case.expected {
            failures.push(format!(
                "  {} — pattern={:?} ruby={:?} expected={} got={}",
                case.label,
                case.pattern,
                std::str::from_utf8(case.ruby).unwrap_or("?"),
                case.expected,
                got,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Verifier known-match failures ({}/{}):\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n"),
    );

    eprintln!(
        "verifier: all {}/{} known-match cases pass",
        cases.len(),
        cases.len(),
    );
}

// ---------- Vendor pattern parse coverage ----------

#[test]
fn verifier_vendor_pattern_parse_coverage() {
    use nitrocop::node_pattern::{Lexer, Parser, walk_vendor_patterns};
    use std::collections::HashMap;

    let vendor_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor");
    if !vendor_root.is_dir() {
        eprintln!("vendor/ directory not found — skipping vendor parse coverage test");
        return;
    }

    let patterns = walk_vendor_patterns(&vendor_root);
    if patterns.is_empty() {
        eprintln!("No vendor patterns extracted — submodules may not be initialized. Skipping.");
        return;
    }

    let mut total = 0;
    let mut parse_ok = 0;
    let mut parse_fail = 0;
    let mut failures: Vec<String> = Vec::new();
    let mut dept_stats: HashMap<String, (usize, usize)> = HashMap::new(); // (ok, fail)

    for (cop_name, extracted) in &patterns {
        total += 1;
        let dept = cop_name.split('/').next().unwrap_or("Unknown").to_string();

        let mut lexer = Lexer::new(&extracted.pattern);
        let tokens = lexer.tokenize();
        let mut parser = Parser::new(tokens);

        let entry = dept_stats.entry(dept.clone()).or_insert((0, 0));

        if parser.parse().is_some() {
            parse_ok += 1;
            entry.0 += 1;
        } else {
            parse_fail += 1;
            entry.1 += 1;
            failures.push(format!(
                "  FAIL: {cop_name}::{} — {}",
                extracted.method_name,
                extracted.pattern.chars().take(80).collect::<String>(),
            ));
        }
    }

    // Report stats
    eprintln!("\n=== Vendor Pattern Parse Coverage ===");
    eprintln!("Total patterns: {total}");
    eprintln!("Parse OK:       {parse_ok}");
    eprintln!("Parse FAIL:     {parse_fail}");
    let rate = if total > 0 {
        (parse_ok as f64 / total as f64) * 100.0
    } else {
        0.0
    };
    eprintln!("Parse rate:     {rate:.1}%");
    eprintln!();

    // Per-department breakdown
    let mut depts: Vec<_> = dept_stats.iter().collect();
    depts.sort_by_key(|(name, _)| *name);
    for (dept, (ok, fail)) in &depts {
        let dept_total = ok + fail;
        let dept_rate = if dept_total > 0 {
            (*ok as f64 / dept_total as f64) * 100.0
        } else {
            0.0
        };
        eprintln!("  {dept:20} {ok:4}/{dept_total:4} ({dept_rate:.0}%)");
    }

    if !failures.is_empty() {
        eprintln!("\nParse failures (first 20):");
        for f in failures.iter().take(20) {
            eprintln!("{f}");
        }
        if failures.len() > 20 {
            eprintln!("  ... and {} more", failures.len() - 20);
        }
    }

    // Assert parse rate > 90% to catch major regressions
    assert!(
        rate > 90.0,
        "Vendor pattern parse rate {rate:.1}% is below 90% threshold. \
         {parse_fail} of {total} patterns failed to parse.",
    );
}

// ---------- Shared module usage lint ----------

/// Ensures cop files don't inline patterns that have shared equivalents.
///
/// Scans `src/cop/` (excluding `src/cop/shared/`) for patterns like
/// `receiver().is_none() && name() == b"..."` that should use
/// `method_dispatch_predicates::is_command()`, operator_loc comparisons
/// that should use `predicate_operator_predicates`, etc.
///
/// This runs with `cargo test` on CI, catching regressions in new code.
#[test]
fn shared_module_usage_lint() {
    use regex::Regex;

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cop_dir = manifest.join("src/cop");
    let shared_dir = cop_dir.join("shared");

    struct Rule {
        pattern: Regex,
        message: &'static str,
    }

    let rules = vec![
        // --- method_dispatch_predicates ---
        Rule {
            pattern: Regex::new(
                r"\.receiver\(\)\.is_none\(\)\s*&&\s*\w+\.name\(\)\.as_slice\(\)\s*==",
            )
            .unwrap(),
            message: "use method_dispatch_predicates::is_command()",
        },
        Rule {
            pattern: Regex::new(
                r#"\.name\(\)\.as_slice\(\)\s*==\s*b"[^"]+"\s*&&\s*\w+\.receiver\(\)\.is_none\(\)"#,
            )
            .unwrap(),
            message: "use method_dispatch_predicates::is_command()",
        },
        Rule {
            pattern: Regex::new(
                r#"\.call_operator_loc\(\)\s*\.\s*is_some_and\(\|[^|]+\|\s*\w+\.as_slice\(\)\s*==\s*b"&\."#,
            )
            .unwrap(),
            message: "use method_dispatch_predicates::is_safe_navigation()",
        },
        Rule {
            pattern: Regex::new(
                r#"\.call_operator_loc\(\)\s*\.\s*is_some_and\(\|[^|]+\|\s*\w+\.as_slice\(\)\s*==\s*b"\."#,
            )
            .unwrap(),
            message: "use method_dispatch_predicates::is_dot_call()",
        },
        Rule {
            pattern: Regex::new(
                r#"\.call_operator_loc\(\)\s*\.\s*is_some_and\(\|[^|]+\|\s*\w+\.as_slice\(\)\s*==\s*b"::"#,
            )
            .unwrap(),
            message: "use method_dispatch_predicates::is_double_colon_call()",
        },
        // --- predicate_operator_predicates ---
        Rule {
            pattern: Regex::new(r#"\.operator_loc\(\)\.as_slice\(\)\s*==\s*b"&&""#).unwrap(),
            message: "use predicate_operator_predicates::is_logical_and()",
        },
        Rule {
            pattern: Regex::new(r#"\.operator_loc\(\)\.as_slice\(\)\s*==\s*b"\|\|""#).unwrap(),
            message: "use predicate_operator_predicates::is_logical_or()",
        },
        Rule {
            pattern: Regex::new(r#"\.operator_loc\(\)\.as_slice\(\)\s*==\s*b"and""#).unwrap(),
            message: "use predicate_operator_predicates::is_semantic_and()",
        },
        Rule {
            pattern: Regex::new(r#"\.operator_loc\(\)\.as_slice\(\)\s*==\s*b"or""#).unwrap(),
            message: "use predicate_operator_predicates::is_semantic_or()",
        },
        // --- shared util begins_its_line ---
        Rule {
            pattern: Regex::new(r"fn\s+begins_its_line\s*\(").unwrap(),
            message: "use shared::util::begins_its_line() instead of defining a local copy",
        },
    ];

    fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    collect_rs_files(&p, out);
                } else if p.extension().is_some_and(|e| e == "rs") {
                    out.push(p);
                }
            }
        }
    }

    let mut rs_files = Vec::new();
    collect_rs_files(&cop_dir, &mut rs_files);
    rs_files.sort();

    let mut violations: Vec<String> = Vec::new();

    for path in &rs_files {
        // Skip shared module files — they define these patterns, not consume them.
        if path.starts_with(&shared_dir) {
            continue;
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Find where #[cfg(test)] starts (if any) and only scan before it.
        let scan_end = content.find("#[cfg(test)]").unwrap_or(content.len());
        let scannable = &content[..scan_end];

        for (line_idx, line) in scannable.lines().enumerate() {
            let trimmed = line.trim();

            // Skip comments.
            if trimmed.starts_with("//") {
                continue;
            }

            for rule in &rules {
                if rule.pattern.is_match(line) {
                    let rel = path.strip_prefix(&manifest).unwrap_or(path);
                    violations.push(format!(
                        "{}:{}: {}",
                        rel.display(),
                        line_idx + 1,
                        rule.message,
                    ));
                }
            }
        }
    }

    if !violations.is_empty() {
        let mut msg = format!(
            "Found {} shared module usage violation(s). \
             Use the shared functions in src/cop/shared/ instead:\n\n",
            violations.len()
        );
        for v in &violations {
            msg.push_str(&format!("  {v}\n"));
        }
        panic!("{msg}");
    }
}
