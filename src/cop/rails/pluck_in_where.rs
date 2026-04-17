use crate::cop::shared::node_type::CALL_NODE;
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
/// ## Investigation (2026-04-17): aggressive style FN=9
///
/// The previous implementation started from each `where`/`rewhere` call and searched its
/// argument subtree for a single nested `pluck`/`ids`. That matched the default corpus, but
/// diverged from RuboCop's `RESTRICT_ON_SEND = %i[pluck ids]` behavior in aggressive mode:
/// RuboCop inspects every `pluck`/`ids` send and decides whether its nearest ancestor send
/// makes it "in where?".
///
/// That difference caused aggressive-only false negatives for:
/// - multiple offenses inside the same keyword hash or array argument
/// - `pluck` inside ternaries nested under `where`
/// - `pluck` inside block bodies whose block node sits under `where`
///
/// Fix: switch to RuboCop's call-centric trigger model. Inspect each `pluck`/`ids` call,
/// walk up to its nearest ancestor call to implement `in_where?`, and keep the conservative
/// style's constant-root check on the `pluck`/`ids` call itself. This preserves non-offenses
/// like `User.pluck(:id).map(...)` inside `where`, because the nearest ancestor call is `map`,
/// not `where`.
pub struct PluckInWhere;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RelationToAncestorCall {
    Receiver,
    Other,
}

#[derive(Clone, Copy)]
struct SearchContext {
    in_where: bool,
    relation: RelationToAncestorCall,
}

impl Cop for PluckInWhere {
    fn name(&self) -> &'static str {
        "Rails/PluckInWhere"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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

        if !Self::call_starts_where_search(&call) {
            return;
        }

        let context = SearchContext {
            in_where: true,
            relation: RelationToAncestorCall::Other,
        };
        if let Some(args) = call.arguments() {
            for arg in args.arguments().iter() {
                self.find_pluck_calls(source, &arg, style, Some(context), diagnostics);
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

    fn is_where_call(name: &[u8]) -> bool {
        name == b"where" || name == b"rewhere"
    }

    fn call_starts_where_search(call: &ruby_prism::CallNode<'_>) -> bool {
        let name = call.name().as_slice();
        Self::is_where_call(name)
            || (name == b"not"
                && call
                    .receiver()
                    .and_then(|recv| recv.as_call_node())
                    .map(|recv_call| Self::is_where_call(recv_call.name().as_slice()))
                    .unwrap_or(false))
    }

    fn report_pluck_call(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        style: &str,
        call: &ruby_prism::CallNode<'_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if style == "conservative" && !self.is_const_rooted(node) {
            return;
        }

        let name = call.name().as_slice();
        let loc = call
            .message_loc()
            .map(|loc| loc.start_offset())
            .unwrap_or_else(|| call.location().start_offset());
        let (line, column) = source.offset_to_line_col(loc);
        let msg = if name == b"ids" {
            "Use `select(:id)` instead of `ids` within `where` query method.".to_string()
        } else {
            "Use `select` instead of `pluck` within `where` query method.".to_string()
        };
        diagnostics.push(self.diagnostic(source, line, column, msg));
    }

    fn find_pluck_calls(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        style: &str,
        context: Option<SearchContext>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if let Some(call) = node.as_call_node() {
            let name = call.name().as_slice();
            let is_pluck_call = name == b"pluck" || name == b"ids";
            if is_pluck_call
                && context.is_some_and(|ctx| {
                    ctx.in_where && ctx.relation == RelationToAncestorCall::Other
                })
            {
                self.report_pluck_call(source, node, style, &call, diagnostics);
            }

            let child_context_base = SearchContext {
                in_where: Self::call_starts_where_search(&call),
                relation: RelationToAncestorCall::Other,
            };

            if let Some(recv) = call.receiver() {
                let receiver_context = SearchContext {
                    relation: RelationToAncestorCall::Receiver,
                    ..child_context_base
                };
                self.find_pluck_calls(source, &recv, style, Some(receiver_context), diagnostics);
            }

            if let Some(args) = call.arguments() {
                for arg in args.arguments().iter() {
                    self.find_pluck_calls(
                        source,
                        &arg,
                        style,
                        Some(child_context_base),
                        diagnostics,
                    );
                }
            }

            if let Some(block) = call.block() {
                self.find_pluck_calls(source, &block, style, context, diagnostics);
            }
            return;
        }

        if let Some(array) = node.as_array_node() {
            for element in array.elements().iter() {
                self.find_pluck_calls(source, &element, style, context, diagnostics);
            }
            return;
        }

        if let Some(kw_hash) = node.as_keyword_hash_node() {
            for element in kw_hash.elements().iter() {
                if let Some(assoc) = element.as_assoc_node() {
                    self.find_pluck_calls(source, &assoc.value(), style, context, diagnostics);
                }
            }
            return;
        }

        if let Some(hash) = node.as_hash_node() {
            for element in hash.elements().iter() {
                if let Some(assoc) = element.as_assoc_node() {
                    self.find_pluck_calls(source, &assoc.value(), style, context, diagnostics);
                }
            }
            return;
        }

        if let Some(block) = node.as_block_node() {
            if let Some(body) = block.body() {
                self.find_pluck_calls(source, &body, style, context, diagnostics);
            }
            return;
        }

        if let Some(stmts) = node.as_statements_node() {
            for child in stmts.body().iter() {
                self.find_pluck_calls(source, &child, style, context, diagnostics);
            }
            return;
        }

        if let Some(if_node) = node.as_if_node() {
            if let Some(stmts) = if_node.statements() {
                self.find_pluck_calls(source, &stmts.as_node(), style, context, diagnostics);
            }
            if let Some(subsequent) = if_node.subsequent() {
                self.find_pluck_calls(source, &subsequent, style, context, diagnostics);
            }
            return;
        }

        if let Some(unless_node) = node.as_unless_node() {
            if let Some(stmts) = unless_node.statements() {
                self.find_pluck_calls(source, &stmts.as_node(), style, context, diagnostics);
            }
            if let Some(else_clause) = unless_node.else_clause() {
                self.find_pluck_calls(source, &else_clause.as_node(), style, context, diagnostics);
            }
            return;
        }

        if let Some(else_node) = node.as_else_node() {
            if let Some(stmts) = else_node.statements() {
                self.find_pluck_calls(source, &stmts.as_node(), style, context, diagnostics);
            }
            return;
        }

        if let Some(begin_node) = node.as_begin_node() {
            if let Some(stmts) = begin_node.statements() {
                self.find_pluck_calls(source, &stmts.as_node(), style, context, diagnostics);
            }
            return;
        }

        if let Some(paren) = node.as_parentheses_node() {
            if let Some(body) = paren.body() {
                self.find_pluck_calls(source, &body, style, context, diagnostics);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(PluckInWhere, "cops/rails/pluck_in_where");
    crate::cop_variant_fixture_tests!(PluckInWhere, "cops/rails/pluck_in_where", aggressive);

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
}
