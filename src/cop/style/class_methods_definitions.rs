use crate::cop::shared::node_type::{DEF_NODE, SELF_NODE, SINGLETON_CLASS_NODE, STATEMENTS_NODE};
use crate::cop::shared::util::line_at;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/ClassMethodsDefinitions cop.
///
/// ## Investigation findings (2026-03-31)
///
/// RuboCop only inspects direct child plain `def` nodes inside `class << self`.
/// Visibility-wrapped forms like `private def helper`, `protected def helper`,
/// and `public def helper` are wrapped in a call node, so they do not count as
/// candidate methods for this cop at all.
///
/// ## Fix (2026-04-02): FP from `?` operator, FN from multi-arg visibility
///
/// **FP bug**: `inline_method_visibility` used `?` on `visibility_name()` which
/// caused early return from the function when encountering any non-visibility
/// call node (e.g. `alias_method`, `wx_redefine_method`). This prevented
/// `private :method_name` from being found when a non-visibility call appeared
/// after it. Fix: replaced `?` with `let Some(...) else { continue }`.
///
/// **FN bug**: RuboCop's `visibility_inline_on_method_name?` node pattern
/// `(send nil? VISIBILITY_SCOPES (sym %method_name))` only matches single-arg
/// forms like `private :foo`. Multi-arg forms like `private :foo, :bar` are NOT
/// recognized, so those methods remain public. Our code previously matched any
/// number of args. Fix: only match when there is exactly one symbol argument.
///
/// ## Fix (2026-04-04): FP from RuboCop autocorrect crash on trailing same-line code
///
/// RuboCop 1.84.2 reports no offense when a direct `def` ends on the same line
/// as the enclosing `class << self` and outer code continues after that `end`
/// on the same line (for example `end end; X = 1`). The cop crashes while
/// registering autocorrections, so the corpus records no offense. Fix: skip
/// that exact rewrite-crash shape while still flagging the nearby general case
/// where the singleton class closes at the end of the line.
///
/// ## Fix (2026-04-05): FN for `EnforcedStyle: self_class`
///
/// Variant checks showed the cop only implemented the default `def_self`
/// branch, so `def self.method_name` was never flagged when
/// `EnforcedStyle: self_class` was configured. RuboCop's `on_defs` logic is
/// simple here: any `def` whose receiver is exactly `self` is an offense,
/// including inside `class << self`; receivers other than `self` are ignored.
pub struct ClassMethodsDefinitions;

impl Cop for ClassMethodsDefinitions {
    fn name(&self) -> &'static str {
        "Style/ClassMethodsDefinitions"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[DEF_NODE, SELF_NODE, SINGLETON_CLASS_NODE, STATEMENTS_NODE]
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
        let enforced_style = config.get_str("EnforcedStyle", "def_self");

        if enforced_style == "self_class" {
            if let Some(def_node) = node.as_def_node() {
                if def_node
                    .receiver()
                    .is_some_and(|receiver| receiver.as_self_node().is_some())
                {
                    let loc = def_node.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Use `class << self` to define a class method.".to_string(),
                    ));
                }
            }

            return;
        }

        // Check for `class << self` with public methods
        if let Some(sclass) = node.as_singleton_class_node() {
            let expr = sclass.expression();
            if expr.as_self_node().is_some() {
                // Check if body has defs and ALL are public
                if sclass.body().is_some() {
                    let sclass_line = source
                        .offset_to_line_col(sclass.location().start_offset())
                        .0;
                    if all_defs_public(source, &sclass, sclass_line) {
                        let loc = sclass.location();
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            "Do not define public methods within class << self.".to_string(),
                        ));
                    }
                }
            }
        }
    }
}

