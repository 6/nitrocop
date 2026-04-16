use crate::cop::shared::access_modifier_predicates;
use crate::cop::shared::node_type::{
    BEGIN_NODE, BLOCK_NODE, CALL_NODE, CASE_MATCH_NODE, CASE_NODE, CLASS_NODE, DEF_NODE, FOR_NODE,
    FORWARDING_SUPER_NODE, IF_NODE, IN_NODE, LAMBDA_NODE, MODULE_NODE, RESCUE_MODIFIER_NODE,
    SINGLETON_CLASS_NODE, STATEMENTS_NODE, SUPER_NODE, UNLESS_NODE, UNTIL_NODE, WHEN_NODE,
    WHILE_NODE,
};
use crate::cop::shared::util::assignment_context_base_col;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Layout/IndentationWidth checks that each body is indented by the configured
/// number of spaces (default 2) relative to its parent keyword/block.
///
/// ## Corpus investigation (2026-03-15)
///
/// Cached corpus oracle reported FP=58, FN=46,990.
///
/// 2026-03-09:
/// - Fixed FP sources from RuboCop's `skip_check?`: bodies that start with bare
///   access modifiers and bodies that are not the first non-whitespace token on
///   their line.
///
/// 2026-03-15:
/// - Remaining large FN volume came from class/module/sclass bodies only checking
///   the first child. RuboCop's `check_members` walks class/module members, checks
///   access modifier indentation, and honors
///   `Layout/IndentationConsistency: indented_internal_methods`.
/// - This port now mirrors that member walk for class/module/sclass bodies and for
///   block bodies that use `indented_internal_methods`, and it reads the sibling
///   `Layout/IndentationConsistency` / `Layout/AccessModifierIndentation` styles
///   through config injection.
///
/// 2026-03-16:
/// - Fixed 159 FPs on tab-indented code (47 from phlex alone). When tabs are used,
///   each tab counts as 1 character width, so a line indented with N+1 tabs relative
///   to N tabs has a "width" of 1, triggering "Use 2 (not 1) spaces for indentation."
///   RuboCop explicitly skips tab-indented lines in Layout/IndentationWidth — tab
///   indentation is handled by Layout/IndentationStyle instead. Added
///   `line_uses_tab_indentation()` check to all three indentation check methods.
///
/// 2026-04-01:
/// - Tab-indentation skip made conditional on `Layout/IndentationStyle: tabs`.
///   When IndentationStyle is 'spaces' (default), tabs count as 1 character
///   column and are flagged as "Use 2 (not 1) spaces", matching RuboCop's
///   behavior. Config injection reads the sibling cop's `EnforcedStyle` into
///   `IndentationStyleEnforced`. Resolved ~62,000 FN from the previous
///   unconditional tab skip.
///
/// 2026-04-04:
/// - Fixed def body with rescue/ensure: when a def has rescue/ensure, the body
///   is an implicit BeginNode. `check_body_indentation` returned empty because
///   BeginNode is not a StatementsNode. Now checks `begin_node.statements()`
///   directly. Resolved ~150 FN from previously unchecked def bodies.
/// - Fixed def base column: was using `end` keyword column as proxy for
///   `start_of_line` indentation, but RuboCop always uses the keyword line's
///   indentation (`node.loc.keyword`). Now uses `line_start_column` which
///   correctly handles `private def foo` (uses private's indent) and misaligned
///   end keywords.
/// - Fixed FP from `private :method`, `public :allocate`, etc. in class member
///   walk. RuboCop's `check_members_for_normal_style` skips ALL access modifier
///   calls (via `member.access_modifier?`), not just bare ones. Changed to use
///   `is_any_access_modifier_call` which matches all forms. Resolved ~83 FP.
///
/// 2026-04-05:
/// - Added lambda block handling (`-> do ... end`). LambdaNode was not in
///   `interested_node_types`, so lambda bodies were never checked.
/// - Added super block handling (`super(args) do ... end`). SuperNode blocks
///   were not checked since only CallNode blocks were handled.
/// - Fixed case/else body base: was using the `else` keyword column, but
///   RuboCop uses the LAST `when` keyword column as base for else body
///   indentation (matching `on_case` which calls
///   `check_indentation(when_branches.last.loc.keyword, else_branch)`).
///   Same fix applied to case/in pattern matching (uses last `in` keyword).
/// - Fixed first member access modifier with args: `select_check_member`
///   checks the first member even when it's `private :method` (access modifier
///   with args). Changed the first-member check from `is_access_modifier_call`
///   (bare only) to `is_any_access_modifier_call` (all forms).
///   Resolved 25+ FN, 0 regressions.
///
/// 2026-04-05 (block rescue/ensure):
/// - Fixed block body indentation when blocks have rescue/ensure/else clauses
///   (e.g., `items.each do ... rescue ... end`). In Prism, such blocks have a
///   BeginNode body instead of StatementsNode. `check_body_indentation` only
///   handled StatementsNode and returned empty for BeginNode, silently skipping
///   all indentation checks for these blocks. Applied same pattern as the
///   existing def handler: check `begin_node.statements()` for the main body
///   and `check_begin_clauses` for rescue/ensure/else bodies. Applied to
///   CallNode block, LambdaNode, and SuperNode block handlers.
///   Resolved 62+ FN, 0 regressions.
///
/// 2026-04-06:
/// - Added InNode (case/in pattern matching) body indentation check. The cop
///   handled WhenNode (case/when) and CaseMatchNode else clause, but never
///   checked InNode body indentation. Added IN_NODE to interested_node_types
///   and an InNode handler analogous to WhenNode. Resolved ~54 FN.
/// - Fixed UTF-8 BOM (U+FEFF) column miscalculation. Prism counts the 3-byte
///   BOM as 1 column, making keywords on line 1 appear at col 1 instead of 0.
///   Added `bom_adjusted_col` correction to all three check methods. Resolved
///   ~35 FP (webmachine, dryrun repos).
/// - Fixed `x = def foo` / `(def bar` body indentation. For defs preceded by
///   non-modifier tokens (assignment `=`, paren `(`), now uses the def keyword
///   column as base instead of `line_start_column`. Modifier-decorated defs
///   (`private def foo`) still use line_start_column. Resolved ~3 FP
///   (elasticgraph `__skip__ = def`).
///
/// 2026-04-06 (continued):
/// - Fixed begin/rescue/end inline FP. When `end` is not the first non-whitespace
///   character on its line (e.g., `begin ... rescue NameError; end` where the `end`
///   is on the same line as `rescue NameError;`), RuboCop's `on_kwbegin` skips the
///   indentation check entirely. Added `end_begins_its_line` helper and the check
///   to the BeginNode handler. Resolved 8+ FP (neo4jrb, foodsoft, activescaffold).
///
/// 2026-04-08:
/// - Fixed begin block alt_base FN. The begin handler used `alt_base = Some(kw_col)`
///   when `end` was at a different column than `begin`, accepting body indentation
///   relative to EITHER `end` or `begin`. But RuboCop's `on_kwbegin` only checks
///   against `node.loc.end`. Removed the alt_base so body is always checked against
///   the `end` keyword column only. This fixes cases like:
///   - `begin..end` where `end` is misindented (body at begin+2 passes alt check)
///   - `result = begin..end` where body aligns with `begin` keyword, not `end`
///
///   Resolved 16+ FN (Netflix Scumblr, DManga, idb, redcar, commitgpt, etc.).
///
/// 2026-04-10:
/// - Fixed multiline modifier `rescue` bodies (`expr rescue\n  fallback`). Prism
///   exposes these as `RescueModifierNode`, but RuboCop's Parser AST checks the
///   fallback expression like a rescue body indented from the `rescue` keyword.
///   We now check `rescue_expression()` against `keyword_loc()`, which also fixes
///   chained rescue modifiers where each fallback starts a new wrapped node.
/// - Fixed class/module/sclass bodies wrapped in an implicit `BeginNode` by
///   `rescue`/`ensure`. The member walker previously treated the whole implicit
///   begin wrapper as one member, so the first real statement on the next line
///   was never checked. We now unwrap implicit begin bodies to their statements
///   and check rescue/ensure/else clauses from the wrapper as well.
/// - Fixed forwarding `super do ... end` blocks. Prism uses `ForwardingSuperNode`
///   for bare `super` with a block, not `SuperNode`, so those block bodies were
///   skipped entirely. The block handler now covers both node types.
///
/// 2026-04-16:
/// - Fixed the `relative_to_receiver` batch-1 divergence. RuboCop does NOT skip
///   tab-indented lines when `Layout/IndentationStyle: tabs`; it compares visual
///   indentation columns and reports messages in tabs. The previous port skipped
///   those checks entirely and also used raw byte columns for mixed tab/space
///   indentation, which created both FPs and FNs.
/// - Fixed `EndAlignment: variable` for assignment `if`/`while`/`until`. The
///   body must align from the assignment target only; the previous `alt_base`
///   logic incorrectly accepted keyword-relative indentation that RuboCop flags.
/// - Fixed first-member `private :foo` / `public :foo` handling under
///   `IndentationConsistency: indented_internal_methods` +
///   `AccessModifierIndentation: outdent`. RuboCop's `select_check_member`
///   skips those access modifiers entirely, even with arguments.
/// - Matched a RuboCop 1.84.2 quirk for assignment-style `case` / `case in`
///   under `EndAlignment: variable`: when the branch body starts before the
///   `when` / `in` keyword column, RuboCop ends up with no offense, so we now
///   suppress that corner too.
pub struct IndentationWidth;

