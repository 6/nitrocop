use ruby_prism::Visit;

use crate::cop::shared::node_type::{
    ASSOC_NODE, BLOCK_ARGUMENT_NODE, BLOCK_NODE, CALL_NODE, CLASS_NODE, DEF_NODE, HASH_NODE,
    KEYWORD_HASH_NODE, STATEMENTS_NODE, SYMBOL_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Matches RuboCop's current migration gating and AST-shape quirks.
///
/// RuboCop only exempts `drop_table` when the send node's immediate parent is a
/// single-statement block body, so multi-statement `each`/`safety_assured`
/// blocks still register. It also keeps traversing nested defs/classes/modules
/// lexically inside `change`, so nested `def change` bodies still count.
pub struct ReversibleMigration;

/// Methods that are always irreversible in a `change` method.
const IRREVERSIBLE_METHODS: &[&[u8]] = &[b"execute", b"change_column"];

/// Methods that are irreversible without certain conditions.
const CONDITIONALLY_IRREVERSIBLE: &[(&[u8], IrreversibleCondition)] = &[
    (b"drop_table", IrreversibleCondition::NeedsBlock),
    (b"remove_column", IrreversibleCondition::NeedsThreeArgs),
    (b"remove_columns", IrreversibleCondition::NeedsTypeOption),
    (b"remove_index", IrreversibleCondition::NeedsColumnOption),
    (
        b"remove_foreign_key",
        IrreversibleCondition::NeedsTwoArgsOrToTable,
    ),
    (b"change_column_default", IrreversibleCondition::NeedsFromTo),
    (b"change_table_comment", IrreversibleCondition::NeedsFromTo),
    (b"change_column_comment", IrreversibleCondition::NeedsFromTo),
];

// All variants intentionally share the `Needs` prefix for readability.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy)]
enum IrreversibleCondition {
    NeedsBlock,
    NeedsThreeArgs,
    NeedsTypeOption,
    NeedsColumnOption,
    NeedsTwoArgsOrToTable,
    NeedsFromTo,
}

/// Visitor that finds irreversible method calls inside a `change` method body.
struct IrreversibleFinder {
    offenses: Vec<(usize, String)>,
    inside_reversible: bool,
    inside_up_only: bool,
    inside_change_table: bool,
    direct_block_body_call_offsets: Vec<usize>,
}

impl<'pr> Visit<'pr> for IrreversibleFinder {
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        if let Some(parameters) = node.parameters() {
            self.visit(&parameters);
        }

        let Some(body) = node.body() else {
            return;
        };

        let direct_body_call_offset = body
            .as_statements_node()
            .filter(|statements| statements.body().len() == 1)
            .and_then(|statements| statements.body().first())
            .and_then(|statement| statement.as_call_node())
            .map(|call| call.location().start_offset());

        if let Some(offset) = direct_body_call_offset {
            self.direct_block_body_call_offsets.push(offset);
        }

        self.visit(&body);

        if direct_body_call_offset.is_some() {
            self.direct_block_body_call_offsets.pop();
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = node.name().as_slice();
        let immediate_parent_is_block = self
            .direct_block_body_call_offsets
            .last()
            .is_some_and(|offset| *offset == node.location().start_offset());

        // Check for `reversible` block
        if name == b"reversible" && node.receiver().is_none() && node.block().is_some() {
            let prev = self.inside_reversible;
            self.inside_reversible = true;
            // Don't flag anything inside reversible blocks
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    if let Some(body) = block_node.body() {
                        self.visit(&body);
                    }
                }
            }
            self.inside_reversible = prev;
            return;
        }

        // Check for `up_only` block
        if name == b"up_only" && node.receiver().is_none() {
            let prev = self.inside_up_only;
            self.inside_up_only = true;
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    if let Some(body) = block_node.body() {
                        self.visit(&body);
                    }
                }
            }
            self.inside_up_only = prev;
            return;
        }

        // Traverse `change_table` bodies recursively so nested `if`/`rescue`
        // wrappers still expose `t.change_default` / `t.remove` calls.
        if name == b"change_table" && node.receiver().is_none() {
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    if let Some(body) = block_node.body() {
                        let prev = self.inside_change_table;
                        self.inside_change_table = true;
                        self.visit(&body);
                        self.inside_change_table = prev;
                    }
                }
            }
            return;
        }

        // If inside reversible or up_only, everything is ok
        if self.inside_reversible || self.inside_up_only {
            ruby_prism::visit_call_node(self, node);
            return;
        }

        if let Some(message) = change_table_message(node, self.inside_change_table) {
            self.offenses
                .push((node.location().start_offset(), message.to_string()));
            return;
        }

        if is_receiver_change_without_from_to(node) {
            self.offenses.push((
                node.location().start_offset(),
                "change(without :from and :to) is not reversible.".to_string(),
            ));
            return;
        }

        // Check always-irreversible methods
        if IRREVERSIBLE_METHODS.contains(&name) && node.receiver().is_none() {
            let method_str = std::str::from_utf8(name).unwrap_or("execute");
            self.offenses.push((
                node.location().start_offset(),
                format!("{method_str} is not reversible."),
            ));
            return;
        }

        // Check conditionally irreversible methods
        for &(method, condition) in CONDITIONALLY_IRREVERSIBLE {
            if name == method
                && node.receiver().is_none()
                && !is_condition_met(node, condition, immediate_parent_is_block)
            {
                let method_str = std::str::from_utf8(name).unwrap_or("method");
                let desc = condition_desc(condition);
                self.offenses.push((
                    node.location().start_offset(),
                    format!("{method_str}({desc}) is not reversible."),
                ));
                return;
            }
        }

        // Continue visiting children (e.g., inside blocks like each)
        ruby_prism::visit_call_node(self, node);
    }
}

