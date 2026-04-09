use crate::cop::shared::node_type::{IF_NODE, UNLESS_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use regex::Regex;
use ruby_prism::Visit;

/// Style/IfUnlessModifier: Checks for `if` and `unless` statements that would
/// fit on one line if written as modifier `if`/`unless`.
///
/// ## Investigation findings (2026-03-15)
///
/// FP root causes (301 FPs):
/// 1. **Chained calls after `end`**: `if test; something; end.inspect` — RuboCop
///    skips via `node.chained?`. nitrocop was missing this check entirely. Fix:
///    detect non-whitespace after `end` keyword on the same line.
/// 2. **Comment on `end` line**: `end # comment` — RuboCop checks
///    `line_with_comment?(node.loc.last_line)`. nitrocop checked comments between
///    body and end but not on the end line itself. Fix: check end line for comments.
/// 3. **Named regexp captures**: `/(?<name>\d)/ =~ str` — RuboCop's
///    `named_capture_in_condition?` checks `match_with_lvasgn_type?`. Fix: detect
///    `MatchWriteNode` in condition (Prism equivalent).
/// 4. **Endless method def in body**: `def method_name = body` — RuboCop's
///    `endless_method?` skips these to avoid `Style/AmbiguousEndlessMethodDefinition`.
///    Fix: check if body is a DefNode with `equal_loc()`.
/// 5. **Pattern matching in condition**: `if [42] in [x]` — RuboCop skips
///    `any_match_pattern_type?`. Fix: detect MatchPredicateNode/MatchRequiredNode.
/// 6. **nonempty_line_count > 3**: Multiline conditions like `if a &&\n  b\n  body\nend`
///    have 4+ non-empty lines. RuboCop skips these. Fix: count non-empty lines in
///    the entire if/unless node source range.
/// 7. **Bare regexp literal on the LHS of `=~`**: `if /foo/ =~ bar` is accepted
///    by RuboCop, but parenthesized conditions like `if(/foo/ =~ bar)`,
///    interpolated regexps like `if /#{foo}/ =~ bar`, and modifier-form lines
///    that become too long are still offenses. Fix: skip only bare
///    non-modifier predicates whose top-level condition is an `=~` call with a
///    plain regexp literal receiver.
///
/// FN root causes (2026-04-01): The biggest remaining cluster was long
/// modifier-form statements like `raise '...' if condition`. The old Rust cop
/// returned immediately for any modifier-form `if`/`unless`, so it never
/// reached RuboCop's `too_long_due_to_modifier?` branch. Fixed by checking
/// modifier-form nodes separately, measuring the rendered line length with the
/// same `Layout/LineLength` allowances RuboCop uses here, and skipping only the
/// narrow `foo if bar; baz` same-line sibling case that RuboCop also ignores.
///
/// FN root cause (2026-04-04): `condition_contains_defined` was a blanket skip
/// for any `defined?()` in the condition. RuboCop only skips when the argument
/// is a local variable or method call (`:lvar`/`:send`) that hasn't been
/// previously assigned (`defined_argument_is_undefined?`). For constants
/// (`JRUBY_VERSION`), class variables (`@@logger`), instance variables, and
/// global variables, `defined?` doesn't change scoping semantics in modifier
/// form, so the cop should still flag. Fixed by checking the DefinedNode's
/// `value()` type in the visitor — only set found=true for
/// LocalVariableReadNode or CallNode arguments.
///
/// FN root cause (2026-04-04): inline expression forms like
/// `after_save { if user then body end }`,
/// `options ||= unless cond ... end || fallback`,
/// and `"...#{if cond then body end}"` were all being skipped by a blanket
/// "any code after `end`" check. RuboCop only skips true chained receivers
/// (`end.inspect`, `end + 2`), not enclosing delimiters or larger expressions.
/// Fixed by allowing closing delimiters / `||` / `&&` after `end`, while still
/// rejecting chained/operator continuations, then measuring the full rendered
/// modifier line as RuboCop does: `code_before + expression + code_after`, with
/// UTF-8 character counts instead of raw byte counts.
///
/// FP root cause (2026-04-06): RuboCop's `non_eligible_condition?` skips any
/// condition containing local-variable assignment nodes (`lvasgn_type?`), which
/// includes `||=`, `&&=`, `+=`, and multi-assignment destructuring. Prism
/// represents those as `LocalVariableOrWriteNode`,
/// `LocalVariableAndWriteNode`, `LocalVariableOperatorWriteNode`, and
/// `MultiWriteNode` with local targets. The old Rust cop only detected plain
/// `LocalVariableWriteNode`, so it falsely flagged conditions like
/// `if (iterations += 1) > MAX_ITERATIONS` and
/// `unless (a, b = matcher(node))`. Fixed by recognizing all Prism local-write
/// variants while keeping the skip limited to local targets, not instance/class
/// variable assignments.
///
/// FN root cause (2026-04-06): `code_after_end_is_disallowed` returned `true`
/// for semicolons (`;`) appearing after `end`, treating them as chained operators
/// like `.` or `&`. But `;` is a Ruby statement separator — `unless defined?(x);
/// foo; end; x` is actually two statements (the unless and `x`), not a chained
/// expression. RuboCop's AST-based `another_statement_on_same_line?` correctly
/// handles this by not treating `;` as a chained operator. Fixed by removing
/// `;` from the disallowed characters list in `code_after_end_is_disallowed`.
///
/// FN root cause (2026-04-07): `has_another_statement_on_same_line` (RuboCop's
/// `another_statement_on_same_line?`) incorrectly returned `true` for modifier
/// forms like `return ret if ret` inside blocks, where the line ends with `; }`.
/// The function found `;` after the if node and returned `true`, but RuboCop's
/// AST check would find no sibling statement (no `end` keyword or next sibling
/// on the same line). The `;` was actually a statement separator, not indicating
/// a sibling. Fixed by checking if the semicolon is followed by actual code
/// vs. just closing delimiters (`}` or `]`).
///
/// FN root cause (2026-04-08): `has_another_statement_on_same_line` treated
/// `; end` after a modifier if/unless as a sibling statement, but `end` is a
/// closing keyword of a parent block (e.g. `unless defined?(x); foo; end; x`).
/// Fixed by extending `is_only_closing_tokens` to also recognize `end` as a
/// closing token alongside `}`, `]`, and `)`.
///
/// FP root cause (2026-04-08): modifier forms on lines with
/// `# rubocop:disable Layout/LineLength` (inline or block-level) were flagged
/// as "too long", but RuboCop's `too_long_single_line?` calls
/// `line_length_enabled_at_line?` which returns false when Layout/LineLength is
/// disabled via directive. Fixed by adding `line_length_disabled_at_line` that
/// scans for both inline `rubocop:disable` on the current line and block-level
/// `rubocop:disable` on preceding lines (tracking enable/disable state).
///
/// FP root cause (2026-04-08): URI-based AllowURI exemption only matched
/// `scheme://` patterns, but RuboCop uses `URI::DEFAULT_PARSER.make_regexp`
/// which also matches bare `scheme:` (e.g. `https:` at end of line in a regex).
/// Fixed by adding `scheme:` as an additional search prefix in
/// `uri_extends_to_end`.
///
/// FP root cause (2026-04-08): multiline parenthesized bodies like
/// `if cond\n  (expr)\nend` were flagged. RuboCop's `non_eligible_body?`
/// returns true for `begin_type?`, which in the parser gem includes
/// parenthesized expressions. In Prism these are `ParenthesesNode`. Fixed by
/// skipping `ParenthesesNode` bodies for normal-form `if`/`unless`.
///
/// FN root cause (2026-04-08): that same `ParenthesesNode` skip also
/// suppressed real modifier-form offenses like `(raise '...') if condition`.
/// RuboCop still flags modifier-form nodes here; only the multiline
/// `if ... (expr) end` form is exempt. Fixed by applying the parenthesized-body
/// skip only to normal-form nodes and still evaluating modifier-form nodes for
/// `MSG_USE_NORMAL`.
///
/// FP root cause (2026-04-08): body EOL comment detection used character offsets
/// from `offset_to_line_col` to index into a byte slice. Multi-byte UTF-8
/// characters (Arabic, em-dash, CJK) caused misalignment — the char offset was
/// smaller than the byte offset, so the `#` search started too early or missed.
/// Fixed by computing byte offset via `line_start_offset` subtraction, and
/// searching for `#` anywhere after the body end rather than just as the first
/// non-whitespace character (handles `body; # comment` patterns).
///
/// FP root cause (2026-04-08): `parenthesize_modifier_form` checked the previous
/// line for trailing `=`, `:`, or `=>` to decide if the modifier form needs
/// parenthesization. Comments like `# :nodoc:` on the previous line falsely
/// matched `:`. Fixed by stripping trailing comments before the check.
///
/// FN root cause (2026-04-08): that trailing-comment stripper also treated
/// string interpolation markers like `#{...}` as comments when they were
/// preceded by whitespace on the previous line. That truncated lines such as
/// `changes = ["Study file updated: #{@study_file.upload_file_name}"]` to a
/// trailing `:`, incorrectly forced `(body if cond)`, and pushed several
/// nested modifier forms from 119 chars to 121 chars. Fixed by ignoring
/// interpolation markers (`#{...}`, `#@ivar`, `#$gvar`) in
/// `strip_trailing_comment`.
///
/// FN root cause (2026-04-08): `first_line_comment_text` only found comments
/// that were the first non-whitespace after the condition/predicate end. Comments
/// after `then` keyword (e.g. `if cond then # comment`) were missed because
/// `then` appeared first. Fixed by searching for `#` anywhere after the
/// predicate (skipping `#{` interpolation markers), matching RuboCop's behavior
/// of finding any comment on the same line as the node.
///
/// FN root cause (2026-04-09): `previous_line_chains_to_if` treated any
/// previous line ending in `!` as an operator continuation. That falsely
/// skipped ordinary offenses after bang method names like `def unlock!`,
/// `parser.parse!`, `m.load_bundler!`, and `item.strip!`. RuboCop's
/// `node.chained?` only skips real operator/receiver chaining, so the fix keeps
/// standalone `!` chaining but ignores `!` when it is just method-name
/// punctuation.
pub struct IfUnlessModifier;

/// Check if a node (or any descendant) contains a heredoc.
/// Heredoc locations in Prism only cover the delimiter, so the actual
/// source spans more lines than the node location suggests.
fn node_contains_heredoc(node: &ruby_prism::Node<'_>) -> bool {
    let mut finder = HeredocFinder { found: false };
    finder.visit(node);
    finder.found
}

struct HeredocFinder {
    found: bool,
}

impl<'pr> Visit<'pr> for HeredocFinder {
    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        if let Some(open) = node.opening_loc() {
            if open.as_slice().starts_with(b"<<") {
                self.found = true;
                return;
            }
        }
        ruby_prism::visit_interpolated_string_node(self, node);
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if let Some(open) = node.opening_loc() {
            if open.as_slice().starts_with(b"<<") {
                self.found = true;
                return;
            }
        }
        ruby_prism::visit_string_node(self, node);
    }
}

