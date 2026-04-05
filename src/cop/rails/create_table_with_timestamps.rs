use ruby_prism::Visit;

use crate::cop::shared::node_type::{
    ASSOC_NODE, BLOCK_NODE, CALL_NODE, FALSE_NODE, KEYWORD_HASH_NODE, STRING_NODE, SYMBOL_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct CreateTableWithTimestamps;

/// Walk a node tree looking for `timestamps` or `datetime :created_at/:updated_at`.
struct TimestampFinder {
    found: bool,
}

impl<'pr> Visit<'pr> for TimestampFinder {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = node.name().as_slice();
        if name == b"timestamps" {
            self.found = true;
            return;
        }
        // Check for `t.datetime :created_at` or `t.datetime :updated_at`
        if name == b"datetime" {
            if let Some(args) = node.arguments() {
                if let Some(first) = args.arguments().iter().next() {
                    if let Some(sym) = first.as_symbol_node() {
                        let val = sym.unescaped();
                        if val == b"created_at" || val == b"updated_at" {
                            self.found = true;
                            return;
                        }
                    }
                    if let Some(s) = first.as_string_node() {
                        let val = s.unescaped();
                        if val == b"created_at" || val == b"updated_at" {
                            self.found = true;
                            return;
                        }
                    }
                }
            }
        }
        if !self.found {
            ruby_prism::visit_call_node(self, node);
        }
    }
}

/// Check if `create_table` has `id: false` option
fn has_id_false(call: &ruby_prism::CallNode<'_>) -> bool {
    let args = match call.arguments() {
        Some(a) => a,
        None => return false,
    };
    for arg in args.arguments().iter() {
        let kw = match arg.as_keyword_hash_node() {
            Some(k) => k,
            None => continue,
        };
        for elem in kw.elements().iter() {
            let assoc = match elem.as_assoc_node() {
                Some(a) => a,
                None => continue,
            };
            let key = match assoc.key().as_symbol_node() {
                Some(s) => s,
                None => continue,
            };
            if key.unescaped() == b"id" && assoc.value().as_false_node().is_some() {
                return true;
            }
        }
    }
    false
}

impl Cop for CreateTableWithTimestamps {
    fn name(&self) -> &'static str {
        "Rails/CreateTableWithTimestamps"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/migrate/**/*.rb"]
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &[
            "db/**/*_create_active_storage_tables.active_storage.rb",
            "db/**/*_create_active_storage_variant_records.active_storage.rb",
        ]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            BLOCK_NODE,
            CALL_NODE,
            FALSE_NODE,
            KEYWORD_HASH_NODE,
            STRING_NODE,
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
        // Start from CallNode `create_table`, then access its block
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.name().as_slice() != b"create_table" {
            return;
        }

        // Skip `create_table :x, id: false` — join tables don't need timestamps
        if has_id_false(&call) {
            return;
        }

        let block = match call.block() {
            Some(b) => b,
            None => return,
        };

        let block_node = match block.as_block_node() {
            Some(b) => b,
            None => return,
        };

        // Walk block body looking for timestamps call
        let body = match block_node.body() {
            Some(b) => b,
            None => {
                // Empty block -- flag it
                let loc = node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Add `t.timestamps` to `create_table` block.".to_string(),
                ));
                return;
            }
        };

        let mut finder = TimestampFinder { found: false };
        finder.visit(&body);

        if finder.found {
            return;
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Add `t.timestamps` to `create_table` block.".to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        CreateTableWithTimestamps,
        "cops/rails/create_table_with_timestamps"
    );
}
