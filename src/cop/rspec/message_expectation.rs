use crate::cop::shared::method_dispatch_predicates;
use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::shared::util::RSPEC_DEFAULT_INCLUDE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-14)
///
/// FP=1 (rpush/rpush): `expect(fake_http2_request).to receive(:on).with(:close), &on_close`
/// was flagged. RuboCop's NodePattern `(send ... :to #receive_message?)` only matches
/// when `.to` has exactly one argument (no block_pass). When `&proc` is passed as a
/// block argument, the Parser AST adds it as a `block_pass` child of the send node,
/// making the pattern not match. Fixed by checking if `.to` has a BlockArgumentNode.
///
/// ## Corpus investigation (2026-03-15)
///
/// FN=43: Missed `expect(...).to all(receive(...))` and similar patterns where
/// `receive` is nested inside matcher arguments (e.g., `all`, compound matchers)
/// rather than being at the root of the receiver chain. RuboCop uses
/// `def_node_search :receive_message?` which does a full subtree search, while
/// nitrocop only walked the receiver chain. Fixed by replacing
/// `call_chain_includes_receive` with `subtree_includes_receive` that recursively
/// searches receiver, arguments, and nested call nodes.
///
/// ## Corpus investigation (2026-03-24)
///
/// FP=1 (nats-io/nats-pure.rb): `expect { ... }.to receive(:stop)` was flagged.
/// RuboCop's NodePattern `$(send nil? {:expect :allow} ...)` requires a plain `send`
/// node as the receiver of `.to`. When `expect` is called with a block, Parser AST
/// represents it as a `block` node (not a `send`), so the pattern never matches.
/// In Prism, `expect { ... }` is a `CallNode` with a `block` field set. Fixed by
/// returning early when `recv_call.block().is_some()`.
pub struct MessageExpectation;

/// Default style is `allow` — flags `expect(...).to receive` in favor of `allow`.
impl Cop for MessageExpectation {
    fn name(&self) -> &'static str {
        "RSpec/MessageExpectation"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
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
        // Config: EnforcedStyle — "allow" (default) or "expect"
        let enforced_style = config.get_str("EnforcedStyle", "allow");

        // Look for: expect(foo).to receive(:bar)
        // The pattern is a call chain: expect(foo).to(receive(:bar))
        // We flag the `expect(...)` part.
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call.name().as_slice();
        if method_name != b"to" {
            return;
        }

        // If .to has a &proc block argument, skip — RuboCop's NodePattern
        // (send ... :to #receive_message?) requires exactly one argument with no
        // block_pass. A block_pass adds an extra child to the send node, preventing
        // the pattern from matching.
        if call
            .block()
            .is_some_and(|b| b.as_block_argument_node().is_some())
        {
            return;
        }

        // Check the argument is `receive` or similar
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        let first_arg = &arg_list[0];
        if !subtree_includes_receive(first_arg) {
            return;
        }

        // Check that the receiver of `.to` is `expect(...)` (not `allow(...)`)
        let receiver = match call.receiver() {
            Some(r) => r,
            None => return,
        };
        let recv_call = match receiver.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let recv_name = recv_call.name().as_slice();
        if recv_call.receiver().is_some() {
            return;
        }

        // If expect/allow is called with a block (e.g. `expect { ... }.to receive(...)`),
        // skip — RuboCop's NodePattern `$(send nil? {:expect :allow} ...)` requires a
        // plain send node as the receiver of `.to`. In Parser AST, `expect { ... }` becomes
        // a `block` node (not a `send` node), so the pattern never matches. In Prism,
        // `expect { ... }` is a CallNode with a block field, so we must guard here.
        if recv_call.block().is_some() {
            return;
        }

        if enforced_style == "expect" {
            // "expect" style: flag `allow(...).to receive(...)`, prefer `expect`
            if recv_name != b"allow" {
                return;
            }
            let loc = recv_call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `expect` for setting message expectations.".to_string(),
            ));
        } else {
            // Default "allow" style: flag `expect(...).to receive(...)`, prefer `allow`
            if recv_name != b"expect" {
                return;
            }
            let loc = recv_call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `allow` for setting message expectations.".to_string(),
            ));
        }
    }
}

/// Deep-search a node subtree for `receive(...)` (a bare `receive` call with no
/// receiver). This mirrors RuboCop's `def_node_search :receive_message?` which
/// searches the entire subtree, not just the receiver chain. This matters for
/// patterns like `expect(foo).to all(receive(:bar))` where `receive` is nested
/// inside the argument of `all`, not in the receiver chain.
fn subtree_includes_receive(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        if method_dispatch_predicates::is_command(&call, b"receive") {
            return true;
        }
        // Recurse into receiver chain
        if let Some(recv) = call.receiver() {
            if subtree_includes_receive(&recv) {
                return true;
            }
        }
        // Recurse into arguments (handles `all(receive(...))` etc.)
        if let Some(args) = call.arguments() {
            for arg in args.arguments().iter() {
                if subtree_includes_receive(&arg) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(MessageExpectation, "cops/rspec/message_expectation");

    #[test]
    fn expect_style_flags_allow_receive() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("expect".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"allow(foo).to receive(:bar)\n";
        let diags = crate::testutil::run_cop_full_with_config(&MessageExpectation, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("expect"));
    }

    #[test]
    fn expect_style_does_not_flag_expect_receive() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("expect".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"expect(foo).to receive(:bar)\n";
        let diags = crate::testutil::run_cop_full_with_config(&MessageExpectation, source, config);
        assert!(diags.is_empty());
    }
}
