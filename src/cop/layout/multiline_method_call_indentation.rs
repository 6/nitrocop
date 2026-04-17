use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::util::{assignment_context_base_col, indentation_of};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-09)
///
/// Corpus oracle reported FP=2,772, FN=12,368 (29.2% match rate).
///
/// ### Root causes identified:
///
/// 1. **Trailing dot style completely unhandled** — When the dot is at the end
///    of a line (`a.\n  b`), the selector on the next line was never checked.
///    RuboCop's `right_hand_side` returns either the dot+selector (for leading
///    dot) or just the selector (for trailing dot), and `begins_its_line?`
///    determines if the RHS starts its line. This was the single biggest source
///    of FNs.
///
/// 2. **Semantic alignment base (`get_dot_right_above`)** — For the "aligned"
///    style, RuboCop first checks if there's a dot on the line directly above at
///    the same column (walking up through ancestors). This was implemented
///    differently and incorrectly via `find_alignment_dot_col` which only walked
///    up the receiver chain, not through all ancestors.
///
/// 3. **`not_for_this_cop?` logic** — RuboCop skips chains inside grouped
///    expressions and inside parenthesized arg lists (but NOT hash pair values).
///    Our `in_paren_args` tracking was overly simplified.
///
/// 4. **Assignment RHS alignment** — For `a = b.c.\n    d`, the alignment base
///    should be `b.c.` (the chain root on the assignment RHS). RuboCop uses
///    `syntactic_alignment_base` which handles assignment context.
///
/// 5. **Message generation** — Alignment base descriptions used wrong text,
///    e.g., showing chain root `User` instead of the actual alignment node
///    `.a` when the first dot is the alignment base.
///
/// ### Fixes applied:
///
/// - Added trailing dot detection: when `call_operator_loc` is on the previous
///   line AND the selector/message_loc starts a new line, treat it as a
///   multiline call with the selector as the RHS.
/// - Rewrote alignment base calculation to match RuboCop's semantic/syntactic
///   alignment approach.
/// - Fixed hash pair value chain alignment.
/// - Fixed message generation for alignment base descriptions.
///
/// ### Known remaining gaps:
///
/// - `indented` and `indented_relative_to_receiver` styles may have edge cases
///   with keyword expressions and block chains.
/// - Some complex patterns involving operation RHS (`a + b\n    .c`) may not
///   be fully handled.
///
/// ## Corpus investigation (2026-04-01, run 23848128960, timed out)
///
/// Baseline: 32,658 matches, 3,962 FP, 7,992 FN (73.2% match rate).
///
/// Attempted fix with three major changes:
/// 1. **Aligned style fallback** — introduced `AlignedExpectation::Base` vs
///    `Fallback`. When no semantic alignment base exists (no dot above, no
///    block chain, no syntactic anchor), fall back to normal indentation
///    (`lhs_indent + width`) instead of accepting any column.
/// 2. **Receiver-chain continuation dot tightening** — added
///    `continuation_anchor_is_valid()` to only reuse an earlier continuation
///    dot when the chain's first continuation call is itself correctly anchored.
/// 3. **Assignment RHS across lines** — added `starts_rhs_after_assignment_line()`
///    to handle `resources =\n  Constant\n    .new(...)` where `=` is on a
///    previous line.
///
/// Also added ancestor tracking (`Vec<Node>`) to ChainVisitor for
/// `find_dot_right_above()` and `find_logical_operator_alignment()`.
///
/// Result: last corpus check showed +623 FP (worse), -437 FP (better),
/// -4 FN (better). Net +186 FP regression. The fallback indentation was
/// firing too aggressively — patterns like standalone method calls at column 0
/// (e.g., `.where(...)` after a long expression) were being flagged. The
/// `continuation_anchor_is_valid()` check was also too strict, rejecting
/// valid continuation dot alignments in some cases.
///
/// An earlier intermediate state showed +154 worse / -321 better (net -167,
/// close to positive), suggesting the approach can work with narrower
/// fallback scoping.
///
/// ## Corpus fix (2026-04-04)
///
/// Baseline: 32,654 matches, 3,962 FP, 7,992 FN (73.2% match rate).
///
/// Two targeted changes:
///
/// 1. **Aligned style fallback to indented behavior** — When `expected_aligned`
///    returns `None` (no semantic or syntactic alignment base), we now fall
///    back to the indented calculation (`base_indent + width`) instead of
///    skipping the check entirely. This matches RuboCop's behavior in
///    `check_regular_indentation` where `@base` is nil.
///
/// 2. **Multi-line assignment detection** — Extended `find_syntactic_alignment`
///    to detect assignment context when `=` is on the previous line (e.g.,
///    `a =\n  b\n  .c`). The existing `assignment_context_base_col` only
///    checked the same line.
///
/// Also fixed `expected_aligned_hash_pair` to return `Some(rhs_col)` instead
/// of `None` for "accept" cases, preventing the fallback from incorrectly
/// flagging hash pair values that are properly aligned.
///
/// Sample-15 corpus validation: 0 FP regression, 0 FN regression,
/// -431 FP resolved, -4,416 FN resolved.
///
/// ## Corpus fix (2026-04-08)
///
/// Baseline: 38,410 matches, 4,625 FP, 2,231 FN.
///
/// Added `find_current_node_block_continuation` — RuboCop's
/// `find_continuation_node` logic. When the current call has a block
/// (do..end or { }) and the receiver has a continuation dot (dot on a
/// line after the receiver's receiver's end), use the receiver's dot
/// column as the alignment base. This prevents flagging block-bearing
/// calls like `.map { }`, `.collect { }`, `.each do...end` at the end
/// of misaligned chains.
///
/// Sample-15 corpus validation: 0 FP regression, 0 FN regression,
/// -1,092 FP resolved, -329 FN resolved.
///
/// ## Corpus fix (2026-04-11)
///
/// Baseline: 38,410 matches, 3,184 FP, 2,231 FN.
///
/// Added text-based `get_dot_right_above` check. RuboCop walks AST
/// ancestors to find a dot on the line directly above at the same
/// column. Since nitrocop doesn't track parent pointers, we scan the
/// text of the previous line instead, but only accept the match when
/// the dot is NOT in the current call's receiver chain (to avoid
/// suppressing legitimate receiver-chain alignment checks).
///
/// Root cause: in Prism's AST, calls inside non-parenthesized arguments
/// have a receiver chain that diverges from the visual nesting. E.g.,
/// `.and` in `expect { }.to(...).and(...)` chains on `.with_payload`
/// (an inner call), not on `.to`. RuboCop's ancestor traversal finds
/// `.to`'s dot directly above; our receiver-chain-only approach missed it.
///
/// This fixes the most common FP pattern: RSpec matcher chains with
/// `.to ... .and ...` where continuation dots are properly aligned with
/// each other but flagged because the receiver chain leads to an inline
/// dot at a different column.
///
/// Sample-15 corpus validation: 0 FP regression, 0 FN regression,
/// -1,324 FP resolved, -190 FN resolved.
///
/// ## Corpus fix (2026-04-17)
///
/// Two remaining FN clusters came from Prism details that are narrower than
/// the current port treated them:
///
/// 1. `call.block()` is present for both real blocks (`{}` / `do..end`) and
///    block-pass arguments (`&:strip`). RuboCop's block-chain alignment only
///    applies to real blocks, so calls like `.collect(&:strip)\n  .compact`
///    must still align `.compact` with the earlier chain base instead of with
///    `.collect`.
/// 2. Prism exposes `opening_loc` for `[]` / `[]=` calls as well as `(` arg
///    lists. For the default `aligned` style, RuboCop's
///    `inside_arg_list_parentheses?` only skips actual `(` argument lists, so
///    chains in `[]=` RHS expressions such as
///    `headers['Cookie'] = final_cookies_hash.\n  map { ... }` must still be
///    checked. The non-default styles keep the older broad skip behavior for
///    now because narrowing them regressed their corpus baselines.
pub struct MultilineMethodCallIndentation;