/// Check if a node (or any descendant) contains a `defined?()` call.
///
/// RuboCop skips `if defined?(x)` when the argument is a local variable
/// or method call that might be undefined — converting to modifier form
/// changes the semantics of `defined?` with respect to local variable
/// scoping.  We conservatively skip any condition that contains `defined?`.
fn condition_contains_defined(node: &ruby_prism::Node<'_>) -> bool {
    let mut finder = DefinedFinder { found: false };
    finder.visit(node);
    finder.found
}

struct DefinedFinder {
    found: bool,
}

impl<'pr> Visit<'pr> for DefinedFinder {
    fn visit_defined_node(&mut self, node: &ruby_prism::DefinedNode<'pr>) {
        // RuboCop only skips `defined?` when the argument is a local variable
        // or method call (`:lvar` or `:send`) that hasn't been previously assigned.
        // For constants (`JRUBY_VERSION`), class variables (`@@logger`), instance
        // variables, and global variables, `defined?` doesn't change semantics
        // in modifier form, so the cop should still flag those.
        let value = node.value();
        if value.as_local_variable_read_node().is_some() || value.as_call_node().is_some() {
            self.found = true;
        }
    }
}

/// Check if a node (or any descendant) contains a local variable assignment (lvasgn).
///
/// RuboCop's `non_eligible_condition?` skips conditions that assign local
/// variables, because the modifier form may change scoping semantics.
fn condition_contains_lvasgn(node: &ruby_prism::Node<'_>) -> bool {
    let mut finder = LvasgnFinder { found: false };
    finder.visit(node);
    finder.found
}

struct LvasgnFinder {
    found: bool,
}

fn target_contains_local_variable(node: &ruby_prism::Node<'_>) -> bool {
    if node.as_local_variable_target_node().is_some() {
        return true;
    }

    if let Some(splat) = node.as_splat_node() {
        return splat
            .expression()
            .is_some_and(|expr| target_contains_local_variable(&expr));
    }

    if let Some(multi_target) = node.as_multi_target_node() {
        for target in multi_target.lefts().iter() {
            if target_contains_local_variable(&target) {
                return true;
            }
        }

        if multi_target
            .rest()
            .is_some_and(|target| target_contains_local_variable(&target))
        {
            return true;
        }

        for target in multi_target.rights().iter() {
            if target_contains_local_variable(&target) {
                return true;
            }
        }
    }

    false
}

