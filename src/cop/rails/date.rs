use crate::cop::shared::constant_predicates;
use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

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

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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
        let style = config.get_str("EnforcedStyle", "flexible");
        let allow_to_time = config.get_bool("AllowToTime", true);

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method = call.name().as_slice();

        // `to_time_in_current_zone` is always deprecated, regardless of EnforcedStyle.
        // RuboCop requires a receiver (`node.receiver && ...`), so implicit-self calls
        // like bare `to_time_in_current_zone` inside ActiveSupport are not flagged.
        if method == b"to_time_in_current_zone" && call.receiver().is_some() {
            let msg_loc = match call.message_loc() {
                Some(loc) => loc,
                None => return,
            };
            let (line, column) = source.offset_to_line_col(msg_loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "`to_time_in_current_zone` is deprecated. Use `in_time_zone` instead.".to_string(),
            ));
            return;
        }

        // In strict mode, also flag `to_time` (requires explicit receiver, same as RuboCop)
        if method == b"to_time" && call.receiver().is_some() && !allow_to_time && style == "strict"
        {
            let msg_loc = match call.message_loc() {
                Some(loc) => loc,
                None => return,
            };
            let (line, column) = source.offset_to_line_col(msg_loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Do not use `to_time` in strict mode.".to_string(),
            ));
        }

        // In flexible mode, only `Date.today` is flagged.
        // In strict mode, `Date.today`, `Date.yesterday`, `Date.tomorrow`, and `Date.current`
        // are all flagged.
        let is_bad_date_method = match method {
            b"today" => true,
            b"yesterday" | b"tomorrow" | b"current" => style == "strict",
            _ => false,
        };
        if !is_bad_date_method {
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

        let msg_loc = match call.message_loc() {
            Some(loc) => loc,
            None => return,
        };
        let (line, column) = source.offset_to_line_col(msg_loc.start_offset());

        // `method` is `&[u8]` - convert to &str for message formatting
        let method_str = std::str::from_utf8(method).unwrap_or("");
        // In strict mode, `Date.current` is flagged with a different message
        // ("Use Time.zone.today instead") while the others use their own method name.
        let msg = if style == "strict" && method == b"current" {
            "Do not use `Date.current` without zone. Use `Time.zone.today` instead.".to_string()
        } else if style == "strict" {
            format!(
                "Do not use `Date.{}` without zone. Use `Time.zone.{}` instead.",
                method_str, method_str
            )
        } else {
            "Use `Date.current` instead of `Date.today`.".to_string()
        };

        diagnostics.push(self.diagnostic(source, line, column, msg));
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