impl Cop for MultilineMethodCallIndentation {
    fn name(&self) -> &'static str {
        "Layout/MultilineMethodCallIndentation"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "aligned");
        let width = config.get_usize("IndentationWidth", 2);
        let mut visitor = ChainVisitor {
            cop: self,
            source,
            style,
            width,
            diagnostics: Vec::new(),
            in_paren_args: false,
            in_hash_value: false,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

enum MsgStyle {
    Aligned,
    Indented,
    ReceiverRelative,
}

struct ChainVisitor<'a> {
    cop: &'a MultilineMethodCallIndentation,
    source: &'a SourceFile,
    style: &'a str,
    width: usize,
    diagnostics: Vec<Diagnostic>,
    in_paren_args: bool,
    /// True when visiting the value side of a hash pair (AssocNode).
    /// RuboCop checks chain indentation inside hash pair values even
    /// when they're also inside parenthesized arguments.
    in_hash_value: bool,
}

impl ChainVisitor<'_> {
    fn check_call(&mut self, call_node: &ruby_prism::CallNode<'_>) {
        // Must have a receiver (chained call)
        let receiver = match call_node.receiver() {
            Some(r) => r,
            None => return,
        };

        // Must have a call operator (the `.` part) — skip `[]` calls etc
        let dot_loc = match call_node.call_operator_loc() {
            Some(loc) => loc,
            None => return,
        };

        // Skip assignment methods like `foo.bar = x` — RuboCop's left_hand_side
        // walks up through parents and skips assignment_method? calls.
        if is_assignment_method(call_node) {
            return;
        }

        let receiver_loc = receiver.location();
        let (recv_end_line, _) = self.source.offset_to_line_col(receiver_loc.end_offset());
        let (dot_line, dot_col) = self.source.offset_to_line_col(dot_loc.start_offset());

        // Determine the RHS position — what RuboCop checks for alignment.
        // Two cases:
        // 1. Leading dot (continuation dot): `.bar` starts on a new line
        //    RHS = the dot position (dot_col)
        // 2. Trailing dot: `a.\n  bar` — dot is at end of previous line,
        //    selector is on the next line. RHS = selector position.
        let (rhs_line, rhs_col, is_trailing_dot) = if dot_line > recv_end_line
            && is_first_on_line(self.source, dot_loc.start_offset())
        {
            // Case 1: Leading dot (continuation dot)
            (dot_line, dot_col, false)
        } else if dot_line == recv_end_line && dot_line < get_selector_line(self.source, call_node)
        {
            // Case 2: Trailing dot — dot is on receiver's line, selector is on next line
            let (sel_line, sel_col) = get_selector_position(self.source, call_node);
            if !is_first_on_line_at(self.source, sel_line, sel_col) {
                return;
            }
            (sel_line, sel_col, true)
        } else {
            // Same line — not multiline
            return;
        };

        // RuboCop skips chain indentation checks inside parenthesized call
        // arguments, except when the chain is inside a hash pair value.
        if self.in_paren_args && !self.in_hash_value {
            return;
        }

        // Compute expected column and message style based on EnforcedStyle
        let (expected, msg_style) = match self.style {
            "indented" => (
                self.expected_indented(call_node, &receiver),
                MsgStyle::Indented,
            ),
            "indented_relative_to_receiver" => {
                let col = self.expected_relative_to_receiver(call_node, &receiver);
                (col, MsgStyle::ReceiverRelative)
            }
            _ => {
                // "aligned" (default)
                match self.expected_aligned(
                    call_node,
                    &receiver,
                    rhs_line,
                    rhs_col,
                    is_trailing_dot,
                ) {
                    Some(col) => (col, MsgStyle::Aligned),
                    None => {
                        // No alignment base found — fall back to indented behavior,
                        // matching RuboCop's `indentation(lhs) + correct_indentation(node)`
                        (
                            self.expected_indented(call_node, &receiver),
                            MsgStyle::Indented,
                        )
                    }
                }
            }
        };

        if rhs_col != expected {
            let msg = match msg_style {
                MsgStyle::Aligned => self.aligned_message(call_node, &receiver, is_trailing_dot),
                MsgStyle::ReceiverRelative => {
                    self.receiver_relative_message(call_node, &receiver, is_trailing_dot)
                }
                MsgStyle::Indented => self.indented_message(call_node, &receiver, rhs_col),
            };
            self.diagnostics
                .push(self.cop.diagnostic(self.source, rhs_line, rhs_col, msg));
        }
    }

    fn expected_indented(
        &self,
        call_node: &ruby_prism::CallNode<'_>,
        receiver: &ruby_prism::Node<'_>,
    ) -> usize {
        let chain_start_line = find_chain_start_line(self.source, receiver);
        let base_line = find_non_continuation_ancestor_line(self.source, chain_start_line);
        let base_line_bytes = self.source.lines().nth(base_line - 1).unwrap_or(b"");
        let base_indent = indentation_of(base_line_bytes);
        let kw_extra = keyword_extra_indent(self.source, call_node, self.width);
        base_indent + self.width + kw_extra
    }

    /// RuboCop's `receiver_alignment_base` + `extra_indentation` for
    /// `indented_relative_to_receiver` style.
    ///
    /// Returns: expected column = base_col + effective_width.
    ///
    /// The base is determined by `find_hash_method_base_col` (for hash/paren
    /// receivers) or `find_chain_root_col` (the chain root receiver).
    /// No keyword extra indent — `@base` is always set for this style.
    ///
    /// Width is adjusted for splat (`*`) and kwsplat (`**`) — RuboCop's
    /// `extra_indentation` subtracts the operator length.
    fn expected_relative_to_receiver(
        &self,
        _call_node: &ruby_prism::CallNode<'_>,
        receiver: &ruby_prism::Node<'_>,
    ) -> usize {
        let splat_adj = splat_operator_length(self.source, receiver);
        let effective_width = self.width.saturating_sub(splat_adj);

        if let Some(base_col) = find_hash_method_base_col(self.source, receiver) {
            base_col + effective_width
        } else {
            find_chain_root_col(self.source, receiver) + effective_width
        }
    }

    fn expected_aligned(
        &self,
        call_node: &ruby_prism::CallNode<'_>,
        receiver: &ruby_prism::Node<'_>,
        rhs_line: usize,
        rhs_col: usize,
        is_trailing_dot: bool,
    ) -> Option<usize> {
        if self.in_hash_value {
            return self.expected_aligned_hash_pair(
                call_node,
                receiver,
                rhs_line,
                rhs_col,
                is_trailing_dot,
            );
        }

        // Try block chain continuation — when receiver is a call with a
        // single-line block, align with the block-bearing call's dot.
        if let Some(col) = find_block_chain_alignment(self.source, call_node, rhs_line) {
            return Some(col);
        }

        // When the CURRENT node has a block (do..end or { }), check if the
        // receiver has a continuation dot. RuboCop's find_continuation_node:
        // if the receiver's dot is on a line after the receiver's receiver's
        // end, use the receiver's dot column as alignment base.
        if !is_trailing_dot {
            if let Some(col) =
                find_current_node_block_continuation(self.source, call_node, receiver)
            {
                return Some(col);
            }
        }

        // RuboCop's `get_dot_right_above`: check if any ancestor (not just
        // the receiver chain) has a dot on the line directly above at the
        // same column. This handles cases where the receiver chain goes
        // through a different AST branch (e.g., `.and` chaining on
        // `.with_payload` in RSpec matcher chains while `.to` has a dot
        // directly above). We only accept this when the dot above is NOT
        // in the receiver chain — receiver chain dots are handled by the
        // normal alignment logic.
        // Only for `aligned` style — `indented_relative_to_receiver` expects
        // indent relative to the receiver, not alignment with dots above.
        if !is_trailing_dot
            && self.style == "aligned"
            && has_dot_at_col(self.source, rhs_line.saturating_sub(1), rhs_col)
            && !is_dot_in_receiver_chain(self.source, receiver, rhs_line - 1, rhs_col)
        {
            return Some(rhs_col);
        }

        if !is_trailing_dot {
            // Try first_call_alignment_node — when there's a first inline dot
            // in the chain, align with it (semantic alignment).
            if let Some(col) = find_first_dot_alignment(self.source, call_node) {
                return Some(col);
            }
        }

        // Try syntactic alignment: assignment RHS, keyword expression, operation.
        if let Some(col) =
            find_syntactic_alignment(self.source, call_node, receiver, is_trailing_dot)
        {
            return Some(col);
        }

        if !is_trailing_dot {
            // Try previous continuation dot alignment — when there's a
            // continuation dot on a previous line in the chain, align with it.
            if let Some(col) = find_previous_continuation_dot(self.source, receiver, rhs_line) {
                return Some(col);
            }
        }

        // For trailing dot: the receiver's chain root (LHS) determines the base
        // indentation. Check if indentation is wrong.
        if is_trailing_dot {
            let lhs_line = find_chain_start_line(self.source, receiver);
            let lhs_bytes = self.source.lines().nth(lhs_line - 1).unwrap_or(b"");
            let lhs_indent = indentation_of(lhs_bytes);
            return Some(lhs_indent + self.width);
        }

        None
    }

    fn expected_aligned_hash_pair(
        &self,
        _call_node: &ruby_prism::CallNode<'_>,
        receiver: &ruby_prism::Node<'_>,
        rhs_line: usize,
        rhs_col: usize,
        is_trailing_dot: bool,
    ) -> Option<usize> {
        // Inside a hash pair value: RuboCop uses the chain root's
        // start column as the alignment base, BUT with escape hatches.

        if !is_trailing_dot {
            // `aligned_with_first_line_dot?`: if the current dot's column
            // matches an inline dot on the chain's first line, accept.
            let chain_root_line = find_chain_start_line(self.source, receiver);
            if has_matching_dot_on_line(self.source, receiver, chain_root_line, rhs_col) {
                return Some(rhs_col); // Accept — aligned with first line dot
            }

            // Block chain continuation
            if let Some(col) = find_block_chain_col(self.source, receiver, rhs_line) {
                return Some(col);
            }
        }

        Some(find_chain_root_col(self.source, receiver))
    }

    fn aligned_message(
        &self,
        call_node: &ruby_prism::CallNode<'_>,
        receiver: &ruby_prism::Node<'_>,
        is_trailing_dot: bool,
    ) -> String {
        let selector = call_node.name().as_slice();
        let selector_str = std::str::from_utf8(selector).unwrap_or("?");

        let (base_name, base_line) = if self.in_hash_value {
            // In hash pair context, show the full chain source on the first line
            find_chain_source_description(self.source, receiver)
        } else {
            find_alignment_base_description(self.source, call_node, receiver, is_trailing_dot)
        };

        if is_trailing_dot {
            format!("Align `{selector_str}` with `{base_name}` on line {base_line}.")
        } else {
            format!("Align `.{selector_str}` with `{base_name}` on line {base_line}.")
        }
    }

    fn indented_message(
        &self,
        call_node: &ruby_prism::CallNode<'_>,
        receiver: &ruby_prism::Node<'_>,
        rhs_col: usize,
    ) -> String {
        let chain_start_line = find_chain_start_line(self.source, receiver);
        let base_line = find_non_continuation_ancestor_line(self.source, chain_start_line);
        let chain_line_bytes = self.source.lines().nth(base_line - 1).unwrap_or(b"");
        let chain_indent = indentation_of(chain_line_bytes);
        let _ = call_node;
        format!(
            "Use {} (not {}) spaces for indentation of a chained method call.",
            self.width,
            rhs_col.saturating_sub(chain_indent)
        )
    }

    /// Message for `indented_relative_to_receiver` style.
    /// Format: "Indent `.method` N spaces more than `base_source` on line L."
    fn receiver_relative_message(
        &self,
        call_node: &ruby_prism::CallNode<'_>,
        receiver: &ruby_prism::Node<'_>,
        is_trailing_dot: bool,
    ) -> String {
        let selector_str = if is_trailing_dot {
            // For trailing dot, the selector (method name) is the RHS
            let name = call_node.name().as_slice();
            std::str::from_utf8(name).unwrap_or("?").to_string()
        } else if call_node.message_loc().is_some() {
            let name = call_node.name().as_slice();
            format!(".{}", std::str::from_utf8(name).unwrap_or("?"))
        } else {
            // Implicit call (proc call) — `a\n  .(args)`
            ".(".to_string()
        };

        let (base_name, base_line) = find_receiver_relative_base_description(self.source, receiver);

        format!(
            "Indent `{selector_str}` {} spaces more than `{base_name}` on line {base_line}.",
            self.width
        )
    }
}

