use crate::cop::shared::util::{RSPEC_DEFAULT_INCLUDE, is_rspec_example, is_rspec_hook};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Flags usage of bare `subject` inside examples/hooks when it should be named.
///
/// ## Corpus investigation (FP=2, FN=88):
/// - Fixed: `find_subject_in_block` now recognizes `subject!` definitions (not just
///   `subject`). This affects `named_only` style where `subject!(:foo) { ... }` should
///   be treated as a named subject definition.
/// - The remaining FNs (without corpus data to confirm) may be from edge cases in
///   how `subject` references are found in deeply nested AST structures, or from
///   config resolution differences.
///
/// ## Corpus investigation (2026-03-14)
///
/// FP=2 fixed. Root cause: `subject(&b)` passes a block via `call.block()` as a
/// `BlockArgumentNode`, not via `call.arguments()`. So `arguments().is_none()` is
/// true for `subject(&b)`, causing it to look like a bare `subject` reference.
/// RuboCop's `(send nil? :subject)` pattern does NOT match `subject(&b)` because
/// in RuboCop AST it has a `(block_pass ...)` child. Fix: added guard
/// `node.block().map_or(true, |b| b.as_block_argument_node().is_none())`.
///
/// ## Corpus investigation (FN=90, 2026-03-15)
///
/// Two code bugs found and fixed:
///
/// 1. **`def subject.method_name` pattern**: `visit_def_node` was returning early
///    without visiting the DefNode's receiver. In `def subject.foo(...)`, the
///    `subject` is a CallNode that is the receiver of the DefNode. RuboCop's
///    `def_node_search :subject_usage, '$(send nil? :subject)'` finds it because
///    it searches all descendants. Fix: visit DefNode's receiver.
///
/// 2. **`it` blocks inside helper method definitions**: Ruby metaprogramming patterns
///    like `def self.it_should_have(key) ... it "..." do subject.send(key) end end`
///    define example blocks inside method bodies. Since `visit_def_node` returned
///    early, these `it` blocks (and their `subject` references) were never visited.
///    Fix: visit DefNode's body too. The `in_example_or_hook` guard ensures bare
///    `subject` calls directly in method bodies (not inside examples) are still
///    correctly ignored.
///
/// ## Corpus investigation (FN=41, 2026-03-18)
///
/// All 41 FNs were `subject` references inside `shared_context` blocks.
/// Root cause: `is_shared_group_call` incorrectly included `shared_context`
/// alongside `shared_examples`/`shared_examples_for`. RuboCop's
/// `shared_example?` matcher uses `#SharedGroups.examples` (not `.all`),
/// which only matches `shared_examples` and `shared_examples_for` — NOT
/// `shared_context`. So `IgnoreSharedExamples: true` should not suppress
/// offenses inside `shared_context` blocks. Fix: removed `shared_context`
/// from `is_shared_group_call`.
///
/// ## Corpus verification (2026-03-18)
///
/// Verified all 41 FNs are exclusively `subject` references in
/// `shared_context` blocks across 9 repos (opf/openproject 13, shoes/shoes4 10,
/// solidus 7, puppetlabs/r10k 4, decidim 2, apartment 2, synapse 1,
/// spidr 1, forem 1). Patterns include `subject.method_call` as receiver
/// (e.g., `expect(subject.status)`), `subject` in conditionals
/// (e.g., `if subject.is_a?(Foo)`), and `subject` in before/around hooks
/// within shared_context. All patterns confirmed handled by the
/// `is_shared_group_call` fix. Added 3 inline tests covering these patterns.
///
/// ## Corpus investigation (FP=2, named_only, 2026-04-08)
///
/// Both FPs were in `ruby-concurrency/concurrent-ruby` with
/// `subject(:actor, &subject_definition)` inside an example group. In RuboCop's
/// parser AST, that shape is a `send` with a `block_pass`, not a `subject { ... }`
/// block node, so `NamedSubject` does not treat it as a subject definition for
/// `named_only`. Prism stores the `&subject_definition` as `call.block()`, which
/// made `find_subject_in_block` incorrectly classify it as a named subject
/// definition. Fix: only treat `subject`/`subject!` calls with an actual
/// `BlockNode` body as subject definitions.
pub struct NamedSubject;

