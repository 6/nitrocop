use crate::cop::shared::node_type::{
    BLOCK_NODE, CALL_NODE, CLASS_VARIABLE_READ_NODE, CONSTANT_PATH_NODE, CONSTANT_READ_NODE,
    GLOBAL_VARIABLE_READ_NODE, INSTANCE_VARIABLE_READ_NODE, LOCAL_VARIABLE_READ_NODE,
    STATEMENTS_NODE, SYMBOL_NODE,
};
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Checks for consistent style of change matcher.
///
/// Enforces either passing a receiver and message as method arguments,
/// or a block.
///
/// `EnforcedStyle: method_call` (default): flags `change { obj.attr }` and
/// suggests `change(obj, :attr)`. The receiver must be a constant or bare
/// method call (no receiver, no arguments, no block).
///
/// `EnforcedStyle: block`: flags `change(obj, :attr)` and suggests
/// `change { obj.attr }`. RuboCop's pattern accepts ANY first argument type
/// (not just constants/variables), so the Rust implementation was too
/// restrictive — it was missing global variables (`$token`) and chained
/// method calls (e.g., `users.green`).
///
/// This cop is visited by the corpus oracle for both style variants.
pub struct ExpectChange;

impl Cop for ExpectChange {
    fn name(&self) -> &'static str {
        "RSpec/ExpectChange"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            BLOCK_NODE,
            CALL_NODE,
            CLASS_VARIABLE_READ_NODE,
            CONSTANT_PATH_NODE,
            CONSTANT_READ_NODE,
            GLOBAL_VARIABLE_READ_NODE,
            INSTANCE_VARIABLE_READ_NODE,
            LOCAL_VARIABLE_READ_NODE,
            STATEMENTS_NODE,
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
        // Config: EnforcedStyle — "method_call" (default) or "block"
        let enforced_style = config.get_str("EnforcedStyle", "method_call");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.receiver().is_some() {
            return;
        }

        if call.name().as_slice() != b"change" {
            return;
        }

        if enforced_style == "block" {
            // "block" style: flag `change(Obj, :attr)` — prefer block form
            let args = match call.arguments() {
                Some(a) => a,
                None => return,
            };
            let arg_list: Vec<_> = args.arguments().iter().collect();
            if arg_list.len() != 2 {
                return;
            }
            // RuboCop's pattern `$_` accepts ANY first argument type.
            // The second argument must be a symbol or string.
            if arg_list[1].as_symbol_node().is_none() {
                return;
            }
            let loc = call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `change { }` over `change(obj, :attr)`.".to_string(),
            ));
        }

        // Default: "method_call" style — flag `change { User.count }`
        // and suggest `change(User, :count)`.
        let block_node_raw = match call.block() {
            Some(b) => b,
            None => return,
        };

        let block = match block_node_raw.as_block_node() {
            Some(b) => b,
            None => return,
        };

        // If it already has positional arguments, it's method_call style — fine
        if call.arguments().is_some() {
            return;
        }

        // Check if the block body is a simple method call: Receiver.method (no args)
        let body = match block.body() {
            Some(b) => b,
            None => return,
        };

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        let stmt_list: Vec<_> = stmts.body().iter().collect();
        if stmt_list.len() != 1 {
            return;
        }

        let inner_call = match stmt_list[0].as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Must be a method call on a receiver with no arguments
        if inner_call.receiver().is_none() {
            return;
        }

        if inner_call.arguments().is_some() {
            return;
        }

        // Calls with their own block are not "simple message sends" and should
        // stay in block form (e.g. `change { Sidekiq.redis { ... } }`).
        if inner_call.block().is_some() {
            return;
        }

        // The receiver must match RuboCop's pattern: a constant or bare method
        // call (no receiver). Local variables and instance variables do NOT match
        // because RuboCop's pattern `(send nil? _)` only matches bare method calls,
        // not `(lvar ...)` or `(ivar ...)`.
        let recv = inner_call.receiver().unwrap();
        let is_simple_receiver = recv.as_constant_read_node().is_some()
            || recv.as_constant_path_node().is_some()
            || (recv.as_call_node().is_some_and(|c| {
                c.receiver().is_none() && c.arguments().is_none() && c.block().is_none()
            }));
        if !is_simple_receiver {
            return;
        }

        let recv_loc = recv.location();
        let recv_text = source.byte_slice(recv_loc.start_offset(), recv_loc.end_offset(), "");
        let method = std::str::from_utf8(inner_call.name().as_slice()).unwrap_or("");

        let loc = call.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!("Prefer `change({recv_text}, :{method})`."),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ExpectChange, "cops/rspec/expect_change");

    #[test]
    fn block_style_flags_method_call_form() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("block".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect { x }.to change(User, :count)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ExpectChange, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("change { }"));
    }

    #[test]
    fn block_style_does_not_flag_block_form() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("block".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect { x }.to change { User.count }\n";
        let diags = crate::testutil::run_cop_full_with_config(&ExpectChange, source, config);
        assert!(diags.is_empty());
    }

    #[test]
    fn block_style_flags_global_variable_first_arg() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("block".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect { run }.to change($token, :value)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ExpectChange, source, config);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn block_style_flags_instance_variable_first_arg() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("block".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect { run }.to change(@user, :name)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ExpectChange, source, config);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn block_style_flags_chained_method_call_first_arg() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("block".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect { run }.to change(users.green, :count)\n";
        let diags = crate::testutil::run_cop_full_with_config(&ExpectChange, source, config);
        assert_eq!(diags.len(), 1);
    }
}