/// Check if a call node is a setter method (e.g., `foo.bar = x`).
fn is_assignment_method(call: &ruby_prism::CallNode<'_>) -> bool {
    method_identifier_predicates::is_assignment_method(call.name().as_slice())
}

/// Get the line number of the selector/method name for a call node.
fn get_selector_line(source: &SourceFile, call: &ruby_prism::CallNode<'_>) -> usize {
    if let Some(msg_loc) = call.message_loc() {
        let (line, _) = source.offset_to_line_col(msg_loc.start_offset());
        line
    } else {
        // Implicit call (proc call) — use the opening paren location
        if let Some(open_loc) = call.opening_loc() {
            let (line, _) = source.offset_to_line_col(open_loc.start_offset());
            line
        } else {
            let dot_loc = call.call_operator_loc().unwrap();
            let (line, _) = source.offset_to_line_col(dot_loc.start_offset());
            line
        }
    }
}

/// Get the (line, col) of the selector for a call node. For trailing dot style,
/// this is the method name on the next line.
fn get_selector_position(source: &SourceFile, call: &ruby_prism::CallNode<'_>) -> (usize, usize) {
    if let Some(msg_loc) = call.message_loc() {
        source.offset_to_line_col(msg_loc.start_offset())
    } else if let Some(open_loc) = call.opening_loc() {
        // Implicit call — `a\n.(args)`
        // The dot is the call operator; for trailing dot, check if `.(` starts
        // the next line.
        let dot_loc = call.call_operator_loc().unwrap();
        let (dot_line, _) = source.offset_to_line_col(dot_loc.start_offset());
        let (open_line, _) = source.offset_to_line_col(open_loc.start_offset());
        if open_line > dot_line {
            // The `.(` is on the next line — use dot position
            source.offset_to_line_col(dot_loc.start_offset())
        } else {
            source.offset_to_line_col(open_loc.start_offset())
        }
    } else {
        let dot_loc = call.call_operator_loc().unwrap();
        source.offset_to_line_col(dot_loc.start_offset())
    }
}

