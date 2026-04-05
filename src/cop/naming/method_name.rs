use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::node_type::{ALIAS_METHOD_NODE, CALL_NODE, DEF_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Naming/MethodName cop — checks that method names use the configured naming style.
///
/// ## Investigation (2026-03-08)
/// FP=0, FN=590 in corpus. Root causes:
/// 1. AllowedPatterns used substring matching instead of regex — fixed to use regex::Regex.
/// 2. Only handled DEF_NODE — missing attr_reader/attr_writer/attr_accessor/attr (CALL_NODE),
///    define_method/define_singleton_method (CALL_NODE), Struct.new/Data.define member names
///    (CALL_NODE), alias keyword (ALIAS_METHOD_NODE), and alias_method (CALL_NODE).
/// 3. All of these are now handled, matching RuboCop's on_def/on_defs/on_send/on_alias handlers.
///
/// Follow-up (2026-03-08): FP=8 regressed at sites using
/// `# rubocop:disable Style/MethodName`. RuboCop still suppresses
/// `Naming/MethodName` for that moved legacy name because the short name stayed
/// `MethodName`. Fixed centrally in `parse/directives.rs`.
///
/// ## Investigation (2026-03-09)
/// Corpus oracle reported FP=2, FN=162.
///
/// FN=162: Root cause was an incorrect CamelCase singleton method skip.
/// Lines 155-161 skipped ALL uppercase-starting singleton methods (`def self.IF`,
/// `def self.UNLESS`, `def self.Dimension`) as "factory methods". But RuboCop
/// has NO such exception — it flags all non-snake_case methods uniformly.
/// The skip was an incorrect approximation. Removed it entirely.
///
/// FP=2: Separate issues (discourse report.rb, jekyll-seo-tag json_ld_drop.rb).
/// Not investigated yet — likely alias_method or config resolution differences.
///
/// ## Corpus investigation (2026-03-09)
///
/// Corpus oracle reported FP=15, FN=21.
///
/// FP=15: class-emitter methods like `def self.ImageQuality` and
/// `def base.Start` should be allowed when the current lexical scope defines a
/// matching class/module. nitrocop was treating them as ordinary CamelCase
/// method names and flagging them.
///
/// FN=21: non-letter Unicode names like `def ❤` and `alias_method :☠, :exit`
/// were incorrectly treated as operator methods because the old helper only
/// checked for ASCII letters. RuboCop uses an explicit operator-name allowlist.
///
/// ## Investigation (2026-03-10)
/// Corpus oracle reported FP=2, FN=2.
///
/// FP #1: `alias_method :@type, :type` in jekyll-seo-tag. RuboCop's snake_case
/// regex (`/^@{0,2}[\d[[:lower:]]_]+[!?=]?$/`) allows up to 2 leading `@` chars,
/// so `@type` is valid snake_case. Fixed `is_method_snake_case` and
/// `is_lower_camel_case` to strip leading `@` chars before checking.
///
/// FP #2 + FN #1: `attr_accessor :prev30Days` in discourse. RuboCop reports the
/// offense at the call site (range from selector end to expression end), not at
/// the individual symbol. For multiline attr_accessor, our symbol-level location
/// was on a different line than RuboCop's call-site location, creating both an
/// FP (wrong line) and FN (missing at correct line). Fixed `check_attr_accessor`
/// to use `message_loc.end + 1` as the offense location, matching RuboCop's
/// `range_position`.
///
/// FN #2: `def self.Types(...)` in dry-types. The `has_class_emitter_in_scope`
/// check was matching `module Types` as an emitter, but RuboCop only checks
/// `:class` children (not `:module`). Fixed `collect_emitter_name` to only
/// collect class nodes, matching `node.parent.each_child_node(:class)`.
///
/// ## Corpus investigation (2026-03-23) — extended corpus
///
/// Extended corpus reported FP=6 (all vendor-path repos), FN=1.
/// FP=6: all from 3 repos (Tubalr, stackneveroverflow, supply_drop) with massive
/// systemic divergence (2k-9k FP each across all cops). Root cause: repos without
/// `.rubocop.yml` had no default AllCops.Exclude patterns applied. Fixed in
/// `config/mod.rs` by applying `fallback_default_excludes()` in the no-config case.
///
/// FN=1: `def self.String(s)` in skylight/vendor/cli/highline/string_extensions.rb.
/// Root cause: `collect_emitter_name` used `last_constant_segment` to extract
/// just "String" from `class HighLine::String`, which incorrectly matched
/// `def self.String` as a class emitter method. RuboCop's
/// `c.loc.name.is?(name.to_s)` compares the full constant path text
/// ("HighLine::String" != "String"), so it correctly flags the offense.
/// Fixed by using the full constant path text in `collect_emitter_name`.
///
/// ## Variant divergence fix (2026-04-05)
/// camelCase variant: FP=10, FN=0.
/// All 10 FPs were methods named `_` (bare underscore), common in gettext
/// stubs. RuboCop's camelCase regex `/^@{0,2}(?:_|...)[!?=]?$/` explicitly
/// allows a bare `_` via the first alternative. Our `is_lower_camel_case`
/// returned false for `_` because after consuming the leading underscore it
/// required a second character. Fixed by returning true when the name is
/// exactly `_` (with optional method suffix stripped).
pub struct MethodName;

/// Bundles config values needed for method name checking.
struct MethodNameConfig {
    enforced_style: String,
    allowed_patterns: Option<Vec<String>>,
    forbidden_identifiers: Option<Vec<String>>,
    forbidden_patterns: Option<Vec<String>>,
}

impl MethodNameConfig {
    fn from_cop_config(config: &CopConfig) -> Self {
        Self {
            enforced_style: config.get_str("EnforcedStyle", "snake_case").to_string(),
            allowed_patterns: config.get_string_array("AllowedPatterns"),
            forbidden_identifiers: config.get_string_array("ForbiddenIdentifiers"),
            forbidden_patterns: config.get_string_array("ForbiddenPatterns"),
        }
    }
}

/// Check if a method name matches AllowedPatterns using regex matching.
fn matches_allowed_pattern(name: &str, allowed_patterns: &Option<Vec<String>>) -> bool {
    if let Some(patterns) = allowed_patterns {
        for p in patterns {
            if let Ok(re) = regex::Regex::new(p) {
                if re.is_match(name) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a method name is forbidden by ForbiddenIdentifiers or ForbiddenPatterns.
fn is_forbidden_name(name: &str, cfg: &MethodNameConfig) -> bool {
    if let Some(forbidden) = &cfg.forbidden_identifiers {
        if forbidden.iter().any(|f| f == name) {
            return true;
        }
    }
    if let Some(patterns) = &cfg.forbidden_patterns {
        for pattern in patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if re.is_match(name) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check naming style compliance.
fn style_ok(name: &[u8], enforced_style: &str) -> bool {
    let Ok(name) = std::str::from_utf8(name) else {
        return false;
    };

    match enforced_style {
        "camelCase" => is_lower_camel_case(name),
        _ => is_method_snake_case(name),
    }
}

fn style_msg(enforced_style: &str) -> &str {
    match enforced_style {
        "camelCase" => "camelCase",
        _ => "snake_case",
    }
}

/// Extract a method name string from a symbol or string node.
fn extract_name_from_sym_or_str<'a>(
    node: &'a ruby_prism::Node<'a>,
) -> Option<(Vec<u8>, ruby_prism::Location<'a>)> {
    if let Some(sym) = node.as_symbol_node() {
        Some((sym.unescaped().to_vec(), sym.location()))
    } else {
        node.as_string_node()
            .map(|s| (s.unescaped().to_vec(), s.location()))
    }
}

impl Cop for MethodName {
    fn name(&self) -> &'static str {
        "Naming/MethodName"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[DEF_NODE, CALL_NODE, ALIAS_METHOD_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let cfg = MethodNameConfig::from_cop_config(config);

        if let Some(def_node) = node.as_def_node() {
            check_def_node(self, source, &def_node, parse_result, &cfg, diagnostics);
        } else if let Some(call_node) = node.as_call_node() {
            check_call_node(self, source, &call_node, &cfg, diagnostics);
        } else if let Some(alias_node) = node.as_alias_method_node() {
            check_alias_node(self, source, &alias_node, &cfg, diagnostics);
        }
    }
}

struct ScopeInfo {
    emitter_names: Vec<Vec<u8>>,
}

struct ClassEmitterScopeFinder<'a> {
    target_offset: usize,
    target_name: &'a [u8],
    found: bool,
    scope_stack: Vec<ScopeInfo>,
}

impl<'pr> Visit<'pr> for ClassEmitterScopeFinder<'_> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        self.scope_stack.push(ScopeInfo {
            emitter_names: collect_direct_child_emitters(node.body()),
        });
        ruby_prism::visit_class_node(self, node);
        self.scope_stack.pop();
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
        self.scope_stack.push(ScopeInfo {
            emitter_names: collect_direct_child_emitters(node.body()),
        });
        ruby_prism::visit_module_node(self, node);
        self.scope_stack.pop();
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if self.found {
            return;
        }

        if node.location().start_offset() == self.target_offset {
            self.found = self.scope_stack.last().is_some_and(|scope| {
                scope
                    .emitter_names
                    .iter()
                    .any(|candidate| candidate.as_slice() == self.target_name)
            });
            return;
        }

        ruby_prism::visit_def_node(self, node);
    }
}

fn has_class_emitter_in_scope(
    parse_result: &ruby_prism::ParseResult<'_>,
    def_node: &ruby_prism::DefNode<'_>,
) -> bool {
    let mut finder = ClassEmitterScopeFinder {
        target_offset: def_node.location().start_offset(),
        target_name: def_node.name().as_slice(),
        found: false,
        scope_stack: Vec::new(),
    };
    finder.visit(&parse_result.node());
    finder.found
}

fn check_def_node(
    cop: &MethodName,
    source: &SourceFile,
    def_node: &ruby_prism::DefNode<'_>,
    parse_result: &ruby_prism::ParseResult<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let method_name = def_node.name().as_slice();
    let method_name_str = std::str::from_utf8(method_name).unwrap_or("");

    if method_identifier_predicates::is_operator_method(method_name) {
        return;
    }

    if def_node.receiver().is_some()
        && starts_with_uppercase(method_name_str)
        && has_class_emitter_in_scope(parse_result, def_node)
    {
        return;
    }

    if matches_allowed_pattern(method_name_str, &cfg.allowed_patterns) {
        return;
    }

    let loc = def_node.name_loc();
    let (line, column) = source.offset_to_line_col(loc.start_offset());

    if is_forbidden_name(method_name_str, cfg) {
        diagnostics.push(cop.diagnostic(
            source,
            line,
            column,
            format!("`{method_name_str}` is forbidden, use another method name instead."),
        ));
        return;
    }

    if !style_ok(method_name, &cfg.enforced_style) {
        diagnostics.push(cop.diagnostic(
            source,
            line,
            column,
            format!("Use {} for method names.", style_msg(&cfg.enforced_style)),
        ));
    }
}

fn check_call_node(
    cop: &MethodName,
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let method = call_node.name().as_slice();
    let method_str = std::str::from_utf8(method).unwrap_or("");

    match method_str {
        "define_method" | "define_singleton_method" => {
            check_define_method(cop, source, call_node, cfg, diagnostics);
        }
        "alias_method" => {
            check_alias_method_call(cop, source, call_node, cfg, diagnostics);
        }
        "attr" | "attr_reader" | "attr_writer" | "attr_accessor" => {
            check_attr_accessor(cop, source, call_node, cfg, diagnostics);
        }
        "new" | "define" => {
            check_struct_or_data(cop, source, call_node, cfg, diagnostics);
        }
        _ => {}
    }
}

fn check_define_method(
    cop: &MethodName,
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let args = match call_node.arguments() {
        Some(a) => a,
        None => return,
    };
    let args_list: Vec<_> = args.arguments().iter().collect();
    if args_list.is_empty() {
        return;
    }

    let (name_bytes, loc) = match extract_name_from_sym_or_str(&args_list[0]) {
        Some(v) => v,
        None => return,
    };

    let name_str = match std::str::from_utf8(&name_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    if method_identifier_predicates::is_operator_method(&name_bytes) {
        return;
    }

    emit_method_name_offense(cop, source, name_str, &name_bytes, &loc, cfg, diagnostics);
}

fn check_alias_method_call(
    cop: &MethodName,
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let args = match call_node.arguments() {
        Some(a) => a,
        None => return,
    };
    let args_list: Vec<_> = args.arguments().iter().collect();

    // RuboCop requires exactly 2 arguments
    if args_list.len() != 2 {
        return;
    }

    let (name_bytes, loc) = match extract_name_from_sym_or_str(&args_list[0]) {
        Some(v) => v,
        None => return,
    };

    let name_str = match std::str::from_utf8(&name_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    emit_method_name_offense(cop, source, name_str, &name_bytes, &loc, cfg, diagnostics);
}

fn check_attr_accessor(
    cop: &MethodName,
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Must have no receiver (bare attr_reader, not obj.attr_reader)
    if call_node.receiver().is_some() {
        return;
    }

    let args = match call_node.arguments() {
        Some(a) => a,
        None => return,
    };

    // RuboCop reports the offense at range_position(node), which is
    // selector_end_pos + 1 to expr_end_pos. We use the start of that range
    // (right after the method selector) as the offense location.
    let call_site_offset = call_node
        .message_loc()
        .map(|ml| ml.start_offset() + ml.as_slice().len() + 1)
        .unwrap_or(call_node.location().start_offset());
    let (call_line, call_col) = source.offset_to_line_col(call_site_offset);

    for arg in args.arguments().iter() {
        let (name_bytes, _loc) = match extract_name_from_sym_or_str(&arg) {
            Some(v) => v,
            None => continue,
        };

        let name_str = match std::str::from_utf8(&name_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if matches_allowed_pattern(name_str, &cfg.allowed_patterns) {
            continue;
        }

        if is_forbidden_name(name_str, cfg) {
            diagnostics.push(cop.diagnostic(
                source,
                call_line,
                call_col,
                format!("`{name_str}` is forbidden, use another method name instead."),
            ));
            continue;
        }

        if !style_ok(&name_bytes, &cfg.enforced_style) {
            diagnostics.push(cop.diagnostic(
                source,
                call_line,
                call_col,
                format!("Use {} for method names.", style_msg(&cfg.enforced_style)),
            ));
        }
    }
}

fn check_struct_or_data(
    cop: &MethodName,
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let method = call_node.name().as_slice();
    let receiver = match call_node.receiver() {
        Some(r) => r,
        None => return,
    };

    let is_struct_new = method == b"new" && is_const_named(&receiver, b"Struct");
    let is_data_define = method == b"define" && is_const_named(&receiver, b"Data");

    if !is_struct_new && !is_data_define {
        return;
    }

    let args = match call_node.arguments() {
        Some(a) => a,
        None => return,
    };

    let args_list: Vec<_> = args.arguments().iter().collect();

    // For Struct.new, skip the first argument if it's a string (class name)
    let start_idx = if is_struct_new {
        if args_list
            .first()
            .is_some_and(|a| a.as_string_node().is_some())
        {
            1
        } else {
            0
        }
    } else {
        0
    };

    for arg in &args_list[start_idx..] {
        let (name_bytes, loc) = match extract_name_from_sym_or_str(arg) {
            Some(v) => v,
            None => continue,
        };

        let name_str = match std::str::from_utf8(&name_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };

        emit_method_name_offense(cop, source, name_str, &name_bytes, &loc, cfg, diagnostics);
    }
}

fn check_alias_node(
    cop: &MethodName,
    source: &SourceFile,
    alias_node: &ruby_prism::AliasMethodNode<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let new_name = alias_node.new_name();
    let sym = match new_name.as_symbol_node() {
        Some(s) => s,
        None => return,
    };

    let name_bytes = sym.unescaped().to_vec();
    let name_str = match std::str::from_utf8(&name_bytes) {
        Ok(s) => s,
        Err(_) => return,
    };

    let loc = sym.location();
    emit_method_name_offense(cop, source, name_str, &name_bytes, &loc, cfg, diagnostics);
}

/// Emit an offense for a method name that violates naming rules.
fn emit_method_name_offense(
    cop: &MethodName,
    source: &SourceFile,
    name_str: &str,
    name_bytes: &[u8],
    loc: &ruby_prism::Location<'_>,
    cfg: &MethodNameConfig,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if matches_allowed_pattern(name_str, &cfg.allowed_patterns) {
        return;
    }

    let (line, column) = source.offset_to_line_col(loc.start_offset());

    if is_forbidden_name(name_str, cfg) {
        diagnostics.push(cop.diagnostic(
            source,
            line,
            column,
            format!("`{name_str}` is forbidden, use another method name instead."),
        ));
        return;
    }

    if method_identifier_predicates::is_operator_method(name_bytes) {
        return;
    }

    if !style_ok(name_bytes, &cfg.enforced_style) {
        diagnostics.push(cop.diagnostic(
            source,
            line,
            column,
            format!("Use {} for method names.", style_msg(&cfg.enforced_style)),
        ));
    }
}

/// Check if a node is a constant reference to the given name (handles both `Foo` and `::Foo`).
fn is_const_named(node: &ruby_prism::Node<'_>, name: &[u8]) -> bool {
    if let Some(cr) = node.as_constant_read_node() {
        return cr.name().as_slice() == name;
    }
    if let Some(cp) = node.as_constant_path_node() {
        if cp.parent().is_none() {
            if let Some(child_name) = cp.name() {
                return child_name.as_slice() == name;
            }
        }
    }
    false
}

/// Returns true if the name is lowerCamelCase.
fn is_lower_camel_case(name: &str) -> bool {
    let core = strip_method_suffix(name);
    // RuboCop allows up to 2 leading @ chars (e.g., alias_method :@type, :type)
    let core = core.trim_start_matches('@');
    if core.is_empty() {
        return false;
    }

    let mut chars = core.chars();
    let first = chars.next().unwrap();
    let lead = if first == '_' {
        match chars.next() {
            Some(ch) => ch,
            // RuboCop's camelCase regex: /^@{0,2}(?:_|...)[!?=]?$/
            // A bare underscore is valid camelCase.
            None => return true,
        }
    } else {
        first
    };

    if !lead.is_lowercase() {
        return false;
    }

    chars.all(|ch| ch.is_lowercase() || ch.is_uppercase() || ch.is_ascii_digit())
}

fn is_method_snake_case(name: &str) -> bool {
    let core = strip_method_suffix(name);
    // RuboCop allows up to 2 leading @ chars (e.g., alias_method :@type, :type)
    let core = core.trim_start_matches('@');
    if core.is_empty() {
        return false;
    }

    core.chars()
        .all(|ch| ch == '_' || ch.is_lowercase() || ch.is_ascii_digit())
}

fn strip_method_suffix(name: &str) -> &str {
    match name.chars().next_back() {
        Some('!') | Some('?') | Some('=') => {
            &name[..name.len() - name.chars().next_back().unwrap().len_utf8()]
        }
        _ => name,
    }
}

fn starts_with_uppercase(name: &str) -> bool {
    name.chars().next().is_some_and(|ch| ch.is_uppercase())
}

fn collect_direct_child_emitters(body: Option<ruby_prism::Node<'_>>) -> Vec<Vec<u8>> {
    let Some(body) = body else {
        return Vec::new();
    };

    let mut emitters = Vec::new();
    if let Some(stmts) = body.as_statements_node() {
        for stmt in stmts.body().iter() {
            collect_emitter_name(stmt, &mut emitters);
        }
    } else {
        collect_emitter_name(body, &mut emitters);
    }
    emitters
}

fn collect_emitter_name(node: ruby_prism::Node<'_>, emitters: &mut Vec<Vec<u8>>) {
    // RuboCop only checks :class children, NOT :module children.
    // See configurable_formatting.rb: node.parent.each_child_node(:class)
    // RuboCop compares c.loc.name.is?(name.to_s), which uses the full constant
    // path text (e.g. "HighLine::String"), NOT just the last segment ("String").
    // So `class HighLine::String` does NOT exempt `def self.String`.
    if let Some(class_node) = node.as_class_node() {
        emitters.push(class_node.constant_path().location().as_slice().to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(MethodName, "cops/naming/method_name");

    #[test]
    fn config_enforced_style_camel_case() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("camelCase".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def myMethod\nend\n";
        let diags = run_cop_full_with_config(&MethodName, source, config);
        assert!(
            diags.is_empty(),
            "camelCase method should not be flagged in camelCase mode"
        );
    }

    #[test]
    fn config_enforced_style_camel_case_flags_snake() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("camelCase".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"def my_method\nend\n";
        let diags = run_cop_full_with_config(&MethodName, source, config);
        assert!(
            !diags.is_empty(),
            "snake_case method should be flagged in camelCase mode"
        );
    }

    #[test]
    fn config_forbidden_identifiers() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "ForbiddenIdentifiers".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("destroy".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"def destroy\nend\n";
        let diags = run_cop_full_with_config(&MethodName, source, config);
        assert!(!diags.is_empty(), "Forbidden identifier should be flagged");
        assert!(diags[0].message.contains("forbidden"));
    }

    #[test]
    fn config_forbidden_patterns() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "ForbiddenPatterns".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("_v1\\z".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"def release_v1\nend\n";
        let diags = run_cop_full_with_config(&MethodName, source, config);
        assert!(!diags.is_empty(), "Forbidden pattern should be flagged");
        assert!(diags[0].message.contains("forbidden"));
    }

    #[test]
    fn allowed_patterns_uses_regex() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "AllowedPatterns".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String(
                    "\\AonSelectionBulkChange\\z".into(),
                )]),
            )]),
            ..CopConfig::default()
        };
        let source = b"def onSelectionBulkChange(arg)\nend\n";
        let diags = run_cop_full_with_config(&MethodName, source, config.clone());
        assert!(
            diags.is_empty(),
            "Method matching AllowedPatterns regex should not be flagged"
        );

        let source2 = b"def otherCamelCase\nend\n";
        let diags2 = run_cop_full_with_config(&MethodName, source2, config);
        assert!(
            !diags2.is_empty(),
            "Non-matching camelCase should still be flagged"
        );
    }

    #[test]
    fn camel_case_allows_bare_underscore_method() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("camelCase".into()),
            )]),
            ..CopConfig::default()
        };
        // bare `_` is valid camelCase per RuboCop regex: /^@{0,2}(?:_|_?...)[!?=]?$/
        let source = b"def _(msg)\n  msg\nend\n";
        let diags = run_cop_full_with_config(&MethodName, source, config.clone());
        assert!(
            diags.is_empty(),
            "bare underscore method should be valid camelCase, got: {:?}",
            diags
        );

        // also check define_singleton_method :_
        let source2 = b"define_singleton_method :_ do\nend\n";
        let diags2 = run_cop_full_with_config(&MethodName, source2, config.clone());
        assert!(
            diags2.is_empty(),
            "define_singleton_method :_ should be valid camelCase, got: {:?}",
            diags2
        );

        // also check alias _ foo
        let source3 = b"alias _ foo\n";
        let diags3 = run_cop_full_with_config(&MethodName, source3, config);
        assert!(
            diags3.is_empty(),
            "alias _ should be valid camelCase, got: {:?}",
            diags3
        );
    }

    #[test]
    fn class_emitter_without_class_sibling_is_flagged() {
        // def self.String without a class String sibling should be flagged
        let source =
            b"class HighLine\n  def self.String(s)\n    HighLine::String.new(s)\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&MethodName, source);
        assert_eq!(
            diags.len(),
            1,
            "def self.String without class String sibling should be flagged"
        );
    }

    #[test]
    fn class_emitter_with_class_sibling_is_allowed() {
        // def self.String WITH a class String sibling should be allowed
        let source = b"class HighLine\n  def self.String(s)\n    String.new(s)\n  end\n  class String < ::String\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&MethodName, source);
        assert!(
            diags.is_empty(),
            "def self.String with class String sibling should be allowed"
        );
    }
}
