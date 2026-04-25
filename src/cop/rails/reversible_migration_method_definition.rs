use crate::cop::shared::node_type::CLASS_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

const MSG: &str =
    "Migrations must contain either a `change` method, or both an `up` and a `down` method.";

/// FN root causes:
/// - Prism represents `def self.up` / `def self.down` as `DefNode`s with a
///   receiver, and this cop was incorrectly counting those singleton methods as
///   valid reversible migration methods.
/// - Migration classes with no valid instance `change`/`up`/`down` methods at
///   all were skipped because the old logic only reported when exactly one of
///   `up` or `down` was present.
/// - The raw text superclass check overmatched bare `ActiveRecord::Migration`
///   and undermatched `::ActiveRecord::Migration[...]`, causing a large FP wave
///   and additional misses.
/// - `default_include` was narrower than RuboCop's `db/**/*.rb`, so migrations
///   under non-standard directories like `db/migrate_save/` were never checked.
///
/// Fix: match RuboCop's versioned migration superclass shape
/// (`ActiveRecord::Migration[6.0]` or `::ActiveRecord::Migration[6.0]`), count
/// only receiver-less instance method definitions, and register an offense
/// unless the class defines `change` or both `up` and `down`.
pub struct ReversibleMigrationMethodDefinition;

#[derive(Default)]
struct MigrationMethodCollector {
    has_change: bool,
    has_up: bool,
    has_down: bool,
}

impl MigrationMethodCollector {
    fn is_reversible(&self) -> bool {
        self.has_change || (self.has_up && self.has_down)
    }
}

impl<'pr> Visit<'pr> for MigrationMethodCollector {
    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        if node.receiver().is_none() {
            match node.name().as_slice() {
                b"change" => self.has_change = true,
                b"up" => self.has_up = true,
                b"down" => self.has_down = true,
                _ => {}
            }
        }

        ruby_prism::visit_def_node(self, node);
    }
}

fn is_active_record_constant(node: ruby_prism::Node<'_>) -> bool {
    node.as_constant_read_node()
        .is_some_and(|constant| constant.name().as_slice() == b"ActiveRecord")
        || node.as_constant_path_node().is_some_and(|constant_path| {
            constant_path.parent().is_none()
                && constant_path
                    .name()
                    .is_some_and(|name| name.as_slice() == b"ActiveRecord")
        })
}

fn is_active_record_migration(receiver: ruby_prism::Node<'_>) -> bool {
    receiver
        .as_constant_path_node()
        .is_some_and(|constant_path| {
            constant_path
                .name()
                .is_some_and(|name| name.as_slice() == b"Migration")
                && constant_path
                    .parent()
                    .is_some_and(is_active_record_constant)
        })
}

fn is_migration_superclass(node: ruby_prism::Node<'_>) -> bool {
    node.as_call_node().is_some_and(|call| {
        call.name().as_slice() == b"[]"
            && call.arguments().is_some_and(|arguments| {
                let arguments = arguments.arguments();
                arguments.len() == 1
                    && arguments
                        .iter()
                        .next()
                        .is_some_and(|argument| argument.as_float_node().is_some())
            })
            && call.receiver().is_some_and(is_active_record_migration)
    })
}

impl Cop for ReversibleMigrationMethodDefinition {
    fn name(&self) -> &'static str {
        "Rails/ReversibleMigrationMethodDefinition"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/**/*.rb"]
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
        let class_node = match node.as_class_node() {
            Some(class_node) => class_node,
            None => return,
        };

        let superclass = match class_node.superclass() {
            Some(superclass) => superclass,
            None => return,
        };
        if !is_migration_superclass(superclass) {
            return;
        }

        let mut methods = MigrationMethodCollector::default();
        if let Some(body) = class_node.body() {
            methods.visit(&body);
        }

        if methods.is_reversible() {
            return;
        }

        let location = class_node.location();
        let (line, column) = source.offset_to_line_col(location.start_offset());
        diagnostics.push(self.diagnostic(source, line, column, MSG.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        ReversibleMigrationMethodDefinition,
        "cops/rails/reversible_migration_method_definition"
    );
}