/// Check whether the byte at the given offset is the first non-whitespace
/// character on its line.
fn is_first_on_line(source: &SourceFile, offset: usize) -> bool {
    let bytes = source.as_bytes();
    let mut pos = offset;
    while pos > 0 && bytes[pos - 1] != b'\n' {
        pos -= 1;
    }
    while pos < offset {
        if bytes[pos] != b' ' && bytes[pos] != b'\t' {
            return false;
        }
        pos += 1;
    }
    true
}

/// Check if the character at (line, col) is the first non-whitespace on its line.
fn is_first_on_line_at(source: &SourceFile, line: usize, col: usize) -> bool {
    let line_bytes = source.lines().nth(line - 1).unwrap_or(b"");
    for (i, &b) in line_bytes.iter().enumerate() {
        if i >= col {
            return true;
        }
        if b != b' ' && b != b'\t' {
            return false;
        }
    }
    true
}

/// Find alignment for block chain patterns. When the receiver is a call with
/// a single-line block, align with that call's dot.
fn find_block_chain_alignment(
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    current_line: usize,
) -> Option<usize> {
    let receiver = call_node.receiver()?;

    // Direct receiver with block
    if let Some(call) = receiver.as_call_node() {
        if has_real_block(&call) {
            if let Some(dot_loc) = call.call_operator_loc() {
                let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
                let loc = call.location();
                let (end_line, _) = source.offset_to_line_col(loc.end_offset());
                // Single-line block: dot to end on same line, before current
                if dot_line == end_line && dot_line < current_line {
                    return Some(dot_col);
                }
                // Multiline block: align with the dot of the block-bearing call
                if end_line > dot_line && is_first_on_line(source, dot_loc.start_offset()) {
                    return Some(dot_col);
                }
            }
        }
    }

    None
}

/// RuboCop's `find_continuation_node`: when the CURRENT call has a block
/// (do..end or { }), check whether the receiver has a continuation dot
/// (its dot is on a line after the receiver's receiver's last line).
/// If so, use the receiver's dot column as the alignment base.
///
/// This handles patterns like:
/// ```ruby
/// Foo.all
///   .select("name")    # continuation dot (line > receiver end)
///   .map { |e| ... }   # has block → aligns with .select's dot
/// ```
fn find_current_node_block_continuation(
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    receiver: &ruby_prism::Node<'_>,
) -> Option<usize> {
    // Current node must have a real block (do..end or { }), not a block argument (&:foo)
    let block = call_node.block()?;
    block.as_block_node()?;

    let recv_call = receiver.as_call_node()?;

    // Case 1: receiver is itself a single-line block call — use its inner call's dot
    if let Some(recv_block) = recv_call.block() {
        if recv_block.as_block_node().is_some() {
            let loc = recv_call.location();
            let (start_line, _) = source.offset_to_line_col(loc.start_offset());
            let (end_line, _) = source.offset_to_line_col(loc.end_offset());
            if start_line == end_line {
                if let Some(dot_loc) = recv_call.call_operator_loc() {
                    let (_, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
                    return Some(dot_col);
                }
            }
        }
    }

    // Case 2: receiver has a continuation dot (dot is on a line after
    // the receiver's receiver's end line)
    let dot_loc = recv_call.call_operator_loc()?;
    let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());

    let inner_recv = recv_call.receiver()?;
    let inner_loc = inner_recv.location();
    let (inner_end_line, _) = source.offset_to_line_col(inner_loc.end_offset());

    if dot_line > inner_end_line {
        return Some(dot_col);
    }

    None
}

/// Check if a given line has a `.` or `&.` at a specific column.
/// Used for text-based approximation of RuboCop's `get_dot_right_above`.
fn has_dot_at_col(source: &SourceFile, line: usize, col: usize) -> bool {
    if line < 1 {
        return false;
    }
    let line_bytes = match source.lines().nth(line - 1) {
        Some(b) => b,
        None => return false,
    };
    if col >= line_bytes.len() {
        return false;
    }
    let ch = line_bytes[col];
    if ch == b'.' {
        return true;
    }
    if ch == b'&' && col + 1 < line_bytes.len() && line_bytes[col + 1] == b'.' {
        return true;
    }
    false
}

