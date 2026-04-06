use crate::cop::shared::node_type::CLASS_NODE;
use crate::cop::shared::util::{class_body_calls, is_dsl_call};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct ActiveRecordCallbacksOrder;

const CALLBACK_ORDER: &[&[u8]] = &[
    b"after_initialize",
    b"before_validation",
    b"after_validation",
    b"before_save",
    b"around_save",
    b"before_create",
    b"around_create",
    b"after_create",
    b"before_update",
    b"around_update",
    b"after_update",
    b"before_destroy",
    b"around_destroy",
    b"after_destroy",
    b"after_save",
    b"after_commit",
    b"after_rollback",
    b"after_find",
    b"after_touch",
];

fn callback_order_index(name: &[u8]) -> Option<usize> {
    CALLBACK_ORDER.iter().position(|&c| c == name)
}

impl Cop for ActiveRecordCallbacksOrder {
    fn name(&self) -> &'static str {
        "Rails/ActiveRecordCallbacksOrder"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CLASS_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let class = match node.as_class_node() {
            Some(c) => c,
            None => return,
        };

        let calls = class_body_calls(&class);

        // Collect (callback_name, order_index, offset) for known callbacks
        let mut callbacks: Vec<(&[u8], usize, usize)> = Vec::new();

        for call in &calls {
            // RuboCop only considers send nodes (callbacks without blocks).
            // Callbacks with blocks (before_save do...end, after_commit { }) are skipped.
            if call.block().is_some() {
                continue;
            }
            for &cb_name in CALLBACK_ORDER {
                if is_dsl_call(call, cb_name) {
                    if let Some(idx) = callback_order_index(cb_name) {
                        let loc = call.message_loc().unwrap_or(call.location());
                        callbacks.push((cb_name, idx, loc.start_offset()));
                    }
                    break;
                }
            }
        }

        let mut prev_idx: isize = -1;
        let mut prev_name: &[u8] = b"";

        for &(name, idx, offset) in &callbacks {
            let idx_signed = idx as isize;
            if idx_signed < prev_idx {
                let (line, column) = source.offset_to_line_col(offset);
                let name_str = String::from_utf8_lossy(name);
                let other_str = String::from_utf8_lossy(prev_name);
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("`{name_str}` is supposed to appear before `{other_str}`."),
                ));
            }
            prev_idx = idx_signed;
            prev_name = name;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        ActiveRecordCallbacksOrder,
        "cops/rails/active_record_callbacks_order"
    );
}
