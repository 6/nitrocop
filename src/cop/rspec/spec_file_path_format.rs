use crate::cop::shared::util::{RSPEC_DEFAULT_INCLUDE, is_rspec_example_group};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// RSpec/SpecFilePathFormat — checks that spec file paths match the described class.
///
/// ## Root cause of prior FPs/FNs
///
/// Two issues caused ~27 FPs and ~1,075 FNs:
///
/// 1. **Missing `top_level_nodes()` recursion (~1,050 FNs):** The old implementation only
///    checked direct children of ProgramNode. RuboCop's `TopLevelGroup` mixin recursively
///    unwraps `module`, `class`, and `begin` wrappers to find example groups nested inside
///    namespace modules (e.g., `module Foo; describe Bar do; end; end`). Without this
///    recursion, most real-world specs were missed entirely.
///
/// 2. **Missing namespace extraction (~900 FNs, overlapping with #1):** RuboCop's `Namespace`
///    mixin traverses ancestor `module`/`class` nodes and prepends their names to the expected
///    path. For example, `module Foo; describe Bar; end` expects `foo/bar*_spec.rb`. The old
///    implementation had no namespace awareness, so even when it found a describe inside a
///    module, it generated the wrong expected path.
///
/// ## Fix (round 1)
///
/// - Switched from `check_node(PROGRAM_NODE)` to `check_source` with manual AST traversal.
/// - Implemented `top_level_nodes()` that recursively unwraps module/class/begin wrappers,
///   mirroring RuboCop's `TopLevelGroup#top_level_nodes`.
/// - Implemented namespace extraction that collects enclosing module/class names when
///   traversing into wrappers, mirroring RuboCop's `Namespace#namespace`.
/// - CustomTransform is checked per-component (namespace + class parts individually).
///
/// ## Root cause of remaining FPs/FNs (round 2, ~1,487 FP + ~1,023 FN)
///
/// Five issues identified by comparing against vendor RuboCop source:
///
/// 1. **`path_matches` used `contains()` instead of regex (~major FP/FN source):** RuboCop
///    builds a regex like `my_class[^/]*_spec\.rb$` and matches it against the expanded file
///    path. nitrocop used case-insensitive `contains()` which could match the class name
///    anywhere in the path (e.g., `/home/foo/spec/bar_spec.rb` falsely matched class `Foo`
///    because `foo` appeared in a parent directory). Fixed by implementing regex-based matching
///    that anchors the class path pattern to the end of the file path.
///
/// 2. **Method name cleaning difference:** RuboCop uses `gsub(/\s/, '_').gsub(/\W/, '')` which
///    replaces whitespace with `_` then strips non-word chars. nitrocop replaced ALL non-
///    alphanumeric with `_`, causing double underscores (e.g., backtick-surrounded text
///    produced `via__local_failures` instead of `via_local_failures`).
///
/// 3. **Shared groups excluded from top-level count (~FP source):** RuboCop's `top_level_groups`
///    includes `shared_examples`/`shared_context` in the count for the `.one?` check. If a file
///    has 1 describe + 1 shared_examples, RuboCop sees 2 groups and skips. nitrocop filtered
///    shared groups out before counting, so it saw 1 and proceeded to flag.
///
/// 4. **No block requirement:** RuboCop only matches `(block (send ...))` — the describe must
///    have a block. nitrocop matched bare call nodes without blocks.
///
/// 5. **No receiver check:** RuboCop requires `#rspec?` receiver (nil or `RSpec` constant).
///    nitrocop accepted any receiver, causing FPs on `SomeLib.describe MyClass`.
///
/// ## Root cause of remaining FPs (round 3, ~1,726 FP + ~4 FN)
///
/// The hand-rolled `simple_regex_match` function only found the FIRST occurrence of the
/// class path pattern in the file path (via `haystack.find()`). RuboCop uses a real regex
/// engine that tries ALL positions. When the class path (e.g., `my_class`) appeared in a
/// parent directory (e.g., `/home/my_class_app/spec/models/my_class_spec.rb`), the first
/// match was in `my_class_app/`, which failed the `[^/]*` check because of the intervening
/// `/`. The correct match at `my_class_spec.rb` was never tried, causing a false positive.
///
/// Fixed by replacing the hand-rolled matcher with the `regex` crate, which correctly
/// tries all positions in the string (matching Ruby's `String#match?` semantics).
///
/// ## Root cause of remaining FPs (round 4, ~1,480 FP + ~3 FN)
///
/// `collect_top_level_spec_groups` recursed into ALL module/class children at every
/// level, but RuboCop's `top_level_nodes(root_node)` only unwraps module/class when
/// the node IS the one being processed (not a sibling). Specifically:
///
/// - If the AST root is `:begin` (multiple top-level statements like `require` +
///   `module Foo`), RuboCop returns ALL children flat — the module is NOT unwrapped.
///   Since `spec_group?` requires `(any_block (send ...))`, a module node fails,
///   giving 0 top-level groups → the file is skipped.
/// - Only when the root IS a `:module`/`:class` (single top-level statement) does
///   RuboCop recurse into the body.
///
/// Most real-world spec files have `require 'spec_helper'` at the top, making the
/// root `:begin`. If the describe is wrapped in a module, RuboCop skips the file
/// but nitrocop was processing it, generating ~1,480 false positives.
///
/// Fixed by rewriting `collect_top_level_spec_groups` to only unwrap module/class/begin
/// when they are the SOLE statement at the current level, exactly mirroring RuboCop's
/// `top_level_nodes` recursion pattern.
///
/// ## Root cause of remaining FPs (round 5, ~10 FP + ~0 FN)
///
/// Source-text-based constant path extraction: for `ConstantPathNode` (e.g., `Foo::Bar`),
/// the old code extracted raw source text via `location().start_offset()..end_offset()`.
/// When the constant path spanned multiple lines (e.g., `Foo::\n  Bar`), the extracted
/// text included whitespace/newlines, producing wrong expected paths like `foo/\n  bar`
/// instead of `foo/bar`. RuboCop's Parser gem uses AST-based `const_name` which always
/// returns clean `Foo::Bar` regardless of source formatting.
///
/// Fixed by replacing source-text extraction with recursive AST traversal
/// (`collect_constant_path_segments`) that walks `ConstantPathNode.parent()` and
/// `ConstantPathNode.name()` to collect clean name segments.
pub struct SpecFilePathFormat;

