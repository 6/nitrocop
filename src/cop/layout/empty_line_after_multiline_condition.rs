use crate::cop::shared::node_type::{CASE_NODE, IF_NODE, UNLESS_NODE, UNTIL_NODE, WHILE_NODE};
use crate::cop::shared::util::is_blank_or_whitespace_line;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Enforces empty line after multiline condition.
///
/// ## Corpus investigation (227 FP, 801 FN)
///
/// **FP root causes (round 1):**
/// - Modifier if/unless/while/until at last position (no right sibling) were
///   being flagged. RuboCop only flags modifier forms when there's a subsequent
///   statement (`right_sibling`). Without AST parent pointers, we approximate
///   by scanning for the next non-blank line after the condition and checking
///   if it looks like a continuation statement.
/// - Multiline check compared keyword line vs predicate end line, but RuboCop's
///   `condition.multiline?` compares the predicate's own first_line vs last_line.
///   This caused FPs when `if`/`unless` is at end of line with a single-line
///   predicate on the next line (e.g., `raise ... if\n  cond`). Fixed by comparing
///   predicate start line vs end line instead.
///
/// **FP root causes (round 2, 39 FPs):**
/// - Used `is_blank_line` which only treats empty lines as blank; RuboCop's
///   `blank?` also treats whitespace-only lines as blank. Fixed by switching to
///   `is_blank_or_whitespace_line`.
/// - `elsif case ...` patterns: when the predicate of if/elsif is a CaseNode,
///   the multiline nature comes from the case structure, not a simple boolean
///   condition. RuboCop may not flag these. Fixed by skipping when predicate
///   is a CaseNode.
/// - `has_right_sibling` heuristic was too aggressive: treated comment lines
///   as right siblings, and didn't recognize `when` as a structural keyword.
///   Fixed by skipping comment lines and adding `when` to the structural
///   keyword list.
///
/// **FP root causes (round 3, 21 FP → 0 FP):**
/// - Offense location placed at keyword (`if`/`unless`/`elsif`) instead of
///   condition node. When keyword is at end of line and condition starts on
///   next line, this creates FP on keyword line + FN on condition line.
///   Fixed by reporting offense at predicate start, matching RuboCop's
///   `add_offense(condition)`.
/// - `BlockNode.multiline?` override in rubocop-ast: when condition is a
///   block call (e.g., `.all? { ... }`), RuboCop checks whether the block
///   delimiters (`{`/`}` or `do`/`end`) span multiple lines, not the full
///   expression. A multiline method chain with single-line `{ }` block is
///   NOT considered multiline. Fixed by checking block delimiter lines when
///   predicate is a CallNode with a block argument.
///
/// **FN root causes (round 1):**
/// - Missing `case/when` support: multiline when conditions need an empty line
///   after the last condition before the body.
/// - Missing `rescue` support: multiline rescue exception lists need an empty
///   line after the last exception before the handler body.
/// - Message format mismatch: RuboCop uses "Use empty line after multiline condition."
///   (no "an"), the old message had "an".
///
/// **FN root causes (round 2, 21 FN → ~10 FN):**
/// - `expr while cond` was treated as modifier (check right_sibling), but
///   Parser gem treats it as regular `while` (always check). Only
///   `begin...end while cond` is `while_post` (check right_sibling). Fixed
///   by using Prism's `is_begin_modifier()` flag instead of `closing_loc().is_none()`.
///
/// **FN root causes (round 3, 12 FN → 0 FN):**
/// - `has_right_sibling` treated `else`/`elsif`/`rescue`/`ensure` as scope
///   terminators (returning false), but in RuboCop's Parser AST, when a modifier
///   if/unless IS the direct body of an outer `if` node (single-statement body),
///   `right_sibling` returns the else/elsif body as the next child. Similarly,
///   `rescue`/`ensure` in a `begin` block are sibling positions. Fixed by removing
///   `else`/`elsif`/`rescue`/`ensure` from the terminator list, keeping only `end`,
///   `}`, and `when` as true scope-closers.
///
/// **FP root causes (round 4, 13 FP → 2 FP):**
/// - `has_right_sibling` treated `else`/`elsif`/`rescue`/`ensure` as always
///   indicating a right sibling. But in RuboCop's Parser AST, these keywords
///   are only right siblings when the modifier is the SOLE statement in its
///   parent body. When the modifier is the last of multiple statements, they
///   are wrapped in a `begin` node and `right_sibling` returns nil.
/// - Additionally, inside a `rescue` handler body, the next `rescue`/`ensure`
///   is a sibling of the `resbody` node, never of the body statement.
/// - Inside a `when` body, `else` is a child of `case`, not `when`.
/// - Fixed by passing statement_start_line to `has_right_sibling` and using
///   `is_sole_body_statement` to look backwards: if preceding non-blank
///   non-comment line has the same indentation → multiple statements → no
///   right sibling. If enclosing scope is `when` or `rescue` handler → no
///   right sibling for `else`/`rescue`.
///
/// **FP root causes (round 5, 2 FP → 0 FP):**
/// - Both FPs in `camping__camping` were caused by minified/compressed Ruby code
///   where scope-closing tokens (`}`, `end;`) appear on the same line as the
///   condition end, after the condition. The line-based `has_right_sibling`
///   heuristic only checked subsequent lines, missing scope closers on the
///   condition-end line itself.
/// - Fixed by checking the tail of the condition-end line (bytes after the
///   condition end offset) for `}` or `end` at a word boundary. Also added
///   `end;` and `};` recognition to the line-based scope-closer check.
///
/// **FN root causes (round 4, 2 FN → 0 FN):**
/// - Both FNs in `camping__camping` were caused by minified Ruby where the
///   condition-end line tail has a `;` BEFORE the scope closer (e.g.,
///   `});right_sibling;end`). The old `tail_has_scope_closer` treated any `}`
///   as a scope closer, but `}` here closes a block within the condition, and
///   the `;` after it starts a right sibling statement.
/// - Fixed by making `tail_scope_closer_check` return `RightSibling` when `;`
///   appears before any `}` or `end`, indicating content on the same line.
/// - Additionally, `check_multiline_condition` only checked the NEXT LINE for
///   blank/non-blank. In minified code, the right sibling can be on the SAME
///   line as the condition end. Fixed by also checking the tail of the
///   condition-end line for non-whitespace content.
///
/// **FP root causes (round 6, 11 FP → 0 FP):**
/// - All 11 FPs caused by trailing comments on the condition's last line.
///   The tail check in `check_multiline_condition` looked for non-whitespace
///   content after the predicate end offset. A trailing `# comment` has `#`
///   as non-whitespace, so it was treated as real content (like minified code),
///   triggering a false offense. Fixed by checking if the first non-whitespace
///   byte in the tail is `#` — if so, it's a comment and should be ignored.
///
/// **FP root causes (round 7, 2 FP → 0 FP):**
/// - FP in `ruby__rdoc`: modifier `unless` inside a parenthesized expression
///   `(stat.mtime unless (cond and\n  other_cond))`. In Parser AST, the outer
///   `()` creates a `begin` node wrapping the modifier, so `right_sibling`
///   returns nil. Our `has_right_sibling` didn't recognize `)` as a scope
///   closer in `tail_scope_closer_check`. After the predicate end, the tail
///   has `)` (closing the outer parens), which is now treated as a scope
///   closer — no right sibling within the parenthesized scope.
/// - FP in `samg__timetrap`: CRLF line endings caused `\r` on the condition
///   end line to be treated as non-whitespace content in the tail check of
///   `check_multiline_condition`. The `\r` was not included in the whitespace
///   byte set (`' '`, `'\t'`), so it triggered a false offense. Fixed by
///   adding `\r` to the whitespace check.
///
/// **FP root causes (round 8, 11 FP):**
/// - The `then` keyword after a multiline parenthesized condition (e.g.,
///   `if (\n...\n) then`) was treated as real content in the tail check of
///   `check_multiline_condition`. The tail after the predicate end offset
///   contains ` then`, and `t` is non-whitespace/non-comment, so the cop
///   falsely fired. Fixed by also skipping `then` keyword in the tail check
///   via `is_then_keyword()`.
///
/// **FN root causes (round 5, 2 FN → 0 FN):**
/// - The old `check_multiline_condition` skipped `CaseNode` and
///   `CaseMatchNode` predicates entirely. That was based on a stale assumption:
///   current RuboCop does flag multiline `case` expressions when they are used
///   as `if`/`unless`/`elsif` conditions, including `unless case ... end`.
///   Fixed by treating multiline `case` predicates like any other multiline
///   condition.
/// - Modifier `if` conditions wrapped by a rescue modifier
///   (`expr if cond rescue nil`) have a right sibling in RuboCop's AST: the
///   rescue expression. The line-based `has_right_sibling` heuristic only
///   looked for `;`, scope closers, and later lines, so it missed same-line
///   `rescue` tails and suppressed valid offenses. Fixed by treating a same-line
///   `rescue` tail after the predicate as a right sibling.
///
/// **FP root causes (round 9, 10 FP in diagnosis packet):**
/// - Prism excludes a trailing line-continuation backslash from the predicate
///   range. For `next if cond \` with the blank line after the continued
///   condition, the tail check saw `\` after the parsed predicate and treated it
///   as real same-line content, causing a false offense. Fixed by treating a
///   tail of only `\` plus whitespace as continuation syntax, not content.
/// - Modifier conditions wrapped in a parenthesized or indexed expression can
///   be followed by a line that starts with `)`/`],`/`),`. That line closes the
///   enclosing expression; it is not a `right_sibling` in RuboCop's Parser AST.
///   Fixed by treating lines that start with `)` or `]` like other scope
///   closers in the sibling heuristic.
pub struct EmptyLineAfterMultilineCondition;

