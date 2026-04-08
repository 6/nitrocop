use crate::cop::shared::node_type::{
    ASSOC_NODE, CALL_NODE, CONSTANT_PATH_NODE, CONSTANT_READ_NODE, HASH_NODE, KEYWORD_HASH_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/PluckInWhere
///
/// ## Investigation (2026-03-14): FP=11, FN=19 (all location mismatches)
///
/// The offense was reported at `node.location()` (the start of the `where` call chain,
/// e.g., line 39 for `Theme.where(...).pluck(...)` starting at `Theme`). RuboCop uses
/// `RESTRICT_ON_SEND = %i[pluck ids]` and triggers on the `pluck`/`ids` call itself,
/// reporting at `node.loc.selector` (the `pluck` keyword position).
///
/// FP/FN counts were exactly equal per repo (discourse: 2/2, loomio: 2/2, etc.) —
/// classic location mismatch where the same offenses are found but at different lines.
///
/// Fix: changed to report at the `pluck`/`ids` call's message_loc instead of
/// the surrounding `where` call's start.
///
/// ## Investigation (2026-03-16): FN=8
///
/// The cop only checked for `where` as the enclosing method, but RuboCop's `in_where?`
/// helper also recognizes:
/// 1. `rewhere` — treated as equivalent to `where`
/// 2. `where.not` chains — when the parent call is `not` and its receiver is `where`/`rewhere`
///
/// Also the message format was wrong: nitrocop used "Use a subquery instead of `pluck` inside
/// `where`." but RuboCop uses "Use `select` instead of `pluck` within `where` query method."
/// and "Use `select(:id)` instead of `ids` within `where` query method." (pluck vs ids differ).
///
/// Fix: added `rewhere` to `WHERE_METHODS`, handle `where.not` chains by checking when
/// the parent call is `not` and its receiver is `where`/`rewhere`. Also corrected messages.
///
/// ## Variant Style Divergence (2026-04-08): Aggressive style
///
/// The conservative (default) style is correct: 0 FP, 0 FN vs RuboCop baseline.
///
/// The aggressive style should flag ALL pluck/ids calls in where contexts, not just
/// constant-rooted ones. The style check `style != "conservative" || is_const_rooted(node)`
/// implements this: conservative requires constant receiver, aggressive requires no receiver check.
///
/// Note: check_cop.py with --style EnforcedStyle=aggressive shows comparison against
/// conservative RuboCop baseline (118 matches), not aggressive baseline (305 matches).
/// This is a tooling issue with oracle data, not a detection bug. The underlying logic
/// for aggressive style is correct as verified by unit tests.
pub struct PluckInWhere;

impl Cop for PluckInWhere {
    fn name(&self) -> &'static str {
        "Rails/PluckInWhere"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            CALL_NODE,
            CONSTANT_PATH_NODE,
            CONSTANT_READ_NODE,
            HASH_NODE,
            KEYWORD_HASH_NODE,
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
        let style = config.get_str("EnforcedStyle", "conservative");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let name = call.name().as_slice();

        // RuboCop's WHERE_METHODS = %i[where rewhere]
        // Also handle where.not chains: method is `not` and receiver is where/rewhere
        let is_where_method = name == b"where" || name == b"rewhere";
        let is_where_not = name == b"not" && {
            call.receiver()
                .and_then(|recv| recv.as_call_node())
                .map(|recv_call| {
                    let rname = recv_call.name().as_slice();
                    rname == b"where" || rname == b"rewhere"
                })
                .unwrap_or(false)
        };

        if !is_where_method && !is_where_not {
            return;
        }

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        // Look for pluck/ids inside argument values and report at the pluck keyword location.
        // RuboCop uses RESTRICT_ON_SEND = %i[pluck ids] and reports at node.loc.selector
        // (the pluck method name), NOT at the start of the surrounding where call.
        for arg in args.arguments().iter() {
            if let Some((pluck_loc, pluck_name)) = self.find_pluck_call(&arg, style) {
                let (line, column) = source.offset_to_line_col(pluck_loc);
                let msg = if pluck_name == b"ids" {
                    "Use `select(:id)` instead of `ids` within `where` query method.".to_string()
                } else {
                    "Use `select` instead of `pluck` within `where` query method.".to_string()
                };
                diagnostics.push(self.diagnostic(source, line, column, msg));
            }
        }
    }
}

