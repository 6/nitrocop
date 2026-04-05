use crate::cop::shared::node_type::{
    ASSOC_NODE, CALL_NODE, CONSTANT_PATH_NODE, CONSTANT_READ_NODE, KEYWORD_HASH_NODE, PROGRAM_NODE,
    STRING_NODE, SYMBOL_NODE,
};
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use std::collections::HashMap;

/// RSpec/DescribeClass: The first argument to top-level describe should be
/// the class or module being tested.
///
/// ## Investigation findings (2026-03-11)
///
/// **FN=154 — Not recursing into module/class wrappers**: RuboCop's `TopLevelGroup`
/// mixin uses `top_level_nodes` which recurses into `module` and `class` bodies to
/// find top-level describe calls. nitrocop only checked direct `program.statements()`.
/// Fixed by adding `collect_top_level_nodes` that recurses into module/class/begin nodes.
///
/// **FP=4 — IgnoredMetadata value checking**: RuboCop's `IgnoredMetadata` is a hash
/// of `key: [values]` (e.g., `type: [request, controller]`). It checks BOTH key AND
/// value. nitrocop had a separate `has_type_metadata` that accepted ANY `type:` value,
/// and `has_ignored_metadata` that only checked key presence. Fixed by removing the
/// separate `type` check and implementing proper key+value matching against
/// `IgnoredMetadata` config (which includes `type` with specific allowed values in
/// its defaults).
///
/// **looks_like_constant regex mismatch**: RuboCop uses `/^(?:(?:::)?[A-Z]\w*)+$/`
/// which requires each segment after `::` to start with uppercase. nitrocop's version
/// only checked the first segment. Fixed to validate each `::` segment starts uppercase.
///
/// **FP=239 — Unconditional module/class unwrapping**: `visit_top_level_nodes` was
/// recursing into ALL module/class wrappers unconditionally. But RuboCop's
/// `TopLevelGroup#top_level_nodes` only unwraps module/class/begin when it is the
/// **sole** child at that nesting level. When there are multiple siblings (e.g.,
/// `require 'spec_helper'` + `module Foo`), none are unwrapped — each is checked
/// directly. Fixed by adding a `stmts.len() == 1` guard before recursing into
/// module/class wrappers. Same pattern as SpecFilePathFormat's
/// `collect_top_level_spec_groups`.
///
/// ## Corpus investigation (2026-03-12)
///
/// FP=4 remaining. Root cause: `check_top_level_describe` did not verify the
/// describe call has a block. RuboCop's `TopLevelGroup` only fires for block
/// nodes wrapping describe calls (`(block (send ...))` pattern). A bare
/// `describe 'foo'` without `do...end` is not a spec group. Fixed by adding
/// `call.block().is_none()` guard.
///
/// ## Corpus investigation (2026-03-14)
///
/// Corpus oracle reported FP=3, FN=0.
///
/// FP=3: Fixed. Root cause: `describe '...', &(proc do end)` uses BlockArgumentNode
/// in `call.block()`, so `call.block().is_none()` guard did NOT skip it. RuboCop's
/// `TopLevelGroup` uses the `(block (send ...))` AST pattern which requires an actual
/// BlockNode wrapper around the describe call, not block_pass. Fix: changed guard to
/// require `call.block().as_block_node().is_some()` — only actual BlockNode qualifies.
pub struct DescribeClass;

impl Cop for DescribeClass {
    fn name(&self) -> &'static str {
        "RSpec/DescribeClass"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &[
            "**/spec/features/**/*",
            "**/spec/requests/**/*",
            "**/spec/routing/**/*",
            "**/spec/system/**/*",
            "**/spec/views/**/*",
        ]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            CALL_NODE,
            CONSTANT_PATH_NODE,
            CONSTANT_READ_NODE,
            KEYWORD_HASH_NODE,
            PROGRAM_NODE,
            STRING_NODE,
            SYMBOL_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let program = match node.as_program_node() {
            Some(p) => p,
            None => return,
        };