impl Cop for EmptyLineAfterMultilineCondition {
    fn name(&self) -> &'static str {
        "Layout/EmptyLineAfterMultilineCondition"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[IF_NODE, UNLESS_NODE, UNTIL_NODE, WHILE_NODE, CASE_NODE]
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
        // RescueNode is not dispatched via visit_branch_node_enter in Prism's
        // visitor, so check_node never sees it. Use a dedicated visitor here.
        let mut visitor = RescueVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.append(&mut visitor.diagnostics);
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
        // Check if/unless nodes
        if let Some(if_node) = node.as_if_node() {
            let kw_loc = match if_node.if_keyword_loc() {
                Some(loc) => loc,
                None => return,
            };
            let kw_slice = kw_loc.as_slice();
            if kw_slice != b"if" && kw_slice != b"unless" && kw_slice != b"elsif" {
                return;
            }

            // Skip ternary (no end keyword, but has `?` then keyword)
            let is_ternary = if_node.end_keyword_loc().is_none()
                && if_node
                    .then_keyword_loc()
                    .is_some_and(|t| t.as_slice() == b"?");
            if is_ternary {
                return;
            }

            // Modifier form: no end keyword (ternary already excluded above)
            let is_modifier = if_node.end_keyword_loc().is_none();

            if is_modifier {
                // For modifier forms, only flag if there's a right sibling.
                let predicate = if_node.predicate();
                let pred_end = predicate.location().end_offset().saturating_sub(1);
                let (pred_end_line, _) = source.offset_to_line_col(pred_end);
                let (stmt_start_line, _) =
                    source.offset_to_line_col(if_node.location().start_offset());
                if has_right_sibling(
                    source,
                    stmt_start_line,
                    pred_end_line,
                    predicate.location().end_offset(),
                ) {
                    diagnostics.extend(self.check_multiline_condition(source, &predicate));
                }
            } else {
                let predicate = if_node.predicate();
                diagnostics.extend(self.check_multiline_condition(source, &predicate));
            }
            return;
        }

