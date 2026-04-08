use crate::cop::shared::node_type::{
    ASSOC_NODE, HASH_NODE, IMPLICIT_NODE, KEYWORD_HASH_NODE, LOCAL_VARIABLE_READ_NODE, SYMBOL_NODE,
};
use crate::cop::shared::util::{is_modifier_if, is_modifier_unless};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/HashSyntax: checks hash literal syntax (rocket vs ruby19).
///
/// Fixed: quoted symbol keys like `:"chef version"` and interpolated symbol keys
/// like `:"#{field}_string"` were incorrectly treated as unconvertible. Prism
/// parses the latter as `InterpolatedSymbolNode`, but RuboCop's `any_sym_type?`
/// treats both forms as symbol keys. The cop now accepts both plain and
/// interpolated quoted symbols when deciding whether `=>` can become Ruby 1.9
/// label syntax on Ruby >= 2.2.
///
/// Fixed: quoted symbol keys already in 1.9 syntax (e.g. `"font-variant":`)
/// have Prism opening `"` or `'` (without `:` prefix), unlike rocket-syntax
/// `:"key" =>` which has opening `:"`. The `is_acceptable_19_symbol` check
/// now recognizes both forms, so hashes mixing 1.9-style and rocket-style
/// quoted symbol keys correctly flag only the rocket entries.
///
/// Fixed variant divergence for shorthand syntax and mixed-key styles:
/// RuboCop evaluates `EnforcedShorthandSyntax` alongside `EnforcedStyle`
/// rather than as a separate early-exit pass. The previous implementation
/// short-circuited after shorthand checks, collapsed `consistent` into a
/// single hash-level offense, and returned early from `ruby19_no_mixed_keys`
/// when non-symbol keys were present. The cop now classifies each pair the
/// same way RuboCop does, emits pair-level shorthand diagnostics at the
/// correct key/value location, flags colon pairs in `ruby19_no_mixed_keys`
/// when mixed with non-symbol keys, and reports the mixed pair itself for
/// `no_mixed_keys` instead of the whole hash.
///
/// Fixed remaining shorthand divergence: RuboCop treats unbraced keyword hashes
/// inside modifier-form ancestors (`if`, `unless`, `while`, `until`) as
/// requiring explicit values unless the enclosing call is parenthesized. That
/// includes outer wrappers like `end if cond`, so shorthand analysis now runs
/// in a visitor that can see modifier ancestors before classifying each pair.
pub struct HashSyntax;

const MSG_19: &str = "Use the new Ruby 1.9 hash syntax.";
const MSG_NO_MIXED_KEYS: &str = "Don't mix styles in the same hash.";
const MSG_HASH_ROCKETS: &str = "Use hash rockets syntax.";
const OMIT_HASH_VALUE_MSG: &str = "Omit the hash value.";
const INCLUDE_HASH_VALUE_MSG: &str = "Include the hash value.";
const DO_NOT_MIX_OMIT_VALUE_MSG: &str =
    "Do not mix explicit and implicit hash values. Omit the hash value.";
const DO_NOT_MIX_INCLUDE_VALUE_MSG: &str =
    "Do not mix explicit and implicit hash values. Include the hash value.";

