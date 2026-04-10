use crate::cop::shared::node_type::{CALL_NODE, STRING_NODE, SYMBOL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

pub struct ContentTag;

impl Cop for ContentTag {
    fn name(&self) -> &'static str {
        "Rails/ContentTag"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb", "**/config/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, STRING_NODE, SYMBOL_NODE]
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
        // minimum_target_rails_version 5.1
        if !config.rails_version_at_least(5.1) {
            return;
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // RuboCop's ContentTag checks legacy `tag()` calls, NOT `content_tag()`.
        // RESTRICT_ON_SEND = [:tag]
        if call.name().as_slice() != b"tag" {
            return;
        }

        // Must be a receiverless call
        if call.receiver().is_some() {
            return;
        }

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        // RuboCop: return if node.arguments.count >= 3
        if arg_list.len() >= 3 {
            return;
        }

        let first_arg = &arg_list[0];

        // Allow variables, method calls, constants, splats
        // Only flag when first arg is a string or symbol literal with a valid tag name
        let tag_name = if let Some(s) = first_arg.as_string_node() {
            s.unescaped().to_vec()
        } else if let Some(sym) = first_arg.as_symbol_node() {
            sym.unescaped().to_vec()
        } else {
            // Not a literal string/symbol — skip (variable, send, const, splat, etc.)
            return;
        };

        // Must be a valid HTML tag name: starts with letter, only letters/digits/hyphens
        if !is_valid_tag_name(&tag_name) {
            return;
        }

        let tag_name_str = String::from_utf8_lossy(&tag_name);
        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!("Use `tag.{tag_name_str}` instead of `tag(:{tag_name_str})`."),
        ));
    }
}

/// Check if the bytes form a valid HTML tag name: ^[a-zA-Z-][a-zA-Z0-9-]*$
fn is_valid_tag_name(name: &[u8]) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name[0];
    if !first.is_ascii_alphabetic() && first != b'-' {
        return false;
    }
    name.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_rails_fixture_tests!(ContentTag, "cops/rails/content_tag", 5.1);
}
