use ruby_prism::Visit;

use crate::cop::rake::RAKE_DEFAULT_INCLUDE;
use crate::cop::shared::method_dispatch_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Checks that Rake tasks have a description defined with the `desc` method.
///
/// Tasks without descriptions don't appear in `rake -T` output and lack
/// documentation. The `:default` task is exempt.
///
/// RuboCop's corpus invocation runs with an external baseline config against
/// repo directories, which lints `*.rake` files but does not inspect
/// extensionless `Rakefile` paths. Nitrocop previously flagged those
/// `Rakefile` tasks and created corpus-only false positives, so this cop
/// skips files whose basename is exactly `Rakefile`.
///
/// RuboCop only checks task siblings where a `desc` could actually be inserted.
/// Single-statement bodies under `def`, `class`, `module`, and `if`/`unless`
/// parents stay attached to those parent nodes, so tasks there are exempt. Once
/// those bodies contain multiple statements, Parser wraps them in `begin`, and
/// RuboCop starts checking sibling `desc` calls again. Prism visits every
/// `StatementsNode` directly, so nitrocop previously over-flagged single-task
/// bodies and then, after a first narrowing attempt, under-flagged valid
/// offenses inside multi-statement bodies. We now mirror RuboCop by checking
/// top-level/block/begin bodies unconditionally and other statement bodies only
/// when they contain multiple statements.
///
/// RuboCop's `can_insert_desc_to?` only allows `:begin`, `:block`, `:kwbegin`
/// as parent types. In RuboCop AST, a single-statement rescue/ensure body has
/// parent `:resbody`/`:ensure` (not allowed), so tasks there are exempt. With
/// multiple statements, RuboCop wraps them in `:begin` (allowed), so tasks ARE
/// flagged. We mirror this by overriding `visit_rescue_node`/`visit_ensure_node`
/// to skip `check_statements` only for single-statement bodies.
pub struct Desc;