impl Cop for HashSyntax {
    fn name(&self) -> &'static str {
        "Style/HashSyntax"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            HASH_NODE,
            IMPLICIT_NODE,
            KEYWORD_HASH_NODE,
            LOCAL_VARIABLE_READ_NODE,
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
        // Handle both explicit hashes `{ k: v }` and implicit keyword hashes `foo(k: v)`
        let pairs: Vec<ruby_prism::AssocNode<'_>> = if let Some(hash_node) = node.as_hash_node() {
            hash_node
                .elements()
                .iter()
                .filter_map(|element| element.as_assoc_node())
                .collect()
        } else if let Some(kw_hash) = node.as_keyword_hash_node() {
            kw_hash
                .elements()
                .iter()
                .filter_map(|element| element.as_assoc_node())
                .collect()
        } else {
            return;
        };

        if pairs.is_empty() {
            return;
        }

        let enforced_style = config.get_str("EnforcedStyle", "ruby19");
        let use_rockets_symbol_vals = config.get_bool("UseHashRocketsWithSymbolValues", false);
        let prefer_rockets_nonalnum =
            config.get_bool("PreferHashRocketsForNonAlnumEndingSymbols", false);
        let target_ruby_version = target_ruby_version(config);

        match enforced_style {
            "ruby19" => {
                if use_rockets_symbol_vals
                    && pairs.iter().any(|assoc| is_symbol_node(&assoc.value()))
                {
                    return;
                }

                if sym_indices(&pairs, prefer_rockets_nonalnum, target_ruby_version) {
                    check_pairs_with_delimiter(self, source, &pairs, b"=>", MSG_19, diagnostics);
                }
            }
            "ruby19_no_mixed_keys" => {
                if use_rockets_symbol_vals
                    && pairs.iter().any(|assoc| is_symbol_node(&assoc.value()))
                {
                    check_pairs_with_delimiter(
                        self,
                        source,
                        &pairs,
                        b":",
                        MSG_HASH_ROCKETS,
                        diagnostics,
                    );
                    return;
                }

                if sym_indices(&pairs, prefer_rockets_nonalnum, target_ruby_version) {
                    check_pairs_with_delimiter(self, source, &pairs, b"=>", MSG_19, diagnostics);
                } else {
                    check_pairs_with_delimiter(
                        self,
                        source,
                        &pairs,
                        b":",
                        MSG_NO_MIXED_KEYS,
                        diagnostics,
                    );
                }
            }
            "hash_rockets" => {
                check_pairs_with_delimiter(
                    self,
                    source,
                    &pairs,
                    b":",
                    MSG_HASH_ROCKETS,
                    diagnostics,
                );
            }
            "no_mixed_keys" => {
                let delimiter = if sym_indices(&pairs, prefer_rockets_nonalnum, target_ruby_version)
                {
                    if uses_hash_rocket(&pairs[0]) {
                        b":".as_slice()
                    } else {
                        b"=>".as_slice()
                    }
                } else {
                    b":".as_slice()
                };

                check_pairs_with_delimiter(
                    self,
                    source,
                    &pairs,
                    delimiter,
                    MSG_NO_MIXED_KEYS,
                    diagnostics,
                );
            }
            _ => {}
        }
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
        let enforced_shorthand = config.get_str("EnforcedShorthandSyntax", "either");
        let target_ruby_version = target_ruby_version(config);

        if enforced_shorthand == "either" || target_ruby_version <= 3.0 {
            return;
        }

        let mut visitor = HashSyntaxShorthandVisitor {
            cop: self,
            source,
            enforced_shorthand,
            target_ruby_version,
            diagnostics,
            ancestors: Vec::new(),
        };
        ruby_prism::Visit::visit(&mut visitor, &parse_result.node());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShorthandKind {
    Omitted,
    Omittable,
    Needed,
}