/// EnforcedStyle:
/// - `always` (default): flag every bare `subject` reference in examples/hooks
/// - `named_only`: only flag when the nearest enclosing subject definition is named
impl Cop for NamedSubject {
    fn name(&self) -> &'static str {
        "RSpec/NamedSubject"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_DEFAULT_INCLUDE
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "always");
        let named_only = style == "named_only";
        // Config: IgnoreSharedExamples — skip shared example groups
        let ignore_shared = config.get_bool("IgnoreSharedExamples", true);

        // Walk the AST to find bare `subject` references
        let mut finder = BareSubjectFinder {
            source,
            cop: self,
            ignore_shared,
            named_only,
            in_shared: false,
            in_example_or_hook: false,
            // Stack tracking whether the nearest enclosing scope has a named
            // subject definition. `None` = no subject defined in this scope,
            // `Some(true)` = named, `Some(false)` = unnamed.
            subject_named_stack: Vec::new(),
            diags: Vec::new(),
        };
        finder.visit(&parse_result.node());
        diagnostics.extend(finder.diags);
    }
}

/// Check whether any direct child statement of a block body is a `subject`
/// definition (a call to `subject` with a block). Returns:
/// - `Some(true)` if the subject has arguments (named: `subject(:foo) { ... }`)
/// - `Some(false)` if the subject has no arguments (unnamed: `subject { ... }`)
/// - `None` if no subject definition is found
fn find_subject_in_block(block_node: &ruby_prism::BlockNode<'_>) -> Option<bool> {
    let body = block_node.body()?;
    let stmts = body.as_statements_node()?;
    for stmt in stmts.body().iter() {
        if let Some(call) = stmt.as_call_node() {
            let name = call.name().as_slice();
            if (name == b"subject" || name == b"subject!")
                && call.receiver().is_none()
                // `subject(:name, &builder)` stores the block pass in
                // `call.block()` as a BlockArgumentNode. RuboCop only treats
                // real `subject { ... }` / `subject! { ... }` block nodes as
                // subject definitions for `named_only`.
                && call.block().is_some_and(|block| block.as_block_node().is_some())
            {
                return Some(call.arguments().is_some());
            }
        }
    }
    None
}

/// Check if a call node is a shared example group definition, including
/// both receiverless (`shared_examples`) and qualified (`RSpec.shared_examples`).
fn is_shared_group_call(node: &ruby_prism::CallNode<'_>) -> bool {
    let name = node.name().as_slice();
    // Only shared_examples and shared_examples_for are "shared example groups"
    // for IgnoreSharedExamples purposes. shared_context is NOT — RuboCop's
    // `shared_example?` matcher uses `#SharedGroups.examples` (not `.all`),
    // which excludes shared_context.
    let is_shared_name = name == b"shared_examples" || name == b"shared_examples_for";
    if !is_shared_name {
        return false;
    }
    // Receiverless or RSpec.shared_*
    if node.receiver().is_none() {
        return true;
    }
    node.receiver().is_some_and(|r| {
        crate::cop::shared::constant_predicates::constant_short_name(&r)
            .is_some_and(|n| n == b"RSpec" || n.starts_with(b"RSpec::"))
    })
}

struct BareSubjectFinder<'a> {
    source: &'a SourceFile,
    cop: &'a NamedSubject,
    ignore_shared: bool,
    named_only: bool,
    in_shared: bool,
    in_example_or_hook: bool,
    /// Stack of subject-named states for enclosing blocks.
    /// Each entry is `Some(true)` (named subject), `Some(false)` (unnamed), or
    /// `None` (no subject definition in that scope).
    subject_named_stack: Vec<Option<bool>>,
    diags: Vec<Diagnostic>,
}