fn is_condition_met(
    call: &ruby_prism::CallNode<'_>,
    condition: IrreversibleCondition,
    immediate_parent_is_block: bool,
) -> bool {
    match condition {
        IrreversibleCondition::NeedsBlock => {
            // RuboCop treats a drop_table send as reversible when the send node is
            // itself the direct body of a block (single-statement block body).
            if immediate_parent_is_block {
                return true;
            }
            // Must have a block or a & argument
            if call.block().is_some() {
                return true;
            }
            // Check for &:proc argument
            if let Some(args) = call.arguments() {
                for arg in args.arguments().iter() {
                    if arg.as_block_argument_node().is_some() {
                        return true;
                    }
                }
            }
            // Also no arguments means we can't flag it (might be dynamic)
            if call.arguments().is_none() {
                return true;
            }
            false
        }
        IrreversibleCondition::NeedsThreeArgs => {
            // remove_column needs at least 3 args (table, column, type)
            if let Some(args) = call.arguments() {
                let count = args.arguments().iter().count();
                count >= 3
            } else {
                false
            }
        }
        IrreversibleCondition::NeedsTypeOption => has_type_option(call),
        IrreversibleCondition::NeedsColumnOption => {
            // remove_index needs :column option or 2 positional args
            if let Some(args) = call.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                // 2 positional args: remove_index(:table, :column)
                if arg_list.len() >= 2 {
                    // Check second arg isn't just a hash with name:
                    if arg_list[1].as_keyword_hash_node().is_none()
                        && arg_list[1].as_hash_node().is_none()
                    {
                        return true;
                    }
                    // Check for column: keyword
                    if has_keyword_option(call, b"column") {
                        return true;
                    }
                }
            }
            false
        }
        IrreversibleCondition::NeedsTwoArgsOrToTable => {
            // remove_foreign_key needs 2 table args or :to_table option
            if has_keyword_option(call, b"to_table") {
                return true;
            }
            if let Some(args) = call.arguments() {
                let positional_count = args
                    .arguments()
                    .iter()
                    .filter(|a| a.as_keyword_hash_node().is_none() && a.as_hash_node().is_none())
                    .count();
                positional_count >= 2
            } else {
                false
            }
        }
        IrreversibleCondition::NeedsFromTo => has_from_and_to_args(call),
    }
}

fn has_from_and_to_args(call: &ruby_prism::CallNode<'_>) -> bool {
    has_keyword_option(call, b"from") && has_keyword_option(call, b"to")
}

fn has_hash_argument(call: &ruby_prism::CallNode<'_>) -> bool {
    let Some(args) = call.arguments() else {
        return false;
    };

    args.arguments()
        .iter()
        .any(|arg| arg.as_keyword_hash_node().is_some() || arg.as_hash_node().is_some())
}

fn has_type_option(call: &ruby_prism::CallNode<'_>) -> bool {
    has_keyword_option(call, b"type")
}