impl<'pr> Visit<'pr> for LvasgnFinder {
    fn visit_local_variable_write_node(&mut self, _node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.found = true;
    }

    fn visit_local_variable_or_write_node(
        &mut self,
        _node: &ruby_prism::LocalVariableOrWriteNode<'pr>,
    ) {
        self.found = true;
    }

    fn visit_local_variable_and_write_node(
        &mut self,
        _node: &ruby_prism::LocalVariableAndWriteNode<'pr>,
    ) {
        self.found = true;
    }

    fn visit_local_variable_operator_write_node(
        &mut self,
        _node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>,
    ) {
        self.found = true;
    }

    fn visit_multi_write_node(&mut self, node: &ruby_prism::MultiWriteNode<'pr>) {
        for target in node.lefts().iter() {
            if target_contains_local_variable(&target) {
                self.found = true;
                return;
            }
        }

        if node
            .rest()
            .is_some_and(|target| target_contains_local_variable(&target))
        {
            self.found = true;
            return;
        }

        for target in node.rights().iter() {
            if target_contains_local_variable(&target) {
                self.found = true;
                return;
            }
        }

        ruby_prism::visit_multi_write_node(self, node);
    }
}

/// Check if the condition contains a named regexp capture (`/(?<x>...)/ =~ str`).
///
/// RuboCop's `named_capture_in_condition?` checks `match_with_lvasgn_type?`.
/// In Prism, this is represented as a `MatchWriteNode`.
fn condition_contains_named_capture(node: &ruby_prism::Node<'_>) -> bool {
    let mut finder = NamedCaptureFinder { found: false };
    finder.visit(node);
    finder.found
}

struct NamedCaptureFinder {
    found: bool,
}

impl<'pr> Visit<'pr> for NamedCaptureFinder {
    fn visit_match_write_node(&mut self, _node: &ruby_prism::MatchWriteNode<'pr>) {
        self.found = true;
    }
}

/// Check whether the top-level condition is a bare regexp literal on the left
/// side of `=~`, e.g. `if /foo/ =~ bar`.
///
/// RuboCop still flags parenthesized conditions like `if(/foo/ =~ bar)`,
/// interpolated regexps like `if /#{foo}/ =~ bar`, and modifier-form lines
/// that are too long, so this intentionally stays narrow.
fn condition_is_bare_regexp_lhs_match(node: &ruby_prism::Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };

    if call.name().as_slice() != b"=~" {
        return false;
    }

    let Some(receiver) = call.receiver() else {
        return false;
    };

    receiver.as_regular_expression_node().is_some()
}

/// Check if the condition contains pattern matching (`in` operator).
///
/// RuboCop's `pattern_matching_nodes` checks `any_match_pattern_type?`.
/// In Prism, `[42] in [x]` is a `MatchPredicateNode` and `[42] => x` is
/// `MatchRequiredNode`.
fn condition_contains_pattern_matching(node: &ruby_prism::Node<'_>) -> bool {
    let mut finder = PatternMatchFinder { found: false };
    finder.visit(node);
    finder.found
}

struct PatternMatchFinder {
    found: bool,
}

impl<'pr> Visit<'pr> for PatternMatchFinder {
    fn visit_match_predicate_node(&mut self, _node: &ruby_prism::MatchPredicateNode<'pr>) {
        self.found = true;
    }
    fn visit_match_required_node(&mut self, _node: &ruby_prism::MatchRequiredNode<'pr>) {
        self.found = true;
    }
}

/// Check if a body node is an endless method definition (`def method_name = body`).
///
/// RuboCop skips these to avoid conflict with `Style/AmbiguousEndlessMethodDefinition`.
fn body_is_endless_method(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(def_node) = node.as_def_node() {
        return def_node.equal_loc().is_some();
    }
    false
}

/// Check if a node (or any descendant) contains a nested conditional
/// (if/unless/ternary). RuboCop's `nested_conditional?` on IfNode checks
/// whether any branch contains a nested `:if` node (which includes ternaries).
/// We check the body for any descendant IfNode or UnlessNode.
fn body_contains_nested_conditional(node: &ruby_prism::Node<'_>) -> bool {
    let mut finder = NestedConditionalFinder { found: false };
    finder.visit(node);
    finder.found
}

struct NestedConditionalFinder {
    found: bool,
}

impl<'pr> Visit<'pr> for NestedConditionalFinder {
    fn visit_if_node(&mut self, _node: &ruby_prism::IfNode<'pr>) {
        self.found = true;
    }
    fn visit_unless_node(&mut self, _node: &ruby_prism::UnlessNode<'pr>) {
        self.found = true;
    }
}

/// Strip trailing comment from a line. Finds the first `#` preceded by
/// whitespace (or at position 0) and returns the trimmed text before it.
/// This prevents comment text like `# :nodoc:` from falsely matching
/// operators like `=`, `:`, or `=>`. Ignore interpolation markers like
/// `#{...}`, `#@ivar`, and `#$gvar` — they are part of string content,
/// not Ruby comments.
fn strip_trailing_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#'
            && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t')
            && !matches!(bytes.get(i + 1), Some(b'{' | b'@' | b'$'))
        {
            return line[..i].trim_end();
        }
    }
    line.trim_end()
}

fn parenthesize_modifier_form(source: &SourceFile, kw_loc: &ruby_prism::Location<'_>) -> bool {
    let (kw_line, kw_col) = source.offset_to_line_col(kw_loc.start_offset());
    let kw_line_start = kw_loc.start_offset().saturating_sub(kw_col);
    let before_kw = &source.as_bytes()[kw_line_start..kw_loc.start_offset()];
    let before_kw_trimmed = String::from_utf8_lossy(before_kw).trim_end().to_string();

    if before_kw_trimmed.ends_with('=')
        || before_kw_trimmed.ends_with(':')
        || before_kw_trimmed.ends_with("=>")
    {
        return true;
    }

    if before_kw_trimmed.is_empty() && kw_line >= 2 {
        let lines: Vec<&[u8]> = source.lines().collect();
        let prev_line = lines[kw_line - 2];
        let prev_trimmed = String::from_utf8_lossy(prev_line).trim_end().to_string();
        // Strip trailing comment: find `#` preceded by whitespace (or at line start)
        // so that `# :nodoc:` or `# = Page =` don't falsely trigger parenthesization.
        let prev_code = strip_trailing_comment(&prev_trimmed);
        if !prev_code.is_empty()
            && (prev_code.ends_with('=') || prev_code.ends_with(':') || prev_code.ends_with("=>"))
        {
            return true;
        }
    }

    false
}