        // Check unless nodes (Prism has a separate UnlessNode)
        if let Some(unless_node) = node.as_unless_node() {
            let kw_loc = unless_node.keyword_loc();
            if kw_loc.as_slice() != b"unless" {
                return;
            }
            let is_modifier = unless_node.end_keyword_loc().is_none();
            let predicate = unless_node.predicate();
            if is_modifier {
                let pred_end = predicate.location().end_offset().saturating_sub(1);
                let (pred_end_line, _) = source.offset_to_line_col(pred_end);
                let (stmt_start_line, _) =
                    source.offset_to_line_col(unless_node.location().start_offset());
                if has_right_sibling(
                    source,
                    stmt_start_line,
                    pred_end_line,
                    predicate.location().end_offset(),
                ) {
                    diagnostics.extend(self.check_multiline_condition(source, &predicate));
                }
            } else {
                diagnostics.extend(self.check_multiline_condition(source, &predicate));
            }
            return;
        }

        // Check while nodes
        if let Some(while_node) = node.as_while_node() {
            let kw_loc = while_node.keyword_loc();
            if kw_loc.as_slice() != b"while" {
                return;
            }
            let predicate = while_node.predicate();
            // In RuboCop: `on_while` always checks (block and `expr while cond`),
            // only `on_while_post` (`begin...end while cond`) checks right_sibling.
            // Prism's `is_begin_modifier()` distinguishes the post form.
            let is_begin_modifier =
                while_node.closing_loc().is_none() && while_node.is_begin_modifier();
            if is_begin_modifier {
                let pred_end = predicate.location().end_offset().saturating_sub(1);
                let (pred_end_line, _) = source.offset_to_line_col(pred_end);
                let (stmt_start_line, _) =
                    source.offset_to_line_col(while_node.location().start_offset());
                if has_right_sibling(
                    source,
                    stmt_start_line,
                    pred_end_line,
                    predicate.location().end_offset(),
                ) {
                    diagnostics.extend(self.check_multiline_condition(source, &predicate));
                }
            } else {
                diagnostics.extend(self.check_multiline_condition(source, &predicate));
            }
            return;
        }

        // Check until nodes
        if let Some(until_node) = node.as_until_node() {
            let kw_loc = until_node.keyword_loc();
            if kw_loc.as_slice() != b"until" {
                return;
            }
            let predicate = until_node.predicate();
            // Same as while: only begin...end until form checks right_sibling
            let is_begin_modifier =
                until_node.closing_loc().is_none() && until_node.is_begin_modifier();
            if is_begin_modifier {
                let pred_end = predicate.location().end_offset().saturating_sub(1);
                let (pred_end_line, _) = source.offset_to_line_col(pred_end);
                let (stmt_start_line, _) =
                    source.offset_to_line_col(until_node.location().start_offset());
                if has_right_sibling(
                    source,
                    stmt_start_line,
                    pred_end_line,
                    predicate.location().end_offset(),
                ) {
                    diagnostics.extend(self.check_multiline_condition(source, &predicate));
                }
            } else {
                diagnostics.extend(self.check_multiline_condition(source, &predicate));
            }
            return;
        }

        // Check case/when nodes
        if let Some(case_node) = node.as_case_node() {
            for condition in case_node.conditions().iter() {
                if let Some(when_node) = condition.as_when_node() {
                    let conditions = when_node.conditions();
                    if conditions.is_empty() {
                        continue;
                    }
                    let first = conditions.iter().next().unwrap();
                    let last = conditions.iter().last().unwrap();
                    let (first_line, _) =
                        source.offset_to_line_col(first.location().start_offset());
                    let last_end = last.location().end_offset().saturating_sub(1);
                    let (last_line, _) = source.offset_to_line_col(last_end);

                    // Only check multiline when conditions
                    if first_line == last_line {
                        continue;
                    }

                    let lines: Vec<&[u8]> = source.lines().collect();
                    let next_line_num = last_line + 1;
                    if next_line_num > lines.len() {
                        continue;
                    }
                    let next_line = lines[next_line_num - 1];
                    if !is_blank_or_whitespace_line(next_line) {
                        let when_kw_loc = when_node.keyword_loc();
                        let (line, col) = source.offset_to_line_col(when_kw_loc.start_offset());
                        diagnostics.push(self.diagnostic(source, line, col, MSG.to_string()));
                    }
                }
            }
        }
    }
}