impl Cop for SpecFilePathFormat {
    fn name(&self) -> &'static str {
        "RSpec/SpecFilePathFormat"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/spec/routing/**/*"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let custom_transform = config
            .get_string_hash("CustomTransform")
            .unwrap_or_default();
        let ignore_methods = config.get_bool("IgnoreMethods", false);
        let ignore_metadata = config.get_string_hash("IgnoreMetadata").unwrap_or_default();
        let _inflector_path = config.get_str("InflectorPath", "");
        let _enforced_inflector = config.get_str("EnforcedInflector", "default");

        let program = match parse_result.node().as_program_node() {
            Some(p) => p,
            None => return,
        };

        let stmts: Vec<ruby_prism::Node<'_>> = program.statements().body().iter().collect();

        // Collect ALL top-level spec groups (example groups + shared groups),
        // unwrapping module/class/begin wrappers.
        // This mirrors RuboCop's TopLevelGroup#top_level_nodes + spec_group? filter.
        let mut all_spec_groups: Vec<(ruby_prism::CallNode<'_>, Vec<String>, bool)> = Vec::new();
        let namespace: Vec<String> = Vec::new();
        collect_top_level_spec_groups(&stmts, source, &namespace, &mut all_spec_groups);

        // If not exactly one top-level spec group, skip (ambiguous or none).
        // This matches RuboCop's `return unless top_level_groups.one?`.
        // Note: shared_examples/shared_context count toward this total.
        if all_spec_groups.len() != 1 {
            return;
        }

        // The single group must be an example group (not shared_examples/shared_context).
        // RuboCop only calls on_top_level_example_group for example_group? nodes.
        let (call, namespace, is_example_group) = &all_spec_groups[0];
        if !is_example_group {
            return;
        }
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        // First arg must be a constant (class name)
        let first_arg = &arg_list[0];
        let class_name = if let Some(cr) = first_arg.as_constant_read_node() {
            std::str::from_utf8(cr.name().as_slice())
                .unwrap_or("")
                .to_string()
        } else if let Some(cp) = first_arg.as_constant_path_node() {
            // Walk the AST to extract constant path segments, avoiding source
            // text which can contain whitespace/newlines between :: and names.
            collect_constant_path_segments(&cp.as_node()).join("::")
        } else {
            return;
        };

        // IgnoreMetadata: skip check if metadata matches ignored key:value pairs
        if !ignore_metadata.is_empty() && arg_list.len() >= 2 {
            for arg in &arg_list[1..] {
                if let Some(hash) = arg.as_keyword_hash_node() {
                    for elem in hash.elements().iter() {
                        if let Some(assoc) = elem.as_assoc_node() {
                            if let Some(sym) = assoc.key().as_symbol_node() {
                                let key_str = std::str::from_utf8(sym.unescaped()).unwrap_or("");
                                if let Some(expected_value) = ignore_metadata.get(key_str) {
                                    let actual_value = if let Some(val_sym) =
                                        assoc.value().as_symbol_node()
                                    {
                                        std::str::from_utf8(val_sym.unescaped())
                                            .unwrap_or("")
                                            .to_string()
                                    } else if let Some(val_str) = assoc.value().as_string_node() {
                                        std::str::from_utf8(val_str.unescaped())
                                            .unwrap_or("")
                                            .to_string()
                                    } else {
                                        String::new()
                                    };
                                    if actual_value == *expected_value {
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Build expected path: namespace segments + class name segments
        let expected_path = build_expected_path(namespace, &class_name, &custom_transform);

        // Get optional method description from second argument.
        // RuboCop's ignore?() returns true if the argument is nil or not a string,
        // or if IgnoreMethods is true. When not ignored, name_pattern() applies
        // gsub(/\s/, '_').gsub(/\W/, '') to get the cleaned method name.
        let has_method_arg = arg_list.len() >= 2 && arg_list[1].as_string_node().is_some();
        let is_ignored = ignore_methods || !has_method_arg;

        let method_part = if is_ignored {
            None
        } else {
            let s = arg_list[1].as_string_node().unwrap();
            let val = std::str::from_utf8(s.unescaped()).unwrap_or("");
            // Match RuboCop: gsub(/\s/, '_').gsub(/\W/, '')
            // First: replace whitespace with underscore
            let step1: String = val
                .chars()
                .map(|c| if c.is_whitespace() { '_' } else { c })
                .collect();
            // Second: remove non-word characters (keep [a-zA-Z0-9_])
            let cleaned: String = step1
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            Some(cleaned)
        };

        // Build the regex pattern matching RuboCop's correct_path_pattern:
        //   expected_path [.* name_pattern] [^/]*_spec\.rb
        // When the method arg is not ignored, insert .* between class and method,
        // and the method name before [^/]*_spec\.rb.
        let regex_pattern = if is_ignored {
            format!(r"{expected_path}[^/]*_spec\.rb$")
        } else {
            let m = method_part.as_deref().unwrap_or("");
            format!(r"{expected_path}.*{m}[^/]*_spec\.rb$")
        };

        // Build human-readable suffix (glob-like) for the offense message.
        // RuboCop does: pattern.sub('.*', '*').sub('[^/]*', '*').sub('\.', '.')
        let expected_suffix = if is_ignored {
            format!("{expected_path}*_spec.rb")
        } else {
            let m = method_part.as_deref().unwrap_or("");
            format!("{expected_path}*{m}*_spec.rb")
        };

        let file_path = source.path_str();
        let normalized = file_path.replace('\\', "/");

        if !path_matches_regex(&normalized, &regex_pattern) {
            let loc = call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!("Spec path should end with `{expected_suffix}`."),
            ));
        }
    }
}

/// Mirror RuboCop's `TopLevelGroup#top_level_nodes` exactly.
///
/// RuboCop's algorithm starts from the AST root and follows a chain of single
/// module/class/begin wrappers. Once it reaches a `:begin` node (multiple siblings)
/// or any non-wrapper node, it returns all nodes at that level WITHOUT further
/// recursion. The caller then filters with `spec_group?`.
///
/// Critical behavior: if the root is `:begin` (e.g., `require` + `module`), the
/// module is returned as-is — it is NOT unwrapped. Since `spec_group?` requires
/// `(any_block (send ...))`, a module node fails the check. This means files with
/// `require 'spec_helper'` + `module Foo; describe Bar; end` have 0 top-level
/// spec groups and are SKIPPED entirely.
///
/// The old nitrocop implementation incorrectly recursed into ALL module/class
/// children regardless of siblings, causing ~1,480 FPs.
fn collect_top_level_spec_groups<'pr>(
    stmts: &[ruby_prism::Node<'pr>],
    source: &SourceFile,
    namespace: &[String],
    found: &mut Vec<(ruby_prism::CallNode<'pr>, Vec<String>, bool)>,
) {
    // If there is exactly one statement and it's a module/class/begin wrapper,
    // unwrap it (recursing into its body). This mirrors RuboCop where the root
    // node is `:module`/`:class` → `top_level_nodes(node.body)`.
    if stmts.len() == 1 {
        let stmt = &stmts[0];

        if let Some(module_node) = stmt.as_module_node() {
            let module_names = extract_defined_name(source, &module_node.constant_path());
            if !module_names.is_empty() {
                let mut new_ns = namespace.to_vec();
                new_ns.extend(module_names);
                if let Some(body) = module_node.body() {
                    let children: Vec<_> = body
                        .as_statements_node()
                        .iter()
                        .flat_map(|s| s.body().iter())
                        .collect();
                    collect_top_level_spec_groups(&children, source, &new_ns, found);
                }
            }
            return;
        }

        if let Some(class_node) = stmt.as_class_node() {
            let class_names = extract_defined_name(source, &class_node.constant_path());
            if !class_names.is_empty() {
                let mut new_ns = namespace.to_vec();
                new_ns.extend(class_names);
                if let Some(body) = class_node.body() {
                    let children: Vec<_> = body
                        .as_statements_node()
                        .iter()
                        .flat_map(|s| s.body().iter())
                        .collect();
                    collect_top_level_spec_groups(&children, source, &new_ns, found);
                }
            }
            return;
        }

        if let Some(begin_node) = stmt.as_begin_node() {
            if let Some(stmts_node) = begin_node.statements() {
                let children: Vec<_> = stmts_node.body().iter().collect();
                collect_top_level_spec_groups(&children, source, namespace, found);
            }
            return;
        }
    }

    // Multiple statements OR single non-wrapper statement: these are the "top level".
    // Check each for being a spec group — do NOT recurse into modules/classes.
    // This matches RuboCop's `:begin` case where children are returned flat.
    for stmt in stmts {
        if let Some(call) = stmt.as_call_node() {
            if call.block().is_some() {
                if let Some(entry) = check_spec_group_call(call, namespace) {
                    found.push(entry);
                }
            }
        }
    }
}

/// Check if a call node is a spec group (example group or shared group) with valid receiver.
/// Returns (call, namespace, is_example_group) or None if not a spec group.
fn check_spec_group_call<'pr>(
    call: ruby_prism::CallNode<'pr>,
    namespace: &[String],
) -> Option<(ruby_prism::CallNode<'pr>, Vec<String>, bool)> {
    let name = call.name().as_slice();

    // Check if it's an example group or shared group
    let is_shared =
        name == b"shared_examples" || name == b"shared_examples_for" || name == b"shared_context";
    let is_example = is_rspec_example_group(name) && !is_shared;

    if !is_example && !is_shared {
        return None;
    }

    // Check receiver: must be nil (receiverless) or RSpec constant
    // This matches RuboCop's `#rspec?` pattern: `{#explicit_rspec? nil?}`
    if let Some(recv) = call.receiver() {
        if let Some(cr) = recv.as_constant_read_node() {
            if cr.name().as_slice() != b"RSpec" {
                return None;
            }
        } else if let Some(cp) = recv.as_constant_path_node() {
            // ::RSpec
            let is_rspec =
                cp.name().is_some_and(|n| n.as_slice() == b"RSpec") && cp.parent().is_none();
            if !is_rspec {
                return None;
            }
        } else {
            return None;
        }
    }

    Some((call, namespace.to_vec(), is_example))
}

/// Extract the defined name segments from a module/class constant path.
/// Uses AST traversal instead of source text to avoid whitespace/newline issues.
fn extract_defined_name(_source: &SourceFile, node: &ruby_prism::Node<'_>) -> Vec<String> {
    collect_constant_path_segments(node)
}

/// Recursively walk a ConstantPathNode or ConstantReadNode to collect name segments.
/// For `Foo::Bar::Baz`, returns `["Foo", "Bar", "Baz"]`.
/// For `::Foo::Bar`, returns `["Foo", "Bar"]` (leading `::` is ignored).
fn collect_constant_path_segments(node: &ruby_prism::Node<'_>) -> Vec<String> {
    if let Some(cr) = node.as_constant_read_node() {
        let name = std::str::from_utf8(cr.name().as_slice()).unwrap_or("");
        return vec![name.to_string()];
    }
    if let Some(cp) = node.as_constant_path_node() {
        let mut segments = if let Some(parent) = cp.parent() {
            collect_constant_path_segments(&parent)
        } else {
            // Leading :: (e.g., ::Foo) — parent is None
            Vec::new()
        };
        if let Some(name_id) = cp.name() {
            let name = std::str::from_utf8(name_id.as_slice()).unwrap_or("");
            segments.push(name.to_string());
        }
        return segments;
    }
    Vec::new()
}

/// Build the expected file path from namespace + class name, applying CustomTransform.
fn build_expected_path(
    namespace: &[String],
    class_name: &str,
    custom_transform: &std::collections::HashMap<String, String>,
) -> String {
    let class_parts: Vec<&str> = class_name.split("::").collect();
    let all_segments: Vec<String> = namespace
        .iter()
        .map(|s| s.to_string())
        .chain(class_parts.iter().map(|s| s.to_string()))
        .collect();

    let path_parts: Vec<String> = all_segments
        .iter()
        .map(|name| {
            if let Some(custom) = custom_transform.get(name.as_str()) {
                custom.clone()
            } else {
                camel_to_snake(name)
            }
        })
        .collect();

    path_parts.join("/")
}

/// Convert CamelCase to snake_case using the same algorithm as RuboCop's
/// DefaultInflector:
///   string.gsub(/([^A-Z])([A-Z]+)/, '\1_\2')
///         .gsub(/([A-Z])([A-Z][^A-Z\d]+)/, '\1_\2')
///         .downcase
///
/// This differs from `schema::camel_to_snake` for constants containing
/// underscores (e.g., `RSA_AES_CBC` → `rs_a__ae_s__cbc` not `rsa_aes_cbc`).
fn camel_to_snake(s: &str) -> String {
    // Step 1: gsub(/([^A-Z])([A-Z]+)/, '\1_\2')
    let re1 = regex::Regex::new(r"([^A-Z])([A-Z]+)").unwrap();
    let step1 = re1.replace_all(s, "${1}_${2}");

    // Step 2: gsub(/([A-Z])([A-Z][^A-Z\d]+)/, '\1_\2')
    let re2 = regex::Regex::new(r"([A-Z])([A-Z][^A-Z\d]+)").unwrap();
    let step2 = re2.replace_all(&step1, "${1}_${2}");

    // Step 3: downcase
    step2.to_lowercase()
}

/// Match the file path against the expected regex pattern.
/// RuboCop uses `expanded_file_path.match?("#{pattern}$")` which is a regex match
/// on the absolute file path.
fn path_matches_regex(path: &str, regex_pattern: &str) -> bool {
    // Use the regex crate for correct matching. The pattern is a valid regex
    // already (uses [^/]*, \., .*, and $). RuboCop's match? finds any match
    // in the string (unanchored at start), which the regex crate handles correctly.
    match regex::Regex::new(regex_pattern) {
        Ok(re) => re.is_match(path),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_scenario_fixture_tests!(
        SpecFilePathFormat,
        "cops/rspec/spec_file_path_format",
        scenario_wrong_class = "wrong_class.rb",
        scenario_wrong_method = "wrong_method.rb",
        scenario_wrong_path = "wrong_path.rb",
        scenario_module_wrong_path = "module_wrong_path.rb",
        scenario_nested_module_wrong_path = "nested_module_wrong_path.rb",
        scenario_wrong_class_backtick_method = "wrong_class_backtick_method.rb",
    );

    #[test]
    fn custom_transform_overrides_class_path() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let mut transform = serde_yml::Mapping::new();
        transform.insert(
            serde_yml::Value::String("MyClass".into()),
            serde_yml::Value::String("custom_dir".into()),
        );
        let config = CopConfig {
            options: HashMap::from([(
                "CustomTransform".into(),
                serde_yml::Value::Mapping(transform),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe MyClass do\nend\n";
        let diags =
            crate::testutil::run_cop_full_with_config(&SpecFilePathFormat, source, config.clone());
        assert!(!diags.is_empty(), "Should still flag with wrong filename");
        assert!(
            diags[0].message.contains("custom_dir"),
            "Expected path should use custom_dir from CustomTransform, got: {}",
            diags[0].message
        );
    }

    #[test]
    fn custom_transform_with_namespace() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let mut transform = serde_yml::Mapping::new();
        transform.insert(
            serde_yml::Value::String("FooFoo".into()),
            serde_yml::Value::String("foofoo".into()),
        );
        let config = CopConfig {
            options: HashMap::from([(
                "CustomTransform".into(),
                serde_yml::Value::Mapping(transform),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe FooFoo::Some::Class, '#bar' do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            config,
            "foofoo/some/class/bar_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "CustomTransform should apply to namespace parts, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignore_metadata_skips_check_when_value_matches() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let mut ignore_meta = serde_yml::Mapping::new();
        ignore_meta.insert(
            serde_yml::Value::String("type".into()),
            serde_yml::Value::String("routing".into()),
        );
        let config = CopConfig {
            options: HashMap::from([(
                "IgnoreMetadata".into(),
                serde_yml::Value::Mapping(ignore_meta),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe MyClass, type: :routing do\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&SpecFilePathFormat, source, config);
        assert!(
            diags.is_empty(),
            "IgnoreMetadata should skip path check when metadata value matches"
        );
    }

    #[test]
    fn ignore_metadata_does_not_skip_when_value_differs() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let mut ignore_meta = serde_yml::Mapping::new();
        ignore_meta.insert(
            serde_yml::Value::String("type".into()),
            serde_yml::Value::String("routing".into()),
        );
        let config = CopConfig {
            options: HashMap::from([(
                "IgnoreMetadata".into(),
                serde_yml::Value::Mapping(ignore_meta),
            )]),
            ..CopConfig::default()
        };
        let source = b"describe MyClass, type: :controller do\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&SpecFilePathFormat, source, config);
        assert!(
            !diags.is_empty(),
            "IgnoreMetadata should NOT skip when metadata value differs"
        );
    }

    #[test]
    fn camel_to_snake_handles_acronyms() {
        assert_eq!(camel_to_snake("URLValidator"), "url_validator");
        assert_eq!(camel_to_snake("MyClass"), "my_class");
        assert_eq!(camel_to_snake("HTTPSConnection"), "https_connection");
        assert_eq!(camel_to_snake("FooBar"), "foo_bar");
        assert_eq!(camel_to_snake("Foo"), "foo");
        assert_eq!(camel_to_snake("API"), "api");
        assert_eq!(camel_to_snake("HTMLParser"), "html_parser");
        // Constants with underscores: RuboCop's DefaultInflector inserts _
        // between uppercase sequences and existing underscores
        assert_eq!(camel_to_snake("RSA_AES_CBC"), "rs_a__ae_s__cbc");
    }

    #[test]
    fn underscore_constant_name_matches_rubocop_inflector() {
        // Arachni::Support::Crypto::RSA_AES_CBC should produce path ending
        // with support/crypto/rs_a__ae_s__cbc
        let source = b"describe Arachni::Support::Crypto::RSA_AES_CBC do\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "arachni/support/crypto/rs_a__ae_s__cbc_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when path matches RuboCop's DefaultInflector for underscored constants, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignore_methods_skips_method_check() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("IgnoreMethods".into(), serde_yml::Value::Bool(true))]),
            ..CopConfig::default()
        };
        let source = b"describe MyClass, '#create' do\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&SpecFilePathFormat, source, config);
        assert!(
            diags.iter().all(|d| !d.message.contains("create")),
            "IgnoreMethods should not check method part"
        );
    }

    #[test]
    fn module_wrapped_describe_no_offense() {
        let source = b"module Very\n  module Medium\n    describe MyClass do; end\n  end\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "very/medium/my_class_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when path matches namespace + class, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn module_wrapped_describe_offense() {
        let source = b"module Very\n  module Medium\n    describe MyClass do; end\n  end\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "very/long/my_class_spec.rb",
        );
        assert!(
            !diags.is_empty(),
            "Should flag when path doesn't match namespace"
        );
        assert!(
            diags[0].message.contains("very/medium/my_class"),
            "Message should include namespace path, got: {}",
            diags[0].message
        );
    }

    #[test]
    fn class_wrapped_describe_with_namespace() {
        let source = b"class MyApp\n  describe Widget do; end\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "my_app/widget_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when path matches class-namespace + describe class, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn no_describe_in_file_no_offense() {
        let source = b"class Foo\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
        );
        assert!(
            diags.is_empty(),
            "Should not flag files without describe calls"
        );
    }

    // Issue: path_matches uses contains() which matches class name anywhere in path.
    // RuboCop uses regex anchored to the end, so class path must appear at the end
    // of the file path before _spec.rb.
    #[test]
    fn path_contains_class_elsewhere_should_offense() {
        // Path has "foo" in parent dir but filename is "bar_spec.rb"
        // RuboCop flags this because the path doesn't end with foo*_spec.rb
        let source = b"describe Foo do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "/home/foo/spec/models/bar_spec.rb",
        );
        assert!(
            !diags.is_empty(),
            "Should flag when class name only appears in parent directory, not in filename suffix"
        );
    }

    // Issue: shared_examples should be counted as top-level groups for the .one? check
    #[test]
    fn describe_with_shared_examples_skips_check() {
        // RuboCop counts shared_examples in top_level_groups, so if there are
        // 1 describe + 1 shared_examples, that's 2 top-level groups → skip
        let source = b"describe MyClass do; end\nshared_examples 'foo' do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "wrong_path_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should skip when multiple top-level groups (describe + shared_examples)"
        );
    }

    // Issue: method description cleaning difference — RuboCop uses gsub(/\W/, '')
    // which removes non-word chars, while nitrocop replaces them with underscore
    #[test]
    fn method_description_with_backticks() {
        // "via `local_failures`" should become "via_local_failures" not "via__local_failures_"
        let source = b"describe MyClass, \"via `local_failures`\" do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "my_class_via_local_failures_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when path matches cleaned method description, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    // Issue: describe without a block should not be checked
    #[test]
    fn describe_without_block_no_offense() {
        let source = b"describe MyClass\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "wrong_path_spec.rb",
        );
        assert!(diags.is_empty(), "Should not flag describe without a block");
    }

    // Issue: call with non-nil receiver that isn't RSpec should be skipped
    #[test]
    fn describe_with_non_rspec_receiver_no_offense() {
        let source = b"SomeLib.describe MyClass do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "wrong_path_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag describe with non-RSpec receiver"
        );
    }

    // No-offense: RSpec.describe with correct path
    #[test]
    fn rspec_describe_with_correct_path_no_offense() {
        let source = b"RSpec.describe MyClass do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "my_class_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag RSpec.describe with correct path"
        );
    }

    // Ensure class path must appear at the END of the path (regex anchored)
    #[test]
    fn class_path_in_correct_suffix_no_offense() {
        let source = b"describe Some::Class do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "parent_dir/some/class_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when path ends with correct class path suffix"
        );
    }

    // RuboCop allows arbitrary directory prefix before the class path
    #[test]
    fn instance_method_in_subdirectory_no_offense() {
        let source = b"describe Some::Class, '#inst' do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "some/class/instance_methods/inst_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when method in subdirectory"
        );
    }

    // The method name pattern should allow arbitrary chars between class and method (via .*)
    #[test]
    fn class_method_flat_hierarchy_no_offense() {
        let source = b"describe Some::Class, '.inst' do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "some/class_inst_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when method name appears flat in filename"
        );
    }

    // Non-alphanumeric in method name (like ?) should be stripped
    #[test]
    fn predicate_method_no_offense() {
        let source = b"describe Some::Class, '#pred?' do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "some/class/pred_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when predicate method name matches"
        );
    }

    // Operator method name — all non-word chars removed, leaving empty method
    #[test]
    fn operator_method_no_offense() {
        let source = b"describe MyLittleClass, '#<=>' do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "my_little_class/spaceship_operator_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when operator method (all non-word) yields empty name_pattern"
        );
    }

    // Verify top-level ::ClassName is handled
    #[test]
    fn top_level_constant_no_offense() {
        let source = b"describe ::MyClass do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "my_class_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag ::MyClass with correct path"
        );
    }

    // Symbol argument should be ignored (not treated as method description)
    #[test]
    fn symbol_argument_no_offense() {
        let source = b"describe MyClass, :foo do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "my_class_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when symbol argument and class path matches"
        );
    }

    // Class name appears in parent directory — the simple_regex_match must try all positions
    // not just the first. E.g., path `/home/my_class_app/spec/my_class_spec.rb` has `my_class`
    // in both parent dir and filename. The first match in parent dir fails the [^/]* check
    // but the second match in the filename succeeds.
    #[test]
    fn class_name_in_parent_dir_no_offense() {
        let source = b"describe MyClass do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "/home/my_class_app/spec/models/my_class_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when class name appears in parent dir AND correct suffix, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    // Namespaced class path appears in parent directory
    #[test]
    fn namespaced_class_in_parent_dir_no_offense() {
        let source = b"describe Some::Class do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "/home/some/project/spec/models/some/class_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when namespaced class path appears in parent dir AND correct suffix, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    // Wrong path with incorrect collapsed namespace should be flagged
    #[test]
    fn incorrect_collapsed_namespace_offense() {
        let source = b"describe Very::Long::Namespace::MyClass do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "/home/foo/spec/very/my_class_spec.rb",
        );
        assert!(
            !diags.is_empty(),
            "Should flag when namespace is incorrectly collapsed"
        );
        assert!(
            diags[0].message.contains("very/long/namespace/my_class"),
            "Should show full namespace path, got: {}",
            diags[0].message
        );
    }

    // FP fix: RuboCop's top_level_nodes only recurses through module/class wrappers
    // from the ROOT. When the root is :begin (multiple top-level statements like
    // require + module), module children are NOT recursed into. The spec_group? filter
    // sees a module node, not a block, and finds 0 spec groups → skips.
    // nitrocop was recursing into ALL modules/classes regardless, finding describes
    // inside module wrappers that RuboCop skips.
    #[test]
    fn module_with_require_sibling_no_offense() {
        // RuboCop skips this: the root is :begin (require + module), so module is
        // not unwrapped. 0 top-level spec groups.
        let source = b"require 'rails_helper'\n\nmodule Alchemy\n  describe Modules do; end\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "spec/libraries/modules_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should skip when module wrapper has sibling statements (require), got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    // Same FP pattern but with class wrapper
    #[test]
    fn class_with_require_sibling_no_offense() {
        let source = b"require 'spec_helper'\n\nclass MyApp\n  describe Widget do; end\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "spec/models/widget_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should skip when class wrapper has sibling statements (require), got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    // When module IS the only top-level statement, it should be unwrapped
    #[test]
    fn module_as_sole_statement_still_checks() {
        let source = b"module Alchemy\n  describe Modules do; end\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "spec/libraries/modules_spec.rb",
        );
        // This SHOULD flag: module is sole statement, so it's unwrapped.
        // Expected path is alchemy/modules, file is libraries/modules_spec.rb.
        assert!(
            !diags.is_empty(),
            "Should flag when module is sole statement and path doesn't match"
        );
    }

    // Module as sole statement with correct path — no offense
    #[test]
    fn module_as_sole_statement_correct_path_no_offense() {
        let source = b"module Alchemy\n  describe Modules do; end\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "alchemy/modules_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag when module is sole statement and path matches"
        );
    }

    // Multiple statements with nested describe AND bare describe — 2 groups, skip
    #[test]
    fn require_plus_module_plus_bare_describe_skips() {
        let source = b"require 'spec_helper'\ndescribe Other do; end\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "wrong_path_spec.rb",
        );
        // The describe is a direct child of the root (alongside require).
        // Only 1 spec group → should check.
        assert!(
            !diags.is_empty(),
            "Should flag bare describe with require sibling"
        );
    }

    // FP: multi-line constant path should be parsed from AST, not source text
    #[test]
    fn multiline_constant_path_no_offense() {
        let source = b"describe Foo::\n  Bar do\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "foo/bar_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag multi-line constant path with correct path, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    // FP: constant path with spaces around :: should still match
    #[test]
    fn constant_path_with_spaces_no_offense() {
        let source = b"describe Foo ::Bar do\nend\n";
        let diags = crate::testutil::run_cop_full_internal(
            &SpecFilePathFormat,
            source,
            CopConfig::default(),
            "foo/bar_spec.rb",
        );
        assert!(
            diags.is_empty(),
            "Should not flag constant path with spaces, got: {:?}",
            diags.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }
}
