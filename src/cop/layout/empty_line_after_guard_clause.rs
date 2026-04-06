use crate::cop::shared::node_type::{
    BREAK_NODE, CALL_NODE, IF_NODE, NEXT_NODE, RETURN_NODE, UNLESS_NODE,
};
use crate::cop::shared::util;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Enforces empty line after guard clause.
///
/// ## Corpus conformance investigation (2026-03-11, updated 2026-03-15)
///
/// **Root causes of FN (nitrocop misses offenses RuboCop catches):**
/// - `and`/`or` guard clauses: Fixed by recursing into the `right` child
///   of and/or nodes, matching RuboCop's `operator_keyword?` → `rhs` handling.
/// - Heredoc guard clauses: `raise "msg", <<-MSG unless cond` has the heredoc
///   body after the if node's location. Fixed by walking the guard's AST to find
///   heredoc arguments and using the heredoc end marker line as the effective end.
/// - Ternary guard clauses: `a ? raise(e) : b` is an IfNode with no if_keyword.
///   Fixed by detecting ternary if nodes where either branch is a guard statement.
///
/// **Root causes of FP (nitrocop flags things RuboCop doesn't):**
/// - Comment-then-blank pattern: `guard; # comment; blank; code` — fixed by
///   matching RuboCop's behavior: check the immediate next line for blank/directive
///   instead of skipping all comments to find the first code line.
/// - Heredoc interference: Fixed via heredoc end line detection.
///
/// - Whitespace-only blank lines: `is_blank_line` only matched truly empty lines,
///   but many files have trailing spaces/tabs on "blank" lines. Switched all blank
///   line checks to `is_blank_or_whitespace_line` to match RuboCop's `blank?`.
///
/// **FP fix (2026-03-17): Multi-line guard statements and continuation lines**
/// - Multi-line guard STATEMENTS (fail/raise/return call spanning lines via `\`,
///   multi-line strings, comma-separated args, or braces) are not guard clauses
///   per RuboCop's `match_guard_clause?` which requires `single_line?`. Added
///   the same check for modifier form (previously only block form was checked).
///   Exception: heredoc arguments make the statement multi-line in Prism but
///   RuboCop still treats them as valid guards.
/// - Consecutive guards with line continuations: `raise "msg" \\\n unless cond`
///   followed by `raise "msg" \\\n if cond` — the NEXT guard's `if`/`unless`
///   is on a continuation line. Extended `is_guard_line` → `is_guard_line_with_continuations`
///   to follow backslash/operator/comma continuations and multi-line expressions
///   to find the modifier keyword.
/// - Guard → comment → blank → guard pattern: RuboCop uses AST-level sibling
///   analysis which ignores comments/blanks. Extended the else branch in step 3
///   to also check for guard clauses after blank lines.
///
/// **FP fix (2026-03-18): Multi-line if/unless conditions in guard blocks**
/// - `is_multiline_guard_block` now tracks parenthesis/bracket depth and
///   continuation operators (`||`, `&&`, `==`, `===`, `\`, `,`, etc.) across
///   the `if`/`unless` condition to correctly skip condition continuation lines
///   before checking the body for a guard statement. Previously, multi-line
///   conditions like `if cond_a &&\n  cond_b\n  return\nend` would mistake the
///   second condition line for the body and fail to recognize the block as a
///   guard clause. This fixed FPs where a modifier guard was followed by a
///   multi-line block-form guard (the next sibling IS a guard, so no offense).
///
/// **FP/FN fix (2026-03-19): Ternary guard line detection was too broad**
/// - `is_ternary_guard_line` matched any line containing `?` and a guard keyword,
///   causing false negatives on lines like `return lines.include?("text")` where
///   `?` is part of the method name, not a ternary operator. Fixed by requiring
///   the `?` to be preceded by a space/`)` (expression boundary) AND followed by
///   space, plus a `:` separator — matching actual ternary `cond ? expr : expr`
///   syntax. This was the root cause of ~170 FNs across the corpus.
///
/// **FP fix (2026-03-19): String/regex-aware bracket counting and keyword matching**
/// - `is_multiline_guard_block` naively counted `[`, `{`, `(` inside string
///   literals and regex patterns, causing incorrect paren depth and wrong
///   condition continuation detection. Added `count_bracket_depth_change()` which
///   skips characters inside single/double-quoted strings and regex literals.
/// - `is_bare_guard_in_block` matched `if`/`unless` keywords inside string
///   literals (e.g., `raise "columns if you join"`), incorrectly treating bare
///   guard statements as modifier-form. Added `contains_word_outside_strings()`
///   to match keywords only outside string literals.
/// - `ends_with_continuation` had an overly strict word boundary check for
///   ` and`/` or` at end of line that rejected `to_sym or` because the `m`
///   before the space was an ident char. Removed the extra check since the
///   leading space in the pattern already ensures word separation.
///
/// **Remaining gaps:** Some edge cases with heredocs inside conditions
/// (e.g., `return true if <<~TEXT.length > bar`) may still differ.
///
/// ## Corpus conformance investigation (2026-03-20)
///
/// A remaining FN pattern came from lines like:
/// `return 'Object' if duck_type?` followed by
/// `return condition ? a : b`.
///
/// The previous `is_ternary_guard_line` helper worked on raw text and treated
/// any next line containing a ternary with a guard keyword somewhere on the
/// line as if the next sibling were a ternary guard clause. RuboCop's AST-level
/// check only suppresses offenses when the next sibling itself is an `if`/`unless`
/// node (including ternary `IfNode`), not when a bare `return` statement happens
/// to contain a ternary expression in its value. Fix: reject ternary-guard
/// suppression when the line itself starts with a guard keyword.
///
/// A remaining FP pattern came from consecutive guard blocks where the next
/// `if` condition continues onto the next line after a plain comparison
/// operator, e.g. `if (size || other) >` and the threshold on the next line.
/// `is_multiline_guard_block` uses `ends_with_continuation()` to keep scanning
/// condition lines, but that helper recognized `>=`/`<=` and not bare `>`/`<`.
/// Fix: treat trailing `<` and `>` as continuation operators while scanning
/// multi-line `if`/`unless` conditions.
///
/// Another remaining FP came from `next unless ...` followed by an
/// `unless..raise..end` guard block where the raise string contained bracket
/// characters like `[` or `{`. `is_bare_guard_in_block` used naive byte-level
/// bracket counting to reject multi-line guard statements, so brackets inside
/// string literals made the raise look like an unterminated expression and the
/// sibling guard block was missed. Fix: reuse `count_bracket_depth_change()`
/// there as well so bracket counting ignores string and regex content.
///
/// A remaining FN pattern came from normal expression lines that happened to
/// contain an embedded `return ... if ...` inside a block, e.g.
/// `items.each { |x| return true if predicate(x) }`. The old
/// `contains_modifier_guard()` helper matched any line containing a guard
/// keyword plus `if`/`unless`, even when both were nested inside a block body.
/// RuboCop only suppresses offenses when the next sibling itself is a guard
/// clause, not when the line merely contains an embedded guard expression.
/// Fix: require the guard keyword and modifier keyword to appear at top level
/// on the line (outside strings and bracketed subexpressions).
///
/// Another remaining FN pattern came from `if`/`unless` blocks whose first body
/// line only contained an operator guard nested inside braces, e.g.
/// `items.each { |x| predicate(x) and return x }`. `is_bare_guard_in_block`
/// treated any line containing `and`/`or`/`&&`/`||` plus a guard keyword as a
/// guard block body, even when both tokens were nested inside the block literal.
/// RuboCop only treats the first branch statement itself as a guard clause.
/// Fix: require operator guards inside block bodies to appear at top level too.
///
/// Another remaining FN cluster came from CRLF files. For modifier guards,
/// the cop checks whether there is more code after the `if`/`unless` node on
/// the same physical line and skips embedded expressions like
/// `arr.each { return x if cond }`. In CRLF files, that raw suffix slice could
/// begin with `\r`, and the old code treated `\r` as non-whitespace "code",
/// causing real guard clauses to be skipped. Fix: treat `\r`/`\n` as ignorable
/// whitespace in the same-line suffix check.
///
/// Another remaining FN family came from block-form `if` nodes with `elsif`
/// or `else` branches. The old Prism port rejected any `if` with
/// `subsequent().is_some()`, but RuboCop's `contains_guard_clause?` only asks
/// whether the node's `if_branch` is a guard clause. That means a multi-branch
/// `if` whose first branch is a guard still counts and should require a blank
/// line before the following statement. Fix: keep handling ternaries
/// separately, but do not reject ordinary `if` nodes just because they have
/// `elsif`/`else` branches.
///
/// **Reverted experiment (2026-03-20): UTF-8 same-line slicing and raw-text
/// guard-keyword boundary tightening**
/// - Attempted fix: commit `b6471854` (reverted in `74be9231`) fixed real FN
///   patterns where modifier guards contained UTF-8 bytes (`"✓"`, `"✅"`) and
///   where the next sibling line merely contained guard-like method names such
///   as `::Kernel.raise`, `fail!`, or `next!`.
/// - Acceptance gate before the experiment:
///   `check-cop.py --verbose --rerun` reported `expected=21,085 actual=21,124
///   excess=39 missing=0`, and `verify-cop-locations.py` was at `FN 16 remain`.
/// - Acceptance gate after the experiment:
///   `check-cop.py --verbose --rerun` reported `expected=21,085 actual=21,134
///   excess=49 missing=0`, while `verify-cop-locations.py` improved to
///   `FN 10 remain`.
/// - Effect: the change fixed 6 known oracle FN locations, but introduced 10
///   additional aggregate excess offenses elsewhere in the corpus.
/// - Root cause of regression: the raw-text sibling classification became
///   broader/narrower in ways that improved known examples but changed
///   guard-line suppression behavior on previously-unseen repo patterns.
/// - A correct future fix needs repo-level identification of those new excess
///   offenses first; do not reland the reverted boundary-tightening approach
///   without isolating the regressions.
///
/// ## Corpus investigation (2026-03-23)
///
/// Corpus oracle reported FP=1, FN=28.
///
/// FP=1: `if raise_error ... else ... end` with `or raise` in the if-branch
/// was treated as a guard clause. RuboCop's `guard_clause_node` returns nil
/// for `if...else...end` without elsif (regular if/else is never a guard).
/// Fix: skip block-form if nodes that have a direct else clause (not elsif).
///
/// FN=28: Various modifier guard patterns (return/next/break if) followed by
/// code without a blank line. Not addressed in this batch — requires deeper
/// investigation of the text-based next-sibling classification.
///
/// ## Corpus investigation (2026-03-23, batch 2: FP=0, FN=45)
///
/// Three root causes identified for FN:
///
/// 1. **`is_ident_char` excluded `!` and `?`**: Ruby method names can end in
///    `!` or `?` (e.g., `next!`, `fail!`, `include?`). The word boundary check
///    treated `!`/`?` as non-ident characters, so `next` matched in `next!` and
///    `fail` matched in `fail!`. The next-sibling guard-line classification then
///    falsely suppressed the offense. Fix: include `!` and `?` in `is_ident_char`.
///
/// 2. **Qualified receiver calls like `::Kernel.raise`**: `contains_modifier_guard`
///    matched `raise` inside `::Kernel.raise ... if cond` because `.` was a valid
///    word boundary. RuboCop's `match_guard_clause?` requires `(send nil? ...)` —
///    bare calls with no receiver. Fix: added `contains_guard_keyword_at_top_level`
///    which rejects matches preceded by `.`.
///
/// 3. **UTF-8 byte/char offset mismatch**: The embedded-expression check used
///    `end_col` (UTF-8 character count from `offset_to_line_col`) as a byte index
///    into the raw line. For lines with multi-byte characters (emojis like `✅`,
///    `✓`, CJK characters), the character count is smaller than the byte offset,
///    causing the cop to think there was code after the guard on the same line and
///    skip real offenses. Fix: compute byte position using `effective_end_offset`
///    minus the line's start byte offset.
///
/// 4. **Block-form `if...else...end` skip was too broad**: The previous FP=1 fix
///    blanket-skipped all `if...else...end` nodes. But RuboCop DOES flag
///    `if...raise...else...end` as guard clauses (it checks `if_branch.guard_clause?`,
///    not whether there's an else). The suppression only applies when the `if` node
///    has no right_sibling (embedded in assignment like `ret = if...else...end`).
///    Fix: only skip when the `if` keyword has code before it on the same line.
///
/// ## Corpus investigation (2026-03-24: FP=0, FN=13)
///
/// Two root causes identified for FN:
///
/// 1. **`unless...else...end` with guard in unless body**: The cop blanket-skipped
///    all `UnlessNode` with `else_clause().is_some()`. But in RuboCop's parser AST,
///    `unless cond; A; else; B; end` is stored as `(if cond B A)`, and `if_branch`
///    returns the unless body `A`, not the else body `B`. So `unless cond; raise; else; code; end`
///    has `raise` as the guard clause and should be checked. Fix: handle unless/else
///    like if/else — check if the unless keyword is preceded by code on its line
///    (embedded expression like `ret = unless...else...end`), and only skip in
///    that case. Otherwise, proceed to check the unless body for guard clauses.
///
/// 2. **Multiline guard with code after it on end line**: The embedded-expression
///    check compared end position to detect code sharing a line with the guard
///    (e.g., `arr.each { return x if cond }`). But for multiline guards spanning
///    lines N to M (like `raise "x" if cond.find { |x| x.valid? }; next_stmt`),
///    the code after the guard on line M is a SEPARATE statement — not an embedded
///    expression. RuboCop's `multiple_statements_on_line?` uses `same_line?` which
///    compares START lines, so a guard starting on line N with next stmt on line M
///    (N != M) is not considered "same line". Fix: only skip embedded expressions
///    when `if_start_line == if_end_line` (single-line guard). For multiline guards
///    with code on the end line, directly flag the offense.
///
/// ## Corpus investigation (2026-03-30: fixture FN batch)
///
/// A remaining FN cluster came from next-sibling text classification being too
/// permissive in four narrow ways:
///
/// 1. **Inline comments leaking `if` into bare returns**: lines like
///    `return '.html' # if all else falls through` were treated as modifier
///    guards because `is_guard_line` searched the full line text. Fix: strip
///    inline comments outside strings before same-line sibling classification.
///
/// 2. **Top-level `rescue` modifiers are not guard siblings**: lines like
///    `return foo rescue 0 if cond` or `return "ip" if IPAddr.new(x) rescue false`
///    are not `if_branch.guard_clause?` in RuboCop because the modifier `if`
///    sits under a `rescue` expression. Fix: reject next-sibling guard
///    suppression when the line contains a top-level `rescue`.
///
/// 3. **Nested ternaries inside `if` conditions**: a line like
///    `if items.find { cond ? nil : break }` is not itself a ternary sibling.
///    Fix: require ternary `? :` and guard keywords to appear at top level.
///
/// 4. **Block-form guard bodies with unterminated string literals**: a sibling
///    `if..raise..end` whose raise string continues onto the next line is not a
///    single-line guard clause. Fix: treat unclosed string/regex literals in the
///    first body line as multiline, so they do not suppress the offense.
///
/// 5. **Percent-string interpolation is not an inline comment**: lines like
///    `next %(#{$1}...) if cond` and `raise Error, %(#{value})` were truncated at
///    `#{...}` by `strip_inline_comment`, so consecutive guard detection broke.
///    Fix: teach inline-comment stripping and literal-closure checks about `%(...)`
///    literals.
///
/// 6. **`end if ...` suffix belongs to the same statement**: for a multiline
///    block guard followed by a trailing modifier (e.g. `end if next_line...`),
///    the raw-text fast path treated the suffix as a separate next statement and
///    raised a false positive. Fix: ignore attached modifier suffixes when checking
///    for same-line trailing code after a multiline guard.
///
/// ## Corpus investigation (2026-03-30: semicolon and nested-modifier batch)
///
/// Four remaining mismatches came from next-sibling classification still being
/// broader than RuboCop in a few line-oriented cases:
///
/// 1. **Top-level semicolon before a guard**: lines like
///    `foo(); return if cond` are two sibling statements on one line. The next
///    sibling is the `foo()` call, not a guard clause. Fix: reject
///    `contains_modifier_guard()` matches when a top-level `;` appears on the line.
///
/// 2. **Nested modifiers need split handling**:
///    - Nested modifier lines like `return if cond unless outer` and
///      `return if a if b` are not standalone guard clauses and should not be
///      reported themselves.
///    - But those same lines also must NOT suppress the previous guard as a
///      "next sibling guard clause".
///      Fix: next-sibling text classification now requires exactly one top-level
///      modifier keyword on the line, and modifier nodes with any trailing
///      attached modifier suffix are skipped.
///
/// 3. **Symbol literals named like guard keywords**: `if node.type == :return`
///    is not a guard block. Fix: reject top-level guard-keyword text matches
///    when they are immediately preceded by `:`.
///
/// 4. **Open ternary returns in block siblings**: `return cond ?` on the first
///    body line of the next `if` block is multiline and fails RuboCop's
///    `single_line?` check. Fix: treat top-level ternaries without a same-line
///    `:` as multiline in `is_bare_guard_in_block`.
///
/// ## Corpus investigation (2026-03-31: FN batch — semicolons, do-keyword,
/// backslash args, operator guard modifier, heredoc in condition)
///
/// 1. **Trailing semicolons on guards**: `return if cond;` was treated as
///    embedded expression. Fix: semicolons are statement terminators; check
///    for code after the semicolon — only flag multiline guards with code
///    after the semicolon (e.g., `}; do_something`).
///
/// 2. **`do` keyword in next-sibling blocks**: `return...do |x|` blocks were
///    not recognized as guard-suppressing blocks. Fix: detect `do` keyword
///    (with optional block params) via `ends_with_do_keyword()`.
///
/// 3. **Backslash between raise args**: `raise "str1"\ "str2" if cond` was
///    incorrectly seen as single-line guard. Fix: track backslash continuations
///    in `is_multiline_modifier_guard` and mark args as multi-line when the
///    continuation joins arguments (not modifier keywords).
///
/// 4. **Operator guard with modifier not suppressing**: `do_thing and return
///    if cond` followed by a block-form if should not suppress. Fix: reject
///    operator guards with modifier keywords in `is_bare_guard_in_block`.
///
/// 5. **Heredoc in condition**: `return if cond && <<~MSG` — the heredoc
///    extends the guard's effective end line but offense location should be
///    at the guard line (not heredoc marker). Fix: search body first for
///    heredoc, then full node; only use heredoc end marker for offense
///    location when heredoc is in the body.
///
/// 6. **Semicolons in next-sibling guard detection**: lines like
///    `return if a; return if b` have two `if` keywords, failing the
///    `modifier_count == 1` check. Fix: truncate at the first top-level
///    semicolon before counting modifiers in `is_guard_line`.
///
/// ## Corpus investigation (2026-04-01: inline brace-block semicolon sibling)
///
/// A remaining FP cluster came from single-line guard clauses inside inline
/// brace/lambda bodies where another statement follows after a top-level `;`,
/// e.g. `break if cond; work`. RuboCop treats these as the same-line
/// multi-statement case and skips the cop, but nitrocop only treated `; code`
/// as embedded context for multiline guards. Fix: if a single-line guard has
/// real code after a top-level semicolon, skip the offense.
///
/// ## Corpus investigation (2026-04-02: embedded expressions, spaced directives,
/// attached rescue suffixes)
///
/// Remaining FP came from five narrow parser/text mismatches:
///
/// 1. **Block guards used as assignment values**: multiline `x = if ... end`
///    and `x ||= if ... end` have no right sibling in RuboCop and should be
///    skipped. Fix: skip block-form if/unless guards whose immediate parent is
///    an assignment/write node.
///
/// 2. **Multiline guards that end with only closing delimiters**: embedded
///    guards inside inline blocks/calls such as
///    `items.each { return unless cond.call(...\n  ...) }` were treated as if
///    `}`/`)` were a following statement. Fix: when the guard's end line is
///    followed by at least one `)`, `]`, or `}` plus optional comment/whitespace,
///    treat that as the same enclosing expression and skip.
///
/// 3. **Nested semicolons inside parens**: `return (foo = (a; b)) if cond`
///    should still count as a guard line. Fix: `find_top_level_semicolon`
///    now ignores semicolons inside bracketed subexpressions/literals.
///
/// 4. **`end rescue nil` attached to a guard block**: RuboCop treats the
///    rescue modifier as attached syntax on the same expression, not as a new
///    following statement. Fix: treat same-line `rescue` suffixes like other
///    attached modifiers and skip the cop.
///
/// 5. **Spaced directive comments**: RuboCop accepts both
///    `# rubocop:enable` and `# rubocop: enable`. Fix: normalize optional
///    spaces after `rubocop:` before matching `enable`.
///
/// Remaining FN in fixtures came from heredocs in the CONDITION of a modifier
/// guard: RuboCop checks spacing after the guard line itself, not after the
/// heredoc terminator. Fix: only extend the effective end line for heredocs
/// that live in the guard body/arguments, not condition-only heredocs.
///
/// ## Corpus investigation (2026-04-02: `@if` ivars and `?>` char literals)
///
/// Two remaining corpus mismatches came from overly broad text heuristics:
///
/// 1. **`@if` / `$if` were counted as modifier keywords**: next-sibling guard
///    classification uses word-boundary checks on raw text. The old boundary
///    logic treated `@if.nil?` as containing a standalone `if`, so
///    `next unless @if.nil? || @if.call(...)` was rejected as a consecutive
///    guard and nitrocop flagged the previous guard. Fix: treat Ruby variable
///    sigils (`@`, `$`) as identifier characters for keyword-boundary checks.
///
/// 2. **Character literals ending with `>` / `<` were treated as continuation
///    operators**: `is_multiline_guard_block` uses `ends_with_continuation()`
///    to keep scanning multi-line `if` conditions. A line like `if byte == ?>`
///    ends with the bytes `?>`, not a dangling `>` comparison operator, but the
///    old helper still treated it as a continuation. That caused the next body
///    line to be skipped and a later `break` to be misread as the first branch
///    statement, suppressing a real offense. Fix: keep bare `>` / `<`
///    continuation handling for comparison operators, but reject Ruby char
///    literals `?>` and `?<`.
///
/// ## Corpus investigation (2026-04-04: FP=5 → FP=0)
///
/// Five FP root causes identified and fixed:
///
/// 1. **`count_bracket_depth_change` missing percent literal support**: `%{you'l ...}`
///    has an apostrophe inside that triggered false single-quote tracking, hiding the
///    closing `}` and creating depth=1. Added percent literal handling (matching
///    `match_percent_literal`) so `%{...}`, `%q{...}`, etc. are properly skipped.
///
/// 2. **`count_bracket_depth_change` missing inline comment handling**: inline comments
///    like `# (see 14 days / 7 days)` had brackets and regex-starting `/` that confused
///    the depth counter. Added `b'#' => break` to stop counting at inline comments.
///
/// 3. **`count_top_level_word_occurrences` missing regex and percent literal support**:
///    regex `/\A\'|\'\Z/` had `'` chars treated as single-quote delimiters, hiding the
///    modifier `if` keyword. Added regex tracking (same heuristic as other functions)
///    and percent literal tracking so keywords inside literals are properly skipped.
///
/// 4. **`check_ternary_guard` missing heredoc detection**: ternary `cond ? raise(<<-HEREDOC) : val`
///    had the IfNode ending on the ternary line (not the heredoc end), causing the heredoc
///    body to be treated as the "next code line" after the guard. Added
///    `find_heredoc_end_line` traversal to use the heredoc end marker as the effective
///    end line, matching the non-ternary path.
///
/// 5. **Trailing `\` treated as code after multiline guard**: for `next if cond \`
///    spanning multiple lines via backslash continuation, the AST location ends before
///    the `\`, so the trailing `\` was treated as "code after the guard" → false offense.
///    Added explicit check for `trailing == b"\\"` to skip the line continuation marker.
///
/// ## Corpus investigation (2026-04-06: FP=16 → FP=0)
///
/// Two root causes identified for all 16 FPs:
///
/// 1. **Division operator `/` misidentified as regex start**: The `/` regex
///    heuristic in `count_top_level_word_occurrences` treated any `/` preceded
///    by a space as a regex delimiter. For lines like
///    `return [ (amount / @@kib).round(1), "KiB" ] if amount < @@mib`, the `/`
///    in `amount / @@kib` was preceded by a space, so the scanner entered regex
///    mode and stayed there (no closing `/`), hiding the `if` keyword entirely.
///    The next sibling was then not recognized as a guard clause, causing 15 FPs.
///    Fix: added `is_likely_regex_slash()` that looks past whitespace to find the
///    real context character — identifiers and closing brackets indicate division,
///    not regex.
///
/// 2. **`#{}` string interpolation breaking double-quote tracking**: For lines
///    like `"text #{expr} #{"%.8b" % val}" if cond`, the `"` inside `#{"..."}`
///    was treated as closing the outer double-quoted string. The scanner then
///    misidentified `%` as starting a percent literal, hiding the `if` keyword.
///    Fix: added `interp_depth` tracking in `count_top_level_word_occurrences`
///    that enters `#{...}` interpolation mode and only tracks `{}`-depth until
///    the interpolation closes, preventing inner `"` from ending the outer string.
pub struct EmptyLineAfterGuardClause;