const MSG: &str = "Use empty line after multiline condition.";

/// Check if there's a non-blank statement-like line after the given line.
/// This approximates RuboCop's `right_sibling` check for modifier forms.
///
/// In RuboCop's AST (Parser gem), `right_sibling` returns the next child of
/// the parent node. The behavior depends on whether the modifier is the sole
/// statement in its parent body or one of multiple:
///
/// - **Sole statement**: the modifier IS the direct child of the parent node
///   (e.g., `if`/`def`/`begin`), so `else`/`elsif`/`rescue`/`ensure` from
///   the parent structure ARE right siblings → fire.
/// - **Last of multiple**: statements are wrapped in a `begin` node, and
///   the modifier is the last child of `begin` → `right_sibling` returns
///   nil → don't fire.
///
/// Special cases:
/// - Inside a `rescue` handler body: the next `rescue`/`ensure` is a sibling
///   of the `resbody` node, never of the body statement → don't fire.
/// - Inside a `when` body: `else` is a child of `case`, not `when` → don't fire.
fn has_right_sibling(
    source: &SourceFile,
    statement_start_line: usize,
    condition_end_line: usize,
    condition_end_offset: usize,
) -> bool {
    let lines: Vec<&[u8]> = source.lines().collect();

    // Check the remainder of the condition-end line after the condition end offset.
    // In minified/compressed code (e.g., camping), the enclosing scope may close
    // on the same line as the condition via `}`, `};`, `end;`, etc. If the tail
    // of the condition line contains a scope-closing token, the modifier has no
    // right sibling within its scope.
    //
    // However, if a `;` appears before the scope closer, there's a right sibling
    // statement on the same line (e.g., `});right_sibling;end`). In that case,
    // the modifier DOES have a right sibling → fire.
    if condition_end_line >= 1 && condition_end_line <= lines.len() {
        let cond_line = lines[condition_end_line - 1];
        if let Some(line_start_offset) = source.line_col_to_offset(condition_end_line, 0) {
            let col_in_line = condition_end_offset.saturating_sub(line_start_offset);
            if col_in_line < cond_line.len() {
                let tail = &cond_line[col_in_line..];
                match tail_scope_closer_check(tail) {
                    TailResult::ScopeCloser => return false,
                    TailResult::RightSibling => return true,
                    TailResult::Neither => {}
                }
            }
        }
    }

    // Look at lines after the condition end
    for line in lines.iter().skip(condition_end_line) {
        if is_blank_or_whitespace_line(line) {
            continue;
        }
        let trimmed = line.iter().position(|&b| b != b' ' && b != b'\t');
        if let Some(pos) = trimmed {
            let rest = strip_trailing_cr(&line[pos..]);
            // Skip comment lines — comments are not AST siblings
            if rest.starts_with(b"#") {
                continue;
            }
            // `end` and `}` close the parent scope — no right sibling
            if is_line_scope_closer(rest) {
                return false;
            }
            // `when` is a case-branch boundary — the modifier's parent is
            // the when body, and the next when is NOT a right sibling of
            // the modifier node
            if rest.starts_with(b"when ") || rest.starts_with(b"when\n") || rest == b"when" {
                return false;
            }
            // `else`, `elsif`, `rescue`, `ensure` — these are only right
            // siblings if the modifier is the sole statement in its parent body.
            if is_branch_keyword(rest) {
                return is_sole_body_statement(
                    &lines,
                    statement_start_line,
                    is_rescue_keyword(rest),
                );
            }
            // All other lines are right siblings → fire
            return true;
        }
    }
    false
}

/// Strip a trailing `\r` from a line fragment produced by `split('\n')`.
fn strip_trailing_cr(rest: &[u8]) -> &[u8] {
    rest.strip_suffix(b"\r").unwrap_or(rest)
}

/// Check if a line starts with `end` or `}` (scope closers).
fn is_line_scope_closer(rest: &[u8]) -> bool {
    let rest = strip_trailing_cr(rest);
    rest == b"end"
        || rest.starts_with(b"end ")
        || rest.starts_with(b"end\t")
        || rest.starts_with(b"end.")
        || rest.starts_with(b"end)")
        || rest.starts_with(b"end;")
        || rest == b"}"
        || rest.starts_with(b"};")
        || rest.starts_with(b")")
        || rest.starts_with(b"]")
}

/// Result of checking the tail of a line for scope closers and right siblings.
enum TailResult {
    /// Found a scope-closing token (`}` or `end`) before any `;` — no right sibling.
    ScopeCloser,
    /// Found a `;` before any scope closer — there's a right sibling on this line.
    RightSibling,
    /// Found neither — continue checking subsequent lines.
    Neither,
}