        // Parse IgnoredMetadata config: hash of key -> array of allowed values
        let ignored_metadata = parse_ignored_metadata(config);

        let stmts: Vec<_> = program.statements().body().iter().collect();
        visit_top_level_nodes(self, source, &stmts, diagnostics, &ignored_metadata);
    }
}

/// Visit a list of sibling statements, mirroring RuboCop's `TopLevelGroup#top_level_nodes`.
///
/// RuboCop only unwraps module/class when it is the **sole** child at that nesting level.
/// When there are multiple siblings (e.g., `require` + `module`), none are unwrapped —
/// each is checked directly as a potential top-level describe.
fn visit_top_level_nodes(
    cop: &DescribeClass,
    source: &SourceFile,
    stmts: &[ruby_prism::Node<'_>],
    diagnostics: &mut Vec<Diagnostic>,
    ignored_metadata: &HashMap<String, Vec<String>>,
) {
    if stmts.len() == 1 {
        let node = &stmts[0];
        if let Some(module_node) = node.as_module_node() {
            if let Some(body) = module_node.body() {
                visit_body(cop, source, &body, diagnostics, ignored_metadata);
            }
            return;
        }
        if let Some(class_node) = node.as_class_node() {
            if let Some(body) = class_node.body() {
                visit_body(cop, source, &body, diagnostics, ignored_metadata);
            }
            return;
        }
    }
    // Multiple siblings or non-wrapper node: check each directly
    for stmt in stmts {
        check_top_level_describe(cop, source, stmt, diagnostics, ignored_metadata);
    }
}

/// Extract children from a body node (StatementsNode or BeginNode) and recurse.
fn visit_body(
    cop: &DescribeClass,
    source: &SourceFile,
    body: &ruby_prism::Node<'_>,
    diagnostics: &mut Vec<Diagnostic>,
    ignored_metadata: &HashMap<String, Vec<String>>,
) {
    if let Some(stmts) = body.as_statements_node() {
        let children: Vec<_> = stmts.body().iter().collect();
        visit_top_level_nodes(cop, source, &children, diagnostics, ignored_metadata);
    } else if let Some(begin) = body.as_begin_node() {
        if let Some(stmts) = begin.statements() {
            let children: Vec<_> = stmts.body().iter().collect();
            visit_top_level_nodes(cop, source, &children, diagnostics, ignored_metadata);
        }
    } else {
        // Single expression body — treat as sole child
        check_top_level_describe(cop, source, body, diagnostics, ignored_metadata);
    }
}

/// Parse `IgnoredMetadata` config into a HashMap<String, Vec<String>>.
/// The YAML format is: `IgnoredMetadata: { type: [request, controller, ...] }`.
fn parse_ignored_metadata(config: &CopConfig) -> HashMap<String, Vec<String>> {
    let mut result = HashMap::new();
    let val = match config.options.get("IgnoredMetadata") {
        Some(v) => v,
        None => return result,
    };
    if let Some(mapping) = val.as_mapping() {
        for (k, v) in mapping.iter() {
            let key = match k.as_str() {
                Some(s) => s.to_string(),
                None => continue,
            };
            let mut values = Vec::new();
            if let Some(seq) = v.as_sequence() {
                for item in seq {
                    if let Some(s) = item.as_str() {
                        values.push(s.to_string());
                    }
                }
            }
            result.insert(key, values);
        }
    }
    result
}

fn check_top_level_describe(
    cop: &DescribeClass,
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
    diagnostics: &mut Vec<Diagnostic>,
    ignored_metadata: &HashMap<String, Vec<String>>,
) {
    let call = match node.as_call_node() {
        Some(c) => c,
        None => return,
    };

    let name = call.name().as_slice();
    if name != b"describe" {
        return;
    }

    // RuboCop's TopLevelGroup only fires for block nodes (describe ... do/end).
    // A bare `describe 'foo'` without a block is not a spec group.
    // Also, `describe '...', &(proc do end)` uses BlockArgumentNode in call.block(),
    // which RuboCop does NOT treat as a spec group (requires `(block (send ...))` pattern,
    // i.e., an actual BlockNode wrapper, not block_pass).
    if call.block().is_none_or(|b| b.as_block_node().is_none()) {
        return;
    }

    // Must be receiverless or RSpec.describe / ::RSpec.describe
    if let Some(recv) = call.receiver() {
        let is_rspec = if let Some(cr) = recv.as_constant_read_node() {
            cr.name().as_slice() == b"RSpec"
        } else if let Some(cp) = recv.as_constant_path_node() {
            cp.name().is_some_and(|n| n.as_slice() == b"RSpec") && cp.parent().is_none()
        } else {
            false
        };
        if !is_rspec {
            return;
        }
    }

    let args = match call.arguments() {
        Some(a) => a,
        None => return, // No arguments = empty describe, OK
    };

    let arg_list: Vec<_> = args.arguments().iter().collect();
    if arg_list.is_empty() {
        return;
    }

    let first_arg = &arg_list[0];

    // If first arg is a constant or constant path, it's fine
    if first_arg.as_constant_read_node().is_some() || first_arg.as_constant_path_node().is_some() {
        return;
    }

    // If first arg is a string, check if it looks like a class/module name
    if let Some(s) = first_arg.as_string_node() {
        let value = s.unescaped();
        if looks_like_constant(value) {
            return; // String that looks like a constant name is OK
        }
    }

    // Check for IgnoredMetadata (includes type: checks)
    if has_ignored_metadata(&arg_list, ignored_metadata) {
        return;
    }

    // Flag the first argument
    let loc = first_arg.location();
    let (line, col) = source.offset_to_line_col(loc.start_offset());
    diagnostics.push(cop.diagnostic(
        source,
        line,
        col,
        "The first argument to describe should be the class or module being tested.".to_string(),
    ));
}

/// Check if a string value looks like a Ruby constant name.
/// Matches RuboCop's regex: `/^(?:(?:::)?[A-Z]\w*)+$/`
/// Each segment (separated by `::`) must start with an uppercase letter.
fn looks_like_constant(value: &[u8]) -> bool {
    if value.is_empty() {
        return false;
    }
    // Skip leading ::
    let mut i = 0;
    if value.starts_with(b"::") {
        i = 2;
    }
    // Must have at least one segment starting with uppercase
    if i >= value.len() || !value[i].is_ascii_uppercase() {
        return false;
    }
    // Parse segments separated by ::
    while i < value.len() {
        // Each segment must start with uppercase
        if !value[i].is_ascii_uppercase() {
            return false;
        }
        i += 1;
        // Consume word chars (alphanumeric + underscore)
        while i < value.len() && (value[i].is_ascii_alphanumeric() || value[i] == b'_') {
            i += 1;
        }
        // If we're at end, success
        if i >= value.len() {
            return true;
        }
        // Must be ::
        if i + 1 < value.len() && value[i] == b':' && value[i + 1] == b':' {
            i += 2;
            // :: at end is invalid
            if i >= value.len() {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

/// Check if any argument has metadata matching IgnoredMetadata config.
/// Checks both key AND value against the configured hash.
fn has_ignored_metadata(
    args: &[ruby_prism::Node<'_>],
    ignored_metadata: &HashMap<String, Vec<String>>,
) -> bool {
    if ignored_metadata.is_empty() {
        return false;
    }
    for arg in args {
        let elements = if let Some(kw) = arg.as_keyword_hash_node() {
            kw.elements()
        } else if let Some(h) = arg.as_hash_node() {
            h.elements()
        } else {
            continue;
        };
        for elem in elements.iter() {
            if let Some(assoc) = elem.as_assoc_node() {
                if let Some(sym) = assoc.key().as_symbol_node() {
                    let key_name = sym.unescaped();
                    let key_str = match std::str::from_utf8(key_name) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    if let Some(allowed_values) = ignored_metadata.get(key_str) {
                        // If allowed_values is empty, any value matches
                        if allowed_values.is_empty() {
                            return true;
                        }
                        // Check if the value is in the allowed list
                        if let Some(val_sym) = assoc.value().as_symbol_node() {
                            let val_name = val_sym.unescaped();
                            let val_str = match std::str::from_utf8(val_name) {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                            if allowed_values.iter().any(|v| v == val_str) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_scenario_fixture_tests!(
        DescribeClass,
        "cops/rspec/describe_class",
        bare_describe = "bare_describe.rb",
        module_wrapper = "module_wrapper.rb",
        nested_modules = "nested_modules.rb",
        class_wrapper = "class_wrapper.rb",
    );

    #[test]
    fn ignored_metadata_skips_describe_with_matching_key_and_value() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        // IgnoredMetadata: { feature: [true_val] } — key "feature" with value "true_val"
        // Here we use the hash format: key -> array of values
        let mut meta_map = serde_yml::Mapping::new();
        let mut values = serde_yml::Sequence::new();
        values.push(serde_yml::Value::String("true".into()));
        meta_map.insert(
            serde_yml::Value::String("feature".into()),
            serde_yml::Value::Sequence(values),
        );
        let config = CopConfig {
            options: HashMap::from([(
                "IgnoredMetadata".into(),
                serde_yml::Value::Mapping(meta_map),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe 'some feature', feature: :true do\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&DescribeClass, source, config);
        assert!(
            diags.is_empty(),
            "Should skip when IgnoredMetadata key+value matches"
        );
    }

    #[test]
    fn ignored_metadata_flags_when_value_does_not_match() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        // IgnoredMetadata: { type: [request] } — only "request" is allowed
        let mut meta_map = serde_yml::Mapping::new();
        let mut values = serde_yml::Sequence::new();
        values.push(serde_yml::Value::String("request".into()));
        meta_map.insert(
            serde_yml::Value::String("type".into()),
            serde_yml::Value::Sequence(values),
        );
        let config = CopConfig {
            options: HashMap::from([(
                "IgnoredMetadata".into(),
                serde_yml::Value::Mapping(meta_map),
            )]),
            ..CopConfig::default()
        };
        // type: :model is NOT in the allowed list [request], so should flag
        let source = b"describe 'some feature', type: :model do\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&DescribeClass, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag when IgnoredMetadata value doesn't match"
        );
    }

    #[test]
    fn ignored_metadata_still_flags_without_matching_key() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let mut meta_map = serde_yml::Mapping::new();
        let mut values = serde_yml::Sequence::new();
        values.push(serde_yml::Value::String("bar".into()));
        meta_map.insert(
            serde_yml::Value::String("feature".into()),
            serde_yml::Value::Sequence(values),
        );
        let config = CopConfig {
            options: HashMap::from([(
                "IgnoredMetadata".into(),
                serde_yml::Value::Mapping(meta_map),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe 'some feature' do\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&DescribeClass, source, config);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn looks_like_constant_rejects_lowercase_after_separator() {
        assert!(!looks_like_constant(b"Foo::bar"));
        assert!(looks_like_constant(b"Foo::Bar"));
        assert!(looks_like_constant(b"Foo::Bar::Baz"));
        assert!(looks_like_constant(b"::Foo::Bar"));
        assert!(!looks_like_constant(b"activeRecord"));
        assert!(!looks_like_constant(b"2Thing"));
        assert!(!looks_like_constant(b""));
    }

    #[test]
    fn explore_shared_examples_not_flagged() {
        // shared_examples at top level should not be flagged
        let source = b"shared_examples 'Common::Interface' do\n  it 'works' do\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(
            diags.is_empty(),
            "shared_examples should not be flagged: {diags:?}"
        );
    }

    #[test]
    fn explore_shared_examples_for_not_flagged() {
        let source = b"shared_examples_for 'something' do\n  it 'works' do\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(
            diags.is_empty(),
            "shared_examples_for should not be flagged: {diags:?}"
        );
    }

    #[test]
    fn explore_feature_not_flagged() {
        // feature 'something' at top level - describe_class only checks 'describe'
        let source = b"feature 'Login' do\n  scenario 'works' do\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(diags.is_empty(), "feature should not be flagged: {diags:?}");
    }

    #[test]
    fn explore_context_not_flagged() {
        let source = b"context 'something' do\n  it 'works' do\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(diags.is_empty(), "context should not be flagged: {diags:?}");
    }

    #[test]
    fn explore_describe_inside_shared_examples() {
        // describe inside shared_examples should not be flagged (not top-level)
        let source = b"shared_examples 'foo' do\n  describe '#method' do\n    it 'works' do\n    end\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(
            diags.is_empty(),
            "describe inside shared_examples should not be flagged: {diags:?}"
        );
    }

    #[test]
    fn explore_describe_inside_shared_context() {
        let source = b"shared_context 'foo' do\n  describe '#method' do\n    it 'works' do\n    end\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(
            diags.is_empty(),
            "describe inside shared_context should not be flagged: {diags:?}"
        );
    }

    #[test]
    fn explore_describe_with_symbol_first_arg() {
        // describe :symbol — should be flagged (not a constant)
        let source = b"describe :foo do\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert_eq!(diags.len(), 1, "describe with symbol should be flagged");
    }

    #[test]
    fn explore_describe_with_method_call_first_arg() {
        // describe method_call — should be flagged
        let source = b"describe some_method do\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert_eq!(
            diags.len(),
            1,
            "describe with method call should be flagged"
        );
    }

    #[test]
    fn explore_describe_with_self_first_arg() {
        // RuboCop would flag describe(self)
        let source = b"describe self do\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert_eq!(diags.len(), 1, "describe with self should be flagged");
    }

    #[test]
    fn explore_nested_describe_not_flagged() {
        // nested describe inside another describe is not top-level
        let source = b"describe SomeClass do\n  describe 'bad describe' do\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(
            diags.is_empty(),
            "nested describe should not be flagged: {diags:?}"
        );
    }

    #[test]
    fn explore_describe_with_block_pass() {
        // describe &block - should be flagged
        let source = b"describe &block do\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        // This may not parse correctly, just checking behavior
        eprintln!("block_pass diags: {diags:?}");
    }

    #[test]
    fn explore_rspec_shared_examples() {
        // RSpec.shared_examples should not be flagged
        let source = b"RSpec.shared_examples 'something' do\n  it 'works' do\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert!(
            diags.is_empty(),
            "RSpec.shared_examples should not be flagged: {diags:?}"
        );
    }

    #[test]
    fn explore_describe_without_block() {
        // describe 'foo' without a block — RuboCop wouldn't flag (not a spec_group?)
        let source = b"describe 'foo'\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        eprintln!("describe without block: {diags:?}");
    }

    #[test]
    fn explore_describe_inside_begin_rescue() {
        // describe inside begin/rescue at top level
        let source = b"begin\n  describe 'foo' do\n  end\nrescue\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        eprintln!("describe in begin/rescue: {diags:?}");
    }

    #[test]
    fn explore_describe_inside_if() {
        // describe inside if at top level
        let source = b"if ENV['RUN_SPECS']\n  describe 'foo' do\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        eprintln!("describe in if: {diags:?}");
    }

    #[test]
    fn explore_describe_with_heredoc() {
        // RuboCop treats top_level_nodes -> begin (which is the program body)
        // vs nitrocop which iterates program.statements
        let source = b"require 'spec_helper'\n\ndescribe 'something' do\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        assert_eq!(
            diags.len(),
            1,
            "describe with string should be flagged: {diags:?}"
        );
    }

    #[test]
    fn explore_describe_with_interpolated_string() {
        // Interpolated string as first arg
        let source = b"describe \"#{some_var}\" do\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        eprintln!("interpolated string: {diags:?}");
    }

    #[test]
    fn explore_describe_with_hash_rocket_metadata() {
        // type => :model (hash rocket instead of symbol key)
        let source = b"describe 'foo', 'type' => :model do\nend\n";
        let diags = crate::testutil::run_cop_full(&DescribeClass, source);
        eprintln!("hash rocket metadata: {diags:?}");
    }
}