fn collection_context_prefix(prefix: &str) -> bool {
    prefix.ends_with('(')
        || prefix.ends_with('[')
        || prefix.ends_with(',')
        || prefix.ends_with(':')
        || prefix.ends_with("=>")
}

fn single_line_direct_collection_context(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
    kw_loc: &ruby_prism::Location<'_>,
) -> bool {
    let (kw_line, kw_col) = source.offset_to_line_col(kw_loc.start_offset());
    let node_end_off = node
        .location()
        .end_offset()
        .saturating_sub(1)
        .max(node.location().start_offset());
    let (node_end_line, _) = source.offset_to_line_col(node_end_off);
    if kw_line != node_end_line {
        return false;
    }

    let kw_line_start = kw_loc.start_offset().saturating_sub(kw_col);
    let before_kw = &source.as_bytes()[kw_line_start..kw_loc.start_offset()];
    let before_kw_trimmed = String::from_utf8_lossy(before_kw).trim_end().to_string();
    if collection_context_prefix(&before_kw_trimmed) {
        return true;
    }

    if before_kw_trimmed.is_empty() && kw_line >= 2 {
        let lines: Vec<&[u8]> = source.lines().collect();
        let prev_line = lines[kw_line - 2];
        let prev_trimmed = String::from_utf8_lossy(prev_line).trim_end().to_string();
        return collection_context_prefix(&prev_trimmed);
    }

    false
}

fn code_after_end_is_disallowed(after_end: &[u8]) -> bool {
    let trimmed = after_end
        .iter()
        .copied()
        .skip_while(|&b| b == b' ' || b == b'\t')
        .collect::<Vec<_>>();

    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with(b"#") {
        return true;
    }
    if trimmed.starts_with(b"||")
        || trimmed.starts_with(b"&&")
        || trimmed.starts_with(b"or")
        || trimmed.starts_with(b"and")
    {
        return false;
    }

    matches!(
        trimmed[0],
        b'.' | b'&'
            | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'<'
            | b'>'
            | b'='
            | b'!'
            | b'|'
            | b'^'
            | b'~'
            | b'?'
            | b':'
    )
}

fn normalize_ruby_regex(pattern: &str) -> String {
    let mut s = pattern.trim().to_string();

    if s.starts_with('/') {
        s.remove(0);
        if let Some(last_slash) = s.rfind('/') {
            s.truncate(last_slash);
        }
    }

    s.replace("\\A", "^")
        .replace("\\z", "$")
        .replace("\\Z", "$")
}

fn indentation_difference(line: &[u8], indentation_width: usize) -> usize {
    if indentation_width <= 1 || line.first() != Some(&b'\t') {
        return 0;
    }

    let leading_tabs = line.iter().take_while(|&&b| b == b'\t').count();

    leading_tabs * (indentation_width - 1)
}

fn uri_extends_to_end(
    line: &str,
    schemes: &[String],
    max: usize,
    indentation_width: usize,
) -> bool {
    let mut all_starts = Vec::new();
    for scheme in schemes {
        // RuboCop uses URI::DEFAULT_PARSER.make_regexp which matches `scheme:`
        // followed by any valid URI characters (not just `://`). This includes
        // patterns like `https:/path` and bare `https:` at line end.
        for prefix in [
            format!("{scheme}://"),
            format!(r"{scheme}:\/\/"),
            format!("{scheme}:"),
        ] {
            let mut search_from = 0;
            while let Some(pos) = line[search_from..].find(&prefix) {
                let abs_pos = search_from + pos;
                all_starts.push(abs_pos);
                search_from = abs_pos + prefix.len();
            }
        }
    }

    if all_starts.is_empty() {
        return false;
    }

    let indentation_diff = indentation_difference(line.as_bytes(), indentation_width);

    for start in all_starts {
        let uri_end = start
            + line[start..]
                .find(|c: char| c.is_whitespace())
                .unwrap_or(line.len() - start);

        let mut end_pos = uri_end;
        if line.contains('{') && line.ends_with('}') {
            if let Some(brace_pos) = line[end_pos..].rfind('}') {
                end_pos += brace_pos + 1;
            }
        }

        let rest = &line[end_pos..];
        let non_ws_len = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        end_pos += non_ws_len;

        let start_chars = line[..start].chars().count() + indentation_diff;
        if start_chars < max && end_pos >= line.len() {
            return true;
        }
    }

    false
}

/// Check if `Layout/LineLength` is disabled at a given line via rubocop:disable
/// comments (inline or block). RuboCop's `line_length_enabled_at_line?` checks
/// `processed_source.comment_config.cop_enabled_at_line?('Layout/LineLength', line)`.
/// Since cops don't have access to the global DisabledRanges, we scan the source
/// for disable directives ourselves.
fn line_length_disabled_at_line(source: &SourceFile, line_num: usize) -> bool {
    let lines: Vec<&[u8]> = source.lines().collect();
    if line_num == 0 || line_num > lines.len() {
        return false;
    }

    // Check inline: current line has `# rubocop:disable Layout/LineLength` or `all`
    let current_line = String::from_utf8_lossy(lines[line_num - 1]);
    if line_disables_line_length(&current_line) {
        return true;
    }

    // Check block: scan preceding lines for standalone `# rubocop:disable` that
    // covers Layout/LineLength without a matching `# rubocop:enable` before us
    let mut block_disabled = false;
    for line_bytes in lines.iter().take(line_num.saturating_sub(1)) {
        let line_str = String::from_utf8_lossy(line_bytes);
        let trimmed = line_str.trim();
        // Block directives are standalone comments (no code before the `#`)
        if !trimmed.starts_with('#') {
            continue;
        }
        if directive_disables_line_length(trimmed) {
            block_disabled = true;
        } else if directive_enables_line_length(trimmed) {
            block_disabled = false;
        }
    }
    block_disabled
}

/// Check if a line contains an inline `# rubocop:disable` for Layout/LineLength or all.
fn line_disables_line_length(line: &str) -> bool {
    // Inline directives have code before the comment
    if let Some(pos) = line.find("# rubocop:disable") {
        let cops = &line[pos + "# rubocop:disable".len()..];
        return cops_list_includes_line_length(cops);
    }
    false
}

/// Check if a standalone comment directive disables Layout/LineLength.
fn directive_disables_line_length(trimmed: &str) -> bool {
    if let Some(pos) = trimmed.find("rubocop:disable") {
        let cops = &trimmed[pos + "rubocop:disable".len()..];
        return cops_list_includes_line_length(cops);
    }
    false
}

/// Check if a standalone comment directive enables Layout/LineLength.
fn directive_enables_line_length(trimmed: &str) -> bool {
    if let Some(pos) = trimmed.find("rubocop:enable") {
        let cops = &trimmed[pos + "rubocop:enable".len()..];
        return cops_list_includes_line_length(cops);
    }
    false
}