/// Check the tail of a line (after the condition end) for scope-closing tokens
/// and statement separators. Returns:
/// - `ScopeCloser` if `}` or `end` appears before any `;` — the enclosing scope
///   closes on this line, so the modifier has no right sibling.
/// - `RightSibling` if `;` appears before any scope closer — there's another
///   statement on this line after the condition, which is a right sibling.
/// - `Neither` if the tail has no scope closer or semicolon.
fn tail_scope_closer_check(tail: &[u8]) -> TailResult {
    for (i, &b) in tail.iter().enumerate() {
        // A semicolon before any scope closer means there's a right sibling
        // statement on this line.
        if b == b';' {
            return TailResult::RightSibling;
        }
        if b == b'}' || b == b')' {
            return TailResult::ScopeCloser;
        }
        // `expr if cond rescue nil`: the rescue expression is a right sibling
        // of the modifier `if` in RuboCop's AST, even if the enclosing scope
        // closes on the next line.
        if tail[i..].starts_with(b"rescue")
            && (i == 0 || !tail[i - 1].is_ascii_alphanumeric() && tail[i - 1] != b'_')
            && (i + 6 >= tail.len() || !tail[i + 6].is_ascii_alphanumeric() && tail[i + 6] != b'_')
        {
            return TailResult::RightSibling;
        }
        // Check for `end` keyword at a word boundary
        if tail[i..].starts_with(b"end")
            && (i == 0 || !tail[i - 1].is_ascii_alphanumeric() && tail[i - 1] != b'_')
            && (i + 3 >= tail.len() || !tail[i + 3].is_ascii_alphanumeric() && tail[i + 3] != b'_')
        {
            return TailResult::ScopeCloser;
        }
    }
    TailResult::Neither
}

/// Check if a line starts with a branch keyword (`else`, `elsif`, `rescue`, `ensure`).
fn is_branch_keyword(rest: &[u8]) -> bool {
    let rest = strip_trailing_cr(rest);
    rest == b"else"
        || rest.starts_with(b"else ")
        || rest.starts_with(b"else\t")
        || rest.starts_with(b"elsif ")
        || rest.starts_with(b"elsif\t")
        || rest.starts_with(b"rescue")
            && (rest.len() == 6 || rest[6] == b' ' || rest[6] == b'\t' || rest[6] == b'\n')
        || rest.starts_with(b"ensure")
            && (rest.len() == 6 || rest[6] == b' ' || rest[6] == b'\t' || rest[6] == b'\n')
}

/// Check if a line starts with `rescue` or `ensure`.
fn is_rescue_keyword(rest: &[u8]) -> bool {
    let rest = strip_trailing_cr(rest);
    rest.starts_with(b"rescue")
        && (rest.len() == 6 || rest[6] == b' ' || rest[6] == b'\t' || rest[6] == b'\n')
        || rest.starts_with(b"ensure")
            && (rest.len() == 6 || rest[6] == b' ' || rest[6] == b'\t' || rest[6] == b'\n')
}

/// Determine if the modifier statement is the sole body statement in its
/// enclosing scope by looking backwards from the statement start line.
///
/// Returns true if the modifier appears to be the only statement (right sibling
/// exists), false if there are preceding statements (no right sibling).
///
/// When `following_is_rescue` is true and the enclosing scope is itself a
/// `rescue` handler, always returns false — the next `rescue`/`ensure` is
/// a sibling of the `resbody` node, not of the body statement.
fn is_sole_body_statement(
    lines: &[&[u8]],
    statement_start_line: usize,
    following_is_rescue: bool,
) -> bool {
    // Get indentation of the modifier statement
    let stmt_line = if statement_start_line > 0 && statement_start_line <= lines.len() {
        lines[statement_start_line - 1]
    } else {
        return false;
    };
    let stmt_indent = line_indent(stmt_line);

    // Look backwards for the enclosing scope opener or a preceding statement
    for i in (0..statement_start_line.saturating_sub(1)).rev() {
        let line = lines[i];
        if is_blank_or_whitespace_line(line) {
            continue;
        }
        let indent = line_indent(line);
        if indent < stmt_indent {
            // Found the enclosing scope opener at lower indentation.
            // Check if it's a `when` line — if so, `else` from the parent
            // `case` is never a right sibling of content inside `when`.
            let trimmed_pos = line.iter().position(|&b| b != b' ' && b != b'\t');
            if let Some(pos) = trimmed_pos {
                let rest = strip_trailing_cr(&line[pos..]);
                if rest.starts_with(b"when ") || rest.starts_with(b"when\t") || rest == b"when" {
                    return false;
                }
                // If scope opener is a `rescue` line and the following keyword
                // is also rescue/ensure, we're inside a rescue handler body
                // and the next rescue is a sibling of the resbody, not our stmt.
                if following_is_rescue
                    && rest.starts_with(b"rescue")
                    && (rest.len() == 6 || rest[6] == b' ' || rest[6] == b'\t' || rest[6] == b'\n')
                {
                    return false;
                }
            }
            // Sole statement — right sibling exists
            return true;
        }
        if indent == stmt_indent {
            // Found a preceding statement at the same indentation level.
            // Skip comment lines.
            let trimmed_pos = line.iter().position(|&b| b != b' ' && b != b'\t');
            if let Some(pos) = trimmed_pos {
                if line[pos] == b'#' {
                    continue;
                }
            }
            // Multiple statements — no right sibling
            return false;
        }
        // indent > stmt_indent: could be continuation of a previous statement,
        // keep looking backwards
    }
    // Reached beginning of file without finding scope opener — no right sibling
    false
}

/// Check if a byte slice starts with the `then` keyword at a word boundary.
/// Handles `then\n`, `then `, `then\t`, bare `then`, and `then;`.
fn is_then_keyword(rest: &[u8]) -> bool {
    rest.starts_with(b"then")
        && (rest.len() == 4
            || rest[4] == b' '
            || rest[4] == b'\t'
            || rest[4] == b'\n'
            || rest[4] == b'\r'
            || rest[4] == b';')
}

