use crate::cop::shared::constant_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// ## Variant style divergence (2026-04-17)
///
/// Variant oracle reported FN=2 for `EnforcedStyle=strict` (0 FP).
///
/// FN=2: strict mode still missed chained calls rooted at bare `Date` constants, such as
/// `Date.new(...).yesterday` and `Date.new(...).tomorrow`. RuboCop checks the root send whose
/// receiver is bare `Date`, then walks send ancestors and reports the strict `Date.<method>`
/// message on that root selector (`new` here), not on the trailing day method. Fixed by
/// reproducing that root-call ancestor walk only in strict mode so flexible mode stays unchanged.
///
/// ## Variant style divergence (2026-04-08)
///
/// Variant oracle reported FN=1,426 for `EnforcedStyle=strict` (0 FP).
///
/// FN=1,426: `Date.yesterday`, `Date.tomorrow`, and `Date.current` were not flagged in strict
/// mode. RuboCop's `strict` mode flags all four methods (`today`, `yesterday`, `tomorrow`,
/// `current`). Fixed by branching on `EnforcedStyle`: flexible mode only flags `Date.today`,
/// while strict mode flags all four. Messages also differ: flexible uses "Use `Date.current`
/// instead of `Date.today`" while strict uses "Do not use `Date.<method>` without zone."
///
/// ## Corpus investigation (2026-03-26)
///
/// Corpus oracle reported FP=5, FN=0.
///
/// FP=5: All 5 FPs from cjstewart88/Tubalr — `to_time_in_current_zone` called without an
/// explicit receiver (implicit `self`) inside ActiveSupport's own core_ext/date/ files.
/// RuboCop's `on_send` starts with `return unless node.receiver && ...`, so implicit-self
/// calls are never flagged. Fixed by adding a `call.receiver().is_some()` check before
/// flagging `to_time_in_current_zone` (and `to_time` for the same reason).
///
/// ## Corpus investigation (2026-03-19)
///
/// Corpus oracle reported FP=4, FN=1.
///
/// FP=4: All 4 FPs from ecleel/hijri repo — `Hijri::Date.today` and `Hijri::DateTime.now`.
/// RuboCop's NodePattern matches `(const {nil? cbase} :Date)` which only accepts bare `Date`
/// or `::Date`, not qualified paths like `Hijri::Date`. Fixed by replacing `constant_short_name()`
/// (which returns the terminal name) with `is_simple_constant()` which validates the full path.
///
/// FN=1: netzke/netzke-basepack — `to_time_in_current_zone` deprecated method was not detected.
/// Fixed by adding an explicit check for `to_time_in_current_zone` that fires regardless of
/// EnforcedStyle, matching RuboCop's DEPRECATED_METHODS behavior.
pub struct Date;

impl Cop for Date {
    fn name(&self) -> &'static str {
        "Rails/Date"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
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
        let mut visitor = DateVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            strict: config.get_str("EnforcedStyle", "flexible") == "strict",
            allow_to_time: config.get_bool("AllowToTime", true),
            ancestors: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct DateVisitor<'a, 'pr> {
    cop: &'a Date,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    strict: bool,
    allow_to_time: bool,
    ancestors: Vec<ruby_prism::Node<'pr>>,
}

impl<'pr> DateVisitor<'_, 'pr> {
    fn add_call_offense(&mut self, call: &ruby_prism::CallNode<'pr>, message: String) {
        let Some(msg_loc) = call.message_loc() else {
            return;
        };
        let (line, column) = self.source.offset_to_line_col(msg_loc.start_offset());
        self.diagnostics
            .push(self.cop.diagnostic(self.source, line, column, message));
    }

    fn check_call(&mut self, call: &ruby_prism::CallNode<'pr>) {
        let method = call.name().as_slice();

        // `to_time_in_current_zone` is always deprecated, regardless of EnforcedStyle.
        // RuboCop requires a receiver (`node.receiver && ...`), so implicit-self calls
        // like bare `to_time_in_current_zone` inside ActiveSupport are not flagged.
        if method == b"to_time_in_current_zone" && call.receiver().is_some() {
            self.add_call_offense(
                call,
                "`to_time_in_current_zone` is deprecated. Use `in_time_zone` instead.".to_string(),
            );
            return;
        }

        // In strict mode, also flag `to_time` (requires explicit receiver, same as RuboCop)
        if self.strict && method == b"to_time" && call.receiver().is_some() && !self.allow_to_time {
            self.add_call_offense(call, "Do not use `to_time` in strict mode.".to_string());
            return;
        }

        if self.strict {
            if let Some(message) = strict_date_chain_message(call, &self.ancestors) {
                self.add_call_offense(call, message);
            }
            return;
        }

        if method != b"today" {
            return;
        }

        let recv = match call.receiver() {
            Some(r) => r,
            None => return,
        };
        // RuboCop matches `(const {nil? cbase} :Date)` — only bare `Date` or `::Date`,
        // not qualified paths like `Hijri::Date`.
        if !constant_predicates::is_simple_constant(&recv, b"Date") {
            return;
        }

        self.add_call_offense(
            call,
            "Use `Date.current` instead of `Date.today`.".to_string(),
        );
    }
}

impl<'pr> Visit<'pr> for DateVisitor<'_, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.check_call(node);
        ruby_prism::visit_call_node(self, node);
    }
}

fn strict_date_chain_message(
    call: &ruby_prism::CallNode<'_>,
    ancestors: &[ruby_prism::Node<'_>],
) -> Option<String> {
    let recv = call.receiver()?;
    // RuboCop roots this logic at the send whose receiver is bare `Date` / `::Date`.
    if !constant_predicates::is_simple_constant(&recv, b"Date") {
        return None;
    }

    let bad_methods = strict_bad_day_chain(call, ancestors);
    if bad_methods.is_empty() {
        return None;
    }

    let method_name = bad_methods.join(".");
    let day = if method_name == "current" {
        "today".to_string()
    } else {
        method_name.clone()
    };

    Some(format!(
        "Do not use `Date.{}` without zone. Use `Time.zone.{}` instead.",
        method_name, day
    ))
}

fn strict_bad_day_chain(
    call: &ruby_prism::CallNode<'_>,
    ancestors: &[ruby_prism::Node<'_>],
) -> Vec<String> {
    let mut bad_methods = Vec::new();
    push_bad_day_method(&mut bad_methods, call.name().as_slice());

    for ancestor in ancestors.iter().rev().skip(1) {
        if let Some(parent_call) = ancestor.as_call_node() {
            push_bad_day_method(&mut bad_methods, parent_call.name().as_slice());
        }
    }

    bad_methods
}

fn push_bad_day_method(bad_methods: &mut Vec<String>, method: &[u8]) {
    match method {
        b"today" | b"current" | b"yesterday" | b"tomorrow" => {
            bad_methods.push(String::from_utf8_lossy(method).into_owned());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(Date, "cops/rails/date");

    fn strict_config() -> crate::cop::CopConfig {
        use std::collections::HashMap;
        crate::cop::CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("strict".into()),
            )]),
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn strict_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &Date,
            include_bytes!("../../../tests/fixtures/cops/rails/date/strict_offense.rb"),
            strict_config(),
        );
    }

    #[test]
    fn strict_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &Date,
            include_bytes!("../../../tests/fixtures/cops/rails/date/strict_no_offense.rb"),
            strict_config(),
        );
    }
}