/// Check if a comma-separated cop list includes Layout/LineLength, Metrics/LineLength,
/// or `all`.
fn cops_list_includes_line_length(cops_str: &str) -> bool {
    for cop in cops_str.split(',') {
        let cop = cop.trim();
        if cop == "all" || cop == "Layout/LineLength" || cop == "Metrics/LineLength" {
            return true;
        }
    }
    false
}

fn modifier_form_too_long(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
    config: &CopConfig,
) -> bool {
    let max_line_length = config.get_usize("MaxLineLength", 120);
    if max_line_length == 0 || !config.get_bool("LineLengthEnabled", max_line_length > 0) {
        return false;
    }

    let node_src = &source.as_bytes()[node.location().start_offset()..node.location().end_offset()];
    if node_src.contains(&b'\n') {
        return false;
    }

    let (line_num, _) = source.offset_to_line_col(node.location().start_offset());

    // RuboCop's `line_length_enabled_at_line?` — skip if Layout/LineLength
    // is disabled at this line via rubocop:disable comments
    if line_length_disabled_at_line(source, line_num) {
        return false;
    }
    let lines: Vec<&[u8]> = source.lines().collect();
    if line_num == 0 || line_num > lines.len() {
        return false;
    }

    let raw_line = lines[line_num - 1];
    let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
    let line_str = match std::str::from_utf8(line) {
        Ok(s) => s,
        Err(_) => return line.len() > max_line_length,
    };

    let indentation_width = config.get_usize("IndentationWidth", 2);
    let effective_len = line_str.chars().count() + indentation_difference(line, indentation_width);
    if effective_len <= max_line_length {
        return false;
    }

    if config.get_bool("AllowCopDirectives", true) {
        if let Some(comment_start) = line_str.find("# rubocop:") {
            let without_directive_chars = line_str[..comment_start].trim_end().chars().count();
            if without_directive_chars <= max_line_length {
                return false;
            }
        }
    }

    let allowed_patterns = config
        .get_string_array("AllowedPatterns")
        .unwrap_or_default();
    if !allowed_patterns.is_empty() {
        let compiled_patterns: Vec<Regex> = allowed_patterns
            .iter()
            .filter_map(|pattern| Regex::new(&normalize_ruby_regex(pattern)).ok())
            .collect();
        if compiled_patterns
            .iter()
            .any(|regex| regex.is_match(line_str))
        {
            return false;
        }
    }

    if config.get_bool("AllowURI", true) {
        let uri_schemes = config
            .get_string_array("URISchemes")
            .unwrap_or_else(|| vec!["http".into(), "https".into()]);
        if uri_extends_to_end(line_str, &uri_schemes, max_line_length, indentation_width) {
            return false;
        }
    }

    true
}

/// Check if a byte slice consists only of closing tokens: `end` keywords,
/// `}`, `]`, `)`, semicolons, and whitespace. These are not sibling statements
/// but rather closing delimiters of parent blocks.
fn is_only_closing_tokens(bytes: &[u8]) -> bool {
    let mut remaining = bytes;
    loop {
        // Skip whitespace and semicolons
        while remaining
            .first()
            .is_some_and(|&b| b == b' ' || b == b'\t' || b == b';')
        {
            remaining = &remaining[1..];
        }
        if remaining.is_empty() {
            return true;
        }
        // Check for closing delimiters
        if matches!(remaining[0], b'}' | b']' | b')') {
            remaining = &remaining[1..];
            continue;
        }
        // Check for `end` keyword (must not be followed by identifier chars)
        if remaining.starts_with(b"end") {
            let after = &remaining[3..];
            if after.is_empty()
                || (!after[0].is_ascii_alphanumeric()
                    && after[0] != b'_'
                    && after[0] != b'!'
                    && after[0] != b'?')
            {
                remaining = after;
                continue;
            }
        }
        return false;
    }
}

fn has_another_statement_on_same_line(source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
    let (line_num, _) = source.offset_to_line_col(node.location().end_offset());
    let lines: Vec<&[u8]> = source.lines().collect();
    if line_num == 0 || line_num > lines.len() {
        return false;
    }

    let line_start = source.line_start_offset(line_num);
    let line = lines[line_num - 1];
    let after_start = node.location().end_offset().saturating_sub(line_start);
    if after_start >= line.len() {
        return false;
    }

    let after = &line[after_start..];
    let trimmed = after
        .iter()
        .copied()
        .skip_while(|&b| b == b' ' || b == b'\t')
        .collect::<Vec<_>>();

    // Check for semicolon followed by actual code (not just closing delimiters
    // or `end` keywords closing parent blocks)
    if trimmed.first() == Some(&b';') {
        // Make sure there's actual code after the semicolon, not just }, ], or `end`
        let remaining: Vec<_> = trimmed[1..]
            .iter()
            .copied()
            .skip_while(|&b| b == b' ' || b == b'\t')
            .collect();
        if is_only_closing_tokens(&remaining) {
            return false;
        }
        return true;
    }

    // Also check through closing tokens (like `}`, `]`, `)`) for semicolons
    // that indicate sibling statements in enclosing scopes. RuboCop's AST-based
    // `another_statement_on_same_line?` traverses upward to find `begin` nodes
    // with siblings; we detect this textually.
    // Example: `{ |fn| bool = true if cond } ; bool`
    //   After the if-node: ` } ; bool` — the `}` closes the block, and `; bool`
    //   is a sibling statement in the enclosing parenthesized expression.
    {
        let mut remaining = &trimmed[..];
        // Skip closing tokens and whitespace
        while let Some(&b) = remaining.first() {
            if b == b'}' || b == b']' || b == b')' || b == b' ' || b == b'\t' {
                remaining = &remaining[1..];
            } else {
                break;
            }
        }
        if remaining.first() == Some(&b';') {
            let after_semi: Vec<_> = remaining[1..]
                .iter()
                .copied()
                .skip_while(|&b| b == b' ' || b == b'\t')
                .collect();
            if !after_semi.is_empty() && !is_only_closing_tokens(&after_semi) {
                return true;
            }
        }
    }

    false
}