/// Check if the tail after a parsed predicate is only a line-continuation
/// backslash plus optional trailing whitespace.
///
/// Prism excludes the trailing `\` from the predicate range, but RuboCop's
/// `condition.last_line` semantics still treat the following physical line as
/// part of the condition. The `\` is syntax, not real content after the
/// condition, so it must not trigger a same-line offense.
fn is_line_continuation_tail(rest: &[u8]) -> bool {
    rest.starts_with(b"\\")
        && rest[1..]
            .iter()
            .all(|&b| b == b' ' || b == b'\t' || b == b'\r')
}

/// Count the number of leading whitespace characters in a line.
fn line_indent(line: &[u8]) -> usize {
    line.iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

/// Check if a predicate node represents a block call where the block delimiters
/// are on the same line. In RuboCop, `BlockNode.multiline?` checks `loc.begin.line
/// == loc.end.line` (the `{`/`}` or `do`/`end`), NOT the full expression range.
/// This means a multiline method chain with single-line `{ }` block (e.g.,
/// `items\n  .all? { |x| x.valid? }`) is NOT considered multiline.
fn is_single_line_block_condition(source: &SourceFile, predicate: &ruby_prism::Node<'_>) -> bool {
    // Check if the predicate is a CallNode with a block
    if let Some(call_node) = predicate.as_call_node() {
        if let Some(block) = call_node.block() {
            if let Some(block_node) = block.as_block_node() {
                let open_loc = block_node.opening_loc();
                let close_loc = block_node.closing_loc();
                let (open_line, _) = source.offset_to_line_col(open_loc.start_offset());
                let (close_line, _) = source.offset_to_line_col(close_loc.start_offset());
                return open_line == close_line;
            }
        }
    }
    false
}

/// Visitor that handles RescueNode (which Prism dispatches via visit_rescue_node,
/// not visit_branch_node_enter, so the CopWalker never sees it).
struct RescueVisitor<'a> {
    cop: &'a EmptyLineAfterMultilineCondition,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
}

impl<'pr> Visit<'pr> for RescueVisitor<'_> {
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        self.cop
            .check_rescue_node(self.source, node, &mut self.diagnostics);
        // Continue visiting for chained rescue clauses
        ruby_prism::visit_rescue_node(self, node);
    }
}

impl EmptyLineAfterMultilineCondition {
    fn check_multiline_condition(
        &self,
        source: &SourceFile,
        predicate: &ruby_prism::Node<'_>,
    ) -> Vec<Diagnostic> {
        let (pred_start_line, _) = source.offset_to_line_col(predicate.location().start_offset());
        let pred_end = predicate.location().end_offset().saturating_sub(1);
        let (pred_end_line, _) = source.offset_to_line_col(pred_end);

        // Only check multiline conditions — compare predicate's own start vs end line,
        // matching RuboCop's `condition.multiline?` which checks first_line vs last_line.
        if pred_start_line == pred_end_line {
            return Vec::new();
        }

        // If the condition is a block call with single-line delimiters, it's not
        // multiline per RuboCop's BlockNode.multiline? override.
        if is_single_line_block_condition(source, predicate) {
            return Vec::new();
        }

        let lines: Vec<&[u8]> = source.lines().collect();

        // Check if there's non-whitespace content on the condition-end line AFTER
        // the predicate. In minified code (e.g., `cond);next_stmt;end`), the
        // right sibling is on the same line — there's no empty line after the
        // condition, so fire. This must be checked before the next-line check
        // because a trailing newline creates a spurious blank "next line".
        if let Some(line_start) = source.line_col_to_offset(pred_end_line, 0) {
            let col = predicate.location().end_offset().saturating_sub(line_start);
            let cond_line = lines[pred_end_line - 1];
            if col < cond_line.len() {
                let tail = &cond_line[col..];
                // Skip trailing comments — they are not real content after the condition.
                // Find the first non-whitespace byte; if it's `#`, treat as blank tail.
                let first_non_ws = tail
                    .iter()
                    .position(|&b| b != b' ' && b != b'\t' && b != b'\r');
                if let Some(pos) = first_non_ws {
                    let rest = &tail[pos..];
                    // Skip comments and `then` keyword — both are not real
                    // content after the condition. `then` is a syntactic keyword
                    // of `if`/`elsif`/`unless` and should be ignored.
                    if rest[0] != b'#' && !is_then_keyword(rest) && !is_line_continuation_tail(rest)
                    {
                        let (line, col) =
                            source.offset_to_line_col(predicate.location().start_offset());
                        return vec![self.diagnostic(source, line, col, MSG.to_string())];
                    }
                }
            }
        }

        // The line after the condition ends
        let next_line_num = pred_end_line + 1;
        if next_line_num > lines.len() {
            return Vec::new();
        }

        let next_line = lines[next_line_num - 1];
        // Use is_blank_or_whitespace_line to match RuboCop's `blank?` which treats
        // whitespace-only lines as blank.
        if !is_blank_or_whitespace_line(next_line) {
            // Report offense at the condition (predicate) start, matching RuboCop's
            // `add_offense(condition)` which places the offense on the condition node.
            let (line, col) = source.offset_to_line_col(predicate.location().start_offset());
            return vec![self.diagnostic(source, line, col, MSG.to_string())];
        }

        Vec::new()
    }

