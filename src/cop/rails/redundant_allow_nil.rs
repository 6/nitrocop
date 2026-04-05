use crate::cop::shared::node_type::{
    ASSOC_NODE, CALL_NODE, FALSE_NODE, HASH_NODE, KEYWORD_HASH_NODE, SYMBOL_NODE, TRUE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/RedundantAllowNil
///
/// ## Investigation (2026-03-15): FP=4, FN=2
///
/// **FP root cause**: Hash-rocket style `:allow_nil => true` was being matched.
/// RuboCop checks `pair.children.first.source` which returns `":allow_nil"` (with
/// leading colon) for rocket-style, and `"allow_nil"` for keyword-style. The
/// comparison `key == 'allow_nil'` fails for rocket-style, so RuboCop doesn't flag
/// those. Nitrocop was matching `sym.unescaped()` which is `"allow_nil"` regardless
/// of style, causing false positives for rocket-style syntax.
///
/// Fix: Check if the key source bytes start with `:` — skip if so (rocket style).
///
/// **FN root cause**: `allow_nil` and `allow_blank` inside a nested hash option
/// (e.g., `inclusion: {allow_nil: true, allow_blank: true}`) were not found because
/// `find_keyword_pair` only searched top-level keyword args. RuboCop's
/// `find_allow_nil_and_allow_blank` recurses into all child nodes.
///
/// Fix: Added recursive search through all hash/keyword-hash values in the args tree.
pub struct RedundantAllowNil;

struct AllowNilResult {
    offset: usize,
    is_true: bool,
}

/// Search the argument tree for allow_nil + allow_blank pair in the same hash scope.
/// Mirrors RuboCop's recursive `find_allow_nil_and_allow_blank` logic.
fn find_in_node<'a>(
    source: &SourceFile,
    node: &ruby_prism::Node<'a>,
) -> Option<(AllowNilResult, bool)> {
    // Try searching within this node's hash elements
    if let Some(r) = search_hash_elements(source, node) {
        return Some(r);
    }
    // Recurse into sub-nodes
    recurse_into_children(source, node)
}

/// Check if this node is a hash/keyword_hash and search its elements.
fn search_hash_elements<'a>(
    source: &SourceFile,
    node: &ruby_prism::Node<'a>,
) -> Option<(AllowNilResult, bool)> {
    let elements: Vec<ruby_prism::Node<'a>> = if let Some(kw) = node.as_keyword_hash_node() {
        kw.elements().iter().collect()
    } else if let Some(h) = node.as_hash_node() {
        h.elements().iter().collect()
    } else if let Some(args) = node.as_arguments_node() {
        args.arguments().iter().collect()
    } else {
        return None;
    };

    let mut allow_nil: Option<AllowNilResult> = None;
    let mut allow_blank_true: Option<bool> = None;

    for elem in &elements {
        if let Some(assoc) = elem.as_assoc_node() {
            if let Some(sym) = assoc.key().as_symbol_node() {
                // Skip hash-rocket style: key source starts with `:` (e.g., `:allow_nil`)
                // RuboCop only matches keyword-style (`allow_nil: true`) where source is `"allow_nil"`
                let key_src =
                    &source.as_bytes()[sym.location().start_offset()..sym.location().end_offset()];
                if key_src.starts_with(b":") {
                    continue;
                }
                let name = sym.unescaped();
                if name == b"allow_nil" {
                    let is_true = assoc.value().as_true_node().is_some();
                    let is_false = assoc.value().as_false_node().is_some();
                    if is_true || is_false {
                        allow_nil = Some(AllowNilResult {
                            offset: assoc.key().location().start_offset(),
                            is_true,
                        });
                    }
                } else if name == b"allow_blank" {
                    let is_true = assoc.value().as_true_node().is_some();
                    let is_false = assoc.value().as_false_node().is_some();
                    if is_true || is_false {
                        allow_blank_true = Some(is_true);
                    }
                }
            }
        }
        if allow_nil.is_some() && allow_blank_true.is_some() {
            break;
        }
    }

    if let (Some(nil_result), Some(blank_is_true)) = (allow_nil, allow_blank_true) {
        Some((nil_result, blank_is_true))
    } else {
        None
    }
}

/// Recurse into the children of a node looking for allow_nil + allow_blank.
fn recurse_into_children<'a>(
    source: &SourceFile,
    node: &ruby_prism::Node<'a>,
) -> Option<(AllowNilResult, bool)> {
    // Collect sub-nodes to recurse into based on node type
    let sub_nodes: Vec<ruby_prism::Node<'a>> = if let Some(kw) = node.as_keyword_hash_node() {
        kw.elements().iter().collect()
    } else if let Some(h) = node.as_hash_node() {
        h.elements().iter().collect()
    } else if let Some(args) = node.as_arguments_node() {
        args.arguments().iter().collect()
    } else if let Some(assoc) = node.as_assoc_node() {
        vec![assoc.value()]
    } else {
        return None;
    };

    for sub in &sub_nodes {
        if let Some(found) = find_in_node(source, sub) {
            return Some(found);
        }
    }
    None
}

impl Cop for RedundantAllowNil {
    fn name(&self) -> &'static str {
        "Rails/RedundantAllowNil"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            CALL_NODE,
            FALSE_NODE,
            HASH_NODE,
            KEYWORD_HASH_NODE,
            SYMBOL_NODE,
            TRUE_NODE,
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
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let name = call.name().as_slice();
        if name != b"validates" && name != b"validates!" {
            return;
        }
        if call.receiver().is_some() {
            return;
        }

        let found = if let Some(args) = call.arguments() {
            let args_node = args.as_node();
            find_in_node(source, &args_node)
        } else {
            None
        };

        let (nil_result, blank_is_true) = match found {
            Some(r) => r,
            None => return,
        };

        let msg = if nil_result.is_true == blank_is_true {
            "`allow_nil` is redundant when `allow_blank` has the same value."
        } else if !nil_result.is_true && blank_is_true {
            "`allow_nil: false` is redundant when `allow_blank` is true."
        } else {
            return;
        };

        let (line, column) = source.offset_to_line_col(nil_result.offset);
        diagnostics.push(self.diagnostic(source, line, column, msg.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(RedundantAllowNil, "cops/rails/redundant_allow_nil");
}