/// Check if a node is a bare access modifier call (for example `private` with no
/// receiver, args, or block). Matches RuboCop's `bare_access_modifier?`.
fn is_access_modifier_call(node: &ruby_prism::Node<'_>) -> bool {
    node.as_call_node()
        .is_some_and(|call| access_modifier_predicates::is_bare_access_modifier(&call))
}

/// Check if a node is ANY access modifier call (with or without arguments).
/// Matches RuboCop's `access_modifier?` which includes `private`, `private :method`,
/// and `private def foo`. Used in the normal-style member walk to skip access modifier
/// indentation (handled by Layout/AccessModifierIndentation instead).
fn is_any_access_modifier_call(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        if call.receiver().is_some() || call.block().is_some() {
            return false;
        }
        access_modifier_predicates::is_access_modifier_name(call.name().as_slice())
    } else {
        false
    }
}

/// Adjust a column for UTF-8 BOM on line 1. Prism counts the 3-byte BOM as 1 column,
/// but RuboCop strips it, so `module` right after BOM should be at column 0, not 1.
/// This matches the BOM correction used in EndAlignment, IndentationConsistency, etc.
fn bom_adjusted_col(source: &SourceFile, line: usize, col: usize) -> usize {
    if line == 1 {
        let bytes = source.as_bytes();
        if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
            return col.saturating_sub(1);
        }
    }
    col
}

/// Get the column of the first non-whitespace character on the line containing `offset`.
/// This gives the "effective indentation level" of the line, used as the base for
/// `start_of_line` alignment in def bodies (matching RuboCop's behavior of using the
/// def keyword line's indentation, not the end keyword's position).
fn line_start_column(source: &SourceFile, offset: usize) -> usize {
    let bytes = source.as_bytes();
    let mut line_start = offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let mut first_non_ws = line_start;
    while first_non_ws < bytes.len()
        && (bytes[first_non_ws] == b' ' || bytes[first_non_ws] == b'\t')
    {
        first_non_ws += 1;
    }
    first_non_ws - line_start
}

/// Check if the `def` keyword at `kw_offset` is preceded by a modifier identifier
/// (e.g., `private def foo`, `helper_method \` on the previous line). Returns
/// `Some(base_col)` with the correct base column for indentation when a modifier is
/// found, or `None` for non-modifier contexts like `x = def foo` or `(def bar`.
///
/// Handles two cases:
/// 1. Same-line modifier: `private def foo` — returns line_start_column of the def line.
/// 2. Backslash continuation: `helper_method \` / `  def foo` — returns
///    line_start_column of the previous (modifier) line, matching RuboCop's
///    `on_send` + `leftmost_modifier_of` behavior.
fn def_modifier_base_col(source: &SourceFile, kw_offset: usize) -> Option<usize> {
    if kw_offset == 0 {
        return None;
    }
    let bytes = source.as_bytes();
    // Walk back to find start of line
    let mut line_start = kw_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    // If def is at the start of the line (after indentation), there's no same-line modifier
    let first_non_ws = {
        let mut p = line_start;
        while p < bytes.len() && (bytes[p] == b' ' || bytes[p] == b'\t') {
            p += 1;
        }
        p
    };
    if first_non_ws == kw_offset {
        // def is the first token on its line — check if the previous line ends with
        // backslash (line continuation), indicating a modifier on the previous line.
        if line_start > 0 {
            // line_start points to the first byte of this line; line_start-1 is '\n'.
            let prev_line_end = line_start - 1;
            if prev_line_end > 0 {
                // Find the last non-whitespace char before the newline
                let mut p = prev_line_end;
                while p > 0 && (bytes[p - 1] == b' ' || bytes[p - 1] == b'\t') {
                    p -= 1;
                }
                if p > 0 && bytes[p - 1] == b'\\' {
                    // Previous line ends with backslash — use its start column
                    return Some(line_start_column(source, p - 1));
                }
            }
        }
        return None;
    }
    // Skip whitespace before `def`
    let mut pos = kw_offset;
    while pos > line_start && (bytes[pos - 1] == b' ' || bytes[pos - 1] == b'\t') {
        pos -= 1;
    }
    if pos == line_start {
        return None;
    }
    // Check if the character immediately before is alphanumeric/underscore
    let prev_byte = bytes[pos - 1];
    if prev_byte.is_ascii_alphanumeric() || prev_byte == b'_' {
        Some(line_start_column(source, kw_offset))
    } else {
        None
    }
}

fn body_members(body: ruby_prism::Node<'_>) -> Vec<ruby_prism::Node<'_>> {
    if let Some(stmts) = body.as_statements_node() {
        stmts.body().iter().collect()
    } else {
        vec![body]
    }
}

fn body_contains_access_modifier(body: Option<ruby_prism::Node<'_>>) -> bool {
    body.map(body_members)
        .unwrap_or_default()
        .iter()
        .any(is_access_modifier_call)
}

/// Check if a StatementsNode's first child is a bare access modifier.
/// Matches RuboCop's `starts_with_access_modifier?` which checks if the body
/// (when it's a `begin` type / StatementsNode) starts with an access modifier.
fn starts_with_access_modifier(stmts: &ruby_prism::StatementsNode<'_>) -> bool {
    if let Some(first) = stmts.body().iter().next() {
        is_access_modifier_call(&first)
    } else {
        false
    }
}

/// Check if the line at the given byte offset has a tab in its leading indentation.
/// RuboCop uses visual columns when `Layout/IndentationStyle` is `tabs` and either
/// compared line uses tabs.
fn line_uses_tab_indentation(source: &SourceFile, body_offset: usize) -> bool {
    let bytes = source.as_bytes();
    let mut line_start = body_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    // Check if any leading whitespace character is a tab
    let mut pos = line_start;
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        if bytes[pos] == b'\t' {
            return true;
        }
        pos += 1;
    }
    false
}

/// Compute the visual indentation column for the token that starts at `offset`.
/// Tabs count as the configured indentation width, matching RuboCop's
/// `visual_column` helper when `Layout/IndentationStyle` is `tabs`.
fn visual_indentation_column(source: &SourceFile, offset: usize, width: usize) -> usize {
    let bytes = source.as_bytes();
    let mut line_start = offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    let mut column = 0;
    for &byte in &bytes[line_start..offset] {
        match byte {
            b' ' => column += 1,
            b'\t' => column += width,
            _ => break,
        }
    }
    column
}