/// Check if the if/unless keyword is at the start of a line and the previous
/// non-empty, non-comment line ends with an operator that would make the
/// if-expression an operand (e.g., `- \n if foo ... end`).
///
/// RuboCop catches this via `node.chained?` which returns true when the node
/// is the receiver of a method call. Without AST parent access, we detect it
/// by text: when the previous line ends with an operator character that isn't
/// `=`, `:`, or `=>` (which are handled by parenthesization), the if-node is
/// part of a larger expression and cannot be converted to modifier form.
fn previous_line_chains_to_if(source: &SourceFile, kw_loc: &ruby_prism::Location<'_>) -> bool {
    let (kw_line, kw_col) = source.offset_to_line_col(kw_loc.start_offset());
    let kw_line_start = kw_loc.start_offset().saturating_sub(kw_col);
    let before_kw = &source.as_bytes()[kw_line_start..kw_loc.start_offset()];

    // Only applies when the if/unless keyword is at the start of the line
    if !before_kw.iter().all(|&b| b == b' ' || b == b'\t') {
        return false;
    }
    if kw_line < 2 {
        return false;
    }

    let lines: Vec<&[u8]> = source.lines().collect();
    // Find the previous non-empty, non-comment line
    for prev_idx in (0..kw_line - 1).rev() {
        let prev_line = lines[prev_idx];
        let prev_str = String::from_utf8_lossy(prev_line);
        let prev_trimmed = prev_str.trim();
        let prev_trimmed = prev_trimmed.strip_suffix('\r').unwrap_or(prev_trimmed);
        if prev_trimmed.is_empty() {
            continue;
        }
        let prev_code = strip_trailing_comment(prev_trimmed);
        if prev_code.is_empty() {
            continue;
        }
        let code_bytes = prev_code.as_bytes();
        let last_byte = match code_bytes.last() {
            Some(&b) => b,
            None => return false,
        };
        // Symbol literals like `:+`, `:-`, `:*` end with an operator char
        // but are not operators — the preceding `:` marks a symbol.
        if code_bytes.len() >= 2 && code_bytes[code_bytes.len() - 2] == b':' {
            return false;
        }
        if last_byte == b'!' && trailing_bang_is_method_suffix(code_bytes) {
            return false;
        }
        // These operators bind to the if-expression, making it a receiver/operand.
        // Exclude `=`, `:`, `>` (part of `=>`) which are handled by parenthesization.
        // Exclude `)`, `]`, `}` which are closing delimiters, not operators.
        // Exclude `|` because block parameter delimiters `{ |x, y|` end with `|`
        // and are far more common than binary `|` chaining to the next line.
        // Exclude `/` because regexp closers like `when /pattern/` are far more
        // common at end of line than division chaining to the next line.
        return matches!(
            last_byte,
            b'.' | b'+' | b'-' | b'*' | b'%' | b'!' | b'~' | b'^' | b'&' | b'<'
        );
    }
    false
}