/// Check if a dot at (target_line, target_col) belongs to any call node
/// in the receiver chain of the given node. This is used to distinguish
/// ancestor dots (from enclosing expressions) from receiver chain dots
/// (from the current expression's own chain).
fn is_dot_in_receiver_chain(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
    target_line: usize,
    target_col: usize,
) -> bool {
    if let Some(call) = node.as_call_node() {
        if let Some(dot_loc) = call.call_operator_loc() {
            let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
            if dot_line == target_line && dot_col == target_col {
                return true;
            }
        }
        if let Some(recv) = call.receiver() {
            return is_dot_in_receiver_chain(source, &recv, target_line, target_col);
        }
    }
    false
}

/// Find alignment based on the first dot in the chain (RuboCop's
/// `first_call_alignment_node`). For "aligned" style, when the first call in
/// the chain has an inline dot (not starting its line), subsequent continuation
/// dots should align with that first dot.
fn find_first_dot_alignment(
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
) -> Option<usize> {
    let receiver = call_node.receiver()?;

    // Find the first call with a dot in the chain
    let (first_dot_offset, first_dot_line, first_dot_col, _name, first_call_start_line) =
        find_first_call_info(source, &receiver)?;

    // Check that the first dot is inline (not a continuation dot)
    if is_first_on_line(source, first_dot_offset) {
        return None; // First dot is also a continuation dot — no inline base
    }

    // Check the base receiver type. RuboCop skips if the base receiver is
    // a `begin` node and the dot is on the same line as the begin's closing.
    if let Some(begin_end_line) = chain_root_is_paren(source, &receiver) {
        if first_dot_line == begin_end_line {
            return None;
        }
    }

    // For array literal bases, the first dot is valid even on a different line
    if chain_root_is_array(&receiver) {
        return Some(first_dot_col);
    }

    if first_dot_line != first_call_start_line {
        return None; // First dot is on a different line — not inline
    }

    Some(first_dot_col)
}

/// Find syntactic alignment base: assignment RHS, keyword condition, or
/// operation RHS. These are patterns where the alignment base is determined
/// by the syntactic context rather than semantic dot alignment.
fn find_syntactic_alignment(
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    receiver: &ruby_prism::Node<'_>,
    is_trailing_dot: bool,
) -> Option<usize> {
    let root_offset = find_chain_root_offset(receiver);
    let _ = call_node;

    // For trailing dot style, check if the chain root is in a keyword
    // expression (if, while, until, return, unless, for) — align with the
    // keyword's condition expression.
    if is_trailing_dot {
        if let Some(col) = keyword_condition_alignment(source, receiver) {
            return Some(col);
        }
    }

    // Assignment RHS: `a = b\n    .c` — align with `b`
    if assignment_context_base_col(source, root_offset).is_some() {
        let chain_root_col = find_chain_root_col(source, receiver);
        return Some(chain_root_col);
    }

    // Multi-line assignment RHS: `a =\n  b\n  .c` — the `=` is on the
    // previous line, so `assignment_context_base_col` misses it.
    if prev_line_ends_with_assignment(source, receiver) {
        let chain_root_col = find_chain_root_col(source, receiver);
        return Some(chain_root_col);
    }

    None
}

/// For trailing dot in keyword expressions: `return b.\n         c` or
/// `if a.\n   b` — the alignment base is the keyword's condition expression,
/// NOT the indentation-based calculation.
fn keyword_condition_alignment(
    source: &SourceFile,
    receiver: &ruby_prism::Node<'_>,
) -> Option<usize> {
    let root_col = find_chain_root_col(source, receiver);
    let root_offset = find_chain_root_offset(receiver);
    let (root_line, _) = source.offset_to_line_col(root_offset);
    let line_bytes = source.lines().nth(root_line - 1)?;

    // Check if the chain root is preceded by a keyword on the same line
    let trimmed: Vec<u8> = line_bytes
        .iter()
        .copied()
        .skip_while(|&b| b == b' ' || b == b'\t')
        .collect();

    let keywords: &[(&[u8], usize)] = &[
        (b"return ", 7),
        (b"return(", 7),
        (b"if ", 3),
        (b"unless ", 7),
        (b"while ", 6),
        (b"until ", 6),
        (b"for ", 4),
    ];

    for &(kw, _kw_len) in keywords {
        if trimmed.starts_with(kw) {
            // The alignment base is the chain root's column
            return Some(root_col);
        }
    }

    None
}

/// Check if the line above the chain root ends with an assignment operator (`=`, `+=`, etc.).
/// This handles multi-line assignments like `a =\n  b\n  .c` where `assignment_context_base_col`
/// only checks the same line as the chain root.
fn prev_line_ends_with_assignment(source: &SourceFile, receiver: &ruby_prism::Node<'_>) -> bool {
    let root_offset = find_chain_root_offset(receiver);
    let (root_line, _) = source.offset_to_line_col(root_offset);
    if root_line <= 1 {
        return false;
    }
    let prev_bytes = source.lines().nth(root_line - 2).unwrap_or(b"");
    // Find the last non-whitespace character
    let mut last_non_ws = None;
    let mut second_last = None;
    for &b in prev_bytes.iter().rev() {
        if b == b' ' || b == b'\t' || b == b'\r' {
            continue;
        }
        if last_non_ws.is_none() {
            last_non_ws = Some(b);
        } else if second_last.is_none() {
            second_last = Some(b);
            break;
        }
    }
    match last_non_ws {
        Some(b'=') => {
            // Exclude ==, !=, <=, >=
            !matches!(second_last, Some(b'=' | b'!' | b'<' | b'>'))
        }
        _ => false,
    }
}

/// Find the column of a previous continuation dot in the receiver chain.
/// A continuation dot is one that is the first non-whitespace on its line.
/// This is used for "aligned" style when there's no inline first dot.
fn find_previous_continuation_dot(
    source: &SourceFile,
    receiver: &ruby_prism::Node<'_>,
    current_line: usize,
) -> Option<usize> {
    if let Some(call) = receiver.as_call_node() {
        if let Some(dot_loc) = call.call_operator_loc() {
            let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
            if dot_line < current_line && is_first_on_line(source, dot_loc.start_offset()) {
                // Found a continuation dot on an earlier line.
                // Check if there's an even earlier one to use as the alignment base.
                if let Some(recv) = call.receiver() {
                    if let Some(earlier) = find_previous_continuation_dot(source, &recv, dot_line) {
                        return Some(earlier);
                    }
                }
                return Some(dot_col);
            }
            // Dot is inline or on same line; keep looking
            if let Some(recv) = call.receiver() {
                return find_previous_continuation_dot(source, &recv, current_line);
            }
        }
    }
    None
}