/// Check if the `end` keyword is the first non-whitespace character on its line.
/// RuboCop's `on_kwbegin` skips indentation check when the `end` keyword is
/// inline with other code (e.g., `begin ... rescue NameError; end`).
/// This matches RuboCop's `begins_its_line?(node.loc.end)` check.
fn end_begins_its_line(source: &SourceFile, end_offset: usize) -> bool {
    let bytes = source.as_bytes();
    // Find the start of the line containing the end keyword
    let mut line_start = end_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    // Find the first non-whitespace character on this line
    let mut first_non_ws = line_start;
    while first_non_ws < bytes.len()
        && (bytes[first_non_ws] == b' ' || bytes[first_non_ws] == b'\t')
    {
        first_non_ws += 1;
    }
    // Find the end keyword's column
    let end_col = end_offset - line_start;
    // Check if the first non-whitespace is at the same column as end
    first_non_ws - line_start == end_col
}

/// Check if the body node is not the first non-whitespace character on its line.
/// RuboCop's `skip_check?` skips indentation check when the body doesn't start
/// at the beginning of its line (e.g., `else do_something` on one line).
fn body_not_first_on_line(source: &SourceFile, body_col: usize, body_offset: usize) -> bool {
    // Walk backward from body_offset to find the start of the line
    let bytes = source.as_bytes();
    let mut line_start = body_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    // Find the first non-whitespace character on this line
    let mut first_non_ws = line_start;
    while first_non_ws < bytes.len()
        && (bytes[first_non_ws] == b' ' || bytes[first_non_ws] == b'\t')
    {
        first_non_ws += 1;
    }
    let first_col = first_non_ws - line_start;
    body_col != first_col
}

struct MemberStyles<'a> {
    access_modifier: &'a str,
    consistency: &'a str,
}

#[derive(Clone, Copy)]
struct IndentationOptions {
    width: usize,
    use_tabs: bool,
}

impl IndentationWidth {
    fn indentation_message(
        &self,
        width: usize,
        actual_indent: isize,
        use_tabs: bool,
        style_name: Option<&str>,
    ) -> String {
        if use_tabs {
            let actual_tabs = actual_indent / width as isize;
            match style_name {
                Some(style_name) => {
                    format!(
                        "Use 1 (not {}) tabs for {} indentation.",
                        actual_tabs, style_name
                    )
                }
                None => format!("Use 1 (not {}) tabs for indentation.", actual_tabs),
            }
        } else {
            match style_name {
                Some(style_name) => {
                    format!(
                        "Use {} (not {}) spaces for {} indentation.",
                        width, actual_indent, style_name
                    )
                }
                None => format!(
                    "Use {} (not {}) spaces for indentation.",
                    width, actual_indent
                ),
            }
        }
    }

    fn actual_indentation(
        &self,
        source: &SourceFile,
        base_offset: usize,
        base_line: usize,
        base_col: usize,
        body_offset: usize,
        body_line: usize,
        body_col: usize,
        options: IndentationOptions,
    ) -> isize {
        let base_col = bom_adjusted_col(source, base_line, base_col);
        let body_col = bom_adjusted_col(source, body_line, body_col);

        if options.use_tabs
            && (line_uses_tab_indentation(source, base_offset)
                || line_uses_tab_indentation(source, body_offset))
        {
            return visual_indentation_column(source, body_offset, options.width) as isize
                - visual_indentation_column(source, base_offset, options.width) as isize;
        }

        body_col as isize - base_col as isize
    }

    fn in_variable_style_assignment_case(
        &self,
        source: &SourceFile,
        case_keyword_offset: usize,
        end_style: &str,
    ) -> bool {
        end_style == "variable"
            && assignment_context_base_col(source, case_keyword_offset).is_some()
    }

    fn check_member_indentation(
        &self,
        source: &SourceFile,
        base_offset: usize,
        base_col: usize,
        member: &ruby_prism::Node<'_>,
        options: IndentationOptions,
        style_name: Option<&str>,
    ) -> Option<Diagnostic> {
        let (base_line, _) = source.offset_to_line_col(base_offset);
        let loc = member.location();
        let (member_line, member_col) = source.offset_to_line_col(loc.start_offset());
        let member_col = bom_adjusted_col(source, member_line, member_col);

        if member_line == base_line {
            return None;
        }

        if body_not_first_on_line(source, member_col, loc.start_offset()) {
            return None;
        }

        let actual_indent = self.actual_indentation(
            source,
            base_offset,
            base_line,
            base_col,
            loc.start_offset(),
            member_line,
            member_col,
            options,
        );
        if actual_indent == options.width as isize {
            return None;
        }

        Some(self.diagnostic(
            source,
            member_line,
            member_col,
            self.indentation_message(options.width, actual_indent, options.use_tabs, style_name),
        ))
    }

    fn check_class_like_members(
        &self,
        source: &SourceFile,
        base_offset: usize,
        base_col: usize,
        body: Option<ruby_prism::Node<'_>>,
        options: IndentationOptions,
        styles: MemberStyles<'_>,
    ) -> Vec<Diagnostic> {
        let body = match body {
            Some(body) => body,
            None => return Vec::new(),
        };

        let implicit_begin = body
            .as_begin_node()
            .filter(|begin_node| begin_node.begin_keyword_loc().is_none());
        let members = if let Some(ref begin_node) = implicit_begin {
            begin_node
                .statements()
                .map(|stmts| stmts.body().iter().collect())
                .unwrap_or_default()
        } else {
            body_members(body)
        };
        if members.is_empty() {
            return Vec::new();
        }

        let (base_line, _) = source.offset_to_line_col(base_offset);
        let first = &members[0];
        let (first_line, _) = source.offset_to_line_col(first.location().start_offset());
        if first_line == base_line {
            return Vec::new();
        }

        let mut diagnostics = Vec::new();

        if styles.consistency == "indented_internal_methods" {
            if is_any_access_modifier_call(first) {
                if styles.access_modifier != "outdent" {
                    if let Some(diagnostic) = self.check_member_indentation(
                        source,
                        base_offset,
                        base_col,
                        first,
                        options,
                        None,
                    ) {
                        diagnostics.push(diagnostic);
                    }
                }
            } else if let Some(diagnostic) =
                self.check_member_indentation(source, base_offset, base_col, first, options, None)
            {
                diagnostics.push(diagnostic);
            }

            let mut previous_modifier: Option<&ruby_prism::Node<'_>> = None;
            for member in &members {
                if is_access_modifier_call(member) {
                    previous_modifier = Some(member);
                    continue;
                }

                if let Some(modifier) = previous_modifier.take() {
                    let modifier_loc = modifier.location();
                    let (_, modifier_col) = source.offset_to_line_col(modifier_loc.start_offset());
                    if let Some(diagnostic) = self.check_member_indentation(
                        source,
                        modifier_loc.start_offset(),
                        modifier_col,
                        member,
                        options,
                        Some("indented_internal_methods"),
                    ) {
                        diagnostics.push(diagnostic);
                    }
                }
            }

            return diagnostics;
        }

        // RuboCop's `select_check_member` checks the first member specially.
        // If the first member is ANY access modifier (bare or with args), it is
        // checked here (unless `outdent` style) and skipped in the loop below.
        // Non-access-modifier first members are handled by the loop.
        if is_any_access_modifier_call(first) && styles.access_modifier != "outdent" {
            if let Some(diagnostic) =
                self.check_member_indentation(source, base_offset, base_col, first, options, None)
            {
                diagnostics.push(diagnostic);
            }
        }

        // RuboCop's `check_members_for_normal_style` iterates all children and
        // skips access modifiers (bare, with symbol args, or with def).
        // Access modifier indentation is handled by Layout/AccessModifierIndentation.
        for member in &members {
            if is_any_access_modifier_call(member) {
                continue;
            }

            if let Some(diagnostic) =
                self.check_member_indentation(source, base_offset, base_col, member, options, None)
            {
                diagnostics.push(diagnostic);
            }
        }

        if let Some(begin_node) = implicit_begin {
            self.check_begin_clauses(source, &begin_node, options, &mut diagnostics);
        }

        diagnostics
    }