impl Cop for Desc {
    fn name(&self) -> &'static str {
        "Rake/Desc"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_include(&self) -> &'static [&'static str] {
        RAKE_DEFAULT_INCLUDE
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
        if source
            .path
            .file_name()
            .is_some_and(|name| name == "Rakefile")
        {
            return;
        }

        let mut visitor = DescVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            ancestor_kinds: Vec::new(),
            statements_parent_overrides: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct DescVisitor<'a> {
    cop: &'a Desc,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    ancestor_kinds: Vec<AncestorKind>,
    statements_parent_overrides: Vec<AncestorKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AncestorKind {
    Program,
    Block,
    Begin,
    Transparent,
    Other,
}

impl AncestorKind {
    fn allows_desc_insertion(self) -> bool {
        matches!(self, Self::Program | Self::Block | Self::Begin)
    }
}

impl DescVisitor<'_> {
    /// Check if a task call is for the `:default` task.
    fn is_default_task(call: &ruby_prism::CallNode<'_>) -> bool {
        if let Some(name) = crate::cop::rake::extract_task_name(call) {
            return name == "default";
        }
        false
    }

    /// Check if a task call has array prerequisites (e.g., `task default: [:test]`).
    /// Tasks defined only with prerequisites (hash-style) are exempt.
    fn has_only_prerequisites(call: &ruby_prism::CallNode<'_>) -> bool {
        if let Some(args) = call.arguments() {
            let arg_list: Vec<_> = args.arguments().iter().collect();
            // If the only argument is a keyword hash (e.g., `task default: [:test]`)
            if arg_list.len() == 1 {
                if let Some(hash) = arg_list[0].as_keyword_hash_node() {
                    for elem in hash.elements().iter() {
                        if let Some(assoc) = elem.as_assoc_node() {
                            // Check if the value is an array (prerequisites)
                            if assoc.value().as_array_node().is_some() {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if a given statement node is a `desc` call.
    fn is_desc_call(node: &ruby_prism::Node<'_>) -> bool {
        if let Some(call) = node.as_call_node() {
            return method_dispatch_predicates::is_command(&call, b"desc");
        }
        false
    }

    /// Check siblings of a task call in a statements list.
    /// Returns true if a `desc` call precedes the task.
    fn has_preceding_desc(stmts: &[ruby_prism::Node<'_>], task_offset: usize) -> bool {
        if task_offset == 0 {
            return false;
        }
        // Check the immediately preceding sibling
        Self::is_desc_call(&stmts[task_offset - 1])
    }

    fn check_statements(&mut self, stmts: &ruby_prism::StatementsNode<'_>) {
        let body: Vec<_> = stmts.body().iter().collect();
        for (i, stmt) in body.iter().enumerate() {
            let call = if let Some(c) = stmt.as_call_node() {
                c
            } else {
                continue;
            };

            if call.name().as_slice() != b"task" || call.receiver().is_some() {
                continue;
            }

            // Skip :default task
            if Self::is_default_task(&call) {
                continue;
            }

            // Skip tasks with only prerequisites (hash-only args with array values)
            if Self::has_only_prerequisites(&call) {
                continue;
            }

            if !Self::has_preceding_desc(&body, i) {
                let loc = call.message_loc().unwrap_or(call.location());
                let (line, column) = self.source.offset_to_line_col(loc.start_offset());
                self.diagnostics.push(self.cop.diagnostic(
                    self.source,
                    line,
                    column,
                    "Describe the task with `desc` method.".to_string(),
                ));
            }
        }
    }

    fn current_statements_parent_kind(&self) -> AncestorKind {
        self.ancestor_kinds
            .iter()
            .rev()
            .copied()
            .find(|kind| *kind != AncestorKind::Transparent)
            .unwrap_or(AncestorKind::Other)
    }
}

impl<'pr> Visit<'pr> for DescVisitor<'_> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        let kind = match node {
            ruby_prism::Node::ProgramNode { .. } => AncestorKind::Program,
            ruby_prism::Node::BlockNode { .. } => AncestorKind::Block,
            ruby_prism::Node::BeginNode { .. } => AncestorKind::Begin,
            ruby_prism::Node::StatementsNode { .. } => AncestorKind::Transparent,
            _ => AncestorKind::Other,
        };
        self.ancestor_kinds.push(kind);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestor_kinds.pop();
    }

    fn visit_statements_node(&mut self, node: &ruby_prism::StatementsNode<'pr>) {
        let parent_kind = self
            .statements_parent_overrides
            .pop()
            .unwrap_or_else(|| self.current_statements_parent_kind());
        if parent_kind.allows_desc_insertion() || node.body().len() > 1 {
            self.check_statements(node);
        }
        ruby_prism::visit_statements_node(self, node);
    }

    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        // In RuboCop AST, a single-statement rescue body has parent :resbody
        // (not in `can_insert_desc_to?`'s allowed list), so tasks are not flagged.
        // Multiple statements get wrapped in :begin (allowed), so tasks ARE flagged.
        // Mirror this: skip check_statements only for single-statement bodies.
        if let Some(stmts) = node.statements() {
            if stmts.body().len() == 1 {
                for child in stmts.body().iter() {
                    self.visit(&child);
                }
            } else {
                self.statements_parent_overrides.push(AncestorKind::Begin);
                self.visit_statements_node(&stmts);
            }
        }
        if let Some(subsequent) = node.subsequent() {
            self.visit_rescue_node(&subsequent);
        }
    }

    fn visit_ensure_node(&mut self, node: &ruby_prism::EnsureNode<'pr>) {
        // Same single-statement rule as rescue bodies.
        if let Some(stmts) = node.statements() {
            if stmts.body().len() == 1 {
                for child in stmts.body().iter() {
                    self.visit(&child);
                }
            } else {
                self.statements_parent_overrides.push(AncestorKind::Begin);
                self.visit_statements_node(&stmts);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(Desc, "cops/rake/desc");

    #[test]
    fn ignores_root_rakefile_paths() {
        let source = b"task :foo\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &Desc,
            source,
            CopConfig::default(),
            "/tmp/repo/Rakefile",
        );

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn ignores_nested_rakefile_paths() {
        let source = b"task :foo\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &Desc,
            source,
            CopConfig::default(),
            "/tmp/repo/lib/glimmer/Rakefile",
        );

        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn still_flags_rake_task_files() {
        let source = b"task :foo\n";
        let diagnostics = crate::testutil::run_cop_full_internal(
            &Desc,
            source,
            CopConfig::default(),
            "/tmp/repo/lib/tasks/build.rake",
        );

        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }
}