impl Visit<'_> for ChainVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'_>) {
        // Check this call node for alignment issues
        self.check_call(node);

        // Visit receiver normally (inherits current context)
        if let Some(recv) = node.receiver() {
            self.visit(&recv);
        }

        // RuboCop only skips actual parenthesized arg lists here. Prism also
        // reports `opening_loc` for `[]`/`[]=`, but square brackets do not
        // suppress this cop.
        let has_parens = if self.style == "aligned" {
            call_has_parenthesized_args(node)
        } else {
            node.opening_loc().is_some()
        };
        if let Some(args) = node.arguments() {
            if has_parens {
                let saved_paren = self.in_paren_args;
                self.in_paren_args = true;
                self.visit(&args.as_node());
                self.in_paren_args = saved_paren;
            } else {
                self.visit(&args.as_node());
            }
        }

        // Visit block normally (inherits current context)
        if let Some(block) = node.block() {
            self.visit(&block);
        }
    }

    fn visit_parentheses_node(&mut self, node: &ruby_prism::ParenthesesNode<'_>) {
        // Grouped expressions like `(foo\n  .bar)` — RuboCop skips these too
        let saved_paren = self.in_paren_args;
        self.in_paren_args = true;
        if let Some(body) = node.body() {
            self.visit(&body);
        }
        self.in_paren_args = saved_paren;
    }

    fn visit_assoc_node(&mut self, node: &ruby_prism::AssocNode<'_>) {
        // Visit key normally
        self.visit(&node.key());

        // Visit value with in_hash_value = true — RuboCop checks chain
        // indentation inside hash pair values even within parenthesized args.
        let saved_hash = self.in_hash_value;
        self.in_hash_value = true;
        self.visit(&node.value());
        self.in_hash_value = saved_hash;
    }
}

/// Block chain alignment ONLY (no continuation dot search). Used for hash
/// pair values where continuation dot alignment is NOT wanted, but block
/// chain continuation IS.
fn find_block_chain_col(
    source: &SourceFile,
    receiver: &ruby_prism::Node<'_>,
    current_dot_line: usize,
) -> Option<usize> {
    if let Some(call) = receiver.as_call_node() {
        if has_real_block(&call) {
            if let Some(dot_loc) = call.call_operator_loc() {
                let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
                let loc = call.location();
                let (end_line, _) = source.offset_to_line_col(loc.end_offset());
                if dot_line == end_line && dot_line < current_dot_line {
                    return Some(dot_col);
                }
            }
        }
    }
    None
}

/// RuboCop's `aligned_with_first_line_dot?`: check whether the first call
/// with a dot in the receiver chain has a dot on `line` at column `target_col`.
fn has_matching_dot_on_line(
    source: &SourceFile,
    receiver: &ruby_prism::Node<'_>,
    line: usize,
    target_col: usize,
) -> bool {
    let first_call_dot = find_first_call_dot(source, receiver);
    if let Some((fc_line, fc_col, fc_offset)) = first_call_dot {
        // Check `first_call == node.receiver`: if the first call's dot
        // belongs to the direct receiver, skip (return false).
        if let Some(call) = receiver.as_call_node() {
            if let Some(dot_loc) = call.call_operator_loc() {
                if dot_loc.start_offset() == fc_offset {
                    return false;
                }
            }
        }
        return fc_line == line && fc_col == target_col;
    }
    false
}

/// Walk down the receiver chain to find the root (node with no receiver),
/// then return the first call with a dot above it. Returns (line, col, byte_offset).
///
/// Prism keeps multiline blocks attached to the underlying `CallNode`, unlike
/// RuboCop's parser AST where the block wraps the send. When a chain continues
/// after a multiline block via an inline post-block call (`end.compact`), that
/// post-block call is the first alignment anchor RuboCop uses. Stop descending
/// once the current call's receiver is a multiline real-block call so we align
/// to `.compact` rather than tunneling back to the original `.map`.
fn find_first_call_dot(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> Option<(usize, usize, usize)> {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            if !receiver_is_multiline_block_call(source, &recv) {
                if let Some(deeper) = find_first_call_dot(source, &recv) {
                    return Some(deeper);
                }
            }
        }
        if let Some(dot_loc) = call.call_operator_loc() {
            let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
            return Some((dot_line, dot_col, dot_loc.start_offset()));
        }
    }
    None
}

/// Find info about the first call with a dot in the chain.
/// Returns (dot_offset, dot_line, dot_col, method_name, call_start_line).
fn find_first_call_info(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> Option<(usize, usize, usize, String, usize)> {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            if !receiver_is_multiline_block_call(source, &recv) {
                if let Some(deeper) = find_first_call_info(source, &recv) {
                    return Some(deeper);
                }
            }
        }
        if let Some(dot_loc) = call.call_operator_loc() {
            let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
            let name = std::str::from_utf8(call.name().as_slice())
                .unwrap_or("?")
                .to_string();
            let (start_line, _) = source.offset_to_line_col(call.location().start_offset());
            return Some((dot_loc.start_offset(), dot_line, dot_col, name, start_line));
        }
    }
    None
}

fn receiver_is_multiline_block_call(source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
    let call = match node.as_call_node() {
        Some(call) => call,
        None => return false,
    };
    let block = match call.block() {
        Some(block) if has_real_block(&call) => block,
        _ => return false,
    };
    let block = match block.as_block_node() {
        Some(block) => block,
        None => return false,
    };
    let loc = block.location();
    let (start_line, _) = source.offset_to_line_col(loc.start_offset());
    let (end_line, _) = source.offset_to_line_col(loc.end_offset());
    start_line != end_line
}

/// Check if the chain root is a parenthesized expression (begin node).
/// Returns end_line if so.
fn chain_root_is_paren(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<usize> {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            return chain_root_is_paren(source, &recv);
        }
    }
    if node.as_parentheses_node().is_some() {
        let (end_line, _) = source.offset_to_line_col(node.location().end_offset());
        return Some(end_line);
    }
    None
}

/// Check if the chain root is an array literal.
fn chain_root_is_array(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            return chain_root_is_array(&recv);
        }
    }
    node.as_array_node().is_some()
}