    fn check_block_internal_method_members(
        &self,
        source: &SourceFile,
        end_offset: usize,
        end_col: usize,
        body: Option<ruby_prism::Node<'_>>,
        options: IndentationOptions,
        access_modifier_style: &str,
    ) -> Vec<Diagnostic> {
        let body = match body {
            Some(body) => body,
            None => return Vec::new(),
        };

        let members = body_members(body);
        if members.is_empty() {
            return Vec::new();
        }

        let mut diagnostics = Vec::new();
        if is_access_modifier_call(&members[0]) && access_modifier_style != "outdent" {
            if let Some(diagnostic) = self.check_member_indentation(
                source,
                end_offset,
                end_col,
                &members[0],
                options,
                None,
            ) {
                diagnostics.push(diagnostic);
            }
        }

        let mut previous_modifier: Option<&ruby_prism::Node<'_>> = None;
        for member in &members {
            if is_access_modifier_call(member) {
                previous_modifier = Some(member);
                continue;
            }

            if let Some(modifier) = previous_modifier.take() {
                let modifier_loc = modifier.location();
                let (_, modifier_col) = source.offset_to_line_col(modifier_loc.start_offset());
                if let Some(diagnostic) = self.check_member_indentation(
                    source,
                    modifier_loc.start_offset(),
                    modifier_col,
                    member,
                    options,
                    Some("indented_internal_methods"),
                ) {
                    diagnostics.push(diagnostic);
                }
            }
        }

        diagnostics
    }

    /// Check body indentation.
    /// `keyword_offset` is used to determine which line the keyword is on (for same-line skip).
    /// `base_col` is the column that expected indentation is relative to.
    fn check_body_indentation(
        &self,
        source: &SourceFile,
        keyword_offset: usize,
        base_col: usize,
        body: Option<ruby_prism::Node<'_>>,
        options: IndentationOptions,
    ) -> Vec<Diagnostic> {
        let body = match body {
            Some(b) => b,
            None => return Vec::new(),
        };

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return Vec::new(),
        };

        // Skip if body starts with access modifier (RuboCop's starts_with_access_modifier?)
        if starts_with_access_modifier(&stmts) {
            return Vec::new();
        }

        let children: Vec<_> = stmts.body().iter().collect();
        if children.is_empty() {
            return Vec::new();
        }

        let (kw_line, _) = source.offset_to_line_col(keyword_offset);

        // Only check the first child's indentation. Sibling consistency is
        // handled by Layout/IndentationConsistency.
        let first = &children[0];
        let loc = first.location();
        let (child_line, child_col) = source.offset_to_line_col(loc.start_offset());
        let child_col = bom_adjusted_col(source, child_line, child_col);

        // Skip if body is on same line as keyword (single-line construct)
        if child_line == kw_line {
            return Vec::new();
        }

        // Skip if body is not the first non-whitespace char on its line
        // (e.g., `else do_something` on one line)
        if body_not_first_on_line(source, child_col, loc.start_offset()) {
            return Vec::new();
        }

        let actual_indent = self.actual_indentation(
            source,
            keyword_offset,
            kw_line,
            base_col,
            loc.start_offset(),
            child_line,
            child_col,
            options,
        );
        if actual_indent != options.width as isize {
            return vec![self.diagnostic(
                source,
                child_line,
                child_col,
                self.indentation_message(options.width, actual_indent, options.use_tabs, None),
            )];
        }

        Vec::new()
    }

    fn check_statements_indentation(
        &self,
        source: &SourceFile,
        keyword_offset: usize,
        base_col: usize,
        stmts: Option<ruby_prism::StatementsNode<'_>>,
        options: IndentationOptions,
    ) -> Vec<Diagnostic> {
        let stmts = match stmts {
            Some(s) => s,
            None => return Vec::new(),
        };

        let children: Vec<_> = stmts.body().iter().collect();
        if children.is_empty() {
            return Vec::new();
        }

        let (kw_line, _) = source.offset_to_line_col(keyword_offset);

        // Only check the first child's indentation. Sibling consistency is
        // handled by Layout/IndentationConsistency.
        let first = &children[0];
        let loc = first.location();
        let (child_line, child_col) = source.offset_to_line_col(loc.start_offset());
        let child_col = bom_adjusted_col(source, child_line, child_col);

        // Skip if body is on same line as keyword (single-line construct)
        // or before the keyword (modifier if/while/until)
        if child_line <= kw_line {
            return Vec::new();
        }

        // Skip if body is not the first non-whitespace char on its line
        if body_not_first_on_line(source, child_col, loc.start_offset()) {
            return Vec::new();
        }

        let actual_indent = self.actual_indentation(
            source,
            keyword_offset,
            kw_line,
            base_col,
            loc.start_offset(),
            child_line,
            child_col,
            options,
        );
        if actual_indent != options.width as isize {
            return vec![self.diagnostic(
                source,
                child_line,
                child_col,
                self.indentation_message(options.width, actual_indent, options.use_tabs, None),
            )];
        }

        Vec::new()
    }

    /// Check rescue/ensure/else clauses on a BeginNode. These nodes bypass
    /// the generic visit_branch_node_enter callback, so they must be checked
    /// from their parent.
    fn check_begin_clauses(
        &self,
        source: &SourceFile,
        begin_node: &ruby_prism::BeginNode<'_>,
        options: IndentationOptions,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // Check rescue clause(s)
        let mut rescue_opt = begin_node.rescue_clause();
        while let Some(rescue_node) = rescue_opt {
            let kw_offset = rescue_node.keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                kw_col,
                rescue_node.statements(),
                options,
            ));
            rescue_opt = rescue_node.subsequent();
        }

        // Check else clause (in begin/rescue/else/end)
        if let Some(else_clause) = begin_node.else_clause() {
            let kw_offset = else_clause.else_keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                kw_col,
                else_clause.statements(),
                options,
            ));
        }

        // Check ensure clause
        if let Some(ensure_node) = begin_node.ensure_clause() {
            let kw_offset = ensure_node.ensure_keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                kw_col,
                ensure_node.statements(),
                options,
            ));
        }
    }

    /// Check else body indentation for an if/unless subsequent (ElseNode).
    /// ElseNode bypasses visit_branch_node_enter, so must be checked from
    /// the parent IfNode/UnlessNode.
    fn check_else_clause(
        &self,
        source: &SourceFile,
        else_node: &ruby_prism::ElseNode<'_>,
        options: IndentationOptions,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let kw_offset = else_node.else_keyword_loc().start_offset();
        let (_, kw_col) = source.offset_to_line_col(kw_offset);
        diagnostics.extend(self.check_statements_indentation(
            source,
            kw_offset,
            kw_col,
            else_node.statements(),
            options,
        ));
    }
}

impl Cop for IndentationWidth {
    fn name(&self) -> &'static str {
        "Layout/IndentationWidth"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            BEGIN_NODE,
            BLOCK_NODE,
            CALL_NODE,
            CASE_MATCH_NODE,
            CASE_NODE,
            CLASS_NODE,
            DEF_NODE,
            FOR_NODE,
            FORWARDING_SUPER_NODE,
            IF_NODE,
            IN_NODE,
            LAMBDA_NODE,
            MODULE_NODE,
            RESCUE_MODIFIER_NODE,
            SINGLETON_CLASS_NODE,
            STATEMENTS_NODE,
            SUPER_NODE,
            UNLESS_NODE,
            UNTIL_NODE,
            WHEN_NODE,
            WHILE_NODE,
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
        let width = config.get_usize("Width", 2);
        let align_style = config.get_str("EnforcedStyleAlignWith", "start_of_line");
        let consistency_style = config.get_str("IndentationConsistencyStyle", "normal");
        let access_modifier_style = config.get_str("AccessModifierIndentationStyle", "indent");
        let indentation_style = config.get_str("IndentationStyleEnforced", "spaces");
        let options = IndentationOptions {
            width,
            use_tabs: indentation_style == "tabs",
        };
        let allowed_patterns = config
            .get_string_array("AllowedPatterns")
            .unwrap_or_default();

