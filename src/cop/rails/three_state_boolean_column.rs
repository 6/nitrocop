use crate::cop::shared::method_dispatch_predicates;
use crate::cop::shared::util::keyword_arg_value;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Mirrors RuboCop's node-shape matching for boolean-column migration helpers.
/// This keeps `TABLE`/local-variable table expressions eligible for
/// `change_column_null` matching, rejects receiver-proxied `add_column` calls
/// such as `table.add_column`, and stops treating four-argument
/// `change_column_null(..., false, ...)` calls as the three-argument form.
pub struct ThreeStateBooleanColumn;

impl Cop for ThreeStateBooleanColumn {
    fn name(&self) -> &'static str {
        "Rails/ThreeStateBooleanColumn"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/**/*.rb"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = ThreeStateBooleanVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            def_body: None,
            current_table_expr: None,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// Visitor that tracks method def context and create_table/change_table block context.
struct ThreeStateBooleanVisitor<'a, 'pr> {
    cop: &'a ThreeStateBooleanColumn,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    /// The body node of the enclosing def, if any (for searching change_column_null).
    def_body: Option<ruby_prism::Node<'pr>>,
    /// The current table expression from an enclosing create_table/change_table block.
    current_table_expr: Option<Vec<u8>>,
}

impl<'pr> Visit<'pr> for ThreeStateBooleanVisitor<'_, 'pr> {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let old_def_body = self.def_body.take();
        self.def_body = node.body();
        // Visit children (body of the def)
        ruby_prism::visit_def_node(self, node);
        self.def_body = old_def_body;
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let method = node.name().as_slice();

        // If this is a create_table/change_table call with a block, set table context
        // and visit the block body with that context.
        if (method == b"create_table" || method == b"change_table") && node.block().is_some() {
            let old_table_expr = self.current_table_expr.take();
            self.current_table_expr = node
                .arguments()
                .and_then(|args| args.arguments().iter().next())
                .map(|arg| extract_expression(&arg));
            // Visit children (including the block body)
            ruby_prism::visit_call_node(self, node);
            self.current_table_expr = old_table_expr;
            return;
        }

        // Determine if this is a boolean column call and extract table/column info
        let boolean_info = if method == b"add_column" {
            check_add_column(node)
        } else if method == b"column" {
            check_column_method(node, &self.current_table_expr)
        } else if method == b"boolean" {
            check_boolean_method(node, &self.current_table_expr)
        } else {
            None
        };

        if let Some(info) = boolean_info {
            // Check if required options (default: non-nil AND null: false) are present
            let has_default =
                keyword_arg_value(node, b"default").is_some_and(|v| v.as_nil_node().is_none());
            let has_null_false =
                keyword_arg_value(node, b"null").is_some_and(|v| v.as_false_node().is_some());

            if !(has_default && has_null_false) {
                // Check the vendor's skip condition:
                // If inside a def AND (table expression unknown OR change_column_null found), skip.
                let should_skip = self.def_body.is_some()
                    && (info.table_expr.is_none()
                        || self.has_change_column_null(
                            info.table_expr.as_deref(),
                            info.column_expr.as_deref(),
                        ));

                if !should_skip {
                    let loc = node.location();
                    let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                    self.diagnostics.push(self.cop.diagnostic(
                        self.source,
                        line,
                        column,
                        "Boolean columns should always have a default value and a `NOT NULL` constraint."
                            .to_string(),
                    ));
                }
            }
        }

        // Continue visiting children
        ruby_prism::visit_call_node(self, node);
    }
}

/// Information about a boolean column call.
struct BooleanColumnInfo {
    table_expr: Option<Vec<u8>>,
    column_expr: Option<Vec<u8>>,
}

impl ThreeStateBooleanVisitor<'_, '_> {
    /// Search the current def body for a `change_column_null(table, column, false)` call
    /// that matches the given table and column expressions.
    fn has_change_column_null(
        &self,
        table_expr: Option<&[u8]>,
        column_expr: Option<&[u8]>,
    ) -> bool {
        let def_body = match &self.def_body {
            Some(body) => body,
            None => return false,
        };
        let table = match table_expr {
            Some(t) => t,
            None => return false,
        };
        let column = match column_expr {
            Some(c) => c,
            None => return false,
        };
        let mut finder = ChangeColumnNullFinder {
            table_expr: table,
            column_expr: column,
            found: false,
        };
        finder.visit(def_body);
        finder.found
    }
}