fn has_keyword_option(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> bool {
    let args = match call.arguments() {
        Some(a) => a,
        None => return false,
    };
    for arg in args.arguments().iter() {
        if let Some(kw) = arg.as_keyword_hash_node() {
            for elem in kw.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == key {
                            return true;
                        }
                    }
                }
            }
        }
        if let Some(hash) = arg.as_hash_node() {
            for elem in hash.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == key {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn change_table_message(
    call: &ruby_prism::CallNode<'_>,
    inside_change_table: bool,
) -> Option<&'static str> {
    if !inside_change_table || call.receiver().is_none() {
        return None;
    }

    match call.name().as_slice() {
        b"change" => Some("change_table(with change) is not reversible."),
        b"change_default" if !has_from_and_to_args(call) => {
            Some("change_table(with change_default) is not reversible.")
        }
        b"remove" if !has_type_option(call) => Some("t.remove (without type) is not reversible."),
        _ => None,
    }
}

fn is_receiver_change_without_from_to(call: &ruby_prism::CallNode<'_>) -> bool {
    call.name().as_slice() == b"change"
        && call.receiver().is_some()
        && has_hash_argument(call)
        && !has_from_and_to_args(call)
}

fn is_versioned_migration_superclass(super_text: &[u8]) -> bool {
    let super_text = super_text.strip_prefix(b"::").unwrap_or(super_text);
    let Some(inner) = super_text
        .strip_prefix(b"ActiveRecord::Migration[")
        .and_then(|rest| rest.strip_suffix(b"]"))
    else {
        return false;
    };

    let mut saw_dot = false;
    let mut digits_before = 0usize;
    let mut digits_after = 0usize;

    for byte in inner {
        if byte.is_ascii_digit() {
            if saw_dot {
                digits_after += 1;
            } else {
                digits_before += 1;
            }
            continue;
        }

        if *byte == b'.' && !saw_dot {
            saw_dot = true;
            continue;
        }

        return false;
    }

    saw_dot && digits_before > 0 && digits_after > 0
}

fn condition_desc(condition: IrreversibleCondition) -> &'static str {
    match condition {
        IrreversibleCondition::NeedsBlock => "without block",
        IrreversibleCondition::NeedsThreeArgs => "without type",
        IrreversibleCondition::NeedsTypeOption => "without type",
        IrreversibleCondition::NeedsColumnOption => "without column",
        IrreversibleCondition::NeedsTwoArgsOrToTable => "without table",
        IrreversibleCondition::NeedsFromTo => "without :from and :to",
    }
}

impl Cop for ReversibleMigration {
    fn name(&self) -> &'static str {
        "Rails/ReversibleMigration"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            BLOCK_ARGUMENT_NODE,
            BLOCK_NODE,
            CALL_NODE,
            CLASS_NODE,
            DEF_NODE,
            HASH_NODE,
            KEYWORD_HASH_NODE,
            STATEMENTS_NODE,
            SYMBOL_NODE,
        ]
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
        // Only check class definitions that inherit from Migration
        let class_node = match node.as_class_node() {
            Some(c) => c,
            None => return,
        };

        let superclass = match class_node.superclass() {
            Some(s) => s,
            None => return,
        };
        let super_loc = superclass.location();
        let super_text = &source.as_bytes()[super_loc.start_offset()..super_loc.end_offset()];
        if !is_versioned_migration_superclass(super_text) {
            return;
        }

        // Find the `change` method
        let body = match class_node.body() {
            Some(b) => b,
            None => return,
        };
        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        for stmt in stmts.body().iter() {
            if let Some(def_node) = stmt.as_def_node() {
                if def_node.name().as_slice() == b"change" {
                    // Visit the change method body for irreversible calls
                    if let Some(def_body) = def_node.body() {
                        let mut finder = IrreversibleFinder {
                            offenses: Vec::new(),
                            inside_reversible: false,
                            inside_up_only: false,
                            inside_change_table: false,
                            direct_block_body_call_offsets: Vec::new(),
                        };
                        finder.visit(&def_body);

                        for (offset, msg) in finder.offenses {
                            let (line, column) = source.offset_to_line_col(offset);
                            diagnostics.push(self.diagnostic(source, line, column, msg));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ReversibleMigration, "cops/rails/reversible_migration");
}