/// Check if the chain root is inside a keyword expression and return extra indent.
fn keyword_extra_indent(
    source: &SourceFile,
    call_node: &ruby_prism::CallNode<'_>,
    _width: usize,
) -> usize {
    let receiver = match call_node.receiver() {
        Some(r) => r,
        None => return 0,
    };
    let chain_start_line = find_chain_start_line(source, &receiver);
    let chain_line_bytes = source.lines().nth(chain_start_line - 1).unwrap_or(b"");
    let trimmed = chain_line_bytes
        .iter()
        .skip_while(|&&b| b == b' ' || b == b'\t');
    let text: Vec<u8> = trimmed.copied().collect();
    let keywords: &[&[u8]] = &[
        b"return ", b"return(", b"if ", b"while ", b"until ", b"for ", b"unless ",
    ];
    for kw in keywords {
        if text.starts_with(kw) {
            return 2;
        }
    }
    0
}

/// Find the start column of the chain root (deepest receiver).
fn find_chain_root_col(source: &SourceFile, node: &ruby_prism::Node<'_>) -> usize {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            return find_chain_root_col(source, &recv);
        }
    }
    if let Some(block) = node.as_block_node() {
        let (_, col) = source.offset_to_line_col(block.location().start_offset());
        return col;
    }
    let (_, col) = source.offset_to_line_col(node.location().start_offset());
    col
}

fn find_chain_root_offset(node: &ruby_prism::Node<'_>) -> usize {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            return find_chain_root_offset(&recv);
        }
    }
    if let Some(block) = node.as_block_node() {
        return block.location().start_offset();
    }
    node.location().start_offset()
}

/// Detect if the chain root is preceded by a splat (`*`) or kwsplat (`**`)
/// operator on the same line. Returns the operator length (0, 1, or 2).
///
/// RuboCop's `extra_indentation` for `indented_relative_to_receiver` subtracts
/// the operator length from the configured indentation width.
fn splat_operator_length(source: &SourceFile, receiver: &ruby_prism::Node<'_>) -> usize {
    let root_offset = find_chain_root_offset(receiver);
    if root_offset == 0 {
        return 0;
    }
    let bytes = source.as_bytes();
    if bytes[root_offset - 1] == b'*' {
        if root_offset >= 2 && bytes[root_offset - 2] == b'*' {
            return 2; // **expr
        }
        return 1; // *expr
    }
    0
}