impl BareSubjectFinder<'_> {
    /// Check if the nearest enclosing subject definition is named.
    /// Walks the stack from top to bottom, returning `true` if the nearest
    /// scope with a subject definition has a named subject.
    fn nearest_subject_is_named(&self) -> bool {
        #[allow(clippy::never_loop)] // intentional: find-first via early return
        for named in self.subject_named_stack.iter().rev().flatten() {
            return *named;
        }
        false
    }

    fn should_flag(&self) -> bool {
        if self.in_shared || !self.in_example_or_hook {
            return false;
        }
        if self.named_only {
            // Only flag if nearest enclosing subject definition is named
            self.nearest_subject_is_named()
        } else {
            // `always` style: always flag bare subject
            true
        }
    }
}

impl<'pr> Visit<'pr> for BareSubjectFinder<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = node.name().as_slice();

        // Track if we're inside a shared example group
        if is_shared_group_call(node) && self.ignore_shared {
            let was = self.in_shared;
            self.in_shared = true;
            ruby_prism::visit_call_node(self, node);
            self.in_shared = was;
            return;
        }

        // Track if we're inside an example or hook block (it, specify, before, after, etc.)
        let is_example = node.receiver().is_none()
            && node.block().is_some()
            && (is_rspec_example(name) || is_rspec_hook(name));

        if is_example {
            let was = self.in_example_or_hook;
            self.in_example_or_hook = true;

            // Also push subject state for blocks
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    let subject_state = find_subject_in_block(&block_node);
                    self.subject_named_stack.push(subject_state);
                    ruby_prism::visit_call_node(self, node);
                    self.subject_named_stack.pop();
                    self.in_example_or_hook = was;
                    return;
                }
            }

            ruby_prism::visit_call_node(self, node);
            self.in_example_or_hook = was;
            return;
        }

        // Check for `subject` reference (no receiver, no arguments).
        // RuboCop's `subject_usage` matches `(send nil? :subject)` which finds
        // ANY bare `subject` call inside example/hook blocks, including
        // `subject { ... }` (the send node inside the block node). So we don't
        // check for `node.block().is_none()` — a `subject { ... }` inside a
        // hook is still a reference, not a definition.
        //
        // However, `subject(&b)` passes a block argument (BlockArgumentNode) via
        // call.block(). In Prism, BlockArgumentNode is stored in `call.block()`,
        // not in `call.arguments()`, so `arguments().is_none()` is true for
        // `subject(&b)`. RuboCop's `(send nil? :subject)` pattern does NOT match
        // `subject(&b)` because in RuboCop AST it appears as
        // `(send nil :subject (block_pass (lvar :b)))` with a positional child.
        // We guard against this by skipping when block() is a BlockArgumentNode.
        if name == b"subject"
            && node.receiver().is_none()
            && node.arguments().is_none()
            && node
                .block()
                .is_none_or(|b| b.as_block_argument_node().is_none())
            && self.should_flag()
        {
            let loc = node.location();
            let (line, column) = self.source.offset_to_line_col(loc.start_offset());
            self.diags.push(self.cop.diagnostic(
                self.source,
                line,
                column,
                "Name your test subject if you need to reference it explicitly.".to_string(),
            ));
        }

        // When entering any block, check if this scope defines `subject` and
        // push that info onto the stack.
        if let Some(block) = node.block() {
            if let Some(block_node) = block.as_block_node() {
                let subject_state = find_subject_in_block(&block_node);
                self.subject_named_stack.push(subject_state);

                ruby_prism::visit_call_node(self, node);

                self.subject_named_stack.pop();
                return;
            }
        }

        // Continue visiting children
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        // Visit the receiver of `def subject.method_name(...)` — the `subject`
        // there IS a test subject reference (RuboCop's `(send nil? :subject)`
        // matches it). But do NOT descend into the method body for bare
        // `subject` calls (those are regular method calls, not test subject
        // references).
        //
        // However, we DO descend into the body because Ruby metaprogramming
        // patterns define `it`/`before`/`after` blocks inside helper methods
        // (e.g., `def self.it_should_have_view(key, val) ... it "..." do
        // subject.send(key) end ... end`). The `in_example_or_hook` flag
        // ensures only `subject` inside proper example/hook blocks is flagged.
        if let Some(receiver) = node.receiver() {
            self.visit(&receiver);
        }
        if let Some(body) = node.body() {
            self.visit(&body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(NamedSubject, "cops/rspec/named_subject");

    #[test]
    fn named_only_style_skips_without_named_subject() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("named_only".into()),
            )]),
            ..CopConfig::default()
        };
        // File with bare `subject` but no named subject declaration
        let source =
            b"describe Foo do\n  it 'works' do\n    expect(subject).to be_valid\n  end\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&NamedSubject, source, config);
        assert!(
            diags.is_empty(),
            "named_only should not flag without named subject"
        );
    }

    #[test]
    fn ignore_shared_examples_skips_shared_groups() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("IgnoreSharedExamples".into(), serde_yml::Value::Bool(true))]),
            ..CopConfig::default()
        };
        let source = b"shared_examples 'foo' do\n  it { subject }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&NamedSubject, source, config);
        assert!(
            diags.is_empty(),
            "IgnoreSharedExamples should skip shared groups"
        );
    }

    #[test]
    fn ignore_shared_examples_false_flags_shared_groups() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "IgnoreSharedExamples".into(),
                serde_yml::Value::Bool(false),
            )]),
            ..CopConfig::default()
        };
        let source = b"shared_examples 'foo' do\n  it { subject }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&NamedSubject, source, config);
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn shared_context_not_suppressed_by_ignore_shared_examples() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        // IgnoreSharedExamples only applies to shared_examples/shared_examples_for,
        // NOT shared_context. RuboCop's shared_example? matcher uses
        // SharedGroups.examples (not .all), excluding shared_context.
        let config = CopConfig {
            options: HashMap::from([("IgnoreSharedExamples".into(), serde_yml::Value::Bool(true))]),
            ..CopConfig::default()
        };
        let source = b"shared_context 'setup' do\n  before { subject.activate }\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&NamedSubject, source, config);
        assert_eq!(
            diags.len(),
            1,
            "shared_context should NOT be suppressed by IgnoreSharedExamples"
        );
    }

    #[test]
    fn named_only_nearest_unnamed_subject_not_flagged() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("named_only".into()),
            )]),
            ..CopConfig::default()
        };
        // File has a named subject in one context but the nearest subject
        // to the usage is unnamed — should NOT be flagged.
        let source = b"describe Foo do\n\
            \x20 describe '#bar' do\n\
            \x20   subject { described_class.new }\n\
            \x20   it 'uses subject' do\n\
            \x20     expect(subject).to be_valid\n\
            \x20   end\n\
            \x20 end\n\
            \x20 describe '#baz' do\n\
            \x20   subject(:foo) { described_class.new }\n\
            \x20   it 'uses named' do\n\
            \x20     expect(foo).to be_valid\n\
            \x20   end\n\
            \x20 end\n\
            end\n";
        let diags = crate::testutil::run_cop_full_with_config(&NamedSubject, source, config);
        assert!(
            diags.is_empty(),
            "named_only should not flag when nearest subject is unnamed, got: {diags:?}"
        );
    }

    #[test]
    fn named_only_nearest_named_subject_is_flagged() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("named_only".into()),
            )]),
            ..CopConfig::default()
        };
        // Nearest subject definition is named — SHOULD be flagged
        let source = b"describe Foo do\n\
            \x20 subject(:foo) { described_class.new }\n\
            \x20 it 'uses subject' do\n\
            \x20   expect(subject).to be_valid\n\
            \x20 end\n\
            end\n";
        let diags = crate::testutil::run_cop_full_with_config(&NamedSubject, source, config);
        assert_eq!(
            diags.len(),
            1,
            "named_only should flag when nearest subject is named"
        );
    }

    #[test]
    fn subject_inside_block_within_example() {
        // subject inside `expect { subject }` should be flagged
        let source = b"RSpec.describe User do\n  subject { described_class.new }\n\n  it \"works\" do\n    expect { subject }.not_to raise_error\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&NamedSubject, source);
        assert_eq!(
            diags.len(),
            1,
            "subject inside block within example should be flagged, got: {diags:?}"
        );
    }

    #[test]
    fn subject_inside_let_block_not_flagged() {
        // subject inside `let` is not inside an example/hook — should NOT be flagged
        let source = b"RSpec.describe User do\n  subject { described_class.new }\n  let(:result) { subject.process }\n\n  it 'works' do\n    expect(result).to be_truthy\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&NamedSubject, source);
        assert!(
            diags.is_empty(),
            "subject inside let block should not be flagged, got: {diags:?}"
        );
    }

    #[test]
    fn subject_with_empty_parens_flagged() {
        // subject() with empty parens should be flagged same as bare subject
        let source = b"RSpec.describe User do\n  subject { described_class.new }\n\n  it \"works\" do\n    expect(subject()).to be_valid\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&NamedSubject, source);
        assert_eq!(
            diags.len(),
            1,
            "subject() with empty parens should be flagged, got: {diags:?}"
        );
    }

    #[test]
    fn subject_bang_definition_recognized_in_named_only() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("named_only".into()),
            )]),
            ..CopConfig::default()
        };
        // subject! definition is named — should flag bare `subject` usage
        let source = b"RSpec.describe User do\n  subject!(:user) { described_class.new }\n\n  it \"is a User\" do\n    expect(subject).to be_a(User)\n  end\nend\n";
        let diags = crate::testutil::run_cop_full_with_config(&NamedSubject, source, config);
        assert_eq!(
            diags.len(),
            1,
            "subject! named definition should be recognized in named_only mode"
        );
    }

    #[test]
    fn named_only_ignores_subject_definition_with_block_pass() {
        use crate::cop::CopConfig;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("named_only".into()),
            )]),
            ..CopConfig::default()
        };
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &NamedSubject,
            include_bytes!(
                "../../../tests/fixtures/cops/rspec/named_subject/no_offense.named_only.rb"
            ),
            config,
        );
    }

    #[test]
    fn subject_in_shared_context_around_hook() {
        // shared_context is NOT suppressed by IgnoreSharedExamples.
        // subject inside an around hook within shared_context should be flagged.
        let source = b"shared_context 'Tarball' do\n  around(:each) do |example|\n    if subject.is_a?(Foo)\n      subject.settings[:cache_root] = cache_root\n    end\n    example.run\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&NamedSubject, source);
        assert_eq!(
            diags.len(),
            2,
            "subject inside shared_context around hook should be flagged, got: {diags:?}"
        );
    }

    #[test]
    fn subject_method_call_in_shared_context_it_block() {
        // subject.status inside an it block within shared_context should be flagged
        let source = b"shared_context 'test' do\n  it 'succeeds' do\n    expect(subject.status).to eq(200)\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&NamedSubject, source);
        assert_eq!(
            diags.len(),
            1,
            "subject.status inside shared_context it block should be flagged, got: {diags:?}"
        );
    }

    #[test]
    fn subject_in_shared_context_before_hook() {
        // subject inside a before hook within shared_context should be flagged
        let source = b"shared_context 'setup' do\n  before :each do\n    subject.element_width = 43\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&NamedSubject, source);
        assert_eq!(
            diags.len(),
            1,
            "subject inside shared_context before hook should be flagged, got: {diags:?}"
        );
    }
}