/// Visitor that searches for `change_column_null(:table, :column, false)`.
struct ChangeColumnNullFinder<'a> {
    table_expr: &'a [u8],
    column_expr: &'a [u8],
    found: bool,
}

impl<'pr> Visit<'pr> for ChangeColumnNullFinder<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if self.found {
            return;
        }
        if method_dispatch_predicates::is_command(node, b"change_column_null") {
            if let Some(args) = node.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                // change_column_null :table, :column, false
                if arg_list.len() == 3 {
                    let table_matches = arg_list[0].location().as_slice() == self.table_expr;
                    let column_matches = arg_list[1].location().as_slice() == self.column_expr;
                    let is_false = arg_list[2].as_false_node().is_some();
                    if table_matches && column_matches && is_false {
                        self.found = true;
                        return;
                    }
                }
            }
        }
        ruby_prism::visit_call_node(self, node);
    }
}

/// Extract the source bytes for an arbitrary expression node.
fn extract_expression(node: &ruby_prism::Node<'_>) -> Vec<u8> {
    node.location().as_slice().to_vec()
}

/// Check `add_column :table, :col, :boolean` -- returns BooleanColumnInfo if it's a boolean column.
fn check_add_column(call: &ruby_prism::CallNode<'_>) -> Option<BooleanColumnInfo> {
    if call.receiver().is_some() {
        return None;
    }
    let args = call.arguments()?;
    let arg_list: Vec<_> = args.arguments().iter().collect();
    if !(3..=4).contains(&arg_list.len()) {
        return None;
    }
    // Third arg should be :boolean or "boolean"
    let is_boolean = arg_list[2]
        .as_symbol_node()
        .is_some_and(|s| s.unescaped() == b"boolean")
        || arg_list[2]
            .as_string_node()
            .is_some_and(|s| s.unescaped() == b"boolean");
    if !is_boolean {
        return None;
    }
    Some(BooleanColumnInfo {
        table_expr: Some(extract_expression(&arg_list[0])),
        column_expr: Some(extract_expression(&arg_list[1])),
    })
}

/// Check `t.column :col, :boolean` -- returns BooleanColumnInfo if it's a boolean column.
fn check_column_method(
    call: &ruby_prism::CallNode<'_>,
    current_table_expr: &Option<Vec<u8>>,
) -> Option<BooleanColumnInfo> {
    // Must have a receiver (the block variable `t`)
    call.receiver()?;
    let args = call.arguments()?;
    let arg_list: Vec<_> = args.arguments().iter().collect();
    if !(2..=3).contains(&arg_list.len()) {
        return None;
    }
    let is_boolean = arg_list[1]
        .as_symbol_node()
        .is_some_and(|s| s.unescaped() == b"boolean")
        || arg_list[1]
            .as_string_node()
            .is_some_and(|s| s.unescaped() == b"boolean");
    if !is_boolean {
        return None;
    }
    Some(BooleanColumnInfo {
        table_expr: current_table_expr.clone(),
        column_expr: Some(extract_expression(&arg_list[0])),
    })
}

/// Check `t.boolean :col` -- returns BooleanColumnInfo if it's a boolean column call with a receiver.
fn check_boolean_method(
    call: &ruby_prism::CallNode<'_>,
    current_table_expr: &Option<Vec<u8>>,
) -> Option<BooleanColumnInfo> {
    // Must have a receiver (the block variable `t`)
    call.receiver()?;
    let args = call.arguments()?;
    let arg_list: Vec<_> = args.arguments().iter().collect();
    if !(1..=2).contains(&arg_list.len()) {
        return None;
    }
    Some(BooleanColumnInfo {
        table_expr: current_table_expr.clone(),
        column_expr: Some(extract_expression(&arg_list[0])),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        ThreeStateBooleanColumn,
        "cops/rails/three_state_boolean_column"
    );
}