/// Check EnforcedShorthandSyntax for Ruby 3.1 hash value omission.
#[allow(clippy::too_many_arguments)]
fn check_shorthand_syntax(
    cop: &HashSyntax,
    source: &SourceFile,
    pairs: &[ruby_prism::AssocNode<'_>],
    enforced_shorthand: &str,
    target_ruby_version: f64,
    hash_is_braced: bool,
    ancestors: &[ruby_prism::Node<'_>],
    diags: &mut Vec<Diagnostic>,
) {
    if target_ruby_version <= 3.0 {
        return;
    }

    let kinds: Vec<ShorthandKind> = pairs
        .iter()
        .map(|assoc| shorthand_kind(assoc, hash_is_braced, ancestors))
        .collect();

    match enforced_shorthand {
        "always" => {
            for (assoc, kind) in pairs.iter().zip(kinds.iter()) {
                if *kind == ShorthandKind::Omittable {
                    push_diagnostic_at_node(
                        cop,
                        source,
                        &assoc.value(),
                        OMIT_HASH_VALUE_MSG,
                        diags,
                    );
                }
            }
        }
        "never" => {
            for (assoc, kind) in pairs.iter().zip(kinds.iter()) {
                if *kind == ShorthandKind::Omitted {
                    push_diagnostic_at_node(
                        cop,
                        source,
                        &assoc.key(),
                        INCLUDE_HASH_VALUE_MSG,
                        diags,
                    );
                }
            }
        }
        "consistent" | "either_consistent" => {
            let has_omitted = kinds.contains(&ShorthandKind::Omitted);
            let has_omittable = kinds.contains(&ShorthandKind::Omittable);
            let has_needed = kinds.contains(&ShorthandKind::Needed);
            let kind_count =
                usize::from(has_omitted) + usize::from(has_omittable) + usize::from(has_needed);

            if kind_count > 1 {
                if has_needed {
                    for (assoc, kind) in pairs.iter().zip(kinds.iter()) {
                        if *kind == ShorthandKind::Omitted {
                            push_diagnostic_at_node(
                                cop,
                                source,
                                &assoc.key(),
                                DO_NOT_MIX_INCLUDE_VALUE_MSG,
                                diags,
                            );
                        }
                    }
                } else {
                    for (assoc, kind) in pairs.iter().zip(kinds.iter()) {
                        if *kind == ShorthandKind::Omittable {
                            push_diagnostic_at_node(
                                cop,
                                source,
                                &assoc.value(),
                                DO_NOT_MIX_OMIT_VALUE_MSG,
                                diags,
                            );
                        }
                    }
                }
            } else if enforced_shorthand == "consistent" && has_omittable && !has_needed {
                for (assoc, kind) in pairs.iter().zip(kinds.iter()) {
                    if *kind == ShorthandKind::Omittable {
                        push_diagnostic_at_node(
                            cop,
                            source,
                            &assoc.value(),
                            OMIT_HASH_VALUE_MSG,
                            diags,
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

fn shorthand_kind(
    assoc: &ruby_prism::AssocNode<'_>,
    hash_is_braced: bool,
    ancestors: &[ruby_prism::Node<'_>],
) -> ShorthandKind {
    if assoc.value().as_implicit_node().is_some() {
        return ShorthandKind::Omitted;
    }

    let key = assoc.key();
    let key_source = key.location().as_slice();
    let comparable_key = if uses_hash_rocket(assoc) {
        key_source
    } else {
        key_source.strip_suffix(b":").unwrap_or(key_source)
    };

    if !is_symbol_like_key(&key)
        || comparable_key.ends_with(b"!")
        || comparable_key.ends_with(b"?")
        || require_hash_value_for_modifier_context(hash_is_braced, ancestors)
    {
        return ShorthandKind::Needed;
    }

    let value = assoc.value();
    let explicit_value = value.as_local_variable_read_node().is_some()
        || value.as_call_node().is_some_and(|call| {
            call.receiver().is_none() && call.arguments().is_none() && call.block().is_none()
        });

    if explicit_value && value.location().as_slice() == comparable_key {
        ShorthandKind::Omittable
    } else {
        ShorthandKind::Needed
    }
}

struct HashSyntaxShorthandVisitor<'a, 'pr> {
    cop: &'a HashSyntax,
    source: &'a SourceFile,
    enforced_shorthand: &'a str,
    target_ruby_version: f64,
    diagnostics: &'a mut Vec<Diagnostic>,
    ancestors: Vec<ruby_prism::Node<'pr>>,
}

impl<'a, 'pr> HashSyntaxShorthandVisitor<'a, 'pr> {
    fn check_pairs(&mut self, pairs: Vec<ruby_prism::AssocNode<'pr>>, hash_is_braced: bool) {
        if pairs.is_empty() {
            return;
        }

        check_shorthand_syntax(
            self.cop,
            self.source,
            &pairs,
            self.enforced_shorthand,
            self.target_ruby_version,
            hash_is_braced,
            &self.ancestors,
            self.diagnostics,
        );
    }
}

impl<'a, 'pr> ruby_prism::Visit<'pr> for HashSyntaxShorthandVisitor<'a, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'pr>) {
        let pairs = node
            .elements()
            .iter()
            .filter_map(|element| element.as_assoc_node())
            .collect();
        let hash_is_braced = node.opening_loc().as_slice() == b"{";

        self.check_pairs(pairs, hash_is_braced);
        ruby_prism::visit_hash_node(self, node);
    }

    fn visit_keyword_hash_node(&mut self, node: &ruby_prism::KeywordHashNode<'pr>) {
        let pairs = node
            .elements()
            .iter()
            .filter_map(|element| element.as_assoc_node())
            .collect();

        self.check_pairs(pairs, false);
        ruby_prism::visit_keyword_hash_node(self, node);
    }
}

fn require_hash_value_for_modifier_context(
    hash_is_braced: bool,
    ancestors: &[ruby_prism::Node<'_>],
) -> bool {
    !hash_is_braced && modifier_form_without_parenthesized_method_call(ancestors)
}

fn modifier_form_without_parenthesized_method_call(ancestors: &[ruby_prism::Node<'_>]) -> bool {
    let Some((dispatch_idx, dispatch)) = enclosing_method_dispatch(ancestors) else {
        return false;
    };

    if method_dispatch_is_parenthesized(dispatch) {
        return false;
    }

    ancestors[..dispatch_idx]
        .iter()
        .any(is_modifier_form_ancestor)
}

fn is_modifier_form_ancestor(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(if_node) = node.as_if_node() {
        return is_modifier_if(&if_node);
    }

    if let Some(unless_node) = node.as_unless_node() {
        return is_modifier_unless(&unless_node);
    }

    if let Some(while_node) = node.as_while_node() {
        return while_node.closing_loc().is_none();
    }

    node.as_until_node()
        .is_some_and(|until_node| until_node.closing_loc().is_none())
}

fn method_dispatch_is_parenthesized(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        return call.opening_loc().is_some_and(|loc| loc.as_slice() == b"(");
    }

    if let Some(super_node) = node.as_super_node() {
        return super_node.lparen_loc().is_some();
    }

    node.as_yield_node()
        .is_some_and(|yield_node| yield_node.lparen_loc().is_some())
}

fn enclosing_method_dispatch<'pr>(
    ancestors: &'pr [ruby_prism::Node<'pr>],
) -> Option<(usize, &'pr ruby_prism::Node<'pr>)> {
    let len = ancestors.len().saturating_sub(1);

    for idx in (0..len).rev() {
        let node = &ancestors[idx];

        if let Some(call) = node.as_call_node() {
            let name = call.name().as_slice();
            if name == b"[]" || name == b"[]=" {
                return None;
            }
            return Some((idx, node));
        }

        if node.as_super_node().is_some() {
            return Some((idx, node));
        }

        if node.as_yield_node().is_some() {
            return Some((idx, node));
        }
    }

    None
}

fn sym_indices(
    pairs: &[ruby_prism::AssocNode<'_>],
    prefer_rockets_nonalnum: bool,
    target_ruby_version: f64,
) -> bool {
    pairs.iter().all(|assoc| {
        let key = assoc.key();
        is_symbol_like_key(&key)
            && is_acceptable_19_key(&key, prefer_rockets_nonalnum, target_ruby_version)
    })
}

fn check_pairs_with_delimiter(
    cop: &HashSyntax,
    source: &SourceFile,
    pairs: &[ruby_prism::AssocNode<'_>],
    delimiter: &[u8],
    message: &str,
    diags: &mut Vec<Diagnostic>,
) {
    for assoc in pairs {
        if pair_uses_delimiter(assoc, delimiter) {
            push_diagnostic_at_node(cop, source, &assoc.key(), message, diags);
        }
    }
}

fn push_diagnostic_at_node(
    cop: &HashSyntax,
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
    message: &str,
    diags: &mut Vec<Diagnostic>,
) {
    let (line, column) = source.offset_to_line_col(node.location().start_offset());
    diags.push(cop.diagnostic(source, line, column, message.to_string()));
}

fn pair_uses_delimiter(assoc: &ruby_prism::AssocNode<'_>, delimiter: &[u8]) -> bool {
    if delimiter == b"=>" {
        uses_hash_rocket(assoc)
    } else {
        !uses_hash_rocket(assoc)
    }
}

fn uses_hash_rocket(assoc: &ruby_prism::AssocNode<'_>) -> bool {
    assoc
        .operator_loc()
        .is_some_and(|operator| operator.as_slice() == b"=>")
}

fn is_symbol_node(node: &ruby_prism::Node<'_>) -> bool {
    node.as_symbol_node().is_some() || node.as_interpolated_symbol_node().is_some()
}

fn is_symbol_like_key(key: &ruby_prism::Node<'_>) -> bool {
    key.as_symbol_node().is_some() || key.as_interpolated_symbol_node().is_some()
}

fn is_acceptable_19_key(
    key: &ruby_prism::Node<'_>,
    prefer_rockets_nonalnum: bool,
    target_ruby_version: f64,
) -> bool {
    if let Some(sym) = key.as_symbol_node() {
        return is_acceptable_19_symbol(&sym, prefer_rockets_nonalnum, target_ruby_version);
    }

    // Interpolated symbol keys are always quoted (e.g. `:"#{field}_string"`),
    // so they follow RuboCop's quoted-symbol path and are convertible on Ruby >= 2.2.
    key.as_interpolated_symbol_node().is_some() && target_ruby_version > 2.1
}

/// Check if a symbol node represents an acceptable Ruby 1.9 syntax key.
/// This includes simple identifiers (`:foo` → `foo:`) and quoted symbols
/// (`:"chef version"` → `"chef version":`, available since Ruby 2.2).
fn is_acceptable_19_symbol(
    sym: &ruby_prism::SymbolNode,
    prefer_rockets_nonalnum: bool,
    target_ruby_version: f64,
) -> bool {
    let name = sym.unescaped();
    // Quoted symbol keys can have different openings depending on syntax:
    //   - Rocket syntax: `:"key" =>` or `:'key' =>` → opening is `:"` or `:'`
    //   - Ruby 1.9 syntax: `"key":` or `'key':` → opening is `"` or `'`
    // Both forms are convertible to 1.9 label syntax on Ruby >= 2.2.
    let is_quoted_symbol = sym
        .opening_loc()
        .is_some_and(|opening| matches!(opening.as_slice(), b":\"" | b":'" | b"\"" | b"'"));

    if is_quoted_symbol {
        return target_ruby_version > 2.1;
    }

    // Simple identifier: `:foo`, `:foo_bar`, `:foo?`, `:foo!`
    if is_simple_symbol_identifier(name) {
        if prefer_rockets_nonalnum && !name.is_empty() {
            let last = name[name.len() - 1];
            if !last.is_ascii_alphanumeric() {
                return false;
            }
        }
        return true;
    }

    false
}

fn target_ruby_version(config: &CopConfig) -> f64 {
    config
        .options
        .get("TargetRubyVersion")
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_u64().map(|value| value as f64))
        })
        .unwrap_or(2.7)
}

/// Check if a symbol's unescaped name is a simple Ruby identifier.
/// Valid: `foo`, `foo_bar`, `foo?`, `foo!`
/// Invalid: `foo bar`, `123`, `foo=`, empty
fn is_simple_symbol_identifier(name: &[u8]) -> bool {
    if name.is_empty() {
        return false;
    }
    // Must start with a letter or underscore
    let first = name[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return false;
    }
    // Rest must be word characters, optionally ending with ? or !
    // Note: `=` ending symbols (setter methods like `:foo=`) cannot use
    // Ruby 1.9 hash syntax, so they are NOT convertible.
    let (body, _suffix) = if name.len() > 1 {
        let last = name[name.len() - 1];
        if last == b'?' || last == b'!' {
            (&name[1..name.len() - 1], Some(last))
        } else {
            (&name[1..], None)
        }
    } else {
        (&[] as &[u8], None)
    };
    body.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::testutil::{
        assert_cop_no_offenses_full_with_config, assert_cop_offenses_full_with_config,
        run_cop_full_with_config,
    };

    crate::cop_fixture_tests!(HashSyntax, "cops/style/hash_syntax");

    #[test]
    fn config_hash_rockets() {
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("hash_rockets".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"{ a: 1 }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("hash rockets"));
    }

    #[test]
    fn mixed_key_types_skipped_in_ruby19() {
        use crate::testutil::run_cop_full;
        // Hash with string key and symbol key — should not be flagged
        let source = b"{ \"@type\" => \"Person\", :name => \"foo\" }\n";
        let diags = run_cop_full(&HashSyntax, source);
        assert!(diags.is_empty(), "Mixed key hash should not be flagged");
    }

    #[test]
    fn use_hash_rockets_with_symbol_values() {
        let config = CopConfig {
            options: HashMap::from([(
                "UseHashRocketsWithSymbolValues".into(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };
        // Hash with symbol value should not be flagged when UseHashRocketsWithSymbolValues is true
        let source = b"{ :foo => :bar }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Should allow rockets when value is a symbol"
        );
    }

    #[test]
    fn shorthand_never_flags_omission() {
        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedShorthandSyntax".into(),
                    serde_yml::Value::String("never".into()),
                ),
                (
                    "TargetRubyVersion".into(),
                    serde_yml::Value::Number(serde_yml::value::Number::from(3.1_f64)),
                ),
            ]),
            ..CopConfig::default()
        };
        // Ruby 3.1 hash value omission: `{x:}` (shorthand)
        let source = b"x = 1; {x:}\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("Include the hash value")),
            "Should flag shorthand with EnforcedShorthandSyntax: never"
        );
    }

    #[test]
    fn quoted_symbol_keys_require_ruby_22() {
        let config = CopConfig {
            options: HashMap::from([(
                "TargetRubyVersion".into(),
                serde_yml::Value::Number(serde_yml::value::Number::from(2.1)),
            )]),
            ..CopConfig::default()
        };
        let source = b"{ :\"string\" => 0 }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Quoted symbol keys should stay on hash rockets before Ruby 2.2"
        );
    }

    #[test]
    fn interpolated_symbol_keys_require_ruby_22() {
        let config = CopConfig {
            options: HashMap::from([(
                "TargetRubyVersion".into(),
                serde_yml::Value::Number(serde_yml::value::Number::from(2.1)),
            )]),
            ..CopConfig::default()
        };
        let source = br##"{ :"#{field}_string" => nil }"##;
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Interpolated symbol keys should stay on hash rockets before Ruby 2.2"
        );
    }

    #[test]
    fn interpolated_symbol_keys_register_offense() {
        let source =
            br##"task :"setup:#{provider}" => File.join(ARTIFACT_DIR, "#{provider}.box")"##;
        let diags = crate::testutil::run_cop_full(&HashSyntax, source);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message
                .contains("Use the new Ruby 1.9 hash syntax")
        );
    }

    #[test]
    fn shorthand_either_allows_all() {
        // Default "either" should not flag anything shorthand-related
        let source = b"x = 1; {x:}\n";
        use crate::testutil::run_cop_full;
        let diags = run_cop_full(&HashSyntax, source);
        assert!(
            !diags.iter().any(|d| d.message.contains("hash value")),
            "Default 'either' should not flag shorthand"
        );
    }

    #[test]
    fn prefer_rockets_for_nonalnum_ending_symbols() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "PreferHashRocketsForNonAlnumEndingSymbols".into(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };
        // Hash with symbol key ending in `?` should not be flagged (non-alnum ending)
        let source = b"{ :production? => false }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Should allow rockets for non-alnum ending symbols"
        );
    }

    fn shorthand_style_config(style: &str, shorthand: &str) -> CopConfig {
        CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String(style.into()),
                ),
                (
                    "EnforcedShorthandSyntax".into(),
                    serde_yml::Value::String(shorthand.into()),
                ),
                (
                    "TargetRubyVersion".into(),
                    serde_yml::Value::Number(serde_yml::value::Number::from(3.1_f64)),
                ),
            ]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn consistent_ruby19_no_mixed_keys_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/style/hash_syntax/offense.consistent_ruby19_no_mixed_keys.rb"
        );
        assert_cop_offenses_full_with_config(
            &HashSyntax,
            fixture,
            shorthand_style_config("ruby19_no_mixed_keys", "consistent"),
        );
    }

    #[test]
    fn always_hash_rockets_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/style/hash_syntax/offense.always_hash_rockets.rb"
        );
        assert_cop_offenses_full_with_config(
            &HashSyntax,
            fixture,
            shorthand_style_config("hash_rockets", "always"),
        );
    }

    #[test]
    fn always_hash_rockets_no_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/style/hash_syntax/no_offense.always_hash_rockets.rb"
        );
        assert_cop_no_offenses_full_with_config(
            &HashSyntax,
            fixture,
            shorthand_style_config("hash_rockets", "always"),
        );
    }

    #[test]
    fn never_no_mixed_keys_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/style/hash_syntax/offense.never_no_mixed_keys.rb"
        );
        assert_cop_offenses_full_with_config(
            &HashSyntax,
            fixture,
            shorthand_style_config("no_mixed_keys", "never"),
        );
    }

    #[test]
    fn consistent_shorthand_flags_unparenthesized_keyword_hash_without_modifier() {
        let source = b"client_server client: client, server: server\n";
        let diags = run_cop_full_with_config(
            &HashSyntax,
            source,
            shorthand_style_config("ruby19_no_mixed_keys", "consistent"),
        );
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().all(|diag| diag.message == OMIT_HASH_VALUE_MSG));
    }

    #[test]
    fn consistent_shorthand_skips_keyword_hashes_in_modifier_contexts() {
        let source = br#"
return redirect_to destination_url, flash: flash if signed_in?

class TestSSLDhParam < Test::Unit::TestCase
  def test_dhparam_1_3_supplied
    client = { client_unbind: true, ssl_version: %w(TLSv1_3) }
    server = { dhparam: DH_PARAM_FILE, cipher_list: "DHE,EDH", ssl_version: %w(TLSv1_3) }
    client_server client: client, server: server
  end
end if EM.ssl?
"#;

        assert_cop_no_offenses_full_with_config(
            &HashSyntax,
            source,
            shorthand_style_config("ruby19_no_mixed_keys", "consistent"),
        );
    }
}