/// Returns true if the sclass body contains at least one plain `def` node
/// (no receiver) and ALL such `def` nodes are public. This matches RuboCop's
/// `all_methods_public?` which only flags `class << self` when every method
/// can be trivially converted to `def self.method_name`.
///
/// Also returns false (skip) if any plain `def` starts on the same line as
/// the `class << self` keyword — RuboCop does not flag compact single-line forms.
fn all_defs_public(
    source: &SourceFile,
    sclass: &ruby_prism::SingletonClassNode<'_>,
    sclass_line: usize,
) -> bool {
    let Some(body) = sclass.body() else {
        return false;
    };

    let stmts = match body.as_statements_node() {
        Some(s) => s,
        None => {
            // Single-statement body: check if it's a plain def node (no receiver)
            if let Some(def_node) = body.as_def_node() {
                if def_node.receiver().is_some() {
                    return false; // `def self.x` — not a plain def
                }
                // Check single-line: if def is on same line as class << self, skip
                let def_line = source
                    .offset_to_line_col(def_node.location().start_offset())
                    .0;
                return def_line != sclass_line
                    && !rubocop_autocorrect_crash_shape(source, sclass, &def_node);
            }
            return false;
        }
    };

    let stmts_vec: Vec<_> = stmts.body().iter().collect();
    let mut direct_plain_defs = Vec::new();

    for (idx, stmt) in stmts_vec.iter().enumerate() {
        let Some(def_node) = stmt.as_def_node() else {
            continue;
        };

        // Only consider plain defs (no receiver like `def self.x`)
        if def_node.receiver().is_some() {
            continue;
        }

        let def_line = source
            .offset_to_line_col(def_node.location().start_offset())
            .0;
        if def_line == sclass_line {
            return false; // Single-line form — RuboCop does not flag
        }

        direct_plain_defs.push((idx, def_node));
    }

    let Some((_, last_direct_plain_def)) = direct_plain_defs.last() else {
        return false;
    };

    if rubocop_autocorrect_crash_shape(source, sclass, last_direct_plain_def) {
        return false;
    }

    direct_plain_defs.into_iter().all(|(idx, def_node)| {
        direct_def_visibility(&stmts_vec, idx, def_node.name().as_slice())
            == MethodVisibility::Public
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MethodVisibility {
    Public,
    Protected,
    Private,
}

fn direct_def_visibility(
    stmts: &[ruby_prism::Node<'_>],
    idx: usize,
    method_name: &[u8],
) -> MethodVisibility {
    inline_method_visibility(stmts, idx, method_name)
        .or_else(|| enclosing_visibility(stmts, idx))
        .unwrap_or(MethodVisibility::Public)
}

fn inline_method_visibility(
    stmts: &[ruby_prism::Node<'_>],
    idx: usize,
    method_name: &[u8],
) -> Option<MethodVisibility> {
    for stmt in stmts[idx + 1..].iter().rev() {
        let Some(call) = stmt.as_call_node() else {
            continue;
        };
        if call.receiver().is_some() {
            continue;
        }

        let Some(visibility) = visibility_name(call.name().as_slice()) else {
            continue;
        };
        let Some(args) = call.arguments() else {
            continue;
        };

        // Only match single-arg forms: `private :method_name`
        // Multi-arg forms like `private :foo, :bar` are not recognized by
        // RuboCop's visibility_inline_on_method_name? node pattern.
        let args_vec: Vec<_> = args.arguments().iter().collect();
        if args_vec.len() == 1
            && args_vec[0]
                .as_symbol_node()
                .is_some_and(|symbol| symbol.unescaped() == method_name)
        {
            return Some(visibility);
        }
    }

    None
}

fn enclosing_visibility(stmts: &[ruby_prism::Node<'_>], idx: usize) -> Option<MethodVisibility> {
    for stmt in stmts[..idx].iter().rev() {
        let Some(call) = stmt.as_call_node() else {
            continue;
        };

        if call.receiver().is_none() && call.arguments().is_none() {
            if let Some(visibility) = visibility_name(call.name().as_slice()) {
                return Some(visibility);
            }
        }
    }

    None
}

fn visibility_name(name: &[u8]) -> Option<MethodVisibility> {
    match name {
        b"public" => Some(MethodVisibility::Public),
        b"protected" => Some(MethodVisibility::Protected),
        b"private" => Some(MethodVisibility::Private),
        _ => None,
    }
}

fn rubocop_autocorrect_crash_shape(
    source: &SourceFile,
    sclass: &ruby_prism::SingletonClassNode<'_>,
    last_direct_plain_def: &ruby_prism::DefNode<'_>,
) -> bool {
    let sclass_end = sclass.location().end_offset();
    let def_end = last_direct_plain_def.location().end_offset();

    let sclass_end_line = source.offset_to_line_col(sclass_end).0;
    let def_end_line = source.offset_to_line_col(def_end).0;
    if sclass_end_line != def_end_line {
        return false;
    }

    has_same_line_code_after_offset(source, sclass_end)
}

fn has_same_line_code_after_offset(source: &SourceFile, offset: usize) -> bool {
    let (line_number, column) = source.offset_to_line_col(offset);
    let Some(line) = line_at(source, line_number) else {
        return false;
    };

    let mut idx = column.min(line.len());
    while idx < line.len() && line[idx].is_ascii_whitespace() {
        idx += 1;
    }

    while idx < line.len() && line[idx] == b';' {
        idx += 1;
        while idx < line.len() && line[idx].is_ascii_whitespace() {
            idx += 1;
        }
    }

    idx < line.len() && line[idx] != b'#'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;

    crate::cop_fixture_tests!(
        ClassMethodsDefinitions,
        "cops/style/class_methods_definitions"
    );

    fn self_class_config() -> CopConfig {
        let mut config = CopConfig::default();
        config.options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("self_class".into()),
        );
        config
    }

    #[test]
    fn flags_def_self_when_self_class_style_is_enforced() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ClassMethodsDefinitions,
            b"class A\n  def self.one\n  ^^^^^^^^^^^^ Style/ClassMethodsDefinitions: Use `class << self` to define a class method.\n  end\nend\n",
            self_class_config(),
        );
    }

    #[test]
    fn ignores_singleton_methods_not_defined_on_self_for_self_class_style() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ClassMethodsDefinitions,
            b"object = Object.new\ndef object.method\nend\n",
            self_class_config(),
        );
    }
}