fn trailing_bang_is_method_suffix(code_bytes: &[u8]) -> bool {
    code_bytes
        .get(code_bytes.len().saturating_sub(2))
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

/// Check if an IfNode or UnlessNode is a pattern matching guard (e.g., `in "a" if cond`).
/// In Prism, pattern matching guards are IfNode/UnlessNode inside InNode.pattern.
/// We detect this by checking if the text from line start to the node's start is just `in`.
fn is_pattern_matching_guard(source: &SourceFile, node: &ruby_prism::Node<'_>) -> bool {
    let loc = node.location();
    let start = loc.start_offset();
    let (line, _col) = source.offset_to_line_col(start);
    if let Some(line_start) = source.line_col_to_offset(line, 0) {
        if let Some(prefix) = source.try_byte_slice(line_start, start) {
            let trimmed = prefix.trim();
            return trimmed == "in";
        }
    }
    false
}

/// Check if a `# rubocop:disable` or `# rubocop:todo` comment disables
/// `Style/IfUnlessModifier` specifically (or `all`). Comments that disable
/// OTHER cops should still be counted in modifier-form line length.
fn comment_disables_this_cop(comment: &str) -> bool {
    // Match patterns like:
    //   # rubocop:disable Style/IfUnlessModifier
    //   # rubocop:todo Style/IfUnlessModifier
    //   # rubocop:disable all
    //   # rubocop:disable Foo, Style/IfUnlessModifier, Bar
    for keyword in ["rubocop:disable", "rubocop:todo"] {
        if let Some(pos) = comment.find(keyword) {
            let after = &comment[pos + keyword.len()..];
            let after = after.trim_start();
            // Check if any of the comma-separated cop names is "all" or our cop
            for cop in after.split(',') {
                let cop = cop.trim();
                if cop == "all" || cop == "Style/IfUnlessModifier" {
                    return true;
                }
            }
        }
    }
    false
}

fn first_line_comment_text(
    source: &SourceFile,
    kw_line: usize,
    predicate: &ruby_prism::Node<'_>,
) -> Option<String> {
    let lines: Vec<&[u8]> = source.lines().collect();
    if kw_line == 0 || kw_line > lines.len() {
        return None;
    }

    let kw_line_start = source.line_start_offset(kw_line);
    let predicate_end_in_line = predicate
        .location()
        .end_offset()
        .saturating_sub(kw_line_start);
    let kw_line_bytes = lines[kw_line - 1];
    if predicate_end_in_line >= kw_line_bytes.len() {
        return None;
    }

    let after_predicate = &kw_line_bytes[predicate_end_in_line..];
    // Find `#` anywhere after the predicate (not just as first non-whitespace).
    // This handles comments after `then` keyword: `if cond then # comment`.
    // RuboCop's `first_line_comment(node)` uses `processed_source.comments`
    // which finds any comment on the same line regardless of intervening tokens.
    // Skip `#{` which is string interpolation, not a comment.
    let hash_pos = after_predicate
        .iter()
        .enumerate()
        .position(|(i, &b)| b == b'#' && after_predicate.get(i + 1) != Some(&b'{'))?;
    let comment_bytes = &after_predicate[hash_pos..];

    let comment = match std::str::from_utf8(comment_bytes) {
        Ok(comment) => comment,
        Err(_) => return None,
    };

    // Only exclude comments that disable THIS cop (Style/IfUnlessModifier) or
    // all cops. Comments disabling OTHER cops carry over to the modifier form
    // and must be counted in the line length (matching RuboCop's behavior).
    if comment_disables_this_cop(comment) {
        return None;
    }

    Some(comment.to_string())
}

fn code_after_end(source: &SourceFile, end_loc: ruby_prism::Location<'_>) -> Option<String> {
    let (end_line, end_col) = source.offset_to_line_col(end_loc.start_offset());
    let lines: Vec<&[u8]> = source.lines().collect();
    if end_line == 0 || end_line > lines.len() {
        return None;
    }

    let raw_line = lines[end_line - 1];
    // Strip CRLF: \r at end of line inflates modifier form length by 1 character
    let end_line_bytes = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
    let after_end_col = end_col + end_loc.as_slice().len();
    if after_end_col >= end_line_bytes.len() {
        return None;
    }

    Some(String::from_utf8_lossy(&end_line_bytes[after_end_col..]).into_owned())
}

impl Cop for IfUnlessModifier {
    fn name(&self) -> &'static str {
        "Style/IfUnlessModifier"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[IF_NODE, UNLESS_NODE]
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
        // Extract keyword location, predicate, statements, has_else, and keyword name
        // from either IfNode or UnlessNode
        let (kw_loc, predicate, statements, has_else, keyword) =
            if let Some(if_node) = node.as_if_node() {
                let kw_loc = match if_node.if_keyword_loc() {
                    Some(loc) => loc,
                    None => return, // ternary
                };
                // Skip elsif nodes — they are visited as IfNode but can't be
                // converted to modifier form independently
                if kw_loc.as_slice() == b"elsif" {
                    return;
                }
                (
                    kw_loc,
                    if_node.predicate(),
                    if_node.statements(),
                    if_node.subsequent().is_some(),
                    "if",
                )
            } else if let Some(unless_node) = node.as_unless_node() {
                (
                    unless_node.keyword_loc(),
                    unless_node.predicate(),
                    unless_node.statements(),
                    unless_node.else_clause().is_some(),
                    "unless",
                )
            } else {
                return;
            };

        // Skip pattern matching guards (e.g., `in "a" if condition`).
        // Prism wraps the pattern + guard as IfNode/UnlessNode inside InNode.
        if is_pattern_matching_guard(source, node) {
            return;
        }

        // Must not have an else clause
        if has_else {
            return;
        }

        let body = match statements {
            Some(stmts) => stmts,
            None => return,
        };

        let body_stmts = body.body();

        // Must have exactly one statement
        if body_stmts.len() != 1 {
            return;
        }

        let body_node = match body_stmts.iter().next() {
            Some(n) => n,
            None => return,
        };

        let modifier_form = kw_loc.start_offset() > body_node.location().start_offset();

        // Skip parenthesized bodies only for normal-form nodes. RuboCop still
        // checks modifier-form lines like `(raise '...') if condition` for the
        // "modifier form makes the line too long" branch.
        if !modifier_form && body_node.as_parentheses_node().is_some() {
            return;
        }

        // Skip if the body is an endless method definition — conflict with
        // Style/AmbiguousEndlessMethodDefinition (RuboCop: endless_method?).
        if body_is_endless_method(&body_node) {
            return;
        }

        // Skip if the condition contains `defined?()` — converting to modifier
        // form changes semantics for undefined variables/methods.
        if condition_contains_defined(&predicate) {
            return;
        }

        // Skip if the condition contains pattern matching (in/=>) — modifier form
        // changes variable scoping semantics (RuboCop: pattern_matching_nodes).
        if condition_contains_pattern_matching(&predicate) {
            return;
        }

        if modifier_form {
            if !modifier_form_too_long(source, node, config)
                || has_another_statement_on_same_line(source, node)
            {
                return;
            }

            let (line, column) = source.offset_to_line_col(node.location().start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!("Modifier form of `{keyword}` makes the line too long."),
            ));
            return;
        }

        // RuboCop accepts bare non-modifier predicates like `if /foo/ =~ bar`,
        // but still flags parenthesized/interpolated variants and modifier-form
        // lines that become too long.
        if condition_is_bare_regexp_lhs_match(&predicate) {
            return;
        }

        // Skip if the condition contains a local variable assignment — modifier
        // form may change scoping semantics (RuboCop: non_eligible_condition?).
        if condition_contains_lvasgn(&predicate) {
            return;
        }

        // Skip if the condition contains a named regexp capture — modifier form
        // changes semantics (RuboCop: named_capture_in_condition?).
        if condition_contains_named_capture(&predicate) {
            return;
        }

        // Skip if the body contains any nested conditional (if/unless/ternary).
        // RuboCop's `nested_conditional?` checks if any branch contains a nested
        // `:if` node, which includes ternaries (e.g., `a = x ? y : z`).
        if body_contains_nested_conditional(&body_node) {
            return;
        }

        // Body must be on a single line to be eligible for modifier form
        let (body_start_line, _) = source.offset_to_line_col(body_node.location().start_offset());
        let body_end_off = body_node
            .location()
            .end_offset()
            .saturating_sub(1)
            .max(body_node.location().start_offset());
        let (body_end_line, _) = source.offset_to_line_col(body_end_off);
        if body_start_line != body_end_line {
            return;
        }

        if single_line_direct_collection_context(source, node, &kw_loc) {
            return;
        }

        // Skip if the if/unless is "chained" from the previous line — an operator
        // on the previous line makes the if-expression its operand (e.g., `- \n if foo
        // ... end`). RuboCop catches this via `node.chained?`.
        if previous_line_chains_to_if(source, &kw_loc) {
            return;
        }

        // If there are standalone comment lines between keyword and body, don't suggest
        // modifier form — converting would lose the comments. But blank lines and
        // multiline condition continuation lines are OK.
        let (kw_line, _) = source.offset_to_line_col(kw_loc.start_offset());
        if body_start_line > kw_line + 1 {
            let lines: Vec<&[u8]> = source.lines().collect();
            for line_num in (kw_line + 1)..body_start_line {
                if line_num > 0 && line_num <= lines.len() {
                    let line = lines[line_num - 1];
                    let trimmed: Vec<u8> = line
                        .iter()
                        .skip_while(|&&b| b == b' ' || b == b'\t')
                        .copied()
                        .collect();
                    if trimmed.starts_with(b"#") {
                        return;
                    }
                }
            }
        }

        // Check if body contains a heredoc argument. Prism's node location for heredoc
        // references only covers the opening delimiter (<<~FOO), not the heredoc content.
        // The actual output would span more lines than the AST suggests.
        if node_contains_heredoc(&body_node) {
            return;
        }

        // Skip if body line has a comment — RuboCop's `non_eligible_body?` checks
        // `processed_source.contains_comment?(body.source_range)` which returns true
        // if there's any comment on the same LINE as the body, even after semicolons.
        // Use byte offset (not char count from offset_to_line_col) so multi-byte
        // UTF-8 characters don't cause misalignment.
        {
            let lines: Vec<&[u8]> = source.lines().collect();
            if body_start_line > 0 && body_start_line <= lines.len() {
                let body_line = lines[body_start_line - 1];
                let body_line_start = source.line_start_offset(body_start_line);
                let body_end_byte = body_node
                    .location()
                    .end_offset()
                    .saturating_sub(body_line_start);
                // Search for `#` anywhere after the body end on the same line
                if body_end_byte < body_line.len() && body_line[body_end_byte..].contains(&b'#') {
                    return;
                }
            }
        }

        // Skip if there's a comment before `end` on its own line, a comment on the
        // `end` line, or code after `end` on the same line (chained calls like
        // `end.inspect`, `end&.foo`, `end + 2`).
        {
            let end_loc: Option<ruby_prism::Location<'_>> = if let Some(if_node) = node.as_if_node()
            {
                if_node.end_keyword_loc()
            } else if let Some(unless_node) = node.as_unless_node() {
                unless_node.end_keyword_loc()
            } else {
                None
            };
            if let Some(end_loc) = end_loc {
                let end_off = end_loc.start_offset();
                let (end_line, end_col) = source.offset_to_line_col(end_off);
                if end_line > body_start_line + 1 {
                    // There are lines between body and end — check for comments
                    let lines: Vec<&[u8]> = source.lines().collect();
                    for line_num in (body_start_line + 1)..end_line {
                        if line_num > 0 && line_num <= lines.len() {
                            let line = lines[line_num - 1];
                            let trimmed: Vec<u8> = line
                                .iter()
                                .skip_while(|&&b| b == b' ' || b == b'\t')
                                .copied()
                                .collect();
                            if trimmed.starts_with(b"#") {
                                return;
                            }
                        }
                    }
                }

                // Check if the `end` line has a comment or disallowed chained code.
                let lines: Vec<&[u8]> = source.lines().collect();
                if end_line > 0 && end_line <= lines.len() {
                    let end_line_bytes = lines[end_line - 1];
                    let after_end_col = end_col + end_loc.as_slice().len();
                    if after_end_col < end_line_bytes.len() {
                        let after_end = &end_line_bytes[after_end_col..];
                        if code_after_end_is_disallowed(after_end) {
                            return;
                        }
                    }
                }
            }
        }

        // Skip if the entire if/unless node has more than 3 non-empty lines.
        // RuboCop's `non_eligible_node?` checks `node.nonempty_line_count > 3`.
        // This catches multiline conditions like `if a &&\n  b\n  body\nend`.
        {
            let node_src =
                &source.as_bytes()[node.location().start_offset()..node.location().end_offset()];
            let nonempty_count = node_src
                .split(|&b| b == b'\n')
                .filter(|line| line.iter().any(|&b| b != b' ' && b != b'\t' && b != b'\r'))
                .count();
            if nonempty_count > 3 {
                return;
            }
        }

        let max_line_length = config.get_usize("MaxLineLength", 120);
        let line_length_enabled = config.get_bool("LineLengthEnabled", max_line_length > 0);

        let kw_line_start = source.line_start_offset(kw_line);
        let code_before =
            String::from_utf8_lossy(&source.as_bytes()[kw_line_start..kw_loc.start_offset()])
                .into_owned();
        let body_text = String::from_utf8_lossy(
            &source.as_bytes()
                [body_node.location().start_offset()..body_node.location().end_offset()],
        )
        .into_owned();
        let cond_text = String::from_utf8_lossy(
            &source.as_bytes()
                [predicate.location().start_offset()..predicate.location().end_offset()],
        )
        .into_owned();

        let mut expression = format!("{body_text} {keyword} {cond_text}");
        let needs_parens = parenthesize_modifier_form(source, &kw_loc);
        if needs_parens {
            expression = format!("({expression})");
        }
        if let Some(comment) = first_line_comment_text(source, kw_line, &predicate) {
            expression.push(' ');
            expression.push_str(&comment);
        }

        let code_after = if let Some(if_node) = node.as_if_node() {
            if_node
                .end_keyword_loc()
                .and_then(|loc| code_after_end(source, loc))
        } else if let Some(unless_node) = node.as_unless_node() {
            unless_node
                .end_keyword_loc()
                .and_then(|loc| code_after_end(source, loc))
        } else {
            None
        }
        .unwrap_or_default();

        let modifier_line = format!("{code_before}{expression}{code_after}");
        let indentation_width = config.get_usize("IndentationWidth", 2);
        let modifier_len = modifier_line.chars().count()
            + indentation_difference(modifier_line.as_bytes(), indentation_width);

        if !line_length_enabled || modifier_len <= max_line_length {
            let (line, column) = source.offset_to_line_col(kw_loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!(
                    "Favor modifier `{keyword}` usage when having a single-line body. Another good alternative is the usage of control flow `&&`/`||`."
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(IfUnlessModifier, "cops/style/if_unless_modifier");

    #[test]
    fn config_max_line_length() {
        use crate::testutil::{assert_cop_no_offenses_full_with_config, run_cop_full_with_config};
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("MaxLineLength".into(), serde_yml::Value::Number(40.into()))]),
            ..CopConfig::default()
        };
        // Short body + condition fits in 40 chars as modifier => should suggest modifier
        let source = b"if x\n  y\nend\n";
        let diags = run_cop_full_with_config(&IfUnlessModifier, source, config.clone());
        assert!(
            !diags.is_empty(),
            "Should fire with MaxLineLength:40 on short if"
        );

        // Longer body that would exceed 40 chars as modifier => should NOT suggest
        let source2 =
            b"if some_very_long_condition_variable_name\n  do_something_important_here\nend\n";
        assert_cop_no_offenses_full_with_config(&IfUnlessModifier, source2, config);
    }

    #[test]
    fn config_line_length_disabled() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        // When LineLengthEnabled is false (Layout/LineLength disabled),
        // modifier form should always be suggested regardless of line length.
        // This matches RuboCop behavior where `max_line_length` returns nil
        // when the cop is disabled.
        let config = CopConfig {
            options: HashMap::from([
                ("LineLengthEnabled".into(), serde_yml::Value::Bool(false)),
                ("MaxLineLength".into(), serde_yml::Value::Number(40.into())),
            ]),
            ..CopConfig::default()
        };
        // This body + condition would exceed 40 chars, but since line length is
        // disabled, it should still suggest modifier form.
        let source =
            b"if some_very_long_condition_variable_name\n  do_something_important_here\nend\n";
        let diags = run_cop_full_with_config(&IfUnlessModifier, source, config);
        assert!(
            !diags.is_empty(),
            "Should fire when LineLengthEnabled is false regardless of line length"
        );
    }

    #[test]
    fn semicolon_before_closing_brace_not_another_statement() {
        use crate::testutil::run_cop_full;
        // `return ret if ret; }` — the `;` before `}` is not a sibling statement,
        // so the if should be flaggable as modifier form.
        let source = b"items.each { |x| if x\n  return x\nend; }\n";
        let diags = run_cop_full(&IfUnlessModifier, source);
        assert!(
            !diags.is_empty(),
            "Semicolon before closing brace should not suppress modifier suggestion"
        );
    }

    #[test]
    fn previous_line_bang_method_is_not_treated_as_chaining() {
        use crate::testutil::run_cop_full;

        let source = b"m.load_bundler!\nif m.invoked_as_script?\n  load Gem.bin_path(\"bundler\", \"bundle\")\nend\n";
        let diags = run_cop_full(&IfUnlessModifier, source);
        assert!(
            !diags.is_empty(),
            "Bang method names on the previous line should not suppress modifier suggestion"
        );
    }
}