        // Skip if the node's source line matches any allowed pattern
        if !allowed_patterns.is_empty() {
            let (node_line, _) = source.offset_to_line_col(node.location().start_offset());
            if let Some(line_bytes) = source.lines().nth(node_line - 1) {
                if let Ok(line_str) = std::str::from_utf8(line_bytes) {
                    for pattern in &allowed_patterns {
                        if let Ok(re) = regex::Regex::new(pattern) {
                            if re.is_match(line_str) {
                                return;
                            }
                        }
                    }
                }
            }
        }

        // begin...end blocks (Prism's BeginNode for explicit `begin` keyword).
        // RuboCop checks body indentation relative to the `end` keyword, not
        // the `begin` keyword. This handles assignment context correctly:
        //   x = begin
        //     body       # indented from `end`, not from `begin`
        //   end
        //
        // RuboCop's `on_kwbegin` also checks `begins_its_line?(node.loc.end)` —
        // if the `end` keyword is NOT the first non-whitespace on its line
        // (e.g., `begin ... rescue NameError; end` where end is inline with code),
        // indentation is NOT checked. This avoids false positives for inline
        // begin/rescue/end constructs.
        if let Some(begin_node) = node.as_begin_node() {
            if let Some(begin_kw_loc) = begin_node.begin_keyword_loc() {
                // Explicit `begin...end` block
                let kw_offset = begin_kw_loc.start_offset();
                let (_, kw_col) = source.offset_to_line_col(kw_offset);
                let (base_col, end_offset) = if let Some(end_loc) = begin_node.end_keyword_loc() {
                    (
                        source.offset_to_line_col(end_loc.start_offset()).1,
                        Some(end_loc.start_offset()),
                    )
                } else {
                    (kw_col, None)
                };
                // Skip indentation check if `end` is not on its own line
                // (RuboCop's `begins_its_line?` check)
                if let Some(eo) = end_offset {
                    if !end_begins_its_line(source, eo) {
                        // Still check rescue/ensure/else clauses (these bypass the walker)
                        self.check_begin_clauses(source, &begin_node, options, diagnostics);
                        return;
                    }
                }
                diagnostics.extend(self.check_statements_indentation(
                    source,
                    kw_offset,
                    base_col,
                    begin_node.statements(),
                    options,
                ));
                // Check rescue/ensure/else clauses (these bypass the walker)
                self.check_begin_clauses(source, &begin_node, options, diagnostics);
            }
            // Implicit BeginNode (e.g., `def...rescue...end`) — clauses are
            // checked by the parent DefNode handler, skip here to avoid dupes.
            return;
        }