    fn check_rescue_node(
        &self,
        source: &SourceFile,
        rescue_node: &ruby_prism::RescueNode<'_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let exceptions = rescue_node.exceptions();
        if exceptions.len() <= 1 {
            return;
        }

        let first = exceptions.iter().next().unwrap();
        let last = exceptions.iter().last().unwrap();
        let (first_line, _) = source.offset_to_line_col(first.location().start_offset());
        let last_end = last.location().end_offset().saturating_sub(1);
        let (last_line, _) = source.offset_to_line_col(last_end);

        if first_line == last_line {
            return;
        }

        let lines: Vec<&[u8]> = source.lines().collect();
        let next_line_num = last_line + 1;
        if next_line_num > lines.len() {
            return;
        }

        let next_line = lines[next_line_num - 1];
        if !is_blank_or_whitespace_line(next_line) {
            let kw_loc = rescue_node.keyword_loc();
            let (line, col) = source.offset_to_line_col(kw_loc.start_offset());
            diagnostics.push(self.diagnostic(source, line, col, MSG.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        EmptyLineAfterMultilineCondition,
        "cops/layout/empty_line_after_multiline_condition"
    );

    #[test]
    fn unless_multiline_condition() {
        let source = b"unless foo &&\n       bar\n  do_something\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(diags.len(), 1, "Expected one offense for unless");
    }

    #[test]
    fn elsif_multiline_condition() {
        let source =
            b"if condition\n  do_something\nelsif multiline &&\n   condition\n  do_something_else\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(diags.len(), 1, "Expected one offense for elsif");
    }

    #[test]
    fn rescue_multiline_exceptions() {
        let source = b"begin\n  do_something\nrescue FooError,\n  BarError\n  handle_error\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(diags.len(), 1, "Expected one offense for rescue");
    }

    #[test]
    fn case_when_multiline_condition() {
        let source = b"case x\nwhen foo,\n    bar\n  do_something\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(diags.len(), 1, "Expected one offense for case/when");
    }

    #[test]
    fn modifier_if_no_right_sibling() {
        let source = b"def m\n  do_something if multiline &&\n                condition\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "No offense when modifier if has no right sibling"
        );
    }

    #[test]
    fn fp_modifier_if_only_comment_after() {
        // Modifier if with multiline condition, only a comment follows (no real right sibling)
        let source = b"def m\n  true if depth >= 3 &&\n          caller.first.label == name\n          # TODO: incomplete\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire when only comment follows modifier if: {:?}",
            diags
        );
    }

    #[test]
    fn fp_next_if_multiline_at_end_of_block() {
        // next if with multiline condition at end of block
        let source =
            b"items.each do |l|\n  next if\n    # comment\n    l == :foo ||\n    l == :bar\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire on next if at end of block: {:?}",
            diags
        );
    }