/// RuboCop's `find_hash_method_base_in_receiver_chain` for
/// `indented_relative_to_receiver` style.
///
/// Walks the receiver chain downward. For each call node, checks if its
/// receiver is:
/// 1. A hash literal (`HashNode`) → return that call's dot column
/// 2. A parenthesized expression (`ParenthesesNode`) where the dot is on
///    the same line as the closing paren → return that call's dot column
///
/// This handles patterns like:
/// ```ruby
/// { a: 1, b: 2 }.keys     # base = `.keys` dot
///                  .first  # indented relative to `.keys`
///
/// (date_columns + cols).uniq    # base = `.uniq` dot
///                        .each  # indented relative to `.uniq`
/// ```
fn find_hash_method_base_col(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<usize> {
    let call = node.as_call_node()?;
    let recv = call.receiver()?;

    // Check if receiver is a hash literal (HashNode or KeywordHashNode —
    // RuboCop's `hash_type?` matches both)
    if recv.as_hash_node().is_some() || recv.as_keyword_hash_node().is_some() {
        if let Some(dot_loc) = call.call_operator_loc() {
            let (_, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
            return Some(dot_col);
        }
    }
    // Check if receiver is a parenthesized expression with dot on
    // the same line as the closing paren
    if recv.as_parentheses_node().is_some() {
        let (recv_end_line, _) = source.offset_to_line_col(recv.location().end_offset());
        if let Some(dot_loc) = call.call_operator_loc() {
            let (dot_line, dot_col) = source.offset_to_line_col(dot_loc.start_offset());
            if dot_line == recv_end_line {
                return Some(dot_col);
            }
        }
    }

    // Recurse into receiver chain
    find_hash_method_base_col(source, &recv)
}

/// Build base description for `indented_relative_to_receiver` messages.
///
/// Returns (base_source_text, base_line) matching RuboCop's `base_source`
/// which is `@base.source[/[^\n]*/]` — the first line of the base range.
///
/// For hash/paren chains, the base is the dot+selector (e.g., `.keys`).
/// For normal chains, the base is the chain root (e.g., `Thing`).
fn find_receiver_relative_base_description(
    source: &SourceFile,
    receiver: &ruby_prism::Node<'_>,
) -> (String, usize) {
    // Check for hash/paren method base first
    if let Some((desc, line)) = find_hash_method_base_description(source, receiver) {
        return (desc, line);
    }

    // Normal chain: base is the chain root receiver
    find_chain_root_description(source, receiver)
}

/// Build description for hash/paren method base.
/// Walks receiver chain looking for hash/paren receivers (same logic as
/// `find_hash_method_base_col`), returns the dot+selector text.
fn find_hash_method_base_description(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> Option<(String, usize)> {
    let call = node.as_call_node()?;
    let recv = call.receiver()?;

    let is_hash = recv.as_hash_node().is_some() || recv.as_keyword_hash_node().is_some();
    let is_paren_same_line = if recv.as_parentheses_node().is_some() {
        if let Some(dot_loc) = call.call_operator_loc() {
            let (recv_end_line, _) = source.offset_to_line_col(recv.location().end_offset());
            let (dot_line, _) = source.offset_to_line_col(dot_loc.start_offset());
            dot_line == recv_end_line
        } else {
            false
        }
    } else {
        false
    };

    if is_hash || is_paren_same_line {
        let name = std::str::from_utf8(call.name().as_slice()).unwrap_or("?");
        if let Some(dot_loc) = call.call_operator_loc() {
            let (line, _) = source.offset_to_line_col(dot_loc.start_offset());
            return Some((format!(".{name}"), line));
        }
    }

    // Recurse into receiver chain
    find_hash_method_base_description(source, &recv)
}

/// Walk backwards from a given line to find the first line that does NOT
/// start with a continuation dot.
fn find_non_continuation_ancestor_line(source: &SourceFile, start_line: usize) -> usize {
    let lines: Vec<&[u8]> = source.lines().collect();
    let mut line = start_line;
    while line >= 1 {
        if line > lines.len() {
            break;
        }
        let line_bytes = lines[line - 1];
        let trimmed: Vec<u8> = line_bytes
            .iter()
            .copied()
            .skip_while(|&b| b == b' ' || b == b'\t')
            .collect();
        if trimmed.starts_with(b".") || trimmed.starts_with(b"&.") {
            if line <= 1 {
                break;
            }
            line -= 1;
        } else {
            break;
        }
    }
    line
}

fn find_chain_start_line(source: &SourceFile, node: &ruby_prism::Node<'_>) -> usize {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            let (recv_line, _) = source.offset_to_line_col(recv.location().start_offset());
            let (call_msg_line, _) = if let Some(dot_loc) = call.call_operator_loc() {
                source.offset_to_line_col(dot_loc.start_offset())
            } else {
                (recv_line, 0)
            };
            if call_msg_line != recv_line {
                return find_chain_start_line(source, &recv);
            }
        }
    }
    let (line, _) = source.offset_to_line_col(node.location().start_offset());
    line
}

/// For hash pair context: show the full source text of the receiver chain
/// on its first line. This matches RuboCop's `base_source` which returns
/// `@base.source[/[^\n]*/]`.
fn find_chain_source_description(
    source: &SourceFile,
    receiver: &ruby_prism::Node<'_>,
) -> (String, usize) {
    // Get the chain root's start line
    let chain_start_line = find_chain_start_line(source, receiver);
    let root_col = find_chain_root_col(source, receiver);

    // Get the full line text and extract from root_col to end of meaningful content
    let line_bytes = source.lines().nth(chain_start_line - 1).unwrap_or(b"");
    let line_text = std::str::from_utf8(line_bytes).unwrap_or("?");
    let trimmed = line_text.get(root_col..).unwrap_or("?").trim_end();

    (trimmed.to_string(), chain_start_line)
}

/// Find alignment base description for error messages.
fn find_alignment_base_description(
    source: &SourceFile,
    _call_node: &ruby_prism::CallNode<'_>,
    receiver: &ruby_prism::Node<'_>,
    is_trailing_dot: bool,
) -> (String, usize) {
    if !is_trailing_dot {
        // Check for block chain
        if let Some(call) = receiver.as_call_node() {
            if has_real_block(&call) {
                if let Some(dot_loc) = call.call_operator_loc() {
                    let (block_dot_line, _) = source.offset_to_line_col(dot_loc.start_offset());
                    let loc = call.location();
                    let (end_line, _) = source.offset_to_line_col(loc.end_offset());
                    if block_dot_line == end_line || end_line > block_dot_line {
                        let name = std::str::from_utf8(call.name().as_slice()).unwrap_or("?");
                        return (format!(".{name}"), block_dot_line);
                    }
                }
            }
        }

        // Check for first inline dot alignment
        if let Some((first_dot_offset, first_dot_line, _, name, _)) =
            find_first_call_info(source, receiver)
        {
            if !is_first_on_line(source, first_dot_offset) {
                // First dot is inline — use it as alignment base description
                return (format!(".{name}"), first_dot_line);
            }
        }
    }

    // Fall back to chain root
    find_chain_root_description(source, receiver)
}

/// Walk down the receiver chain to find the root and its description.
fn find_chain_root_description(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> (String, usize) {
    if let Some(call) = node.as_call_node() {
        if let Some(recv) = call.receiver() {
            return find_chain_root_description(source, &recv);
        }
        let name = call.name().as_slice();
        let name_str = std::str::from_utf8(name).unwrap_or("?");
        let loc = call.location();
        let (line, _) = source.offset_to_line_col(loc.start_offset());
        let source_text = extract_call_source(call);
        return (source_text.unwrap_or_else(|| name_str.to_string()), line);
    }
    if let Some(_block) = node.as_block_node() {
        let (line, _) = source.offset_to_line_col(node.location().start_offset());
        return ("...".to_string(), line);
    }
    let loc = node.location();
    let (line, _) = source.offset_to_line_col(loc.start_offset());
    let name = std::str::from_utf8(loc.as_slice()).unwrap_or("?");
    let name = name.lines().next().unwrap_or("?").trim_end();
    (name.to_string(), line)
}

/// Extract a concise source representation of a call for messages.
fn extract_call_source(call: ruby_prism::CallNode<'_>) -> Option<String> {
    let name = std::str::from_utf8(call.name().as_slice()).ok()?;
    if let Some(args) = call.arguments() {
        let first_arg = args.arguments().iter().next()?;
        let arg_loc = first_arg.location();
        let arg_text = std::str::from_utf8(arg_loc.as_slice()).ok()?;
        Some(format!("{name}({arg_text})"))
    } else {
        Some(name.to_string())
    }
}

fn call_has_parenthesized_args(call: &ruby_prism::CallNode<'_>) -> bool {
    call.opening_loc().is_some_and(|loc| loc.as_slice() == b"(")
}

fn has_real_block(call: &ruby_prism::CallNode<'_>) -> bool {
    matches!(call.block(), Some(block) if block.as_block_node().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        MultilineMethodCallIndentation,
        "cops/layout/multiline_method_call_indentation"
    );
    crate::cop_variant_fixture_tests!(
        MultilineMethodCallIndentation,
        "cops/layout/multiline_method_call_indentation",
        indented_relative_to_receiver
    );

    #[test]
    fn same_line_chain_ignored() {
        let source = b"foo.bar.baz\n";
        let diags = run_cop_full(&MultilineMethodCallIndentation, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn trailing_dot_no_indent() {
        // a.\n b  — should flag (need 2 spaces, not 0)
        let source = b"a.\nb\n";
        let diags = run_cop_full(&MultilineMethodCallIndentation, source);
        assert!(!diags.is_empty(), "Should flag trailing dot with no indent");
    }

    #[test]
    fn trailing_dot_correct_indent() {
        // a.\n  b  — properly indented, no offense
        let source = b"a.\n  b\n";
        let diags = run_cop_full(&MultilineMethodCallIndentation, source);
        assert!(
            diags.is_empty(),
            "Should not flag correct trailing dot indent"
        );
    }

    #[test]
    fn aligned_unaligned_methods() {
        // User.a\n  .b — should flag `.b` as misaligned with `.a`
        let source = b"User.a\n  .b\n";
        let diags = run_cop_full(&MultilineMethodCallIndentation, source);
        assert!(!diags.is_empty(), "Should flag misaligned .b");
        assert!(diags[0].message.contains(".b"));
        assert!(diags[0].message.contains(".a"));
    }

    #[test]
    fn aligned_methods_correct() {
        // User.a\n    .b — aligned with .a at col 4
        let source = b"User.a\n    .b\n";
        let diags = run_cop_full(&MultilineMethodCallIndentation, source);
        assert!(diags.is_empty(), "Should accept aligned methods");
    }

    #[test]
    fn paren_args_skipped() {
        // Inside parenthesized args, chains should be skipped
        let source = b"foo(bar\n  .baz)\n";
        let diags = run_cop_full(&MultilineMethodCallIndentation, source);
        assert!(
            diags.is_empty(),
            "Should skip chains inside parenthesized args"
        );
    }

    #[test]
    fn grouped_expression_skipped() {
        let source = b"(a.\n b)\n";
        let diags = run_cop_full(&MultilineMethodCallIndentation, source);
        assert!(
            diags.is_empty(),
            "Should skip chains inside grouped expression"
        );
    }
}