        if let Some(class_node) = node.as_class_node() {
            let kw_offset = class_node.class_keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_class_like_members(
                source,
                kw_offset,
                kw_col,
                class_node.body(),
                options,
                MemberStyles {
                    access_modifier: access_modifier_style,
                    consistency: consistency_style,
                },
            ));
            return;
        }

        if let Some(sclass_node) = node.as_singleton_class_node() {
            let kw_offset = sclass_node.class_keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_class_like_members(
                source,
                kw_offset,
                kw_col,
                sclass_node.body(),
                options,
                MemberStyles {
                    access_modifier: access_modifier_style,
                    consistency: consistency_style,
                },
            ));
            return;
        }

        if let Some(module_node) = node.as_module_node() {
            let kw_offset = module_node.module_keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_class_like_members(
                source,
                kw_offset,
                kw_col,
                module_node.body(),
                options,
                MemberStyles {
                    access_modifier: access_modifier_style,
                    consistency: consistency_style,
                },
            ));
            return;
        }

        if let Some(rescue_modifier_node) = node.as_rescue_modifier_node() {
            let kw_offset = rescue_modifier_node.keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            let rescue_expression = rescue_modifier_node.rescue_expression();
            if let Some(diagnostic) = self.check_member_indentation(
                source,
                kw_offset,
                kw_col,
                &rescue_expression,
                options,
                None,
            ) {
                diagnostics.push(diagnostic);
            }
            return;
        }

        if let Some(def_node) = node.as_def_node() {
            let kw_offset = def_node.def_keyword_loc().start_offset();
            let base_col = if align_style == "keyword" {
                // EnforcedStyleAlignWith: keyword — indent relative to `def` keyword column
                source.offset_to_line_col(kw_offset).1
            } else {
                // EnforcedStyleAlignWith: start_of_line (default).
                // RuboCop's on_def always uses node.loc.keyword (def column).
                // For `private def foo`, RuboCop handles it via on_send and
                // ignores the def in on_def. We don't have that mechanism, so
                // we use line_start_column for modifier-decorated defs (which
                // matches on_send using leftmost_modifier_of). For non-modifier
                // contexts like `x = def foo` or `(def bar`, use the def
                // keyword column to match RuboCop's on_def behavior.
                if let Some(modifier_col) = def_modifier_base_col(source, kw_offset) {
                    modifier_col
                } else {
                    source.offset_to_line_col(kw_offset).1
                }
            };

            if let Some(body) = def_node.body() {
                if let Some(begin_node) = body.as_begin_node() {
                    // Implicit begin (def with rescue/ensure/else).
                    // Check the main body statements.
                    diagnostics.extend(self.check_statements_indentation(
                        source,
                        kw_offset,
                        base_col,
                        begin_node.statements(),
                        options,
                    ));
                    // Check rescue/ensure/else clauses.
                    self.check_begin_clauses(source, &begin_node, options, diagnostics);
                } else {
                    // Regular def body (StatementsNode).
                    diagnostics.extend(self.check_body_indentation(
                        source,
                        kw_offset,
                        base_col,
                        Some(body),
                        options,
                    ));
                }
            }
            return;
        }

        if let Some(if_node) = node.as_if_node() {
            if let Some(kw_loc) = if_node.if_keyword_loc() {
                let kw_offset = kw_loc.start_offset();
                let (_, kw_col) = source.offset_to_line_col(kw_offset);

                // When `if` is the RHS of an assignment (e.g., `x = if cond`) and
                // Layout/EndAlignment.EnforcedStyleAlignWith is "variable", body
                // indentation is relative to the assignment variable, not `if`.
                let end_style = config.get_str("EndAlignmentStyle", "keyword");
                let base_col = if end_style == "variable" {
                    if let Some(var_col) = assignment_context_base_col(source, kw_offset) {
                        var_col
                    } else {
                        kw_col
                    }
                } else {
                    kw_col
                };

                diagnostics.extend(self.check_statements_indentation(
                    source,
                    kw_offset,
                    base_col,
                    if_node.statements(),
                    options,
                ));
                // Check else body (ElseNode bypasses the walker).
                // elsif is another IfNode that will be visited directly.
                if let Some(subsequent) = if_node.subsequent() {
                    if let Some(else_node) = subsequent.as_else_node() {
                        self.check_else_clause(source, &else_node, options, diagnostics);
                    }
                }
                return;
            }
        }

        if let Some(unless_node) = node.as_unless_node() {
            let kw_offset = unless_node.keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                kw_col,
                unless_node.statements(),
                options,
            ));
            // Check else clause (ElseNode bypasses the walker)
            if let Some(else_clause) = unless_node.else_clause() {
                self.check_else_clause(source, &else_clause, options, diagnostics);
            }
            return;
        }

        // Handle for loop body indentation.
        if let Some(for_node) = node.as_for_node() {
            let kw_offset = for_node.for_keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);
            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                kw_col,
                for_node.statements(),
                options,
            ));
            return;
        }

        // Handle block body indentation from CallNode (since BlockNode is
        // always a child of CallNode in Prism, and we need access to the
        // call's dot for chained method detection).
        if let Some(call_node) = node.as_call_node() {
            if let Some(block_ref) = call_node.block() {
                if let Some(block) = block_ref.as_block_node() {
                    let opening_offset = block.opening_loc().start_offset();
                    let closing_offset = block.closing_loc().start_offset();
                    let (_, closing_col) = source.offset_to_line_col(closing_offset);

                    // Skip if closing brace/end is not on its own line (inline
                    // block that wraps, e.g., `lambda { |req|\n  body }`).
                    let bytes = source.as_bytes();
                    let mut line_start = closing_offset;
                    while line_start > 0 && bytes[line_start - 1] != b'\n' {
                        line_start -= 1;
                    }
                    if !bytes[line_start..closing_offset]
                        .iter()
                        .all(|&b| b == b' ' || b == b'\t')
                    {
                        return;
                    }

                    // Skip if block parameters are on the same line as the
                    // first body statement (e.g., `reject { \n |x| body }`).
                    if let Some(params) = block.parameters() {
                        if let Some(body_node) = block.body() {
                            if let Some(stmts) = body_node.as_statements_node() {
                                if let Some(first) = stmts.body().iter().next() {
                                    let (params_line, _) =
                                        source.offset_to_line_col(params.location().end_offset());
                                    let (first_line, _) =
                                        source.offset_to_line_col(first.location().start_offset());
                                    if first_line == params_line {
                                        return;
                                    }
                                }
                            }
                        }
                    }

                    // Determine base column: if the call's dot is on a new line
                    // relative to its receiver (multiline chain), use the dot column
                    // as the base (matching RuboCop's `block_body_indentation_base`).
                    // Otherwise, use the `end`/`}` keyword column.
                    let base_col = if let Some(dot_loc) = call_node.call_operator_loc() {
                        if let Some(receiver) = call_node.receiver() {
                            let (recv_end_line, _) =
                                source.offset_to_line_col(receiver.location().end_offset());
                            let (dot_line, dot_col) =
                                source.offset_to_line_col(dot_loc.start_offset());
                            if dot_line > recv_end_line {
                                dot_col
                            } else {
                                closing_col
                            }
                        } else {
                            closing_col
                        }
                    } else {
                        closing_col
                    };
                    if let Some(body) = block.body() {
                        if let Some(begin_node) = body.as_begin_node() {
                            // Block with rescue/ensure — body is implicit BeginNode.
                            // Check main body statements.
                            diagnostics.extend(self.check_statements_indentation(
                                source,
                                opening_offset,
                                base_col,
                                begin_node.statements(),
                                options,
                            ));
                            // Check rescue/ensure/else clauses.
                            self.check_begin_clauses(source, &begin_node, options, diagnostics);
                        } else {
                            diagnostics.extend(self.check_body_indentation(
                                source,
                                opening_offset,
                                base_col,
                                Some(body),
                                options,
                            ));
                        }
                    }
                    if consistency_style == "indented_internal_methods"
                        && body_contains_access_modifier(block.body())
                    {
                        diagnostics.extend(self.check_block_internal_method_members(
                            source,
                            closing_offset,
                            closing_col,
                            block.body(),
                            options,
                            access_modifier_style,
                        ));
                    }
                    return;
                }
            }
        }

        // Lambda blocks (-> do ... end / -> { ... }). Treated like blocks
        // by RuboCop's on_block handler. No dot/receiver, so base is always
        // the closing location (end/}).
        if let Some(lambda_node) = node.as_lambda_node() {
            let opening_offset = lambda_node.opening_loc().start_offset();
            let closing_offset = lambda_node.closing_loc().start_offset();
            let (_, closing_col) = source.offset_to_line_col(closing_offset);

            // Skip if closing brace/end is not on its own line
            let bytes = source.as_bytes();
            let mut line_start = closing_offset;
            while line_start > 0 && bytes[line_start - 1] != b'\n' {
                line_start -= 1;
            }
            if !bytes[line_start..closing_offset]
                .iter()
                .all(|&b| b == b' ' || b == b'\t')
            {
                return;
            }

            if let Some(body) = lambda_node.body() {
                if let Some(begin_node) = body.as_begin_node() {
                    diagnostics.extend(self.check_statements_indentation(
                        source,
                        opening_offset,
                        closing_col,
                        begin_node.statements(),
                        options,
                    ));
                    self.check_begin_clauses(source, &begin_node, options, diagnostics);
                } else {
                    diagnostics.extend(self.check_body_indentation(
                        source,
                        opening_offset,
                        closing_col,
                        Some(body),
                        options,
                    ));
                }
            }
            return;
        }

        // Super with block (super(args) do ... end). Extract the block and
        // check indentation like a non-chained block.
        if let Some(super_node) = node.as_super_node() {
            if let Some(block_ref) = super_node.block() {
                if let Some(block) = block_ref.as_block_node() {
                    let opening_offset = block.opening_loc().start_offset();
                    let closing_offset = block.closing_loc().start_offset();
                    let (_, closing_col) = source.offset_to_line_col(closing_offset);

                    // Skip if closing brace/end is not on its own line
                    let bytes = source.as_bytes();
                    let mut line_start = closing_offset;
                    while line_start > 0 && bytes[line_start - 1] != b'\n' {
                        line_start -= 1;
                    }
                    if !bytes[line_start..closing_offset]
                        .iter()
                        .all(|&b| b == b' ' || b == b'\t')
                    {
                        return;
                    }

                    if let Some(body) = block.body() {
                        if let Some(begin_node) = body.as_begin_node() {
                            diagnostics.extend(self.check_statements_indentation(
                                source,
                                opening_offset,
                                closing_col,
                                begin_node.statements(),
                                options,
                            ));
                            self.check_begin_clauses(source, &begin_node, options, diagnostics);
                        } else {
                            diagnostics.extend(self.check_body_indentation(
                                source,
                                opening_offset,
                                closing_col,
                                Some(body),
                                options,
                            ));
                        }
                    }
                }
            }
            return;
        }

        // Forwarding super with block (`super do ... end`). Prism represents
        // bare super-with-block separately from SuperNode.
        if let Some(forwarding_super_node) = node.as_forwarding_super_node() {
            if let Some(block) = forwarding_super_node.block() {
                let opening_offset = block.opening_loc().start_offset();
                let closing_offset = block.closing_loc().start_offset();
                let (_, closing_col) = source.offset_to_line_col(closing_offset);

                let bytes = source.as_bytes();
                let mut line_start = closing_offset;
                while line_start > 0 && bytes[line_start - 1] != b'\n' {
                    line_start -= 1;
                }
                if !bytes[line_start..closing_offset]
                    .iter()
                    .all(|&b| b == b' ' || b == b'\t')
                {
                    return;
                }

                if let Some(body) = block.body() {
                    if let Some(begin_node) = body.as_begin_node() {
                        diagnostics.extend(self.check_statements_indentation(
                            source,
                            opening_offset,
                            closing_col,
                            begin_node.statements(),
                            options,
                        ));
                        self.check_begin_clauses(source, &begin_node, options, diagnostics);
                    } else {
                        diagnostics.extend(self.check_body_indentation(
                            source,
                            opening_offset,
                            closing_col,
                            Some(body),
                            options,
                        ));
                    }
                }
            }
            return;
        }

        // Check body indentation inside when clauses (when keyword
        // positioning is handled by Layout/CaseIndentation, not here).
        if let Some(when_node) = node.as_when_node() {
            let end_style = config.get_str("EndAlignmentStyle", "keyword");
            let kw_offset = when_node.keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);

            // Skip if body is on the same line as `then` keyword in a
            // multi-line when clause (e.g., `when :a,\n  :b then nil`).
            if let Some(then_loc) = when_node.then_keyword_loc() {
                let (then_line, _) = source.offset_to_line_col(then_loc.start_offset());
                if let Some(stmts) = when_node.statements() {
                    if let Some(first) = stmts.body().iter().next() {
                        let (first_line, _) =
                            source.offset_to_line_col(first.location().start_offset());
                        if first_line == then_line {
                            return;
                        }
                    }
                }
            }

            // RuboCop 1.84.2 crashes and emits no offense for assignment-style
            // `case` bodies under `EndAlignment: variable` when the body starts
            // before the `when` keyword column. Match that no-offense behavior.
            if end_style == "variable" {
                if let Some(stmts) = when_node.statements() {
                    if let Some(first) = stmts.body().iter().next() {
                        let (first_line, first_col) =
                            source.offset_to_line_col(first.location().start_offset());
                        let first_col = bom_adjusted_col(source, first_line, first_col);
                        if first_col < kw_col {
                            return;
                        }
                    }
                }
            }

            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                kw_col,
                when_node.statements(),
                options,
            ));
            return;
        }

        // Check body indentation inside `in` clauses (case/in pattern matching).
        // Analogous to WhenNode handling for case/when.
        if let Some(in_node) = node.as_in_node() {
            let end_style = config.get_str("EndAlignmentStyle", "keyword");
            let kw_offset = in_node.in_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);

            // Skip if body is on the same line as `then` keyword in a
            // multi-line in clause.
            if let Some(then_loc) = in_node.then_loc() {
                let (then_line, _) = source.offset_to_line_col(then_loc.start_offset());
                if let Some(stmts) = in_node.statements() {
                    if let Some(first) = stmts.body().iter().next() {
                        let (first_line, _) =
                            source.offset_to_line_col(first.location().start_offset());
                        if first_line == then_line {
                            return;
                        }
                    }
                }
            }

            if end_style == "variable" {
                if let Some(stmts) = in_node.statements() {
                    if let Some(first) = stmts.body().iter().next() {
                        let (first_line, first_col) =
                            source.offset_to_line_col(first.location().start_offset());
                        let first_col = bom_adjusted_col(source, first_line, first_col);
                        if first_col < kw_col {
                            return;
                        }
                    }
                }
            }

            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                kw_col,
                in_node.statements(),
                options,
            ));
            return;
        }

        // Check else clause on case/when. RuboCop uses the LAST when keyword
        // as the base for else body indentation, not the else keyword itself.
        if let Some(case_node) = node.as_case_node() {
            let end_style = config.get_str("EndAlignmentStyle", "keyword");
            if self.in_variable_style_assignment_case(
                source,
                case_node.case_keyword_loc().start_offset(),
                end_style,
            ) {
                return;
            }
            if let Some(else_clause) = case_node.else_clause() {
                if let Some(last_when) = case_node.conditions().iter().last() {
                    if let Some(when_node) = last_when.as_when_node() {
                        let kw_offset = when_node.keyword_loc().start_offset();
                        let (_, kw_col) = source.offset_to_line_col(kw_offset);
                        diagnostics.extend(self.check_statements_indentation(
                            source,
                            kw_offset,
                            kw_col,
                            else_clause.statements(),
                            options,
                        ));
                    }
                }
            }
            return;
        }

        // Check else clause on case/in pattern matching. Uses last `in` keyword
        // as base, matching RuboCop's on_case_match behavior.
        if let Some(case_match_node) = node.as_case_match_node() {
            let end_style = config.get_str("EndAlignmentStyle", "keyword");
            if self.in_variable_style_assignment_case(
                source,
                case_match_node.case_keyword_loc().start_offset(),
                end_style,
            ) {
                return;
            }
            if let Some(else_clause) = case_match_node.else_clause() {
                if let Some(last_in) = case_match_node.conditions().iter().last() {
                    if let Some(in_node) = last_in.as_in_node() {
                        let kw_offset = in_node.in_loc().start_offset();
                        let (_, kw_col) = source.offset_to_line_col(kw_offset);
                        diagnostics.extend(self.check_statements_indentation(
                            source,
                            kw_offset,
                            kw_col,
                            else_clause.statements(),
                            options,
                        ));
                    }
                }
            }
            return;
        }

        if let Some(while_node) = node.as_while_node() {
            let kw_offset = while_node.keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);

            let end_style = config.get_str("EndAlignmentStyle", "keyword");
            let base_col = if end_style == "variable" {
                if let Some(var_col) = assignment_context_base_col(source, kw_offset) {
                    var_col
                } else {
                    kw_col
                }
            } else {
                kw_col
            };

            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                base_col,
                while_node.statements(),
                options,
            ));
            return;
        }

        if let Some(until_node) = node.as_until_node() {
            let kw_offset = until_node.keyword_loc().start_offset();
            let (_, kw_col) = source.offset_to_line_col(kw_offset);

            let end_style = config.get_str("EndAlignmentStyle", "keyword");
            let base_col = if end_style == "variable" {
                if let Some(var_col) = assignment_context_base_col(source, kw_offset) {
                    var_col
                } else {
                    kw_col
                }
            } else {
                kw_col
            };

            diagnostics.extend(self.check_statements_indentation(
                source,
                kw_offset,
                base_col,
                until_node.statements(),
                options,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full_with_config;

    crate::cop_fixture_tests!(IndentationWidth, "cops/layout/indentation_width");

    fn relative_to_receiver_variant_config() -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyleAlignWith".into(),
                    serde_yml::Value::String("relative_to_receiver".into()),
                ),
                (
                    "EndAlignmentStyle".into(),
                    serde_yml::Value::String("variable".into()),
                ),
                (
                    "IndentationConsistencyStyle".into(),
                    serde_yml::Value::String("indented_internal_methods".into()),
                ),
                (
                    "AccessModifierIndentationStyle".into(),
                    serde_yml::Value::String("outdent".into()),
                ),
                (
                    "IndentationStyleEnforced".into(),
                    serde_yml::Value::String("tabs".into()),
                ),
            ]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn relative_to_receiver_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/layout/indentation_width/offense.relative_to_receiver.rb"
        );
        let fixture = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture
            .strip_prefix("# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver\n")
            .expect("fixture should start with relative_to_receiver config directive");

        crate::testutil::assert_cop_offenses_full_with_config(
            &IndentationWidth,
            source.as_bytes(),
            relative_to_receiver_variant_config(),
        );
    }

    #[test]
    fn relative_to_receiver_no_offense_fixture() {
        let fixture = include_bytes!(
            "../../../tests/fixtures/cops/layout/indentation_width/no_offense.relative_to_receiver.rb"
        );
        let fixture = std::str::from_utf8(fixture).expect("fixture must be valid UTF-8");
        let source = fixture
            .strip_prefix("# nitrocop-config: EnforcedStyleAlignWith: relative_to_receiver\n")
            .expect("fixture should start with relative_to_receiver config directive");

        crate::testutil::assert_cop_no_offenses_full_with_config(
            &IndentationWidth,
            source.as_bytes(),
            relative_to_receiver_variant_config(),
        );
    }

    #[test]
    fn custom_width() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([("Width".into(), serde_yml::Value::Number(4.into()))]),
            ..CopConfig::default()
        };
        // Body indented 2 instead of 4
        let source = b"def foo\n  x = 1\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("Use 4 (not 2) spaces"));
    }

    #[test]
    fn enforced_style_keyword_aligns_to_def() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleAlignWith".into(),
                serde_yml::Value::String("keyword".into()),
            )]),
            ..CopConfig::default()
        };
        // Body indented 2 from column 0, but `def` is at column 8 (after `private `)
        // With keyword style, body should be at column 10 (8 + 2)
        let source = b"private def foo\n  bar\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert_eq!(
            diags.len(),
            1,
            "keyword style should flag body not aligned with def keyword"
        );
        assert!(diags[0].message.contains("Use 2"));
    }

    #[test]
    fn allowed_patterns_skips_matching() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "AllowedPatterns".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("^\\s*module".into())]),
            )]),
            ..CopConfig::default()
        };
        // Module with wrong indentation but matches AllowedPatterns
        let source = b"module Foo\n      x = 1\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert!(
            diags.is_empty(),
            "AllowedPatterns should skip matching lines"
        );
    }

    #[test]
    fn assignment_context_if_body_from_keyword() {
        use crate::testutil::run_cop_full;
        // Body indented 2 from `if` keyword (col 4), body at col 6 — correct
        let source = b"x = if foo\n      bar\n    end\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert!(
            diags.is_empty(),
            "body at if+2 should not flag: {:?}",
            diags
        );
    }

    #[test]
    fn assignment_context_if_wrong_indent() {
        use crate::testutil::run_cop_full;
        // Body at column 2 — should be column 6 (if=4, 4+2=6). Flagged.
        let source = b"x = if foo\n  bar\nend\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert_eq!(
            diags.len(),
            1,
            "should flag wrong indentation in assignment context: {:?}",
            diags
        );
    }

    #[test]
    fn assignment_context_compound_operator() {
        use crate::testutil::run_cop_full;
        // x ||= if foo ... body indented from `if` keyword (col 6), body at col 8 — correct
        let source = b"x ||= if foo\n        bar\n      end\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert!(
            diags.is_empty(),
            "compound assignment context should work: {:?}",
            diags
        );
    }

    #[test]
    fn assignment_context_keyword_style() {
        use crate::testutil::run_cop_full;
        // Keyword style: end aligned with `if`, body indented from `if`
        // @links = if enabled?
        //            body
        //          end
        let source = b"    @links = if enabled?\n               body\n             end\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert!(
            diags.is_empty(),
            "keyword style assignment should not flag: {:?}",
            diags
        );
    }

    #[test]
    fn assignment_variable_style_body_from_variable() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EndAlignmentStyle".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // Variable style: body at col 6 (server=4, 4+2=6), if at col 15
        // server = if cond
        //   body
        // end
        let source = b"    server = if cond\n      body\n    end\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert!(
            diags.is_empty(),
            "variable style should accept body indented from variable: {:?}",
            diags
        );
    }

    #[test]
    fn assignment_variable_style_flags_keyword_indent() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EndAlignmentStyle".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // Variable style: body at col 15 (if=13, 13+2=15) — RuboCop flags
        // keyword-relative indentation and requires alignment from the variable.
        //     server = if cond       (if at col 13)
        //                body        (body at col 15 = 13+2)
        //             end
        let source = b"    server = if cond\n               body\n             end\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert_eq!(
            diags.len(),
            1,
            "variable style should flag keyword-relative indentation: {:?}",
            diags
        );
    }

    #[test]
    fn shovel_operator_variable_style_no_offense() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EndAlignmentStyle".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // << operator with variable style: body indented from receiver, not if keyword
        let source = b"html << if error\n  error\nelse\n  default\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert!(
            diags.is_empty(),
            "variable style << context should not flag body: {:?}",
            diags
        );
    }

    #[test]
    fn shovel_operator_indented_variable_style_no_offense() {
        use std::collections::HashMap;
        let config = CopConfig {
            options: HashMap::from([(
                "EndAlignmentStyle".into(),
                serde_yml::Value::String("variable".into()),
            )]),
            ..CopConfig::default()
        };
        // << operator with variable style at col 8: body indented from @buffer col
        let source = b"        @buffer << if value.safe?\n          value\n        else\n          escape(value)\n        end\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert!(
            diags.is_empty(),
            "variable style << context should not flag body: {:?}",
            diags
        );
    }

    #[test]
    fn indented_internal_methods_flags_method_after_private_in_class_body() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "IndentationConsistencyStyle".into(),
                serde_yml::Value::String("indented_internal_methods".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"class Test\n  private\n  def helper\n  end\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert_eq!(diags.len(), 1, "expected one offense, got: {:?}", diags);
        assert_eq!(
            diags[0].message,
            "Use 2 (not 0) spaces for indented_internal_methods indentation."
        );
    }

    #[test]
    fn indented_internal_methods_flags_method_after_private_in_block_body() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "IndentationConsistencyStyle".into(),
                serde_yml::Value::String("indented_internal_methods".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"concern :Authenticatable do\n  private\n  def helper\n  end\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert_eq!(diags.len(), 1, "expected one offense, got: {:?}", diags);
        assert_eq!(
            diags[0].message,
            "Use 2 (not 0) spaces for indented_internal_methods indentation."
        );
    }

    #[test]
    fn tab_indentation_accepted_when_style_tabs() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "IndentationStyleEnforced".into(),
                serde_yml::Value::String("tabs".into()),
            )]),
            ..CopConfig::default()
        };
        // Tab-indented class body — one extra tab of indentation is correct.
        let source = b"class Foo\n\tdef bar\n\t\tbaz\n\tend\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert!(
            diags.is_empty(),
            "tab-indented code should not be flagged when IndentationStyle is tabs: {:?}",
            diags
        );
    }

    #[test]
    fn tab_indentation_flagged_when_style_tabs() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "IndentationStyleEnforced".into(),
                serde_yml::Value::String("tabs".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"if cond\n\t\tfunc\nend\n";
        let diags = run_cop_full_with_config(&IndentationWidth, source, config);
        assert_eq!(
            diags.len(),
            1,
            "tab-indented code should still be checked when IndentationStyle is tabs: {:?}",
            diags
        );
        assert_eq!(diags[0].message, "Use 1 (not 2) tabs for indentation.");
    }

    #[test]
    fn tab_indentation_flagged_when_style_spaces() {
        use crate::testutil::run_cop_full;
        // Tab-indented class body — should be flagged when IndentationStyle is 'spaces' (default)
        let source = b"class Foo\n\tdef bar\n\tend\nend\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert_eq!(
            diags.len(),
            1,
            "tab-indented code should be flagged when IndentationStyle is spaces: {:?}",
            diags
        );
        assert!(diags[0].message.contains("Use 2 (not 1)"));
    }

    #[test]
    fn assignment_def_no_false_positive() {
        use crate::testutil::run_cop_full;
        // `__skip__ = def foo` — body indented from def keyword, not line start
        let source = b"      __skip__ = def new_field(**kwargs)\n                   body\n                 end\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert!(
            diags.is_empty(),
            "assignment def body indented from def kw should not flag: {:?}",
            diags
        );
    }

    #[test]
    fn private_def_still_checked_from_line_start() {
        use crate::testutil::run_cop_full;
        // `private def foo` — body should be indented from line start (col 0)
        let source = b"private def foo\n  bar\nend\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert!(
            diags.is_empty(),
            "private def with body at col 2 should not flag: {:?}",
            diags
        );
    }

    #[test]
    fn bom_does_not_cause_false_positive() {
        use crate::testutil::run_cop_full;
        // UTF-8 BOM + module Foo / body at col 2 — correctly indented, should not flag
        let source = b"\xEF\xBB\xBFmodule Foo\n  VERSION = '1.0'\nend\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert!(
            diags.is_empty(),
            "BOM should not cause false positive: {:?}",
            diags
        );
    }

    #[test]
    fn bom_still_detects_wrong_indentation() {
        use crate::testutil::run_cop_full;
        // UTF-8 BOM + module Foo with wrong indentation (4 spaces)
        let source = b"\xEF\xBB\xBFmodule Foo\n    VERSION = '1.0'\nend\n";
        let diags = run_cop_full(&IndentationWidth, source);
        assert_eq!(
            diags.len(),
            1,
            "BOM file with wrong indentation should still flag: {:?}",
            diags
        );
        assert!(diags[0].message.contains("Use 2 (not 4)"));
    }
}