    #[test]
    fn case_expression_predicate_is_offense() {
        // Multiline `case` expressions used as conditions are offenses in RuboCop.
        let source = b"if x\n  foo\nelsif case states.last\n      when :initial, :media\n        scan(/foo/)\n      end\n  bar\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should fire on elsif with multiline case predicate: {:?}",
            diags
        );
    }

    #[test]
    fn fp_whitespace_only_blank_line() {
        // Block if with whitespace-only line after condition (treated as blank by RuboCop)
        let source = b"if foo &&\n   bar\n    \n  do_something\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire when whitespace-only line follows condition: {:?}",
            diags
        );
    }

    #[test]
    fn fp_modifier_unless_before_when() {
        // Modifier unless inside when block — next when is not a right sibling
        let source = b"case parent\nwhen Step\n  return render_403 unless can_read?(proto) ||\n                           can_write?(proto)\nwhen Result\n  return render_403 unless can_read_result?(parent)\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire on modifier unless before when: {:?}",
            diags
        );
    }

    #[test]
    fn fp_unless_with_single_line_block_condition() {
        // unless with method chain on next line — block { } is single-line,
        // so condition is NOT multiline per RuboCop's BlockNode.multiline?
        let source = b"def m\n  unless %w[foo bar baz]\n      .all? { |name| File.exist? File.join(path, name) }\n    run(\"command\")\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire on unless with single-line block condition: {:?}",
            diags
        );
    }

    #[test]
    fn fn_modifier_while_non_begin_form() {
        // `nil while code.gsub!(...)` — non-begin modifier while with multiline condition.
        // RuboCop treats this as regular `while` (always check), not `while_post`.
        let source = b"nil while\n    code.gsub!(/pat/) {\n      result\n    }\ndo_something\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should fire on non-begin modifier while with multiline condition: {:?}",
            diags
        );
    }

    #[test]
    fn fn_modifier_if_wrapped_in_rescue_modifier() {
        let source = b"countderef[r.rexpr.name] += 1 if r.kind_of?(C::CExpression) and not r.op and r.rexpr.kind_of?(C::Variable) and\n                                 sizeof(nil, r.type.type) == sizeof(nil, r.rexpr.type.type) rescue nil\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should fire when modifier if is wrapped in rescue modifier: {:?}",
            diags
        );
    }

    #[test]
    fn no_offense_modifier_while_non_begin_at_end() {
        // nil while with multiline condition but no right sibling — RuboCop's on_while
        // always checks, so this IS an offense if the condition is multiline. But here
        // `code.gsub!() { }` has single-line block braces, so condition is NOT multiline.
        let source =
            b"def optimize(code)\n  code = code.dup\n  nil while\n    code.gsub!(/pattern/) { |f| f.upcase }\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire on modifier while with single-line block condition at end: {:?}",
            diags
        );
    }

    #[test]
    fn camping_modifier_if_in_block_closing_same_line() {
        // Camping minified code: two modifier ifs on the same expression.
        // Inner `break x if COND`: condition spans 2 lines, but `}}` at end of
        // condition closes the find block (scope closer) → no right sibling → no offense.
        // Outer `raise"bad route"if COND`: condition spans 2 lines, tail after
        // condition has `;h.any?? u : u` → right sibling → 1 offense.
        let source = b"module Helpers;def R c,*g;p,h=\n/\\(.+?\\)/,g.grep(Hash);g-=h;raise\"bad route\"if !u=c.urls.find{|x|break x if\nx.scan(p).size==g.size&&x.inject(x){|x,a|x.sub p,a}};h.any?? u : u\nend;def run(p) p end\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should fire once for outer modifier if with multiline condition and right sibling: {:?}",
            diags
        );
    }

    #[test]
    fn fp_camping_modifier_if_closing_brace_on_condition_line() {
        // Camping FP 2: modifier if where condition ends on same line as };end end
        // with more content on following lines
        let source = b"constants.map{|c|k=const_get(c);\nk.meta_def(:urls){[f(k,p)]} if (!k\n.respond_to?(:urls) || mu==true)};end end\nX=Controllers\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire on modifier if with }};end on condition line: {:?}",
            diags
        );
    }

    #[test]
    fn fn_camping_outer_modifier_if_multiline_condition_with_right_sibling() {
        // Camping FN: `raise"bad route"if !u=c.urls.find{|x|break x if
        // x.scan(p)...{$1})};h.any?? u : u`
        // The outer modifier `if` has a multiline condition (spans 2 lines).
        // After the condition, `};` closes the find block (part of condition),
        // then `h.any??...` is a right sibling on the same line.
        // tail_has_scope_closer incorrectly treats `}` as scope closer.
        let source = b"def m;raise\"bad route\"if !u=c.urls.find{|x|break x if\nx.scan(p).size==g.size&&x.inject(x){|x,a|x.sub p,a}};h.any?? u : u\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should fire on modifier if with multiline condition and right sibling after block close: {:?}",
            diags
        );
    }

    #[test]
    fn fn_camping_modifier_if_respond_to_multiline() {
        // Camping FN: modifier if with multiline condition spanning two lines,
        // with right sibling on the same line after `;`.
        let source = b"def m(c,a);c=R(c)if c.respond_to?(\n:urls);c=self;URI c end\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should fire on modifier if with multiline condition and right sibling: {:?}",
            diags
        );
    }

    #[test]
    fn fp_next_if_with_hash_arg_crlf_line_endings() {
        // next if with multiline condition in a CRLF file. The condition IS multiline,
        // the next line IS blank (\r\n). The \r in CRLF must be treated as whitespace
        // in the tail check, otherwise it triggers a false offense.
        let source = b"while self.rest?\r\n  next if scan_block( {\"(\"=>\")\", \"do\"=>\"end\", \"{\"=>\"}\"},\r\n                     /pattern/ )\r\n\r\n  next if scan_block( nil, /pat/, {'{' => '}', '[' => ']'} )\r\nend\r\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire when CRLF blank line follows multiline condition: {:?}",
            diags
        );
    }

    #[test]
    fn fp_modifier_unless_no_right_sibling_with_crlf_scope_closer() {
        // In CRLF files, line-based right-sibling checks must still recognize
        // `end\r` as a scope closer. Otherwise the scan looks past the closing
        // `end` and treats the next outer statement as a sibling.
        let source = b"def m\r\n  if outer\r\n    return false unless catch :found do\r\n      p = self\r\n    end\r\n  end\r\n  next_stmt\r\nend\r\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire when CRLF `end` closes the modifier scope: {:?}",
            diags
        );
    }

    #[test]
    fn fp_modifier_unless_inside_parenthesized_expression() {
        // Modifier unless inside parenthesized expression.
        // In RuboCop's Parser AST, parens create a begin node wrapping the modifier,
        // so right_sibling returns nil → no offense.
        let source = b"def m\n  mtime = (stat.mtime unless (last_modified = @last_modified[name] and\n                              stat.mtime.to_i <= last_modified.to_i))\n\n  if force_doc or can_parse?(name) then\n    do_something\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire on modifier unless inside parens: {:?}",
            diags
        );
    }

    #[test]
    fn fp_block_if_with_then_after_multiline_condition() {
        // Block if with `then` keyword after multiline parenthesized condition.
        // The `then` on the pred-end line is not real content — it's a syntactic
        // keyword. The blank line after `then` satisfies the empty line requirement.
        let source = b"if (\n  result.is_a?(Array) &&\n  result[0].is_a?(Class)\n) then\n\n  do_something\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert!(
            diags.is_empty(),
            "Should not fire on block if with then after multiline condition: {:?}",
            diags
        );
    }

    #[test]
    fn offense_if_with_multiline_do_end_block() {
        // if with do..end block condition — block delimiters on different lines → multiline
        let source = b"if items.find do |item|\n     item.ready?\n   end\n  process\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterMultilineCondition, source);
        assert_eq!(
            diags.len(),
            1,
            "Should fire on if with multiline do..end block condition: {:?}",
            diags
        );
    }
}