/// Guard clause keywords that appear at the start of an expression.
const GUARD_METHODS: &[&[u8]] = &[b"return", b"raise", b"fail", b"next", b"break"];

impl Cop for EmptyLineAfterGuardClause {
    fn name(&self) -> &'static str {
        "Layout/EmptyLineAfterGuardClause"
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            BREAK_NODE,
            CALL_NODE,
            IF_NODE,
            NEXT_NODE,
            RETURN_NODE,
            UNLESS_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Extract body statements, the overall location, and whether it's block form.
        // We handle both modifier and block-form if/unless, plus ternaries.
        let (body_stmts, loc, end_keyword_loc, is_ternary) = if let Some(if_node) =
            node.as_if_node()
        {
            // Skip elsif nodes
            if let Some(kw) = if_node.if_keyword_loc() {
                if kw.as_slice() == b"elsif" {
                    return;
                }
            }
            // Ternary: no if_keyword_loc, has else branch
            if if_node.if_keyword_loc().is_none() {
                // Ternary guard: check if either branch contains a guard
                if if_node.subsequent().is_some() {
                    // Has else branch — check if the if-branch is a guard
                    if let Some(stmts) = if_node.statements() {
                        let body: Vec<_> = stmts.body().iter().collect();
                        if body.len() == 1 && is_guard_stmt(&body[0]) {
                            // Ternary with guard in if-branch
                            return self.check_ternary_guard(
                                source,
                                node,
                                &if_node.location(),
                                diagnostics,
                                &mut corrections,
                            );
                        }
                    }
                }
                return;
            }
            // Block-form if/else/end: RuboCop suppresses the offense when the
            // `if` node has no right_sibling (i.e., it's embedded in an assignment
            // like `ret = if...else...end`). We approximate this by checking if the
            // `if` keyword is NOT the first non-whitespace on its line — if there's
            // code before it (like `ret = `), it's an embedded expression and should
            // be skipped.
            if if_node.end_keyword_loc().is_some() {
                if let Some(subsequent) = if_node.subsequent() {
                    if subsequent.as_else_node().is_some() {
                        // Check if `if` keyword is preceded by code on the same line
                        if let Some(kw) = if_node.if_keyword_loc() {
                            let kw_line = source.offset_to_line_col(kw.start_offset()).0;
                            let lines_vec: Vec<&[u8]> = source.lines().collect();
                            if lines_vec.get(kw_line.saturating_sub(1)).is_some() {
                                // Check if all chars before the `if` keyword are whitespace
                                // Use byte offset: compute bytes before the keyword
                                let line_start_offset =
                                    source.line_col_to_offset(kw_line, 0).unwrap_or(0);
                                let kw_byte_offset = kw.start_offset();
                                let prefix = &source.as_bytes()[line_start_offset..kw_byte_offset];
                                let has_code_before =
                                    prefix.iter().any(|&b| b != b' ' && b != b'\t');
                                if has_code_before {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
            match if_node.statements() {
                Some(s) => (s, if_node.location(), if_node.end_keyword_loc(), false),
                None => return,
            }
        } else if let Some(unless_node) = node.as_unless_node() {
            // Handle unless/else forms similarly to if/else:
            // RuboCop checks `if_branch.guard_clause?` — for `unless` nodes in
            // the parser gem, `if_branch` is the unless body (the `statements`),
            // not the else body. So `unless cond; raise; else; code; end` has
            // `raise` as the guard and should be checked for blank line.
            //
            // For embedded `unless...else...end` (like `ret = unless...else...end`),
            // skip the offense (same as if/else embedded handling).
            if unless_node.else_clause().is_some() {
                {
                    let kw = unless_node.keyword_loc();
                    let kw_line = source.offset_to_line_col(kw.start_offset()).0;
                    let line_start_offset = source.line_col_to_offset(kw_line, 0).unwrap_or(0);
                    let kw_byte_offset = kw.start_offset();
                    let prefix = &source.as_bytes()[line_start_offset..kw_byte_offset];
                    let has_code_before = prefix.iter().any(|&b| b != b' ' && b != b'\t');
                    if has_code_before {
                        return;
                    }
                }
            }
            match unless_node.statements() {
                Some(s) => (
                    s,
                    unless_node.location(),
                    unless_node.end_keyword_loc(),
                    false,
                ),
                None => return,
            }
        } else {
            return;
        };

        let is_modifier = end_keyword_loc.is_none() && !is_ternary;

        let stmts: Vec<_> = body_stmts.body().iter().collect();
        if stmts.is_empty() {
            return;
        }

        let first_stmt = &stmts[0];
        if !is_guard_stmt(first_stmt) {
            return;
        }

        // For block form, the body must be a single guard statement
        if !is_modifier && stmts.len() != 1 {
            return;
        }

        // RuboCop's guard_clause? requires the guard statement to be single-line.
        // For block form: a multi-line body like `next foo && bar && ...` is not a guard.
        // For modifier form: a multi-line guard statement like `fail "str1" \\\n "str2" if cond`
        // or `return "\n...\n" if cond` is not a guard — the raise/fail/return call itself
        // must be single-line. Exception: heredoc arguments make the statement multi-line
        // in Prism's AST but RuboCop still treats them as valid guard clauses.
        //
        // For `and`/`or` operator nodes (e.g., `expr || raise(...)`), RuboCop checks
        // `operator_keyword? ? rhs : self` — so only the `rhs` (the guard part) needs
        // to be single-line, not the entire `or` expression.
        {
            let (check_start, check_end) = guard_clause_check_location(first_stmt);
            let stmt_start_line = source.offset_to_line_col(check_start).0;
            let stmt_end_line = source.offset_to_line_col(check_end.saturating_sub(1)).0;
            if stmt_start_line != stmt_end_line {
                // For modifier form, check if the multi-line span is due to a heredoc.
                // Heredoc guards are valid despite spanning multiple lines.
                if is_modifier {
                    if find_heredoc_end_line(source, first_stmt).is_none() {
                        return;
                    }
                } else {
                    return;
                }
            }
        }

        let lines: Vec<&[u8]> = source.lines().collect();
        let if_start_line = source.offset_to_line_col(loc.start_offset()).0;

        if !is_modifier && previous_significant_line_ends_with_assignment(&lines, if_start_line) {
            return;
        }

        // Determine the end offset to use for computing the "last line" of the guard.
        // For modifier form: end of the whole if node.
        // For block form: end of the `end` keyword.
        let effective_end_offset = if let Some(ref end_kw) = end_keyword_loc {
            end_kw.end_offset().saturating_sub(1)
        } else {
            loc.end_offset().saturating_sub(1)
        };

        // The full statement can extend past the block `end` keyword, e.g.
        // `end if cond`. Use the node's actual end to decide whether there is a
        // separate statement after the guard on the same physical line.
        let statement_end_offset = loc.end_offset().saturating_sub(1);

        let if_end_line = source.offset_to_line_col(effective_end_offset).0;

        // Check for heredoc arguments — if present, the "end line" is after the
        // heredoc closing delimiter, not after the if node's source range.
        //
        // RuboCop's `last_heredoc_argument` traverses method-call chains
        // (receiver/arguments) but NOT `and`/`or` nodes. This means:
        // - `return true if <<~TEXT.length > bar` → heredoc found (in condition's
        //   method-call chain) → effective end after TEXT
        // - `return if cond && !yes?(<<~MSG)` → heredoc NOT found (behind `&&`
        //   node) → effective end at `return if ...` line
        // - `raise "msg", <<~MSG unless cond` → heredoc found (in body args) →
        //   effective end after MSG
        //
        // First search the guard body (first_stmt) for heredoc. If none found,
        // fall back to searching the full node (which includes the condition).
        // Track whether the heredoc is in the body (for offense location purposes).
        let (heredoc_end_line, heredoc_in_body) = if is_modifier {
            let body_heredoc = find_heredoc_end_line(source, first_stmt);
            if body_heredoc.is_some() {
                (body_heredoc, true)
            } else {
                (find_condition_heredoc_end_line(source, node), false)
            }
        } else {
            (None, false)
        };
        let effective_end_line = heredoc_end_line.unwrap_or(if_end_line);

        // For the offense location:
        // - Heredoc: start of heredoc end marker content (first non-whitespace on that line)
        // - Block form: start of `end` keyword
        // - Modifier form: start of the if expression
        let offense_offset = if heredoc_in_body {
            if let Some(h_line) = heredoc_end_line {
                // Heredoc in body: offense at the heredoc end marker
                let heredoc_line_content = lines[h_line.saturating_sub(1)];
                let indent = heredoc_line_content
                    .iter()
                    .position(|&b| b != b' ' && b != b'\t')
                    .unwrap_or(0);
                source
                    .line_col_to_offset(h_line, indent)
                    .unwrap_or(loc.start_offset())
            } else {
                loc.start_offset()
            }
        } else if let Some(ref end_kw) = end_keyword_loc {
            end_kw.start_offset()
        } else {
            loc.start_offset()
        };

        // Check if the guard clause is embedded inside a larger expression on the
        // same line (e.g. `arr.each { |x| return x if cond }`). If there is
        // non-comment code after the if node on the same line, skip.
        // Only check this for non-heredoc guards (heredoc guards span multiple lines).
        // Use byte offsets directly to avoid UTF-8 character/byte mismatch.
        //
        // RuboCop's `multiple_statements_on_line?` uses `same_line?(node, node.right_sibling)`
        // which compares START lines. So for a multiline guard spanning lines 5-7 with
        // a semicolon and next statement on line 7, RuboCop does NOT skip (start lines
        // differ). Only skip when the guard is entirely on one line (start == end).
        //
        // For multiline guards (start != end), if there's code after the guard on the
        // end line, that code IS the next statement and there's no blank line between
        // them — this is an offense (not an embedded expression).
        let mut has_code_after_multiline_guard = false;
        let mut has_attached_suffix = false;
        if heredoc_end_line.is_none() {
            let statement_end_line = source.offset_to_line_col(statement_end_offset).0;
            if let Some(cur_line) = lines.get(statement_end_line.saturating_sub(1)) {
                // Compute the byte position within the line where the if node ends.
                // effective_end_offset is a byte offset into the file; subtract the
                // line's start byte offset to get the position within the line.
                let line_start_byte = source
                    .line_col_to_offset(statement_end_line, 0)
                    .unwrap_or(0);
                let end_byte_in_line = statement_end_offset.saturating_sub(line_start_byte);
                let after_pos = end_byte_in_line + 1;
                if after_pos < cur_line.len() {
                    let rest = &cur_line[after_pos..];
                    if let Some(idx) = rest
                        .iter()
                        .position(|&b| b != b' ' && b != b'\t' && b != b'\r' && b != b'\n')
                    {
                        let trailing = &rest[idx..];
                        if is_only_closing_delimiters_then_optional_comment(trailing) {
                            return;
                        }
                        // Trailing backslash is a line continuation marker, not code
                        if trailing == b"\\" {
                            // The guard's AST location ends before the `\`, but the
                            // `\` joins this line with the next. Ignore it.
                            // (handled normally via next-line blank check)
                        } else if starts_with_keyword(trailing, b"if")
                            || starts_with_keyword(trailing, b"unless")
                            || starts_with_keyword(trailing, b"rescue")
                        {
                            has_attached_suffix = true;
                        } else if trailing[0] == b';' {
                            // Semicolon: check if there's code after it.
                            // `return if cond;` or `break if eof?; # comment` — standalone.
                            // A single-line guard with `; next_stmt` is RuboCop's
                            // multiple-statements-on-line case, so skip it.
                            // `}; do_something` on a multiline guard's end line is
                            // still a next statement sharing the line → offense.
                            let after_semi = &trailing[1..];
                            let has_code_after_semi = after_semi
                                .iter()
                                .position(|&b| b != b' ' && b != b'\t')
                                .is_some_and(|i| after_semi[i] != b'#');
                            if has_code_after_semi {
                                if if_start_line == if_end_line {
                                    return;
                                }
                                has_code_after_multiline_guard = true;
                            }
                        } else if trailing[0] != b'#' {
                            if if_start_line == if_end_line {
                                // Single-line guard with code after it on same line:
                                // embedded expression (e.g. `arr.each { return x if cond }`)
                                return;
                            }
                            // Multiline guard with code after it on end line:
                            // next statement shares a line with guard's end → offense
                            has_code_after_multiline_guard = true;
                        }
                    }
                }
            }
        }

        if has_attached_suffix {
            return;
        }

        // If a multiline guard has code after it on the end line (e.g., semicolon-
        // separated next statement), skip the normal next-line analysis and go
        // straight to the offense — there's no blank line between the guard and
        // the next statement.
        if has_code_after_multiline_guard {
            let (line, col) = source.offset_to_line_col(offense_offset);
            let mut diag = self.diagnostic(
                source,
                line,
                col,
                "Add empty line after guard clause.".to_string(),
            );
            if let Some(ref mut corr) = corrections {
                if let Some(offset) = source.line_col_to_offset(effective_end_line + 1, 0) {
                    corr.push(crate::correction::Correction {
                        start: offset,
                        end: offset,
                        replacement: "\n".to_string(),
                        cop_name: self.name(),
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
            }
            diagnostics.push(diag);
            return;
        }

        // Check if next line exists
        if effective_end_line >= lines.len() {
            return;
        }

        // Match RuboCop's logic: check the IMMEDIATE next line after the guard.
        // RuboCop does not skip comments — it checks:
        // 1. Is the next line blank? → no offense
        // 2. Is the next line an allowed directive comment, and the line after that
        //    is blank? → no offense
        // 3. Is the next sibling a guard clause or scope-closing keyword? → no offense
        // 4. Otherwise → offense

        let next_line = lines[effective_end_line]; // 0-indexed: effective_end_line is 1-indexed line number

        // Step 1: immediate next line is blank → no offense
        // Use is_blank_or_whitespace_line to match RuboCop's `blank?` which treats
        // whitespace-only lines as blank (many files have trailing spaces on "empty" lines).
        if util::is_blank_or_whitespace_line(next_line) {
            return;
        }

        // Step 2: directive/nocov comment followed by blank → no offense
        if is_allowed_directive_comment(next_line)
            && (effective_end_line + 1 >= lines.len()
                || util::is_blank_or_whitespace_line(lines[effective_end_line + 1]))
        {
            return;
        }

        // Step 3: Check the next non-comment code line for scope-close or guard.
        // This skips comments (which RuboCop ignores at AST level) to find the
        // actual next sibling statement.
        //
        // If `find_next_code_line` returns None, it either hit a blank line after
        // comments, or reached EOF after comments. In both cases:
        // - If the guard is followed only by comments → end of scope → no offense
        //   (but only if the comments lead to a scope-close like `end`)
        // - If comments → blank → code → it IS an offense (no blank immediately after guard)
        if let Some((code_content, code_line_idx)) = find_next_code_line(&lines, effective_end_line)
        {
            if is_scope_close_or_clause_keyword(code_content) {
                return;
            }
            if is_guard_line_with_continuations(code_content, &lines, code_line_idx) {
                return;
            }
            if is_multiline_guard_block(code_content, &lines, effective_end_line) {
                return;
            }
            if is_ternary_guard_line(code_content) {
                return;
            }
        } else {
            // find_next_code_line returned None — either hit a blank line (after
            // skipping comments) or reached EOF. Since the immediate next line was
            // NOT blank (checked in step 1), we have comments before the blank/EOF.
            // Check if a scope-closing keyword or guard clause follows.
            if let Some((code_after_blank, code_after_idx)) =
                find_first_code_line_anywhere(&lines, effective_end_line)
            {
                if is_scope_close_or_clause_keyword(code_after_blank) {
                    return;
                }
                // Also check if the code after blank is a guard clause —
                // matches RuboCop's AST-level sibling analysis which ignores
                // comments and blank lines between consecutive guards.
                if is_guard_line_with_continuations(code_after_blank, &lines, code_after_idx) {
                    return;
                }
                if is_multiline_guard_block(code_after_blank, &lines, code_after_idx) {
                    return;
                }
                if is_ternary_guard_line(code_after_blank) {
                    return;
                }
            } else {
                // Only comments/blanks until EOF — guard is effectively last stmt
                return;
            }
            // If there's code after the blank that's not a scope-close or guard,
            // fall through to flag the offense.
        }

        let (line, col) = source.offset_to_line_col(offense_offset);
        let mut diag = self.diagnostic(
            source,
            line,
            col,
            "Add empty line after guard clause.".to_string(),
        );
        if let Some(ref mut corr) = corrections {
            // Insert blank line after the guard clause's last line.
            // If a directive comment follows, insert after the directive line.
            let insert_after_line = if is_allowed_directive_comment(next_line) {
                effective_end_line + 1
            } else {
                effective_end_line
            };
            if let Some(offset) = source.line_col_to_offset(insert_after_line + 1, 0) {
                corr.push(crate::correction::Correction {
                    start: offset,
                    end: offset,
                    replacement: "\n".to_string(),
                    cop_name: self.name(),
                    cop_index: 0,
                });
                diag.corrected = true;
            }
        }
        diagnostics.push(diag);
    }
}

impl EmptyLineAfterGuardClause {
    /// Handle ternary guard clauses like `a ? raise(e) : other_thing`.
    /// RuboCop treats the entire ternary as a guard clause if one branch
    /// contains a guard statement (raise, return, etc.).
    fn check_ternary_guard(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        loc: &ruby_prism::Location<'_>,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let lines: Vec<&[u8]> = source.lines().collect();
        let (end_line, end_col) = source.offset_to_line_col(loc.end_offset().saturating_sub(1));

        // Check for heredoc arguments in the ternary — if present, the effective
        // end line is after the heredoc closing delimiter.
        let heredoc_end_line = find_heredoc_end_line(source, node);
        let effective_end_line = heredoc_end_line.unwrap_or(end_line);

        // Check for embedded expression on same line (only when no heredoc)
        if heredoc_end_line.is_none() {
            if let Some(cur_line) = lines.get(end_line.saturating_sub(1)) {
                let after_pos = end_col + 1;
                if after_pos < cur_line.len() {
                    let rest = &cur_line[after_pos..];
                    if let Some(idx) = rest.iter().position(|&b| b != b' ' && b != b'\t') {
                        if rest[idx] != b'#' {
                            return;
                        }
                    }
                }
            }
        }

        if effective_end_line >= lines.len() {
            return;
        }

        let next_line = lines[effective_end_line];
        if util::is_blank_or_whitespace_line(next_line) {
            return;
        }

        if is_allowed_directive_comment(next_line)
            && (effective_end_line + 1 >= lines.len()
                || util::is_blank_or_whitespace_line(lines[effective_end_line + 1]))
        {
            return;
        }

        if let Some((code_content, _)) = find_next_code_line(&lines, effective_end_line) {
            if is_scope_close_or_clause_keyword(code_content) {
                return;
            }
        } else {
            return;
        }

        let (line, col) = source.offset_to_line_col(loc.start_offset());
        let mut diag = self.diagnostic(
            source,
            line,
            col,
            "Add empty line after guard clause.".to_string(),
        );
        if let Some(corr) = corrections {
            if let Some(offset) = source.line_col_to_offset(effective_end_line + 1, 0) {
                corr.push(crate::correction::Correction {
                    start: offset,
                    end: offset,
                    replacement: "\n".to_string(),
                    cop_name: self.name(),
                    cop_index: 0,
                });
                diag.corrected = true;
            }
        }
        diagnostics.push(diag);
    }
}

/// Find the line number of the heredoc end marker if the guard clause
/// contains a heredoc argument. Returns None if no heredoc is found.
/// The returned line number is 1-indexed.
fn find_heredoc_end_line(source: &SourceFile, node: &ruby_prism::Node<'_>) -> Option<usize> {
    use ruby_prism::Visit;

    struct HeredocEndFinder<'a> {
        source: &'a SourceFile,
        max_end_line: Option<usize>,
    }

    impl<'pr> Visit<'pr> for HeredocEndFinder<'_> {
        fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
            if let Some(opening) = node.opening_loc() {
                let bytes = &self.source.as_bytes()[opening.start_offset()..opening.end_offset()];
                if bytes.starts_with(b"<<") {
                    if let Some(closing) = node.closing_loc() {
                        let end_off = closing
                            .end_offset()
                            .saturating_sub(1)
                            .max(closing.start_offset());
                        let (end_line, _) = self.source.offset_to_line_col(end_off);
                        self.max_end_line = Some(
                            self.max_end_line
                                .map_or(end_line, |prev| prev.max(end_line)),
                        );
                    }
                    return;
                }
            }
            ruby_prism::visit_string_node(self, node);
        }

        fn visit_interpolated_string_node(
            &mut self,
            node: &ruby_prism::InterpolatedStringNode<'pr>,
        ) {
            if let Some(opening) = node.opening_loc() {
                let bytes = &self.source.as_bytes()[opening.start_offset()..opening.end_offset()];
                if bytes.starts_with(b"<<") {
                    if let Some(closing) = node.closing_loc() {
                        let end_off = closing
                            .end_offset()
                            .saturating_sub(1)
                            .max(closing.start_offset());
                        let (end_line, _) = self.source.offset_to_line_col(end_off);
                        self.max_end_line = Some(
                            self.max_end_line
                                .map_or(end_line, |prev| prev.max(end_line)),
                        );
                    }
                    return;
                }
            }
            ruby_prism::visit_interpolated_string_node(self, node);
        }
    }

    let mut finder = HeredocEndFinder {
        source,
        max_end_line: None,
    };
    finder.visit(node);
    finder.max_end_line
}

/// Approximate RuboCop's condition-side heredoc handling for modifier guards.
/// RuboCop's `last_heredoc_argument` descends through method-call chains, but
/// not through boolean `&&`/`||` condition wrappers. That means:
/// - `return true if <<~TEXT.length > bar` uses the heredoc end line
/// - `return if cond && !yes?(<<~MSG)` does NOT
fn find_condition_heredoc_end_line(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> Option<usize> {
    let predicate = if let Some(if_node) = node.as_if_node() {
        if_node.predicate()
    } else if let Some(unless_node) = node.as_unless_node() {
        unless_node.predicate()
    } else {
        return None;
    };

    if predicate.as_and_node().is_some() || predicate.as_or_node().is_some() {
        return None;
    }

    find_heredoc_end_line(source, &predicate)
}

/// For `and`/`or` operator nodes, return the location of the RHS (the guard part).
/// For other nodes, return the node's own location. This mirrors RuboCop's
/// `guard_clause?` which checks `operator_keyword? ? rhs : self` and then
/// requires `single_line?` on the result.
fn guard_clause_check_location<'a>(node: &'a ruby_prism::Node<'a>) -> (usize, usize) {
    if let Some(and_node) = node.as_and_node() {
        let right = and_node.right();
        return guard_clause_check_location(&right);
    }
    if let Some(or_node) = node.as_or_node() {
        let right = or_node.right();
        return guard_clause_check_location(&right);
    }
    (node.location().start_offset(), node.location().end_offset())
}

fn is_guard_stmt(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        let name = call.name().as_slice();
        if GUARD_METHODS.contains(&name) && call.receiver().is_none() {
            return true;
        }
    }
    // Bare return/break/next
    if node.as_return_node().is_some()
        || node.as_break_node().is_some()
        || node.as_next_node().is_some()
    {
        return true;
    }
    // `and`/`or` guard clauses: `render :foo and return`, `do_thing || return`
    // RuboCop's guard_clause? checks operator_keyword? and then the rhs.
    if let Some(and_node) = node.as_and_node() {
        return is_guard_stmt(&and_node.right());
    }
    if let Some(or_node) = node.as_or_node() {
        return is_guard_stmt(&or_node.right());
    }
    false
}

/// Find the first non-blank, non-comment line starting from `start_idx` (0-indexed),
/// looking across blank lines (unlike `find_next_code_line` which stops at blanks).
/// Also returns the 0-indexed line index.
fn find_first_code_line_anywhere<'a>(
    lines: &[&'a [u8]],
    start_idx: usize,
) -> Option<(&'a [u8], usize)> {
    for (i, line) in lines[start_idx..].iter().enumerate() {
        if util::is_blank_or_whitespace_line(line) {
            continue;
        }
        if let Some(start) = line.iter().position(|&b| b != b' ' && b != b'\t') {
            let content = &line[start..];
            if content.starts_with(b"#") {
                continue;
            }
            return Some((content, start_idx + i));
        }
    }
    None
}

/// Find the next non-blank, non-comment line starting from `start_idx` (0-indexed).
/// Returns None if a blank line is found first or we reach EOF.
/// Also returns the 0-indexed line index of the found line.
fn find_next_code_line<'a>(lines: &[&'a [u8]], start_idx: usize) -> Option<(&'a [u8], usize)> {
    for (i, line) in lines[start_idx..].iter().enumerate() {
        if util::is_blank_or_whitespace_line(line) {
            return None;
        }
        if let Some(start) = line.iter().position(|&b| b != b' ' && b != b'\t') {
            let content = &line[start..];
            if content.starts_with(b"#") {
                continue;
            }
            return Some((content, start_idx + i));
        }
    }
    None
}

/// Check if trimmed content starts with a scope-closing or clause keyword.
fn is_scope_close_or_clause_keyword(content: &[u8]) -> bool {
    starts_with_keyword(content, b"end")
        || starts_with_keyword(content, b"else")
        || starts_with_keyword(content, b"elsif")
        || starts_with_keyword(content, b"rescue")
        || starts_with_keyword(content, b"ensure")
        || starts_with_keyword(content, b"when")
        || starts_with_keyword(content, b"in")
        || content.starts_with(b"}")
        || content.starts_with(b")")
}

fn starts_with_keyword(content: &[u8], keyword: &[u8]) -> bool {
    content.starts_with(keyword)
        && (content.len() == keyword.len() || !is_ident_char(content[keyword.len()]))
}

fn is_guard_line(content: &[u8]) -> bool {
    let content = strip_inline_comment(content);
    // Semicolons separate statements on the same line. Only check the first
    // statement for guard-ness (e.g., `return if a; return if b` — we only
    // need the first `return if a` to determine this is a guard line).
    let content = if let Some(semi_pos) = find_top_level_semicolon(content) {
        &content[..semi_pos]
    } else {
        content
    };
    let if_modifier_count = count_top_level_word_occurrences(content, b"if");
    let unless_modifier_count = count_top_level_word_occurrences(content, b"unless");
    let modifier_count = if_modifier_count + unless_modifier_count;

    // RuboCop's next_sibling_empty_or_guard_clause? only skips when the next
    // sibling is an if/unless node that contains a guard clause. It does NOT
    // skip for bare guard statements (return, raise, etc.).
    //
    // So we only match:
    // 1. Modifier form on the same line: `return x if cond`, `raise "..." unless something`
    // 2. Lines that start with `if`/`unless` keyword followed by a guard inside
    //    (handled separately by is_multiline_guard_block)
    //
    // Bare guard statements like `raise "error"` or `return foo` are NOT
    // considered guard lines for the purpose of this check.
    for keyword in GUARD_METHODS {
        if starts_with_keyword(content, keyword) {
            if has_top_level_rescue_modifier(content) {
                return false;
            }
            // Guard line requires exactly one top-level modifier keyword
            return modifier_count == 1;
        }
    }
    // Also check modifier if/unless containing a guard
    if contains_modifier_guard(content) {
        return true;
    }
    false
}

/// Like `is_guard_line` but also follows continuation lines to find a modifier
/// `if`/`unless` keyword. Handles patterns like:
/// - `raise "msg" \` + `  unless cond` (backslash continuation)
/// - `raise "msg" +` + `  "more" if cond` (operator continuation)
/// - `return {` + `  ...` + `}.to_json if cond` (multi-line expression)
/// - `raise Error,` + `  "msg" unless cond` (argument continuation)
/// - `raise "msg" if (` + `  long_cond` + `)` (multi-line condition)
fn is_guard_line_with_continuations(content: &[u8], lines: &[&[u8]], line_idx: usize) -> bool {
    let content = strip_inline_comment(content);

    // First check single-line (original logic)
    if is_guard_line(content) {
        return true;
    }

    // Check if the line starts with a guard keyword — if not, it can't be
    // a multi-line guard clause (we don't need to follow continuations for
    // non-guard lines).
    let starts_with_guard = GUARD_METHODS
        .iter()
        .any(|kw| starts_with_keyword(content, kw));
    if !starts_with_guard {
        return false;
    }

    // The line starts with a guard keyword but has no `if`/`unless` on this line.
    // Check continuation lines for the modifier keyword.
    // We look ahead through continuation lines — lines connected by `\` at end,
    // or lines that are part of a multi-line expression (open parens, braces,
    // trailing comma/operator).
    is_multiline_modifier_guard(lines, line_idx)
}

/// Check if the line at `line_idx` starts a multi-line modifier guard clause.
/// Scans forward through continuation lines looking for `if`/`unless` keyword.
///
/// RuboCop requires `single_line?` on the guard statement (the raise/fail/return/etc.
/// call). If the guard's arguments span multiple lines (comma or `+` continuation),
/// the guard is NOT single-line and should NOT suppress the preceding offense.
/// However, if the continuation is only `\` (backslash line wrap), the guard
/// arguments may still be single-line — the `if`/`unless` modifier is just on
/// the next line.
fn is_multiline_modifier_guard(lines: &[&[u8]], line_idx: usize) -> bool {
    let mut depth: i32 = 0; // track paren/brace nesting
    let mut is_first = true;
    let mut guard_args_multiline = false;
    let mut had_backslash_continuation = false;
    for line in &lines[line_idx..] {
        let trimmed_bytes = line
            .iter()
            .position(|&b| b != b' ' && b != b'\t')
            .map(|s| &line[s..])
            .unwrap_or(b"");
        let code = strip_inline_comment(trimmed_bytes);

        // Check if this line has a modifier `if`/`unless` BEFORE tracking depth.
        // This is important because `unless system(` has the modifier keyword before
        // the opening paren, so depth should be 0 when we check.
        if !is_first
            && depth <= 0
            && (contains_word_at_top_level(code, b"if")
                || contains_word_at_top_level(code, b"unless"))
        {
            // If the guard's arguments span multiple lines (comma/plus/open-paren
            // continuation), the guard fails RuboCop's `single_line?` check.
            //
            // For `\` continuations: if the modifier keyword is NOT the first
            // token on this line, then the `\` joined guard arguments (e.g.,
            // `raise "str1"\ "str2" if cond`), making the guard multi-line.
            if had_backslash_continuation {
                let trimmed_code = code
                    .iter()
                    .position(|&b| b != b' ' && b != b'\t')
                    .map(|s| &code[s..])
                    .unwrap_or(code);
                if !starts_with_keyword(trimmed_code, b"if")
                    && !starts_with_keyword(trimmed_code, b"unless")
                {
                    guard_args_multiline = true;
                }
            }
            return !guard_args_multiline;
        }
        is_first = false;

        // Track paren/brace depth to know when a multi-line expression closes
        for &b in code {
            match b {
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth -= 1,
                _ => {}
            }
        }

        // Check if line ends with continuation: backslash, operator, or comma
        let stripped = trim_trailing_whitespace(code);

        let arg_continuation = stripped.ends_with(b",") || stripped.ends_with(b"+") || depth > 0;
        let line_continuation = stripped.ends_with(b"\\");
        had_backslash_continuation = line_continuation;

        if arg_continuation {
            guard_args_multiline = true;
        }

        let continues = arg_continuation || line_continuation;

        if !continues {
            break;
        }
    }
    false
}

/// Check if the next code line starts a multi-line if/unless block that contains
/// a guard clause (return/raise/fail/throw/next/break).
/// Per RuboCop, `contains_guard_clause?` checks `node.if_branch&.guard_clause?` —
/// only the FIRST statement in the if-branch, and it must be a bare guard statement
/// (not modifier-form like `return unless cond`). Also handles `and`/`or` operator
/// guard forms like `redirect_to(@work) && return`.
///
/// Handles multi-line conditions: when the `if`/`unless` condition spans multiple
/// lines (via `||`, `&&`, `\`, or parenthesized expressions), the function skips
/// condition continuation lines to find the actual body.
fn is_multiline_guard_block(content: &[u8], lines: &[&[u8]], start_idx: usize) -> bool {
    if !starts_with_keyword(content, b"if") && !starts_with_keyword(content, b"unless") {
        return false;
    }

    let content_line_idx = match find_line_index_from(lines, start_idx, content) {
        Some(idx) => idx,
        None => return false,
    };

    // Track parenthesis/bracket depth across the condition to detect multi-line
    // conditions. The condition starts on the if/unless line and continues while
    // we have unclosed parens/brackets/braces OR the line ends with a continuation
    // operator (||, &&, and, or, \, ,).
    let mut paren_depth: i32 = 0;
    let mut in_condition = true;

    // Count parens on the if/unless line itself, skipping string/regex content
    let if_line = lines[content_line_idx];
    paren_depth += count_bracket_depth_change(if_line);
    // Check if the if/unless line ends with a continuation
    let if_trimmed_end = trim_trailing_whitespace(if_line);
    if !ends_with_continuation(if_trimmed_end) && paren_depth <= 0 {
        in_condition = false;
    }

    // RuboCop checks `if_branch.guard_clause?` — only the first statement in the
    // if-branch, not all statements or the else-branch. Find the first non-blank
    // non-comment line after the condition (the if-branch body).
    for (i, line) in lines[(content_line_idx + 1)..].iter().enumerate() {
        let Some(start) = line.iter().position(|&b| b != b' ' && b != b'\t') else {
            continue;
        };
        let trimmed = &line[start..];

        // Skip comments
        if trimmed.starts_with(b"#") {
            continue;
        }

        // If we're still in the condition, update paren depth and check for continuation
        if in_condition {
            paren_depth += count_bracket_depth_change(line);
            let stripped = trim_trailing_whitespace(trimmed);
            if !ends_with_continuation(stripped) && paren_depth <= 0 {
                in_condition = false;
            }
            continue;
        }

        // Stop at scope-closing or else keywords (we've gone past the if-branch)
        if starts_with_keyword(trimmed, b"end")
            || starts_with_keyword(trimmed, b"else")
            || starts_with_keyword(trimmed, b"elsif")
        {
            break;
        }

        // The first code line is the if-branch body — check if it's a guard
        return is_bare_guard_in_block(trimmed, lines, content_line_idx + 1 + i);
    }
    false
}

/// Check if a line (already trimmed of trailing whitespace) ends with a continuation
/// pattern that indicates the expression continues on the next line.
fn ends_with_continuation(stripped: &[u8]) -> bool {
    if stripped.is_empty() {
        return false;
    }
    // Common operators and punctuation that indicate continuation
    stripped.ends_with(b"||")
        || stripped.ends_with(b"&&")
        || stripped.ends_with(b"\\")
        || stripped.ends_with(b",")
        || stripped.ends_with(b"+")
        || (stripped.ends_with(b">") && !stripped.ends_with(b"?>"))
        || (stripped.ends_with(b"<") && !stripped.ends_with(b"?<"))
        || stripped.ends_with(b"==")
        || stripped.ends_with(b"!=")
        || stripped.ends_with(b"===")
        || stripped.ends_with(b"<=")
        || stripped.ends_with(b">=")
        || stripped.ends_with(b"<=>")
        || stripped.ends_with(b"=~")
        || stripped.ends_with(b"!~")
        || {
            // Check for `and` or `or` keywords at end (with word boundary).
            // The pattern includes a leading space (e.g., b" or"), which ensures
            // the keyword is separated from the preceding token by a space.
            let len = stripped.len();
            (len >= 4 && &stripped[len - 4..] == b" and")
                || (len >= 3 && &stripped[len - 3..] == b" or")
        }
}

/// Check if a line ends with the `do` keyword (with optional block parameters).
/// Matches: `... do`, `... do |x|`, `... do |x, y|`.
/// This indicates a multi-line block that makes the statement multi-line.
fn ends_with_do_keyword(stripped: &[u8]) -> bool {
    // Check for `do |...|` pattern: line ends with `|`
    if stripped.ends_with(b"|") {
        // Find the matching `|` and then check for `do` before it
        if let Some(open_pipe) = stripped[..stripped.len() - 1]
            .iter()
            .rposition(|&b| b == b'|')
        {
            // Check for `do` keyword before the open pipe, with word boundary
            let before_pipe = trim_trailing_whitespace(&stripped[..open_pipe]);
            return before_pipe.ends_with(b"do")
                && (before_pipe.len() == 2 || !is_ident_char(before_pipe[before_pipe.len() - 3]));
        }
    }
    // Check for bare `do` at end
    stripped.ends_with(b"do")
        && (stripped.len() == 2 || !is_ident_char(stripped[stripped.len() - 3]))
}

/// Trim trailing whitespace, newlines, and carriage returns from a byte slice.
fn trim_trailing_whitespace(line: &[u8]) -> &[u8] {
    let end = line
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t' && b != b'\r' && b != b'\n')
        .map(|e| e + 1)
        .unwrap_or(0);
    &line[..end]
}

fn match_percent_literal(line: &[u8], i: usize) -> Option<(usize, u8, u8)> {
    if line.get(i) != Some(&b'%') {
        return None;
    }

    let mut delimiter_idx = i + 1;
    let next = *line.get(delimiter_idx)?;

    if next.is_ascii_alphabetic() {
        delimiter_idx += 1;
    }

    let open = *line.get(delimiter_idx)?;
    if open.is_ascii_alphanumeric() || open == b'_' || open.is_ascii_whitespace() {
        return None;
    }

    let close = match open {
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        b'<' => b'>',
        _ => open,
    };

    Some((delimiter_idx + 1, open, close))
}

/// Remove a trailing inline comment (`# ...`) from a line, but only when the
/// `#` appears outside quoted strings or regex literals.
fn strip_inline_comment(line: &[u8]) -> &[u8] {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_regex = false;
    let mut in_percent_literal: Option<(u8, u8, usize)> = None;
    let mut i = 0;

    while i < line.len() {
        let b = line[i];
        if let Some((open, close, nested_depth)) = in_percent_literal.as_mut() {
            if b == b'\\' {
                i += 2;
                continue;
            }

            if *open != *close && b == *open {
                *nested_depth += 1;
                i += 1;
                continue;
            }

            if b == *close {
                if *nested_depth == 0 {
                    in_percent_literal = None;
                } else {
                    *nested_depth -= 1;
                }
            }

            i += 1;
            continue;
        }
        if in_single_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'\'' {
                in_single_quote = false;
            }
            i += 1;
            continue;
        }
        if in_double_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }
        if in_regex {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'/' {
                in_regex = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' => in_single_quote = true,
            b'"' => in_double_quote = true,
            b'%' => {
                if let Some((next_i, open, close)) = match_percent_literal(line, i) {
                    in_percent_literal = Some((open, close, 0));
                    i = next_i;
                    continue;
                }
            }
            b'/' => {
                let is_regex_start = if i == 0 {
                    true
                } else {
                    matches!(
                        line[i - 1],
                        b'=' | b'('
                            | b','
                            | b'!'
                            | b'~'
                            | b' '
                            | b'\t'
                            | b'|'
                            | b'&'
                            | b'{'
                            | b'['
                            | b';'
                            | b':'
                    )
                };
                if is_regex_start {
                    in_regex = true;
                }
            }
            b'#' => return trim_trailing_whitespace(&line[..i]),
            _ => {}
        }

        i += 1;
    }

    trim_trailing_whitespace(line)
}

/// Find the first top-level semicolon (not inside strings/regex/percent literals).
/// Returns the byte position if found.
fn find_top_level_semicolon(line: &[u8]) -> Option<usize> {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_regex = false;
    let mut in_percent_literal: Option<(u8, u8, usize)> = None;
    let mut depth: i32 = 0;
    let mut i = 0;

    while i < line.len() {
        let b = line[i];
        if let Some((open, close, nested_depth)) = in_percent_literal.as_mut() {
            if b == b'\\' {
                i += 2;
                continue;
            }

            if *open != *close && b == *open {
                *nested_depth += 1;
                i += 1;
                continue;
            }

            if b == *close {
                if *nested_depth == 0 {
                    in_percent_literal = None;
                } else {
                    *nested_depth -= 1;
                }
            }

            i += 1;
            continue;
        }

        match b {
            b'\\' if in_double_quote || in_single_quote || in_regex => {
                i += 2;
                continue;
            }
            b'\'' if !in_double_quote && !in_regex => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote && !in_regex => in_double_quote = !in_double_quote,
            b'%' if !in_single_quote && !in_double_quote && !in_regex => {
                if let Some((next_i, open, close)) = match_percent_literal(line, i) {
                    in_percent_literal = Some((open, close, 0));
                    i = next_i;
                    continue;
                }
            }
            b'/' if !in_single_quote && !in_double_quote => {
                let is_regex_start = if i == 0 {
                    true
                } else {
                    matches!(
                        line[i - 1],
                        b'=' | b'('
                            | b','
                            | b'!'
                            | b'~'
                            | b' '
                            | b'\t'
                            | b'|'
                            | b'&'
                            | b'{'
                            | b'['
                            | b';'
                            | b':'
                    )
                };
                if is_regex_start {
                    in_regex = !in_regex;
                }
            }
            b'(' | b'{' | b'[' if !in_single_quote && !in_double_quote && !in_regex => depth += 1,
            b')' | b'}' | b']' if !in_single_quote && !in_double_quote && !in_regex => {
                depth = depth.saturating_sub(1)
            }
            b';' if !in_single_quote && !in_double_quote && !in_regex && depth == 0 => {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn has_top_level_rescue_modifier(line: &[u8]) -> bool {
    contains_word_at_top_level(line, b"rescue")
}

/// Return true when the line ends inside a quoted string or regex literal,
/// which means the expression necessarily continues onto the next line.
fn has_unclosed_literal(line: &[u8]) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_regex = false;
    let mut in_percent_literal: Option<(u8, u8, usize)> = None;
    let mut i = 0;

    while i < line.len() {
        let b = line[i];
        if let Some((open, close, nested_depth)) = in_percent_literal.as_mut() {
            if b == b'\\' {
                i += 2;
                continue;
            }

            if *open != *close && b == *open {
                *nested_depth += 1;
                i += 1;
                continue;
            }

            if b == *close {
                if *nested_depth == 0 {
                    in_percent_literal = None;
                } else {
                    *nested_depth -= 1;
                }
            }

            i += 1;
            continue;
        }
        if in_single_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'\'' {
                in_single_quote = false;
            }
            i += 1;
            continue;
        }
        if in_double_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }
        if in_regex {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'/' {
                in_regex = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' => in_single_quote = true,
            b'"' => in_double_quote = true,
            b'%' => {
                if let Some((next_i, open, close)) = match_percent_literal(line, i) {
                    in_percent_literal = Some((open, close, 0));
                    i = next_i;
                    continue;
                }
            }
            b'/' => {
                let is_regex_start = if i == 0 {
                    true
                } else {
                    matches!(
                        line[i - 1],
                        b'=' | b'('
                            | b','
                            | b'!'
                            | b'~'
                            | b' '
                            | b'\t'
                            | b'|'
                            | b'&'
                            | b'{'
                            | b'['
                            | b';'
                            | b':'
                    )
                };
                if is_regex_start {
                    in_regex = true;
                }
            }
            _ => {}
        }

        i += 1;
    }

    in_single_quote || in_double_quote || in_regex || in_percent_literal.is_some()
}

/// Check if a trimmed line inside a block is a bare guard statement.
/// This matches RuboCop's `guard_clause?` which requires `single_line?` AND
/// only matches bare guard calls (return/raise/fail/next/break), NOT modifier-form
/// guards like `return unless condition`.
/// Also handles `and`/`or`/`&&`/`||` return patterns.
fn is_bare_guard_in_block(trimmed: &[u8], lines: &[&[u8]], line_idx: usize) -> bool {
    let trimmed = strip_inline_comment(trimmed);

    // Check for guard keyword at the start of the line
    let has_guard_keyword = GUARD_METHODS
        .iter()
        .any(|kw| starts_with_keyword(trimmed, kw));

    if has_top_level_rescue_modifier(trimmed) {
        return false;
    }

    // If the line starts with a guard keyword but also has a modifier `if`/`unless`,
    // it's NOT a bare guard statement — it's a modifier-form if/unless wrapping the
    // guard. RuboCop's `guard_clause?` does NOT match these.
    // Use `contains_word_outside_strings` to avoid matching `if` inside string literals
    // like `raise "columns if you join a table"`.
    if has_guard_keyword
        && (contains_word_outside_strings(trimmed, b"if")
            || contains_word_outside_strings(trimmed, b"unless"))
    {
        return false;
    }

    // Check for `and`/`or`/`&&`/`||` with guard keyword (e.g., `redirect_to(@work) && return`)
    let has_operator_guard = !has_guard_keyword
        && GUARD_METHODS
            .iter()
            .any(|kw| contains_word_at_top_level(trimmed, kw))
        && (contains_word_at_top_level(trimmed, b"and")
            || contains_word_at_top_level(trimmed, b"or")
            || contains_pattern_at_top_level(trimmed, b"&&", false)
            || contains_pattern_at_top_level(trimmed, b"||", false));

    if !has_guard_keyword && !has_operator_guard {
        return false;
    }

    // Operator guards with a modifier `if`/`unless` wrapper are NOT bare guard
    // statements (e.g., `do_thing and return unless cond`). The `unless` modifier
    // wraps the whole expression, and RuboCop's `guard_clause?` on the `unless`
    // node checks the unless body's guard — but the containing `if` block's
    // `if_branch.guard_clause?` would evaluate the `unless` node, not the bare
    // operator guard directly. This matters for suppression logic.
    if has_operator_guard
        && (contains_word_outside_strings(trimmed, b"if")
            || contains_word_outside_strings(trimmed, b"unless"))
    {
        return false;
    }

    // Guard statement must be single-line: not continuing to the next line.
    let stripped = trimmed
        .iter()
        .rposition(|&b| b != b' ' && b != b'\t' && b != b'\r' && b != b'\n')
        .map(|end| &trimmed[..=end])
        .unwrap_or(trimmed);

    if stripped.ends_with(b"\\")
        || stripped.ends_with(b",")
        || stripped.ends_with(b"+")
        || stripped.ends_with(b".")
        || has_unclosed_literal(stripped)
        || has_open_top_level_ternary(stripped)
        || ends_with_do_keyword(stripped)
    {
        return false;
    }

    // Check for unclosed parens/braces (multi-line argument list)
    if line_idx + 1 < lines.len() {
        let next = lines[line_idx + 1];
        let next_trimmed = next
            .iter()
            .position(|&b| b != b' ' && b != b'\t')
            .map(|s| &next[s..])
            .unwrap_or(b"");
        let paren_depth = count_bracket_depth_change(trimmed);
        if paren_depth > 0 && !next_trimmed.is_empty() {
            return false;
        }
    }

    true
}

fn find_line_index_from(lines: &[&[u8]], from_idx: usize, content: &[u8]) -> Option<usize> {
    for (i, line) in lines.iter().enumerate().skip(from_idx) {
        if let Some(start) = line.iter().position(|&b| b != b' ' && b != b'\t') {
            let trimmed = &line[start..];
            if std::ptr::eq(trimmed.as_ptr(), content.as_ptr()) || trimmed == content {
                return Some(i);
            }
        }
    }
    None
}

/// Check if a line contains a ternary expression with a guard keyword in one branch.
/// Matches patterns like `cond ? raise(e) : other` or `x ? fail(msg) : y`.
/// RuboCop's `next_sibling_empty_or_guard_clause?` checks `next_sibling.if_type? &&
/// contains_guard_clause?(next_sibling)` which matches ternaries with guard if-branches.
///
/// Must distinguish actual ternaries (`cond ? expr : expr`) from method names
/// ending in `?` (like `include?`, `valid?`, `empty?`). A real ternary has
/// ` ? ` (question mark preceded by non-`?` and followed by space) that is NOT
/// inside a string literal, plus a `:` separator.
fn is_ternary_guard_line(content: &[u8]) -> bool {
    let content = strip_inline_comment(content);

    // RuboCop only suppresses when the next sibling itself is a ternary IfNode.
    // Lines that START with `return`/`raise`/`next`/`break` are bare guard
    // statements whose value happens to contain a ternary expression, not
    // ternary guard siblings.
    if starts_with_keyword(content, b"if") || starts_with_keyword(content, b"unless") {
        return false;
    }
    if GUARD_METHODS
        .iter()
        .any(|keyword| starts_with_keyword(content, keyword))
    {
        return false;
    }

    // Look for ` ? ` pattern outside of string literals.
    // A Ruby ternary looks like: `expr ? true_branch : false_branch`
    // Method calls ending in `?` look like: `foo.bar?` or `bar?(args)`
    let mut has_ternary_question = false;
    let mut depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut i = 0;
    while i < content.len() {
        let b = content[i];
        if b == b'\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if b == b'"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if b == b'\\' && (in_single_quote || in_double_quote) {
            i += 1; // skip escaped char
        } else if b == b'?' && !in_single_quote && !in_double_quote {
            if depth != 0 {
                i += 1;
                continue;
            }
            // Distinguish ternary `?` from method `?`:
            // - Ternary: `cond ? expr : expr` — `?` is followed by space, and
            //   preceded by space/`)` (end of condition expression)
            // - Method: `method?` — `?` is part of the name, preceded by ident char,
            //   typically followed by `(`, `)`, `,`, space, or end of line
            let followed_by_space =
                i + 1 < content.len() && (content[i + 1] == b' ' || content[i + 1] == b'\t');
            let preceded_by_end_of_expr =
                i > 0 && matches!(content[i - 1], b' ' | b'\t' | b')' | b']' | b'}');
            // Ternary `?` needs space after it (before true branch) and is typically
            // at an expression boundary (space or closing bracket before it).
            // Method `?` is preceded by an ident char (part of the name).
            if followed_by_space && preceded_by_end_of_expr {
                has_ternary_question = true;
                break;
            }
            // Also handle `cond? expr : expr` where `?` is directly after an ident
            // but followed by space — this is ambiguous. Ruby requires space before
            // `?` for ternary when the condition is a bare name, but single-char
            // ternaries like `a?b:c` are valid. For our purposes, we need the `:`.
            // So we check for this pattern only if followed by space and there's a
            // guard keyword + colon later.
            if followed_by_space && i > 0 && is_ident_char(content[i - 1]) {
                // Could be `a_check ? raise(e) : other` — check further
                // Actually in Ruby, `a_check?` would be a method name.
                // `a_check ? x` requires a space before `?` to be a ternary.
                // So this case is a method call, not a ternary. Skip.
            }
        } else if !in_single_quote && !in_double_quote {
            match b {
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        i += 1;
    }
    if !has_ternary_question {
        return false;
    }
    // Also verify there's a `:` separator (ternary else branch)
    let mut has_colon = false;
    let mut j = i + 1;
    depth = 0;
    in_single_quote = false;
    in_double_quote = false;
    while j < content.len() {
        let b = content[j];
        if b == b'\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if b == b'"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        } else if b == b'\\' && (in_single_quote || in_double_quote) {
            j += 1;
        } else if b == b':' && !in_single_quote && !in_double_quote {
            if depth != 0 {
                j += 1;
                continue;
            }
            // Check it's not a symbol (`:sym`) - should be preceded by space
            if j > 0
                && (content[j - 1] == b' ' || content[j - 1] == b'\t' || content[j - 1] == b')')
            {
                has_colon = true;
                break;
            }
        } else if !in_single_quote && !in_double_quote {
            match b {
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        j += 1;
    }
    if !has_colon {
        return false;
    }
    for keyword in GUARD_METHODS {
        if contains_guard_keyword_at_top_level(content, keyword) {
            return true;
        }
    }
    false
}

/// Count net bracket/paren depth change in a line, skipping characters inside
/// string literals (single/double quoted) and regex literals (starting with `/`
/// after an operator or at line start). This avoids false depth counts from
/// brackets inside strings like `"columns if you join"` or regexes like `/\[/`.
fn count_bracket_depth_change(line: &[u8]) -> i32 {
    let mut depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_regex = false;
    let mut in_percent_literal: Option<(u8, u8, usize)> = None;
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        if let Some((open, close, nested_depth)) = in_percent_literal.as_mut() {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if *open != *close && b == *open {
                *nested_depth += 1;
                i += 1;
                continue;
            }
            if b == *close {
                if *nested_depth == 0 {
                    in_percent_literal = None;
                } else {
                    *nested_depth -= 1;
                }
            }
            i += 1;
            continue;
        }
        if in_single_quote {
            if b == b'\\' {
                i += 1; // skip escaped char
            } else if b == b'\'' {
                in_single_quote = false;
            }
        } else if in_double_quote {
            if b == b'\\' {
                i += 1; // skip escaped char
            } else if b == b'"' {
                in_double_quote = false;
            }
        } else if in_regex {
            if b == b'\\' {
                i += 1; // skip escaped char
            } else if b == b'/' {
                in_regex = false;
            }
        } else {
            match b {
                b'#' => break, // inline comment — stop counting
                b'\'' => in_single_quote = true,
                b'"' => in_double_quote = true,
                b'%' => {
                    if let Some((next_i, open, close)) = match_percent_literal(line, i) {
                        in_percent_literal = Some((open, close, 0));
                        i = next_i;
                        continue;
                    }
                }
                b'/' => {
                    // Heuristic: `/` starts a regex if preceded by operator, `(`, `=`, `,`,
                    // `!`, `~`, space+operator, or at start of expression
                    let is_regex_start = if i == 0 {
                        true
                    } else {
                        let prev = line[i - 1];
                        matches!(
                            prev,
                            b'=' | b'('
                                | b','
                                | b'!'
                                | b'~'
                                | b' '
                                | b'\t'
                                | b'|'
                                | b'&'
                                | b'{'
                                | b'['
                                | b';'
                                | b':'
                        )
                    };
                    if is_regex_start {
                        in_regex = true;
                    }
                }
                b'(' | b'{' | b'[' => depth += 1,
                b')' | b'}' | b']' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
    }
    depth
}

/// Check if `word` appears as a whole word in `content`, but only outside
/// of string literals. This prevents matching keywords inside strings like
/// `"columns if you join a table"`.
fn contains_word_outside_strings(haystack: &[u8], word: &[u8]) -> bool {
    let wlen = word.len();
    if haystack.len() < wlen {
        return false;
    }
    // Track string context
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut i = 0;
    while i < haystack.len() {
        let b = haystack[i];
        if in_single_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'\'' {
                in_single_quote = false;
            }
            i += 1;
            continue;
        }
        if in_double_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }
        if b == b'\'' {
            in_single_quote = true;
            i += 1;
            continue;
        }
        if b == b'"' {
            in_double_quote = true;
            i += 1;
            continue;
        }
        // Check for word match at this position (outside strings)
        if i + wlen <= haystack.len() && &haystack[i..i + wlen] == word {
            let before_ok = i == 0 || !is_ident_char(haystack[i - 1]);
            let after_ok = i + wlen >= haystack.len() || !is_ident_char(haystack[i + wlen]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn contains_modifier_guard(content: &[u8]) -> bool {
    let content = strip_inline_comment(content);

    if has_top_level_rescue_modifier(content) {
        return false;
    }

    if contains_pattern_at_top_level(content, b";", false) {
        return false;
    }

    let modifier_count = count_top_level_word_occurrences(content, b"if")
        + count_top_level_word_occurrences(content, b"unless");
    if modifier_count != 1 {
        return false;
    }
    for keyword in GUARD_METHODS {
        // Use the receiver-aware check: `::Kernel.raise` should NOT match
        // because `raise` is a method call on a receiver, not a bare guard.
        if contains_guard_keyword_at_top_level(content, keyword) {
            return true;
        }
    }
    false
}

/// Like `contains_word_at_top_level` but also rejects matches where the guard
/// keyword is immediately preceded by `.` (dot), indicating it's a method call
/// on a receiver (e.g., `::Kernel.raise`, `Foo.fail`). RuboCop's
/// `match_guard_clause?` requires `(send nil? {:raise :fail} ...)` — a bare
/// call with no receiver.
fn contains_guard_keyword_at_top_level(haystack: &[u8], word: &[u8]) -> bool {
    let plen = word.len();
    if haystack.len() < plen {
        return false;
    }

    let mut depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut i = 0;

    while i < haystack.len() {
        let b = haystack[i];

        if in_single_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'\'' {
                in_single_quote = false;
            }
            i += 1;
            continue;
        }

        if in_double_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' => {
                in_single_quote = true;
                i += 1;
                continue;
            }
            b'"' => {
                in_double_quote = true;
                i += 1;
                continue;
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth -= 1,
            _ => {}
        }

        if depth == 0 && i + plen <= haystack.len() && &haystack[i..i + plen] == word {
            let before_ok = i == 0 || !is_ident_char(haystack[i - 1]);
            let after_ok = i + plen >= haystack.len() || !is_ident_char(haystack[i + plen]);
            // Reject if preceded by `.` (method call on receiver) or `:`
            // (symbol literal like `:return`).
            let not_method_call_or_symbol =
                i == 0 || (haystack[i - 1] != b'.' && haystack[i - 1] != b':');
            if before_ok && after_ok && not_method_call_or_symbol {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn has_open_top_level_ternary(content: &[u8]) -> bool {
    let mut depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_regex = false;
    let mut in_percent_literal: Option<(u8, u8, usize)> = None;
    let mut saw_top_level_ternary_question = false;
    let mut i = 0;

    while i < content.len() {
        let b = content[i];

        if let Some((open, close, nested_depth)) = in_percent_literal.as_mut() {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if *open != *close && b == *open {
                *nested_depth += 1;
                i += 1;
                continue;
            }
            if b == *close {
                if *nested_depth == 0 {
                    in_percent_literal = None;
                } else {
                    *nested_depth -= 1;
                }
            }
            i += 1;
            continue;
        }

        if in_single_quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'\'' {
                in_single_quote = false;
            }
            i += 1;
            continue;
        }

        if in_double_quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }

        if in_regex {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == b'/' {
                in_regex = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' => {
                in_single_quote = true;
                i += 1;
                continue;
            }
            b'"' => {
                in_double_quote = true;
                i += 1;
                continue;
            }
            b'%' => {
                if let Some((next_i, open, close)) = match_percent_literal(content, i) {
                    in_percent_literal = Some((open, close, 0));
                    i = next_i;
                    continue;
                }
            }
            b'/' => {
                let is_regex_start = if i == 0 {
                    true
                } else {
                    matches!(
                        content[i - 1],
                        b'=' | b'('
                            | b','
                            | b'!'
                            | b'~'
                            | b' '
                            | b'\t'
                            | b'|'
                            | b'&'
                            | b'{'
                            | b'['
                            | b';'
                            | b':'
                    )
                };
                if is_regex_start {
                    in_regex = true;
                    i += 1;
                    continue;
                }
            }
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => depth = depth.saturating_sub(1),
            b'?' if depth == 0 => {
                let preceded_by_end_of_expr =
                    i > 0 && matches!(content[i - 1], b' ' | b'\t' | b')' | b']' | b'}');
                let followed_by_space_or_eol = i + 1 >= content.len()
                    || matches!(content[i + 1], b' ' | b'\t' | b'\r' | b'\n');
                if preceded_by_end_of_expr && followed_by_space_or_eol {
                    saw_top_level_ternary_question = true;
                }
            }
            b':' if depth == 0 && saw_top_level_ternary_question => {
                let preceded_by_separator = i > 0 && matches!(content[i - 1], b' ' | b'\t' | b')');
                if preceded_by_separator {
                    return false;
                }
            }
            _ => {}
        }

        i += 1;
    }

    saw_top_level_ternary_question
}

fn contains_word_at_top_level(haystack: &[u8], word: &[u8]) -> bool {
    contains_pattern_at_top_level(haystack, word, true)
}

fn count_top_level_word_occurrences(haystack: &[u8], word: &[u8]) -> usize {
    let plen = word.len();
    if haystack.len() < plen {
        return 0;
    }

    let mut depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut in_regex = false;
    let mut in_percent_literal: Option<(u8, u8, usize)> = None;
    let mut interp_depth: i32 = 0; // tracks #{...} nesting inside double-quoted strings
    let mut count = 0;
    let mut i = 0;

    while i < haystack.len() {
        let b = haystack[i];

        if let Some((open, close, nested_depth)) = in_percent_literal.as_mut() {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if *open != *close && b == *open {
                *nested_depth += 1;
                i += 1;
                continue;
            }
            if b == *close {
                if *nested_depth == 0 {
                    in_percent_literal = None;
                } else {
                    *nested_depth -= 1;
                }
            }
            i += 1;
            continue;
        }

        if in_single_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'\'' {
                in_single_quote = false;
            }
            i += 1;
            continue;
        }

        if in_double_quote {
            // Inside #{...} interpolation: only track brace depth
            if interp_depth > 0 {
                match b {
                    b'{' => interp_depth += 1,
                    b'}' => interp_depth -= 1,
                    _ => {}
                }
                i += 1;
                continue;
            }
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'"' {
                in_double_quote = false;
            } else if b == b'#' && i + 1 < haystack.len() && haystack[i + 1] == b'{' {
                interp_depth = 1;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        if in_regex {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'/' {
                in_regex = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' => {
                in_single_quote = true;
                i += 1;
                continue;
            }
            b'"' => {
                in_double_quote = true;
                i += 1;
                continue;
            }
            b'%' => {
                if let Some((next_i, open, close)) = match_percent_literal(haystack, i) {
                    in_percent_literal = Some((open, close, 0));
                    i = next_i;
                    continue;
                }
            }
            b'/' => {
                if is_likely_regex_slash(haystack, i) {
                    in_regex = true;
                    i += 1;
                    continue;
                }
            }
            b'(' | b'{' | b'[' => {
                depth += 1;
                i += 1;
                continue;
            }
            b')' | b'}' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            _ => {}
        }

        if depth == 0 && i + plen <= haystack.len() && &haystack[i..i + plen] == word {
            let before_ok = i == 0 || !is_ident_char(haystack[i - 1]);
            let after_ok = i + plen >= haystack.len() || !is_ident_char(haystack[i + plen]);
            if before_ok && after_ok {
                count += 1;
            }
        }

        i += 1;
    }

    count
}

fn contains_pattern_at_top_level(
    haystack: &[u8],
    pattern: &[u8],
    require_word_boundaries: bool,
) -> bool {
    let plen = pattern.len();
    if haystack.len() < plen {
        return false;
    }

    let mut depth: i32 = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut i = 0;

    while i < haystack.len() {
        let b = haystack[i];

        if in_single_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'\'' {
                in_single_quote = false;
            }
            i += 1;
            continue;
        }

        if in_double_quote {
            if b == b'\\' {
                i += 2;
                continue;
            } else if b == b'"' {
                in_double_quote = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'\'' => {
                in_single_quote = true;
                i += 1;
                continue;
            }
            b'"' => {
                in_double_quote = true;
                i += 1;
                continue;
            }
            b'(' | b'{' | b'[' => {
                depth += 1;
                i += 1;
                continue;
            }
            b')' | b'}' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            _ => {}
        }

        if depth == 0 && i + plen <= haystack.len() && &haystack[i..i + plen] == pattern {
            let boundaries_ok = !require_word_boundaries
                || ((i == 0 || !is_ident_char(haystack[i - 1]))
                    && (i + plen >= haystack.len() || !is_ident_char(haystack[i + plen])));
            if boundaries_ok {
                return true;
            }
        }

        i += 1;
    }

    false
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'!' | b'?' | b'@' | b'$')
}

/// Heuristic to determine if `/` at position `i` in `haystack` is likely a regex
/// delimiter start rather than a division operator. When the character immediately
/// before `/` is a space or tab, looks further back past whitespace — identifiers
/// and closing brackets/parens indicate division (e.g., `amount / 2`), while
/// operators and opening delimiters indicate regex (e.g., `if /pattern/`).
fn is_likely_regex_slash(haystack: &[u8], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = haystack[i - 1];
    if prev != b' ' && prev != b'\t' {
        return matches!(
            prev,
            b'=' | b'(' | b',' | b'!' | b'~' | b'|' | b'&' | b'{' | b'[' | b';' | b':'
        );
    }
    // Previous char is space/tab — look back past whitespace for context
    for &c in haystack[..i - 1].iter().rev() {
        if c == b' ' || c == b'\t' {
            continue;
        }
        // After identifier chars (variable/method names) or closing delimiters
        // → division operator, not regex
        if is_ident_char(c) || matches!(c, b')' | b']' | b'}') {
            return false;
        }
        // After operators or opening delimiters → regex
        return true;
    }
    // Only whitespace before → start of expression → regex
    true
}

/// Check if a line is an "allowed directive comment" per RuboCop's definition.
/// This includes `rubocop:enable` directives and `:nocov:` comments, but NOT
/// `rubocop:disable` directives. RuboCop treats `rubocop:enable` specially
/// because it pairs with a preceding `rubocop:disable` that wraps the guard,
/// so the blank line should go after the `enable` comment, not between the
/// guard and the `enable`.
fn is_allowed_directive_comment(line: &[u8]) -> bool {
    let Some(trimmed) = trim_to_comment_content(line) else {
        return false;
    };
    // rubocop:enable is allowed (but NOT rubocop:disable). RuboCop accepts
    // optional spaces after `rubocop:`.
    if let Some(after_prefix) = trimmed.strip_prefix(b"rubocop:") {
        let after_prefix = after_prefix
            .iter()
            .position(|&b| b != b' ')
            .map(|idx| &after_prefix[idx..])
            .unwrap_or(b"");
        return after_prefix.starts_with(b"enable");
    }
    trimmed.starts_with(b":nocov:")
}

/// Extract the content after `#` from a comment line, trimming whitespace.
/// Returns None if the line is not a comment.
fn trim_to_comment_content(line: &[u8]) -> Option<&[u8]> {
    let start = line.iter().position(|&b| b != b' ' && b != b'\t')?;
    let content = &line[start..];
    if !content.starts_with(b"#") {
        return None;
    }
    let after_hash = &content[1..];
    let trimmed = after_hash
        .iter()
        .position(|&b| b != b' ')
        .map(|i| &after_hash[i..])
        .unwrap_or(b"");
    Some(trimmed)
}

fn is_only_closing_delimiters_then_optional_comment(trailing: &[u8]) -> bool {
    let mut i = 0;
    let mut saw_closing_delimiter = false;
    while i < trailing.len() {
        match trailing[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b')' | b']' | b'}' => {
                saw_closing_delimiter = true;
                i += 1;
            }
            b'#' => return saw_closing_delimiter,
            _ => return false,
        }
    }
    saw_closing_delimiter
}

fn previous_significant_line_ends_with_assignment(lines: &[&[u8]], line_number: usize) -> bool {
    if line_number <= 1 {
        return false;
    }

    for line in lines[..line_number - 1].iter().rev() {
        let trimmed = trim_trailing_whitespace(strip_inline_comment(line));
        let Some(start) = trimmed.iter().position(|&b| b != b' ' && b != b'\t') else {
            continue;
        };
        let content = &trimmed[start..];
        if content.starts_with(b"#") {
            continue;
        }

        return ends_with_assignment_continuation(content);
    }

    false
}

fn ends_with_assignment_continuation(content: &[u8]) -> bool {
    content.ends_with(b"=")
        && !content.ends_with(b"==")
        && !content.ends_with(b"===")
        && !content.ends_with(b"!=")
        && !content.ends_with(b"<=")
        && !content.ends_with(b">=")
        && !content.ends_with(b"=~")
        && !content.ends_with(b"!~")
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        EmptyLineAfterGuardClause,
        "cops/layout/empty_line_after_guard_clause"
    );
    crate::cop_autocorrect_fixture_tests!(
        EmptyLineAfterGuardClause,
        "cops/layout/empty_line_after_guard_clause"
    );

    #[test]
    fn and_return_guard_detected() {
        let source = b"def bar\n  render :foo and return if condition\n  do_something\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for `and return` guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn or_return_guard_detected() {
        let source = b"def baz\n  render :foo or return if condition\n  do_something\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for `or return` guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_before_begin_detected() {
        let source = b"def foo\n  return another_object if something_different?\n  begin\n    bar\n  rescue SomeException\n    baz\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for guard before begin, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_then_rubocop_disable_detected() {
        let source = b"def foo\n  return if condition\n  # rubocop:disable Department/Cop\n  bar\n  # rubocop:enable Department/Cop\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for guard then rubocop:disable, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn ternary_guard_detected() {
        let source = b"def foo\n  puts 'some action happens here'\nrescue => e\n  a_check ? raise(e) : other_thing\n  true\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for ternary guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_then_rubocop_enable_then_code_detected() {
        let source = b"def foo\n  # rubocop:disable Department/Cop\n  return if condition\n  # rubocop:enable Department/Cop\n  bar\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for guard then rubocop:enable then code, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_before_if_block_with_multiline_raise() {
        // FN: guard clause followed by `if` block containing multi-line raise
        // (multi-line raise is NOT a guard clause per RuboCop's single_line? check)
        let source = b"def foo\n  return if !argv\n  if argv.empty? || argv.length > 2\n    raise Errors::CLIInvalidUsage,\n      help: opts.help.chomp\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for guard before if-with-multiline-raise, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_before_if_block_with_single_line_raise_no_offense() {
        // No offense: guard followed by if block with single-line raise (IS a guard clause)
        let source =
            b"def foo\n  return if !argv\n  if argv.empty?\n    raise \"error\"\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for guard before if-with-single-line-raise, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_followed_by_last_expression_no_offense() {
        // FP: guard clause followed by ternary with guard keyword — RuboCop treats as guard
        let source = b"def foo\n  return unless broken_rule\n  fail_build ? fail(message) : warn(message)\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for guard before ternary expression, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_followed_by_comment_then_if_guard_no_offense() {
        // FP: guard clause followed by comment, then blank, then if-block with guard
        let source = b"def foo\n  return true if result\n  # comment\n  # more comment\n\n  if BCrypt::Password.new(enc) == [password].join\n    return true\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for guard then comment then if-guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn block_guard_followed_by_if_and_return_no_offense() {
        // FP: block-form guard `unless..raise..end` followed by if-block with `&& return`
        let source = b"def foo\n  unless @work\n    raise \"not found\"\n  end\n  if @collection\n    redirect_to(@work) && return\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for block guard before if-with-and-return, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_followed_by_last_expression_with_next() {
        // FP: `next unless check_port` followed by ternary with break/next
        let source = b"items.each do |item|\n  next unless item.check_port\n  item.run || error ? break : next\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for guard before ternary in block, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn block_form_guard_end_followed_by_code() {
        // FN: block-form `unless valid?; raise; end` followed by `if` code
        let source = b"def foo\n  unless valid?(level)\n    raise \"invalid\"\n  end\n  if logger.respond_to?(:add)\n    logger.add(level, message)\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for block guard end then code, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn guard_then_if_block_with_modifier_return_offense() {
        // FN: `return unless doc.blocks?` followed by if-block where body is
        // `return unless (...)` — a modifier-form return is NOT a guard per RuboCop
        let source = b"def foo\n  return unless doc.blocks?\n  if (first_block = doc.blocks[0]).context == :preamble\n    return unless (first_block = first_block.blocks[0])\n  elsif first_block.context == :section\n    return\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for guard before if-with-modifier-return, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fp_multiline_if_block_with_return_next_sibling_guard() {
        // FP: multi-line `if..end` block containing `return` — followed by another guard.
        // RuboCop sees next sibling is a guard clause → no offense for the if..end.
        let source = b"def send\n  return if cond_a\n  if SomeClass ===\n       (\n         begin\n           @msg.message\n         rescue StandardError\n           nil\n         end\n       )\n    return\n  end\n  return skip(reason) if @msg.blank?\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for multiline if-block with return then guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fp_guard_then_multiline_if_with_return() {
        // FP: `return unless cond` followed by multi-line `if..end` block that contains
        // a bare return. RuboCop checks next_sibling.if_type? && contains_guard_clause?
        let source = b"def foo\n  return unless @post.topic\n  if @post.id != @post.topic.category.id &&\n       !(@post.is_first_post?)\n    return\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for guard then multiline if with return, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fp_multiline_if_guard_at_end_of_method() {
        // The `if..end` block is the last statement → its next sibling is nil → no offense.
        let source = b"def foo\n  return unless active?\n  return if status != \"regular\"\n  return if pending.exists?\n  if created_by.bot? || created_by.staff? ||\n       created_by.has_trust_level?(4)\n    return\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        for d in &diags {
            eprintln!("  DIAG: {:?}", d);
        }
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for multiline if guard at end of method, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fp_block_break_then_multiline_if() {
        // Two consecutive block-form if/break guards, second with multi-line condition.
        // Both are at end of method -- no offense.
        let source = b"def foo\n  if (m == l)\n    break\n  end\n  if (@h[m] * (q.abs + r.abs) <\n    eps * (p.abs * (@h[m-1].abs + z.abs +\n    @h[m+1].abs)))\n    break\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        for d in &diags {
            eprintln!(
                "  DIAG: {}:{}:{} {}",
                d.path, d.location.line, d.location.column, d.message
            );
        }
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for consecutive block break guards, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_simple_return_true_if_guard() {
        // FN: simple `return true if cond` followed by non-guard code
        let source =
            b"def ask_user(question)\n  return true if args['-y']\n  $stderr.print question\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for return true if guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_return_nil_unless_guard() {
        // FN: `return nil unless cond` followed by code
        let source = b"def foo\n  return nil unless time\n  Time.at(time)\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for return nil unless guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_raise_if_guard() {
        // FN: `raise e if cond` followed by non-guard code
        let source = b"def foo\nrescue => e\n  raise e if args['--debug']\n  warn e.message\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for raise if guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_next_unless_guard() {
        // FN: `next unless e.end` followed by code
        let source = b"items.each do |e|\n  next unless e.end\n  e.update :sheet => \"_x\"\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for next unless guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_break_if_guard() {
        // FN: `break if nil != sheet` followed by code
        let source =
            b"loop do\n  break if nil != sheet\n  new_dir = File.expand_path('..', dir)\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for break if guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_return_empty_string_unless_guard() {
        // FN: `return '' unless cond` followed by code
        let source = b"def fmt(time)\n  return '' unless time.respond_to?(:strftime)\n  time.strftime('%H:%M:%S')\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for return empty string unless guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_return_unless_before_local_assignment() {
        let source = b"def checks_for_integer_overflow(nbits)\n  reset_subcounts()\n  return  unless nbits >= 6\n  nbits_int_min = n_bits_integer_min(nbits)\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for `return unless` before local assignment, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_return_unless_before_local_assignment_crlf() {
        let source = b"def checks_for_integer_overflow(nbits)\r\n  reset_subcounts()\r\n  return  unless nbits >= 6\r\n  nbits_int_min = n_bits_integer_min(nbits)\r\nend\r\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for CRLF `return unless` before local assignment, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_unless_else_with_guard_in_body() {
        // FN: `unless cond; raise; else; code; end` followed by code
        // RuboCop checks `if_branch.guard_clause?` — for `unless`, the if_branch
        // is the unless body (not the else body). So `unless cond; raise; else; ...; end`
        // has `raise` as the guard clause and should require a blank line.
        let source = b"def foo\n  unless cond\n    raise \"error\"\n  else\n    do_thing\n  end\n  next_code\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for unless/else with guard in body, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn no_offense_unless_else_guard_in_else_body() {
        // No offense: `unless cond; do_thing; else; raise; end`
        // RuboCop's if_branch for unless is the unless body (do_thing), not the else body.
        // do_thing is NOT a guard clause → no offense.
        let source = b"def foo\n  unless cond\n    do_thing\n  else\n    raise \"error\"\n  end\n  next_code\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for unless/else with guard in else body, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_guard_in_rescue_body() {
        // FN: guard clause inside rescue body followed by code
        let source = b"def foo\nrescue => e\n  return if ignored?(e)\n  handle(e)\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for guard in rescue body, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_guard_in_case_when_body() {
        // FN: guard clause inside case/when body followed by code
        let source =
            b"def foo\n  case x\n  when :a\n    return if skip?\n    process\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for guard in case/when body, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_multiline_guard_semicolon_on_end_line() {
        // FN: guard clause spans multiple lines (block in condition), with a
        // semicolon and next statement on the guard's end line. RuboCop uses
        // `same_line?` which checks start lines, so it does NOT treat this as
        // multiple_statements_on_line. Nitrocop's embedded expression check
        // should only skip when code is after the guard on the SAME start line.
        let source = b"def foo\n  raise \"bad\" if !u = items.find { |x|\n    x.valid?\n  }; do_something\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for multiline guard with semicolon on end line, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_block_guard_then_if_multiline_string_raise() {
        let source = b"def foo(connect_string)\n  if GitRepository.repository_exists?(connect_string)\n    raise RepositoryCollision, \"There is already a repository at #{connect_string}\"\n  end\n  if File.exist?(connect_string)\n    raise IOError, \"Could not create a repository at #{connect_string}: some directory with same name exists\n                         already\"\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense for block guard before multiline-string raise block, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_guard_then_rescue_modifier_return() {
        let source = b"def rating_average\n  return self.rating_avg if attributes.has_key?('rating_avg')\n  return (rating_statistic.rating_avg || 0) rescue 0 if acts_as_rated_options[:stats_class]\n  avg\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense when next sibling uses rescue modifier around return, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_guard_then_rescue_modifier_condition() {
        let source = b"def determine_lease_type\n  return nil if group.nil?\n  return \"ip\" if IPAddr.new(group) rescue false\n  return \"local\" if Admin::Group.exists? group\n  return \"external\"\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            2,
            "Expected 2 offenses when rescue modifier is followed by another plain guard, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_guard_then_if_with_nested_ternary_in_condition() {
        let source = b"def foo(remaining)\n  return if remaining.empty?\n  if remaining.find { |n| (type = n.type) == :blank ? nil : ((BLOCK_TYPES.include? type) ? true : break) }\n    el.options[:compound] = true\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense when next if condition contains nested ternary guard keywords, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fn_guard_then_bare_return_comment_contains_if() {
        let source = b"def output_extension(mime)\n  return '.css' if mime.eql? 'text/css'\n  return '.html' # if all else falls trough\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense when next bare return only mentions `if` in a comment, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fp_guard_then_percent_interpolated_guard() {
        let source = b"def inline_link_substitution(text)\n  text.gsub InlineLinkRx do\n    if $2 && !$5\n      next $&.slice 1, $&.length if $1.start_with? RS\n      next %(#{$1}#{$&.slice $1.length + 1, $&.length}) if $3.start_with? RS\n      next $& unless $6\n    end\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for consecutive guards with percent-string interpolation, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fp_block_guard_then_block_guard_with_percent_string() {
        let source = b"def validate_processor(kind_name, block, processor)\n  unless (name = as_symbol processor.name)\n    raise ::ArgumentError, %(No name specified for #{kind_name} extension at #{block.source_location.join ':'})\n  end\n  unless processor.process_block_given?\n    raise ::NoMethodError, %(No block specified to process #{kind_name} extension at #{block.source_location.join ':'})\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for consecutive block guards with percent-string interpolation, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn fp_guard_end_if_before_elsif() {
        let source = b"def parser_comment_guard(normal, next_line, reader, document, attributes)\n  if normal && next_line.start_with? '.'\n    return true\n  elsif !normal || (next_line.start_with? '/')\n    if next_line == '//'\n      return true\n    elsif normal && (uniform? next_line, '/', (ll = next_line.length))\n      unless ll == 3\n        reader.read_lines_until terminator: next_line, skip_first_line: true, preserve_last_line: true, skip_processing: true, context: :comment\n        return true\n      end\n    else\n      return true unless next_line.start_with? '///'\n    end if next_line.start_with? '//'\n  elsif normal && (next_line.start_with? ':') && AttributeEntryRx =~ next_line\n    process_attribute_entry reader, document, attributes, $~\n    return true\n  end\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            0,
            "Expected 0 offenses for a guard block whose statement ends with `end if ...` before `elsif`, got {}: {:?}",
            diags.len(),
            diags
        );
    }

    #[test]
    fn offense_when_statement_follows_end_on_same_line() {
        let source = b"def foo(a)\n  if a\n    return\n  end; work\nend\n";
        let diags = crate::testutil::run_cop_full(&EmptyLineAfterGuardClause, source);
        assert_eq!(
            diags.len(),
            1,
            "Expected 1 offense when a separate statement follows `end` on the same line, got {}: {:?}",
            diags.len(),
            diags
        );
    }
}
