use std::collections::HashMap;

use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Checks for duplicate Rake namespace definitions.
///
/// If namespaces are defined with the same name, Rake executes both
/// in definition order. This is usually unintentional and confusing.
///
/// RuboCop's `OnNamespace` fires on any `namespace` send, not just calls with
/// `do/end` blocks. That matters in two corpus-backed cases:
/// - `namespace :install, &builder.construct(:install)` is a `BlockArgumentNode`
///   in Prism, not a real block wrapper, so RuboCop treats it like a blockless
///   namespace and keys it from ancestor namespaces only.
/// - rubocop-rake's default Include is `Rakefile`, not `**/Rakefile`, so this
///   cop must ignore nested `subdir/Rakefile` paths unless the repo config opts in.
pub struct DuplicateNamespace;

const DUPLICATE_NAMESPACE_DEFAULT_INCLUDE: &[&str] = &["Rakefile", "**/*.rake"];

impl Cop for DuplicateNamespace {
    fn name(&self) -> &'static str {
        "Rake/DuplicateNamespace"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn default_include(&self) -> &'static [&'static str] {
        DUPLICATE_NAMESPACE_DEFAULT_INCLUDE
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
        let mut visitor = DuplicateNamespaceVisitor {
            cop: self,
            source,
            namespace_stack: Vec::new(),
            namespaces: HashMap::new(),
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct NamespaceInfo {
    line: usize,
}

struct DuplicateNamespaceVisitor<'a> {
    cop: &'a DuplicateNamespace,
    source: &'a SourceFile,
    namespace_stack: Vec<String>,
    namespaces: HashMap<String, NamespaceInfo>,
    diagnostics: Vec<Diagnostic>,
}

impl DuplicateNamespaceVisitor<'_> {
    fn full_namespace_name(&self, ns_name: &str) -> String {
        if self.namespace_stack.is_empty() {
            ns_name.to_string()
        } else {
            format!("{}:{}", self.namespace_stack.join(":"), ns_name)
        }
    }
}

impl DuplicateNamespaceVisitor<'_> {
    fn check_duplicate(&mut self, full_name: &str, node: &ruby_prism::CallNode<'_>) {
        let loc = node.message_loc().unwrap_or(node.location());
        let (line, _) = self.source.offset_to_line_col(loc.start_offset());

        if let Some(prev) = self.namespaces.get(full_name) {
            let (dup_line, dup_column) = self.source.offset_to_line_col(loc.start_offset());
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                dup_line,
                dup_column,
                format!(
                    "Namespace `{}` is defined at both {} (line {}) and {} (line {}).",
                    full_name,
                    self.source.path_str(),
                    prev.line,
                    self.source.path_str(),
                    dup_line,
                ),
            ));
        } else {
            self.namespaces
                .insert(full_name.to_string(), NamespaceInfo { line });
        }
    }
}

impl<'pr> Visit<'pr> for DuplicateNamespaceVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = node.name().as_slice();
        let has_block_body = node
            .block()
            .and_then(|block| block.as_block_node())
            .is_some();

        if name == b"namespace" && node.receiver().is_none() {
            if has_block_body {
                // Namespace call with a block: include the call's own name in the path
                if let Some(ns_name) = crate::cop::rake::extract_task_name(node) {
                    let full_name = self.full_namespace_name(&ns_name);
                    self.check_duplicate(&full_name, node);

                    self.namespace_stack.push(ns_name);
                    ruby_prism::visit_call_node(self, node);
                    self.namespace_stack.pop();
                    return;
                }
            } else {
                // Namespace call without a block: path from ancestor stack only
                // (matches RuboCop's OnNamespace which fires on any `namespace` send)
                let full_name = self.namespace_stack.join(":");
                self.check_duplicate(&full_name, node);
            }
        }

        ruby_prism::visit_call_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(DuplicateNamespace, "cops/rake/duplicate_namespace");
}