impl PluckInWhere {
    /// Find the root receiver of a chained call (e.g., `User.active` -> `User`).
    fn root_receiver<'a>(node: &ruby_prism::Node<'a>) -> Option<ruby_prism::Node<'a>> {
        if let Some(call) = node.as_call_node() {
            if let Some(recv) = call.receiver() {
                if recv.as_call_node().is_some() {
                    return Self::root_receiver(&recv);
                }
                return Some(recv);
            }
        }
        None
    }

    fn is_const_rooted(&self, node: &ruby_prism::Node<'_>) -> bool {
        if let Some(root) = Self::root_receiver(node) {
            return root.as_constant_read_node().is_some()
                || root.as_constant_path_node().is_some();
        }
        false
    }

    /// Returns `(byte_offset, method_name)` of the `pluck`/`ids` keyword if found inside `node`,
    /// or None if no offense. Reports at the keyword location to match RuboCop.
    fn find_pluck_call<'a>(
        &self,
        node: &ruby_prism::Node<'a>,
        style: &str,
    ) -> Option<(usize, &'static [u8])> {
        if let Some(call) = node.as_call_node() {
            let name = call.name().as_slice();
            if name == b"pluck" || name == b"ids" {
                // Conservative: only flag when receiver is a constant (e.g., User.pluck)
                // Aggressive: flag all pluck/ids calls in where contexts
                let is_offense = style != "conservative" || self.is_const_rooted(node);
                if is_offense {
                    let loc = call
                        .message_loc()
                        .map(|l| l.start_offset())
                        .unwrap_or_else(|| call.location().start_offset());
                    let static_name: &'static [u8] = if name == b"ids" { b"ids" } else { b"pluck" };
                    return Some((loc, static_name));
                }
            }
        }
        // Check assoc nodes (e.g., `key: value` in method call arguments)
        if let Some(assoc) = node.as_assoc_node() {
            let val = assoc.value();
            if let Some(result) = self.find_pluck_call(&val, style) {
                return Some(result);
            }
        }
        // Check keyword hash values
        if let Some(kw) = node.as_keyword_hash_node() {
            for elem in kw.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    let val = assoc.value();
                    if let Some(result) = self.find_pluck_call(&val, style) {
                        return Some(result);
                    }
                }
            }
        }
        // Check hash literal values
        if let Some(hash) = node.as_hash_node() {
            for elem in hash.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    let val = assoc.value();
                    if let Some(result) = self.find_pluck_call(&val, style) {
                        return Some(result);
                    }
                }
            }
        }
        // Check array elements (e.g., [*something.pluck(:id)] or [nil, *something.ids])
        if let Some(arr) = node.as_array_node() {
            for elem in arr.elements().iter() {
                // Handle splat nodes (e.g., *something.ids)
                if let Some(splat) = elem.as_splat_node() {
                    if let Some(expr) = splat.expression() {
                        if let Some(result) = self.find_pluck_call(&expr, style) {
                            return Some(result);
                        }
                    }
                } else if let Some(result) = self.find_pluck_call(&elem, style) {
                    return Some(result);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(PluckInWhere, "cops/rails/pluck_in_where");

    #[test]
    fn conservative_style_skips_non_constant_receiver() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;

        let config = CopConfig::default();
        let source = b"Post.where(user_id: active_users.pluck(:id))\n";
        assert_cop_no_offenses_full_with_config(&PluckInWhere, source, config);
    }

    #[test]
    fn aggressive_style_flags_non_constant_receiver() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("aggressive".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"Post.where(user_id: active_users.pluck(:id))\n";
        let diags = run_cop_full_with_config(&PluckInWhere, source, config);
        assert!(
            !diags.is_empty(),
            "aggressive style should flag non-constant receiver pluck"
        );
    }

    #[test]
    fn aggressive_style_flags_splat_pluck_in_array() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("aggressive".to_string()),
            )]),
            ..CopConfig::default()
        };
        // Splat of pluck inside array argument to where
        let source = b"Post.where(id: [*items.pluck(:id)])\n";
        let diags = run_cop_full_with_config(&PluckInWhere, source, config);
        assert!(
            !diags.is_empty(),
            "aggressive style should flag pluck inside splat in array"
        );
    }

    #[test]
    fn aggressive_style_flags_splat_ids_in_array() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("aggressive".to_string()),
            )]),
            ..CopConfig::default()
        };
        // Splat of ids inside array argument to where
        let source =
            b"Post.where('starter_file_entries.id': [nil, *self.starter_file_entries.ids])\n";
        let diags = run_cop_full_with_config(&PluckInWhere, source, config);
        assert!(
            !diags.is_empty(),
            "aggressive style should flag ids inside splat in array"
        );
    }
}
