use crate::cop::shared::node_type::ARRAY_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Byte offset of the first non-whitespace character on a line.
/// Equivalent to RuboCop's `source_line =~ /\S/`. Handles both spaces and tabs.
/// Returns the byte count of leading whitespace (tabs count as 1 byte each).
fn first_non_whitespace_column(line: &[u8]) -> usize {
    line.iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

/// Convert a byte offset within a line to a character (codepoint) offset.
/// Counts non-continuation bytes (bytes where (b & 0xC0) != 0x80) in the
/// range [0, byte_col). For ASCII-only lines, byte_col == char_col.
fn byte_col_to_char_col(line_bytes: &[u8], byte_col: usize) -> usize {
    let end = byte_col.min(line_bytes.len());
    line_bytes[..end]
        .iter()
        .filter(|&&b| (b & 0xC0) != 0x80)
        .count()
}

/// Layout/FirstArrayElementIndentation cop.
///
/// ## Investigation findings (2026-03-15)
///
/// **FP root cause (208 FPs, now fixed):** `find_left_paren_on_line` was finding
/// the `(` of method calls like `gc.draw('fmt' % [...])` where the `%` operator
/// sits between `(` and `[`. The array is the RHS of `%`, not a direct argument
/// of the method call. Fix: `is_preceded_by_percent_operator()` checks if `[` is
/// immediately preceded by `%` and falls back to line-relative indent.
///
/// **FP root cause #2 (subset):** `find_left_paren_on_line` stopped at unmatched
/// `{` (hash literal) before `[`, returning `None` and using line-relative. This
/// caused missed offenses for patterns like `method({ key: [...] })` where RuboCop
/// uses paren-relative. Fix: continue scanning past unmatched `{` to find `(`.
/// `is_direct_argument` now also scans past `}` after `]` to detect hash chaining
/// (`{ key: [...] }.to_json`), correctly falling back to line-relative in that case.
///
/// **FP root cause #3 (2026-03-16, 138 FPs):** Two sub-causes:
/// a) Tab indentation: `indentation_of()` only counted spaces, returning 0 for
///    tab-indented lines. But `offset_to_line_col` counted tabs as 1 character.
///    So closing bracket `\t]` had close_col=1 but indent_base=0. Fix: use
///    `first_non_whitespace_column()` (byte offset of first non-whitespace char,
///    matching RuboCop's `source_line =~ /\S/`) for both sides of the comparison.
///    This fixed WhatWeb (~54 FPs) and phlex (~23 FPs).
/// b) Array inside hash that is chained: `method({ key: [...], k2: v }.to_json)`
///    — `is_direct_argument` checked after `]`, saw `,` (hash entry separator),
///    and returned true. But the enclosing hash is chained (`.to_json`), so
///    RuboCop uses line-relative indent. Fix: `find_left_paren_on_line` now
///    tracks unmatched `{` between `(` and `[`; when present, `is_direct_argument`
///    scans forward past hash entries to find `}`, then checks after `}`.
///    This fixed restforce (~24 FPs) and similar patterns.
///
/// **FP root cause #4 (2026-03-16):** Three additional sub-causes:
/// a) Array chained inside hash: `{ "c" => [...].compact }` — when `inside_hash`
///    is true, `is_direct_argument` scanned for `}` without first checking if `]`
///    is directly followed by `.compact`. Fix: check array chain before hash scan.
/// b) Binary operator between `(` and `[`: `(CONST + [...]).freeze` — the `(`
///    is a grouping paren, not a method call paren. Fix: `find_left_paren_on_line`
///    now detects binary operators at depth 0 between `(` and `[`.
/// c) Hash-key-relative indentation: `{ ruby: [...], js: [...] }` — elements
///    indented relative to hash key, not line start. Fix: detect hash key before
///    `[` and accept `key_col + width` / `key_col` as valid indentation.
///
/// **FP/FN root cause #5 (2026-03-18, 34 FP + 82 FN):** Three sub-causes:
/// a) String literal contents not skipped: `find_left_paren_on_line` scanned raw
///    bytes without skipping string literals. Characters like `-`, `/`, `*`, `+`
///    inside strings (e.g., `".section__in-favor"`, `'/'`) were misidentified as
///    binary operators, causing incorrect fallback to line-relative indent.
///    Fix: backward scan now skips `'...'` and `"..."` string literals.
///    This fixed ~20 FPs (decidim, vagrant, zammad, etc.) and ~10 FNs (CocoaPods,
///    endoflife, fae, oga, fluent).
/// b) Lambda `->` treated as binary minus: The `-` in `->` lambda literal was
///    detected as a subtraction operator. Fix: check if `-` is followed by `>`.
///    This fixed 12 FPs in light-service (nested `reduce_until(->(...), [...])` patterns).
/// c) Splat `*` treated as multiplication: `*[...]` and `*%w[...]` splat operators
///    were detected as binary `*`, preventing paren-relative indent.
///    Fix: `is_splat_before_array()` checks if `*` is followed by `[` or `%`.
///    This fixed ~40 FNs (rdoc, image_optim, geocoder, danbooru).
/// d) Hash-key-relative too permissive for single-pair hashes: `matches_hash_key`
///    accepted any hash-key-relative indentation, but RuboCop only accepts it for
///    multi-pair hashes. Fix: `is_multi_pair_hash()` checks for `,` + another key
///    after `]` or before the hash key on the opening line.
///    This fixed ~16 FNs (discourse single-key patterns like `requires_login except: [...]`).
///
/// **FP/FN root cause #6 (2026-03-19, 6 FP fixed):** Three sub-causes:
/// a) Grouping parens misidentified as method call parens: `assert_equal ({...})`
///    and `([...])` have `(` not preceded by a method name (preceded by space, `{`,
///    or line start). Fix: `is_grouping_paren` flag in `ParenScanResult` checks
///    the character before `(`.
/// b) Ternary `?` between `(` and `[`: `(flag ? [...] : nil)` has a ternary
///    operator at depth 0. Fix: `?` added to binary operator detection.
/// c) Byte-vs-char column mismatch: `open_col` was character-based but used as
///    byte index in `find_left_paren_on_line` and `find_hash_key_column`. For
///    multi-byte UTF-8 chars (e.g., `á` in oga repo), the byte scan started at
///    the wrong position, missing the `(`. Fix: compute `open_byte_col` from
///    byte offset arithmetic; convert results back with `byte_col_to_char_col`.
/// d) Hash-value array closing bracket: for arrays that are hash values, RuboCop
///    accepts the closing bracket at line-indent level even when paren-relative
///    is used for elements. Fix: added exemption.
///
/// **Remaining FNs (12):** Multi-pair hash arrays (ManageIQ, puppetlabs) where
/// RuboCop requires hash-key-relative as PRIMARY indentation but nitrocop uses
/// line/paren-relative. Making hash-key-relative primary caused ~600 FPs in the
/// corpus (many cases where paren-relative takes precedence). Empty array closing
/// bracket checks (markevans, gel-rb, jruby, natalie) caused similar FP regression.
/// These patterns need a more nuanced approach to `is_multi_pair_hash` that
/// distinguishes same-line vs cross-line closing bracket + next pair layouts.
///
/// **FP/FN root cause #7 (2026-03-23, 85 FP + 95 FN fixed):** Three sub-causes:
/// a) First element on same line as `[`: RuboCop's `check` returns early when
///    `same_line?(first_elem, left_bracket)`, skipping both element and closing
///    bracket checks. Nitrocop only skipped element checks but still checked
///    closing brackets, producing 83+ FPs (puppetlabs, chef, natalie, vagrant,
///    hexapdf, loomio, fluent, etc.). Fix: `return` early when `elem_line == open_line`.
/// b) Single-pair hash value closing bracket exemption in paren-relative mode:
///    The exemption at `hash_key_col.is_some() && !is_multi_pair && close_col == line_indent`
///    was applied regardless of `base_type`. For paren-relative arrays like
///    `create(:x, :groups => [...])`, RuboCop still enforces paren-relative for the
///    closing bracket. Fix: only apply exemption for `StartOfLine` base type.
///    Also added `intermediate_method_call` check: when there's a `.` between `(`
///    and the hash key (e.g., `expect(client.search body: [...])`), the paren
///    belongs to an outer call, so use line-relative indent. This fixed 95 FNs
///    (NatLabRockies/api-umbrella).
/// c) Comma-separated argument binary op false positive: `find_left_paren_on_line`
///    detected `?` in `create(flag ? :x : nil, key: [...])` as a grouping-paren
///    indicator. Fix: reset `has_binary_op` when `,` at depth 0 is encountered
///    during backward scan, since operators before commas are part of preceding
///    argument expressions. Fixed 2 FPs (antiwork/gumroad).
///
/// **FP/FN root cause #8 (2026-03-24):** Three sub-causes:
/// a) Empty arrays skipped entirely: `elements.is_empty()` returned early, skipping
///    the closing bracket check. RuboCop still checks closing brackets for empty
///    arrays (e.g., `a << [\n  ]` should flag `]`). Fix: removed early return.
/// b) Hash-key-relative not primary: RuboCop's `indent_base` checks `parent_hash_key`
///    BEFORE style-specific checks (paren-relative or line-relative). Nitrocop used
///    hash-key-relative only as a secondary acceptance fallback. Fix: added
///    `ParentHashKey` as a proper `IndentBaseType` variant and check it first when
///    the array is a hash pair value with a right sibling on a subsequent line.
///    This matches RuboCop's `right_sibling_begins_on_subsequent_line?` check.
/// c) Backward multi-pair check too permissive: `is_multi_pair_hash` backward scan
///    detected preceding hash pairs, applying hash-key-relative even for last pairs.
///    RuboCop only uses hash-key-relative when the pair has a RIGHT sibling (forward),
///    not a left sibling. Fix: backward check now requires `has_right_sibling_any_line`
///    to also be true.
/// d) `consistent` style missing hash-key-relative: RuboCop applies hash-key-relative
///    for ALL styles (except `align_brackets`), not just `special_inside_parentheses`.
///    Fix: moved hash-key-relative check before style-specific dispatch.
///
/// **FP fix / fixture repair (2026-03-30):** Arrays inside explicit
/// `super(...)` calls were treated as paren-relative by raw source scanning, so
/// nitrocop flagged `super(:only => [ ... ])` even though RuboCop accepts the
/// line-relative indentation. RuboCop only passes paren context from `on_send`;
/// `super(...)` is a `SuperNode`, so these arrays fall back to the `on_array`
/// path. Fixed by detecting when the unmatched `(` that anchors the array is
/// the `(` from `super(` and suppressing paren-relative indentation in that
/// case. Also repaired four malformed FN fixture snippets that had been pasted
/// into `offense.rb` without their enclosing Ruby context.
///
/// **FN fix (2026-03-30):** Arrays in keyword or single-pair hash arguments
/// were still missed when earlier arguments on the same call contained method
/// chains, for example `load_yaml_file(File.join(...), permitted_classes: %i[`
/// or `FactoryBot.create(... Time.now.utc, :groups => [`. The
/// `has_method_call_between` heuristic treated any top-level `.` between `(`
/// and the hash key as an "intermediate method call", even when that dot was
/// in a PREVIOUS argument separated by a comma. Fix: only consider dots in the
/// current top-level argument segment after the most recent comma.
///
/// **FP/FN root cause #9 (2026-04-01):** Four sub-causes:
/// a) Bracketless nested calls inside outer parens, e.g.
///    `with(expand_paths [ ... ])`, were treated as if the array belonged to
///    the outer `with(` call. RuboCop keeps these line-relative because the
///    array is the argument of the inner command-style call. Fix:
///    `has_command_call_before_array()` suppresses paren-relative indentation
///    when the current top-level argument segment ends with a command-style
///    method call before `[`.
/// b) Complex `=>` hash keys, e.g. `Source.new(...) => [`, were anchored to the
///    tail of the key expression (`)`), not the pair start. RuboCop uses
///    `pair.loc.column`. Fix: `find_pair_start_before_rocket()` now scans back
///    to the start of the whole hash pair expression.
/// c) Hash-key-relative indentation was still applied when the right sibling
///    started on the SAME line as the closing bracket, e.g.
///    `pageids: [ ... ], iilimit: 50`. RuboCop only uses parent-hash-key when
///    the right sibling begins on a SUBSEQUENT line. Fix: remove the backward
///    "any right sibling" heuristic and require
///    `has_right_sibling_on_subsequent_line()` directly.
/// d) Explicit `.(` call syntax, e.g. `array.([`, was misclassified as a
///    grouping paren because `(` is preceded by `.`. Fix: treat `.` as a valid
///    method-call precursor when deciding if `(` is grouping.
///
/// **FP root cause #10 (2026-04-02, 16 FP fixed):** Three remaining scanner
/// edge cases:
/// a) Arrays chained on a following line, e.g. `eq([ ... ]\n  .map ...)`, were
///    treated as direct arguments because chaining was only checked on the same
///    line after `]`. Fix: skip newlines/comments when checking what follows the
///    array (and enclosing hash) before deciding whether it is chained.
/// b) Top-level comma handling in `find_left_paren_on_line` was too blunt: it
///    cleared operators already seen in the CURRENT argument, so
///    `to_cli_argv(config, CONFLAGS.keys - [ ... ])` lost the `-`, while also
///    letting predicate keywords in PREVIOUS args like `valid?: true, events: [`
///    leak into the current scan. Fix: remember when we've crossed the current
///    argument boundary and ignore only later operators; also ignore `?:` labels.
/// c) Old-style `=>` pair starts at the first hash pair in method-call parens,
///    e.g. `setup_settings('gear' => [ ... ])`, were anchored to the method-call
///    line indent instead of the hash pair start because the backward scan only
///    stopped at `{` or `,`. Fix: treat an enclosing top-level `(` as a valid
///    boundary and return the first token after it.
pub struct FirstArrayElementIndentation;

/// Describes what the expected indentation is relative to.
#[derive(Clone, Copy)]
enum IndentBaseType {
    /// `align_brackets` style: relative to the opening bracket `[`
    LeftBracket,
    /// `special_inside_parentheses`: relative to the first position after `(`
    FirstColumnAfterLeftParenthesis,
    /// Default: relative to the start of the line where `[` appears
    StartOfLine,
    /// Hash-key-relative: relative to the parent hash key (multi-pair hashes)
    ParentHashKey,
}

/// Result of scanning backwards from `[` to find an enclosing `(`.
struct ParenScanResult {
    /// Column of the unmatched `(`, if found.
    paren_col: Option<usize>,
    /// Whether there is an unmatched `{` between the `(` and `[`,
    /// indicating the array is nested inside a hash literal.
    has_unmatched_brace: bool,
    /// Whether there is a binary operator (`+`, `-`, `*`, `/`, `|`, `&`, `^`)
    /// or ternary `?` at depth 0 between `(` and `[`, indicating the `(` is a
    /// grouping paren and the array is part of an expression.
    has_binary_operator_at_depth_zero: bool,
    /// Whether the `(` is a grouping paren (not preceded by a method name).
    /// True when `(` is preceded by a non-word character (space, `{`, operator,
    /// start of line) rather than an identifier char.
    is_grouping_paren: bool,
}

/// Scan backwards from `bracket_col` on `line_bytes` to find an unmatched `(`
/// that contains this array. Also tracks whether there's an unmatched `{`
/// between `(` and `[`, indicating hash nesting, and whether there's a binary
/// operator at depth 0 (indicating grouping parens).
///
/// This tracks balanced parens, brackets, and braces. Unmatched `{` or `[`
/// are allowed — the array may be nested inside a hash literal or another
/// array that is itself inside method call parens (e.g.,
/// `method({ key: [...] })`). Only an unmatched `(` is returned.
fn find_left_paren_on_line(line_bytes: &[u8], bracket_col: usize) -> ParenScanResult {
    let end = bracket_col.min(line_bytes.len());
    let mut paren_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut has_unmatched_brace = false;
    let mut has_binary_op = false;
    let mut crossed_argument_boundary = false;
    let mut i = end;
    while i > 0 {
        i -= 1;
        // Skip string literals (scanning backward: when we hit a closing quote,
        // scan backward to the matching opening quote). This prevents characters
        // inside strings (like `-`, `/`, `*`, `+`) from being misidentified as
        // binary operators.
        if line_bytes[i] == b'\'' || line_bytes[i] == b'"' {
            let quote = line_bytes[i];
            if i > 0 {
                i -= 1;
                while i > 0 && line_bytes[i] != quote {
                    i -= 1;
                }
                // i now points at the opening quote (or 0 if not found); skip it
                continue;
            }
        }
        match line_bytes[i] {
            b')' => paren_depth += 1,
            b'(' => {
                if paren_depth == 0 {
                    // Check if `(` is a grouping paren by examining what precedes it.
                    // A method call paren is preceded by a word char (identifier).
                    // A grouping paren is preceded by a non-word char (space, `{`, operator, etc.)
                    // or is at the start of the line.
                    let is_grouping = if i == 0 {
                        true
                    } else {
                        let prev = line_bytes[i - 1];
                        !(prev.is_ascii_alphanumeric()
                            || prev == b'_'
                            || prev == b'!'
                            || prev == b'?'
                            || prev == b'.'
                            || prev == b']'
                            || prev == b')')
                    };
                    return ParenScanResult {
                        paren_col: Some(i),
                        has_unmatched_brace,
                        has_binary_operator_at_depth_zero: has_binary_op,
                        is_grouping_paren: is_grouping,
                    };
                }
                paren_depth -= 1;
            }
            b']' => bracket_depth += 1,
            b'[' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
            }
            b'}' => brace_depth += 1,
            b'{' => {
                if brace_depth > 0 {
                    brace_depth -= 1;
                } else {
                    has_unmatched_brace = true;
                }
            }
            // `,` at depth 0 separates arguments. Binary operators before a
            // comma belong to a PREVIOUS argument, not the current one. Keep
            // any operator already seen in the current argument, but ignore
            // operators encountered later in the backward scan.
            b',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                crossed_argument_boundary = true;
            }
            // Detect binary/ternary operators at depth 0 (not inside nested parens/brackets/braces).
            // These indicate the `(` is a grouping paren, e.g., `(CONST + [...])` or
            // `(flag ? [...] : nil)`.
            b'+' | b'/' | b'|' | b'&' | b'^' | b'?'
                if paren_depth == 0
                    && bracket_depth == 0
                    && brace_depth == 0
                    && !crossed_argument_boundary =>
            {
                if line_bytes[i] != b'?' || !(i + 1 < line_bytes.len() && line_bytes[i + 1] == b':')
                {
                    has_binary_op = true;
                }
            }
            // `-` at depth 0: only treat as binary operator if NOT part of `->` lambda.
            b'-' if paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0
                && !crossed_argument_boundary =>
            {
                if !(i + 1 < end && line_bytes[i + 1] == b'>') {
                    has_binary_op = true;
                }
            }
            // `*` at depth 0: only treat as binary operator if NOT a splat before
            // `[` or `%` (array literal). Splat `*[...]` or `*%w[...]` means the
            // array is still a direct argument, not part of a binary expression.
            // Use full line length (not `end`) since the `[` at bracket_col is the
            // target we need to check against.
            b'*' if paren_depth == 0
                && bracket_depth == 0
                && brace_depth == 0
                && !crossed_argument_boundary =>
            {
                if !is_splat_before_array(line_bytes, i) {
                    has_binary_op = true;
                }
            }
            _ => {}
        }
    }
    ParenScanResult {
        paren_col: None,
        has_unmatched_brace,
        has_binary_operator_at_depth_zero: has_binary_op,
        is_grouping_paren: false,
    }
}

/// Check if `*` at position `star_pos` is a splat operator before an array literal.
/// Returns true if the bytes after `*` (skipping whitespace) are `[` or `%` (for `%w[`, `%i[`).
fn is_splat_before_array(line_bytes: &[u8], star_pos: usize) -> bool {
    let len = line_bytes.len();
    let mut j = star_pos + 1;
    while j < len && (line_bytes[j] == b' ' || line_bytes[j] == b'\t') {
        j += 1;
    }
    if j >= len {
        return false;
    }
    // `*[` or `*%w[`, `*%i[`, `*%W[`, `*%I[`
    line_bytes[j] == b'[' || line_bytes[j] == b'%'
}

/// For `expr => [`, find the start column of the WHOLE hash pair expression on
/// the same line, matching RuboCop's `pair.loc.column`.
///
/// This handles complex keys like `Source.new(...) => [` by scanning backward to
/// the nearest top-level `{` or `,` (or start of line), instead of anchoring to
/// the tail of the key expression.
fn find_pair_start_before_rocket(line_bytes: &[u8], rocket_gt_col: usize) -> Option<usize> {
    if rocket_gt_col == 0 || rocket_gt_col >= line_bytes.len() {
        return None;
    }
    if line_bytes[rocket_gt_col] != b'>' || line_bytes[rocket_gt_col - 1] != b'=' {
        return None;
    }

    let mut end = rocket_gt_col - 1; // index of `=`
    while end > 0 && (line_bytes[end - 1] == b' ' || line_bytes[end - 1] == b'\t') {
        end -= 1;
    }

    let mut paren_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut i = end;
    while i > 0 {
        i -= 1;

        // Skip simple string literals while scanning backward.
        if line_bytes[i] == b'\'' || line_bytes[i] == b'"' {
            let quote = line_bytes[i];
            if i > 0 {
                i -= 1;
                while i > 0 && line_bytes[i] != quote {
                    i -= 1;
                }
                continue;
            }
        }

        match line_bytes[i] {
            b')' => paren_depth += 1,
            b'(' => {
                if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 {
                    let mut start = i + 1;
                    while start < rocket_gt_col
                        && (line_bytes[start] == b' ' || line_bytes[start] == b'\t')
                    {
                        start += 1;
                    }
                    return Some(start);
                }
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            b']' => bracket_depth += 1,
            b'[' => {
                if bracket_depth > 0 {
                    bracket_depth -= 1;
                }
            }
            b'}' => brace_depth += 1,
            b'{' => {
                if brace_depth == 0 && paren_depth == 0 && bracket_depth == 0 {
                    let mut start = i + 1;
                    while start < rocket_gt_col
                        && (line_bytes[start] == b' ' || line_bytes[start] == b'\t')
                    {
                        start += 1;
                    }
                    return Some(start);
                }
                if brace_depth > 0 {
                    brace_depth -= 1;
                }
            }
            b',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                let mut start = i + 1;
                while start < rocket_gt_col
                    && (line_bytes[start] == b' ' || line_bytes[start] == b'\t')
                {
                    start += 1;
                }
                return Some(start);
            }
            _ => {}
        }
    }

    Some(first_non_whitespace_column(line_bytes))
}

/// Skip whitespace and full-line comments after an expression so follow-up
/// chaining checks can see operators and leading dots on the next line.
fn skip_trivia_forward(source_bytes: &[u8], start: usize) -> usize {
    let len = source_bytes.len();
    let mut i = start;

    loop {
        while i < len && matches!(source_bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }

        if i < len && source_bytes[i] == b'#' {
            while i < len && source_bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        return i;
    }
}

/// Find the column of a hash key that precedes `[` on the same line.
/// Detects patterns like `key: [`, `key => [`, and `"key" => [`.
/// Returns the column of the hash key's first character, or `None` if
/// no hash key is found.
fn find_hash_key_column(line_bytes: &[u8], bracket_col: usize) -> Option<usize> {
    let end = bracket_col.min(line_bytes.len());
    if end == 0 {
        return None;
    }
    let mut i = end;
    loop {
        if i == 0 {
            return None;
        }
        i -= 1;
        if line_bytes[i] != b' ' && line_bytes[i] != b'\t' {
            break;
        }
    }
    if line_bytes[i] == b'>' && i > 0 && line_bytes[i - 1] == b'=' {
        return find_pair_start_before_rocket(line_bytes, i);
    }
    // Ruby 1.9 hash syntax: `key: [`
    if line_bytes[i] != b':' {
        return None;
    }
    if i == 0 || !(line_bytes[i - 1].is_ascii_alphanumeric() || line_bytes[i - 1] == b'_') {
        return None;
    }
    if i >= 2 && line_bytes[i - 1] == b':' {
        return None;
    }
    let mut j = i - 1;
    while j > 0
        && (line_bytes[j - 1].is_ascii_alphanumeric()
            || line_bytes[j - 1] == b'_'
            || line_bytes[j - 1] == b'?'
            || line_bytes[j - 1] == b'!')
    {
        j -= 1;
    }
    Some(j)
}

/// Detect a command-style nested call in the current top-level argument segment
/// before `[`, e.g. `with(expand_paths [ ... ])`.
///
/// These arrays belong to the inner call, not the outer `(`. RuboCop therefore
/// keeps them line-relative instead of using the outer parenthesis column.
fn has_command_call_before_array(line_bytes: &[u8], start: usize, bracket_col: usize) -> bool {
    let end = bracket_col.min(line_bytes.len());
    if start >= end {
        return false;
    }

    let mut paren_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut arg_start = start;
    let mut i = start;
    while i < end {
        if line_bytes[i] == b'\'' || line_bytes[i] == b'"' {
            let quote = line_bytes[i];
            i += 1;
            while i < end && line_bytes[i] != quote {
                if line_bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }

        match line_bytes[i] {
            b'(' => paren_depth += 1,
            b')' => paren_depth -= 1,
            b'[' => bracket_depth += 1,
            b']' => bracket_depth -= 1,
            b'{' => brace_depth += 1,
            b'}' => brace_depth -= 1,
            b',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                arg_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }

    let mut seg_start = arg_start;
    while seg_start < end && (line_bytes[seg_start] == b' ' || line_bytes[seg_start] == b'\t') {
        seg_start += 1;
    }
    if seg_start >= end {
        return false;
    }

    let mut seg_end = end;
    while seg_end > seg_start
        && (line_bytes[seg_end - 1] == b' ' || line_bytes[seg_end - 1] == b'\t')
    {
        seg_end -= 1;
    }

    // Command-call syntax requires whitespace between the method name and `[`.
    if seg_end == end {
        return false;
    }

    let last = line_bytes[seg_end - 1];
    if !(last.is_ascii_alphanumeric() || last == b'_' || last == b'!' || last == b'?') {
        return false;
    }

    let mut token_start = seg_end - 1;
    while token_start > seg_start
        && (line_bytes[token_start - 1].is_ascii_alphanumeric()
            || line_bytes[token_start - 1] == b'_'
            || line_bytes[token_start - 1] == b'!'
            || line_bytes[token_start - 1] == b'?')
    {
        token_start -= 1;
    }

    // Reject hash-key segments like `body:` or `key =>`.
    let mut j = seg_start;
    let mut inner_paren_depth: i32 = 0;
    let mut inner_bracket_depth: i32 = 0;
    let mut inner_brace_depth: i32 = 0;
    while j < token_start {
        match line_bytes[j] {
            b'(' => inner_paren_depth += 1,
            b')' => inner_paren_depth -= 1,
            b'[' => inner_bracket_depth += 1,
            b']' => inner_bracket_depth -= 1,
            b'{' => inner_brace_depth += 1,
            b'}' => inner_brace_depth -= 1,
            b':' if inner_paren_depth == 0
                && inner_bracket_depth == 0
                && inner_brace_depth == 0 =>
            {
                if j + 1 >= token_start || line_bytes[j + 1] != b':' {
                    return false;
                }
            }
            b'=' if inner_paren_depth == 0
                && inner_bracket_depth == 0
                && inner_brace_depth == 0
                && j + 1 < token_start
                && line_bytes[j + 1] == b'>' =>
            {
                return false;
            }
            _ => {}
        }
        j += 1;
    }

    true
}

/// Check if there is a method call (`.`) at depth 0 in the SAME top-level
/// argument segment between `start` and `end_col` on the same line.
///
/// This detects patterns like `expect(client.search body: [` where the hash key
/// `body:` is an argument to `client.search` (intermediate method call), not to
/// `expect(`. Dots that appear in earlier arguments separated by a top-level
/// comma must be ignored, e.g. `load_yaml_file(File.join(...), key: [`.
///
/// Tracks balanced parens/brackets/braces and resets at depth-0 commas.
fn has_method_call_between(line_bytes: &[u8], start: usize, end_col: usize) -> bool {
    let end = end_col.min(line_bytes.len());
    let mut paren_depth: i32 = 0;
    let mut bracket_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut saw_method_call_in_current_arg = false;
    let mut i = start;
    while i < end {
        // Skip string literals
        if line_bytes[i] == b'\'' || line_bytes[i] == b'"' {
            let quote = line_bytes[i];
            i += 1;
            while i < end && line_bytes[i] != quote {
                if line_bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        match line_bytes[i] {
            b'(' => paren_depth += 1,
            b')' => paren_depth -= 1,
            b'[' => bracket_depth += 1,
            b']' => bracket_depth -= 1,
            b'{' => brace_depth += 1,
            b'}' => brace_depth -= 1,
            b',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                saw_method_call_in_current_arg = false;
            }
            b'.' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                saw_method_call_in_current_arg = true;
            }
            _ => {}
        }
        i += 1;
    }
    saw_method_call_in_current_arg
}

/// Check if the `[` is immediately preceded by a `%` operator (string formatting).
/// In patterns like `gc.draw('format' % [...])`, the array is the RHS of the `%`
/// operator, not a direct argument of the method call's parentheses. Scans
/// backwards from the `[` position, skipping whitespace.
fn is_preceded_by_percent_operator(line_bytes: &[u8], bracket_col: usize) -> bool {
    let end = bracket_col.min(line_bytes.len());
    for i in (0..end).rev() {
        match line_bytes[i] {
            b' ' | b'\t' => continue,
            b'%' => return true,
            _ => return false,
        }
    }
    false
}

/// Check what follows a given position in source bytes, skipping whitespace,
/// newlines, and comments. Returns `true` if the expression is "chained" or
/// combined with an operator (`.`, `+`, `-`, `*`, `/`, `%`, `&`, `|`, `^`).
fn is_chained_after(source_bytes: &[u8], start: usize) -> bool {
    let len = source_bytes.len();
    let i = skip_trivia_forward(source_bytes, start);
    if i >= len {
        return false;
    }
    matches!(
        source_bytes[i],
        b'.' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^'
    )
}

/// Check if the array is used as a direct argument (not as a receiver of
/// a method chain or part of a binary expression). Checks the source bytes
/// immediately after the array's closing bracket `]`.
///
/// Returns `true` if the array is a standalone argument (next non-whitespace
/// after `]` is `)`, `,`, end of line, or nothing relevant).
/// Returns `false` if `]` is followed by `.`, `+`, `-`, `*`, etc. indicating
/// the array is part of a larger expression.
///
/// When `inside_hash` is true (the array is inside a hash literal within
/// method parens), this scans forward from `]` to find the matching `}`
/// of the enclosing hash, then checks what follows that `}`.
fn is_direct_argument(source_bytes: &[u8], closing_end_offset: usize, inside_hash: bool) -> bool {
    let mut i = closing_end_offset;
    let len = source_bytes.len();

    // First, always check if the array itself is chained (e.g. `].compact`,
    // `].join`). This takes priority even when inside a hash.
    if is_chained_after(source_bytes, i) {
        return false;
    }

    if inside_hash {
        // The array is inside a hash literal. We need to find the enclosing
        // hash's closing `}` and check what follows it. Scan forward from
        // after `]`, tracking brace/bracket/paren depth to find the matching `}`.
        let mut brace_depth: i32 = 1; // we're inside one unmatched `{`
        let mut bracket_depth: i32 = 0;
        let mut paren_depth: i32 = 0;
        while i < len && brace_depth > 0 {
            match source_bytes[i] {
                b'{' => brace_depth += 1,
                b'}' => brace_depth -= 1,
                b'[' => bracket_depth += 1,
                b']' => bracket_depth -= 1,
                b'(' => paren_depth += 1,
                b')' => paren_depth -= 1,
                b'#' => {
                    // Skip to end of line (comment)
                    while i < len && source_bytes[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                b'\'' | b'"' => {
                    // Skip past string literals (simple — no interpolation tracking)
                    let quote = source_bytes[i];
                    i += 1;
                    while i < len && source_bytes[i] != quote {
                        if source_bytes[i] == b'\\' {
                            i += 1; // skip escaped char
                        }
                        i += 1;
                    }
                    // i now points at closing quote
                }
                _ => {}
            }
            i += 1;
        }
        let _ = (bracket_depth, paren_depth); // suppress unused warnings
        // i is now past the `}`. Check what follows.
        return !is_chained_after(source_bytes, i);
    }

    i = skip_trivia_forward(source_bytes, i);
    if i >= len {
        return true;
    }
    match source_bytes[i] {
        // Array is followed by a method call or operator => part of expression
        b'.' | b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' => false,
        // If `]` is followed by `}`, the array is inside a hash. Skip past
        // the `}` and any closing parens/whitespace to check if the HASH
        // is chained with a method call.
        b'}' => {
            i += 1;
            !is_chained_after(source_bytes, i)
        }
        // Everything else (closing paren, comma, newline, etc.) => direct argument
        _ => true,
    }
}

/// Check if there's a right sibling (next hash pair) that begins on a SUBSEQUENT line.
/// This matches RuboCop's `pair.last_line < pair.right_sibling.first_line`.
fn has_right_sibling_on_subsequent_line(source_bytes: &[u8], closing_end_offset: usize) -> bool {
    let len = source_bytes.len();
    let mut i = closing_end_offset;
    // Skip whitespace (not newlines) after `]`
    while i < len && (source_bytes[i] == b' ' || source_bytes[i] == b'\t') {
        i += 1;
    }
    if i >= len || source_bytes[i] != b',' {
        return false;
    }
    i += 1; // skip comma
    // Check if there's a newline before the next token
    let mut crossed_newline = false;
    while i < len && matches!(source_bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        if source_bytes[i] == b'\n' {
            crossed_newline = true;
        }
        i += 1;
    }
    if !crossed_newline {
        return false;
    }
    // Check if next token looks like a hash key
    if i < len
        && (source_bytes[i].is_ascii_alphanumeric()
            || source_bytes[i] == b'_'
            || source_bytes[i] == b':'
            || source_bytes[i] == b'"'
            || source_bytes[i] == b'\'')
    {
        return true;
    }
    false
}

/// Returns true when the unmatched `(` at `paren_col` belongs to an explicit
/// `super(...)` call.
///
/// RuboCop only supplies special paren-relative context from `on_send`;
/// `super(...)` is handled via `on_array`, so arrays anchored to `super(` stay
/// line-relative.
fn is_super_call_paren(line_bytes: &[u8], paren_col: usize) -> bool {
    if paren_col == 0 {
        return false;
    }

    let mut end = paren_col;
    while end > 0 && (line_bytes[end - 1] == b' ' || line_bytes[end - 1] == b'\t') {
        end -= 1;
    }
    if end < 5 {
        return false;
    }

    let mut start = end;
    while start > 0
        && (line_bytes[start - 1].is_ascii_alphanumeric()
            || line_bytes[start - 1] == b'_'
            || line_bytes[start - 1] == b'!'
            || line_bytes[start - 1] == b'?')
    {
        start -= 1;
    }

    &line_bytes[start..end] == b"super"
}

impl Cop for FirstArrayElementIndentation {
    fn name(&self) -> &'static str {
        "Layout/FirstArrayElementIndentation"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[ARRAY_NODE]
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
        let array_node = match node.as_array_node() {
            Some(a) => a,
            None => return,
        };

        let opening_loc = match array_node.opening_loc() {
            Some(loc) => loc,
            None => return,
        };

        let elements: Vec<_> = array_node.elements().iter().collect();

        let (open_line, _) = source.offset_to_line_col(opening_loc.start_offset());

        // Compute byte offset within the line for the opening bracket.
        // This is needed because find_left_paren_on_line and find_hash_key_column
        // operate on bytes, but offset_to_line_col returns character columns.
        // For multi-byte UTF-8 chars, char_col < byte_col.
        let open_byte_col = opening_loc.start_offset() - source.line_start_offset(open_line);

        let style = config.get_str("EnforcedStyle", "special_inside_parentheses");
        let width = config.get_usize("IndentationWidth", 2);

        // Get the indentation of the line where `[` appears
        let open_line_bytes = source.lines().nth(open_line - 1).unwrap_or(b"");
        let open_line_indent = first_non_whitespace_column(open_line_bytes);
        let (_, open_col) = source.offset_to_line_col(opening_loc.start_offset());

        // Check if `[` is preceded by a hash key on the same line.
        // Uses byte offset for scanning; converts result to char offset.
        let hash_key_byte_col = find_hash_key_column(open_line_bytes, open_byte_col);
        let hash_key_col = hash_key_byte_col.map(|bc| byte_col_to_char_col(open_line_bytes, bc));

        // Compute closing_end for multi-pair hash detection
        let closing_end_offset = array_node
            .closing_loc()
            .map(|loc| loc.end_offset())
            .unwrap_or(0);

        // Compute the indent base column (before adding width) and its type.
        // Priority order matches RuboCop's `indent_base`:
        // 1. align_brackets -> bracket-relative
        // 2. parent_hash_key (if multi-pair hash with right sibling on subsequent line)
        // 3. special_inside_parentheses -> paren-relative
        // 4. Default -> line-relative
        let (indent_base, base_type) = {
            if style == "align_brackets" {
                (open_col, IndentBaseType::LeftBracket)
            } else {
                // Check hash-key-relative BEFORE style-specific checks.
                // RuboCop uses `parent_hash_key` when: array is hash pair value,
                // key and value on same line, and right sibling on subsequent line.
                // Only apply when elements exist (RuboCop's `hash_pair_where_value_beginning_with`
                // returns nil when first_elem is nil).
                let use_hash_key = !elements.is_empty()
                    && hash_key_col.is_some()
                    && has_right_sibling_on_subsequent_line(source.as_bytes(), closing_end_offset);

                if use_hash_key {
                    (hash_key_col.unwrap(), IndentBaseType::ParentHashKey)
                } else if style == "consistent" {
                    (open_line_indent, IndentBaseType::StartOfLine)
                } else {
                    // "special_inside_parentheses" (default):
                    let paren_scan = find_left_paren_on_line(open_line_bytes, open_byte_col);
                    if let Some(paren_byte_col) = paren_scan.paren_col {
                        let paren_col = byte_col_to_char_col(open_line_bytes, paren_byte_col);
                        let super_call_paren = is_super_call_paren(open_line_bytes, paren_byte_col);
                        let intermediate_method_call = hash_key_byte_col.is_some_and(|hk| {
                            has_method_call_between(open_line_bytes, paren_byte_col + 1, hk)
                        });
                        let nested_command_call = has_command_call_before_array(
                            open_line_bytes,
                            paren_byte_col + 1,
                            open_byte_col,
                        );
                        let use_paren_relative = !super_call_paren
                            && !is_preceded_by_percent_operator(open_line_bytes, open_byte_col)
                            && !paren_scan.has_binary_operator_at_depth_zero
                            && !paren_scan.is_grouping_paren
                            && !intermediate_method_call
                            && !nested_command_call
                            && is_direct_argument(
                                source.as_bytes(),
                                closing_end_offset,
                                paren_scan.has_unmatched_brace,
                            );
                        if use_paren_relative {
                            (
                                paren_col + 1,
                                IndentBaseType::FirstColumnAfterLeftParenthesis,
                            )
                        } else {
                            (open_line_indent, IndentBaseType::StartOfLine)
                        }
                    } else {
                        (open_line_indent, IndentBaseType::StartOfLine)
                    }
                }
            }
        };

        // Check first element indentation (only if array has elements)
        if !elements.is_empty() {
            let first_element = &elements[0];
            let first_loc = first_element.location();
            let (elem_line, elem_col) = source.offset_to_line_col(first_loc.start_offset());

            // Skip entire check (elements + closing bracket) if first element is
            // on the same line as the opening bracket. RuboCop's `check` method
            // returns early with `same_line?(first_elem, left_bracket)`.
            if elem_line == open_line {
                return;
            }

            let expected_elem = indent_base + width;

            if elem_col != expected_elem {
                // For single-pair hash value arrays with line-relative indent
                // (StartOfLine base type), accept element at line-indent level.
                // This applies to `consistent` style. For `align_brackets` style
                // (LeftBracket base type), bracket-relative indentation is required.
                let accept_line_relative_element = hash_key_col.is_some()
                    && !matches!(base_type, IndentBaseType::ParentHashKey)
                    && matches!(base_type, IndentBaseType::StartOfLine)
                    && elem_col == open_line_indent + width;

                if !accept_line_relative_element {
                    let base_description = match base_type {
                        IndentBaseType::LeftBracket => "the position of the opening bracket",
                        IndentBaseType::FirstColumnAfterLeftParenthesis => {
                            "the first position after the preceding left parenthesis"
                        }
                        IndentBaseType::ParentHashKey => "the parent hash key",
                        IndentBaseType::StartOfLine => {
                            "the start of the line where the left square bracket is"
                        }
                    };
                    diagnostics.push(self.diagnostic(
                        source,
                        elem_line,
                        elem_col,
                        format!(
                            "Use {} spaces for indentation in an array, relative to {}.",
                            width, base_description
                        ),
                    ));
                }
            }
        }

        // Check closing bracket indentation
        if let Some(closing_loc) = array_node.closing_loc() {
            let (close_line, close_col) = source.offset_to_line_col(closing_loc.start_offset());

            // Only check if the closing bracket is on a different line from
            // the opening bracket and on its own line (only whitespace before it)
            if close_line == open_line {
                return;
            }

            // For empty arrays, also skip if closing is on the line right after opening
            // and elements exist on the same line as opening (handled by elem_line == open_line above)
            let close_line_bytes = source.lines().nth(close_line - 1).unwrap_or(b"");
            let only_whitespace_before = close_line_bytes[..close_col.min(close_line_bytes.len())]
                .iter()
                .all(|&b| b == b' ' || b == b'\t');

            if !only_whitespace_before {
                return;
            }

            // For StartOfLine, compare using first_non_whitespace_column instead
            // of character column — this matches RuboCop's `source_line =~ /\S/`
            // and handles tab-indented files correctly (tabs count as 1 byte).
            let effective_close_col = match base_type {
                IndentBaseType::StartOfLine => first_non_whitespace_column(close_line_bytes),
                _ => close_col,
            };

            if effective_close_col != indent_base {
                // For single-pair hash value arrays with line-relative indent
                // (StartOfLine base type), accept closing bracket at line-indent
                // level. This applies to `consistent` style. For `align_brackets`
                // style (LeftBracket base type), bracket-relative closing position
                // is required.
                if matches!(base_type, IndentBaseType::StartOfLine)
                    && hash_key_col.is_some()
                    && !matches!(base_type, IndentBaseType::ParentHashKey)
                    && effective_close_col == open_line_indent
                {
                    return;
                }
                let msg = match base_type {
                    IndentBaseType::LeftBracket => {
                        "Indent the right bracket the same as the left bracket.".to_string()
                    }
                    IndentBaseType::FirstColumnAfterLeftParenthesis => {
                        "Indent the right bracket the same as the first position \
                         after the preceding left parenthesis."
                            .to_string()
                    }
                    IndentBaseType::ParentHashKey => {
                        "Indent the right bracket the same as the parent hash key.".to_string()
                    }
                    IndentBaseType::StartOfLine => {
                        "Indent the right bracket the same as the start of the line \
                         where the left bracket is."
                            .to_string()
                    }
                };
                diagnostics.push(self.diagnostic(source, close_line, close_col, msg));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        FirstArrayElementIndentation,
        "cops/layout/first_array_element_indentation"
    );

    #[test]
    fn same_line_elements_ignored() {
        let source = b"x = [1, 2, 3]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn align_brackets_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("align_brackets".into()),
            )]),
            ..CopConfig::default()
        };
        // Element at bracket_col + width (4 + 2 = 6), bracket at bracket_col (4) => good
        let src = b"x = [\n      1\n    ]\n";
        let diags = run_cop_full_with_config(&FirstArrayElementIndentation, src, config.clone());
        assert!(
            diags.is_empty(),
            "align_brackets should accept element at bracket_col + width: {:?}",
            diags
        );

        // Element indented normally (2 from line start) should be flagged
        let src2 = b"x = [\n  1\n]\n";
        let diags2 = run_cop_full_with_config(&FirstArrayElementIndentation, src2, config.clone());
        assert!(
            diags2.len() >= 1,
            "align_brackets should flag element not at bracket_col + width: {:?}",
            diags2
        );

        // Bracket not aligned with opening bracket should be flagged
        let src3 = b"x = [\n      1\n]\n";
        let diags3 = run_cop_full_with_config(&FirstArrayElementIndentation, src3, config);
        assert_eq!(
            diags3.len(),
            1,
            "align_brackets should flag bracket not at opening bracket column: {:?}",
            diags3
        );
    }

    #[test]
    fn special_inside_parentheses_method_call() {
        let src = b"foo([\n      :bar,\n      :baz\n    ])\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "array arg with [ on same line as ( should not be flagged"
        );
    }

    #[test]
    fn special_inside_parentheses_nested_call() {
        let src =
            b"expect(cli.run([\n                 :a,\n                 :b\n               ]))\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "nested call array arg should use innermost paren"
        );
    }

    #[test]
    fn keyword_hash_value_after_prior_method_call_arg_still_uses_paren_relative() {
        let src = b"outer(File.join(dir, basename), key: [\n        :a,\n      ])\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "method calls in earlier arguments should not suppress paren-relative indentation: {:?}",
            diags
        );
    }

    #[test]
    fn array_with_method_chain_uses_line_indent() {
        let src = b"expect(x).to eq([\n  'hello',\n  'world'\n].join(\"\\n\"))\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "array with .join chain should use line-relative indent"
        );
    }

    #[test]
    fn array_in_grouping_paren_uses_line_indent() {
        let src = b"X = (%i[\n  a\n  b\n] + other).freeze\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "array in grouping paren with + operator should use line-relative indent"
        );
    }

    #[test]
    fn percent_i_array_inside_method_call_paren() {
        let src = b"eq(%i[\n     :a,\n     :b\n   ])\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "%i[ inside method call paren should use paren-relative indent: {:?}",
            diags
        );
    }

    #[test]
    fn percent_i_array_inside_method_call_paren_wrong_indent() {
        let src = b"eq(%i[\n  :a,\n  :b\n])\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert_eq!(
            diags.len(),
            2,
            "%i[ inside method call paren with wrong indent should flag element and bracket: {:?}",
            diags
        );
    }

    #[test]
    fn closing_bracket_wrong_indent_in_method_call() {
        let src =
            b"    expect(validation_attributes).to eq(%i[\n      client_id\n      client\n    ])\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert_eq!(
            diags.len(),
            2,
            "should flag both element and bracket in method call: {:?}",
            diags
        );
        let bracket_diag = diags
            .iter()
            .find(|d| d.message.contains("right bracket"))
            .unwrap();
        assert!(
            bracket_diag
                .message
                .contains("first position after the preceding left parenthesis"),
            "bracket message should reference left parenthesis: {}",
            bracket_diag.message
        );
    }

    #[test]
    fn closing_bracket_on_same_line_as_last_element_not_flagged() {
        let src = b"x = [\n  1,\n  2]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "bracket on same line as last element should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn closing_bracket_correct_indent_no_parens() {
        let src = b"x = [\n  1,\n  2\n]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "bracket at line indent should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn percent_operator_array_not_paren_relative() {
        let src = b"gc.draw('text %d,%d' % [\n  left.round,\n  header_height\n])\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "array after % operator should use line-relative indent, not paren-relative: {:?}",
            diags
        );
    }

    #[test]
    fn percent_operator_array_indented_in_method() {
        let src = b"    gc.draw('rect %d,%d %d,%d' % [\n      0, 0, width, height\n    ])\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "indented % operator array should use line-relative indent: {:?}",
            diags
        );
    }

    #[test]
    fn array_inside_hash_arg_inside_parens_flags_paren_relative() {
        let src = b"build_type(\"test\", { \"associations\" => [\n  {\n    \"key\" => \"docs\",\n  },\n] })\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            !diags.is_empty(),
            "array inside hash arg inside parens should flag with paren-relative indent: {:?}",
            diags
        );
        assert!(
            diags[0]
                .message
                .contains("first position after the preceding left parenthesis"),
            "should use paren-relative message: {}",
            diags[0].message
        );
    }

    #[test]
    fn hash_with_to_json_chain_uses_line_indent() {
        let src = b"foo(status: 200, body: { \"responses\" => [\n  \"code\" => 200, \"body\" => \"OK\"\n] }.to_json)\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "array in hash chained with .to_json should use line-relative: {:?}",
            diags
        );
    }

    #[test]
    fn tab_indented_closing_bracket_not_flagged() {
        let src = b"\tauthors [\n\t\t\"name\",\n\t]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        let bracket_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("right bracket"))
            .collect();
        assert!(
            bracket_diags.is_empty(),
            "tab-indented closing bracket at same level should not be flagged: {:?}",
            bracket_diags
        );
    }

    #[test]
    fn tab_indented_nested_closing_bracket_not_flagged() {
        let src = b"\t\tmatches [\n\t\t\t{ :text => \"test\" },\n\t\t]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        let bracket_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("right bracket"))
            .collect();
        assert!(
            bracket_diags.is_empty(),
            "nested tab-indented closing bracket should not be flagged: {:?}",
            bracket_diags
        );
    }

    #[test]
    fn array_inside_chained_hash_in_method_call() {
        let src = b"  client.\n    with(endpoint, { requests: [\n      { method: 'POST' }\n    ], flag: true }.to_json).\n    and_return(response)\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "array inside chained hash should use line-relative indent: {:?}",
            diags
        );
    }

    #[test]
    fn closing_bracket_wrong_indent_no_parens() {
        let src = b"x = [\n  1,\n  2\n  ]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert_eq!(
            diags.len(),
            1,
            "bracket at wrong indent should be flagged: {:?}",
            diags
        );
        assert!(
            diags[0].message.contains("right bracket"),
            "should be a bracket message: {}",
            diags[0].message
        );
    }

    #[test]
    fn empty_array_wrong_closing_bracket_indent() {
        // RuboCop flags `a << [\n  ]` — closing bracket should be at line indent (col 0)
        let src = b"a << [\n  ]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert_eq!(
            diags.len(),
            1,
            "empty array with wrong closing bracket indent should be flagged: {:?}",
            diags
        );
        assert!(
            diags[0].message.contains("right bracket"),
            "should be a bracket message: {}",
            diags[0].message
        );
    }

    #[test]
    fn empty_array_correct_closing_bracket_indent() {
        // `a << [\n]` — closing bracket at line indent (col 0) is correct
        let src = b"a << [\n]\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "empty array with correct closing bracket should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn empty_array_same_line_not_flagged() {
        // `a = []` — single-line empty array should never be flagged
        let src = b"a = []\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "single-line empty array should not be flagged: {:?}",
            diags
        );
    }

    #[test]
    fn multi_pair_hash_right_sibling_subsequent_line_uses_hash_key_relative() {
        // RuboCop uses hash-key-relative when pair has right sibling on subsequent line
        // func(x: [\n  :a\n],\n     y: 1)
        // x: at col 5, elements should be at 5+2=7, closing at 5
        let src = b"func(x: [\n       :a\n     ],\n     y: 1)\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "multi-pair hash with right sibling on subsequent line should use hash-key-relative: {:?}",
            diags
        );
    }

    #[test]
    fn multi_pair_hash_right_sibling_same_line_uses_paren_relative() {
        // When right sibling is on same line as `]`, hash-key-relative doesn't apply
        // func(:x, y: [\n       :a\n     ], z: 1)
        // Paren-relative: paren_col+1=5, expected=7. Elements at 7, closing at 5.
        let src = b"func(:x, y: [\n       :a\n     ], z: 1)\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            diags.is_empty(),
            "multi-pair hash with right sibling on same line should use paren-relative: {:?}",
            diags
        );
    }

    #[test]
    fn multi_pair_hash_wrong_element_indent_flags_parent_hash_key() {
        // Elements at wrong indent for hash-key-relative should get "parent hash key" message
        let src = b"func(x: [\n  :a,\n  :b\n],\n     y: 1)\n";
        let diags = run_cop_full(&FirstArrayElementIndentation, src);
        assert!(
            !diags.is_empty(),
            "wrong indent in multi-pair hash should be flagged: {:?}",
            diags
        );
        let elem_diag = diags
            .iter()
            .find(|d| d.message.contains("indentation"))
            .unwrap();
        assert!(
            elem_diag.message.contains("parent hash key"),
            "should reference parent hash key: {}",
            elem_diag.message
        );
    }

    #[test]
    fn consistent_style_multi_pair_hash_uses_hash_key_relative() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("consistent".into()),
            )]),
            ..CopConfig::default()
        };
        // In consistent style, multi-pair hash arrays still use hash-key-relative
        // func(x: [\n       :a\n     ],\n     y: 1)
        let src = b"func(x: [\n       :a\n     ],\n     y: 1)\n";
        let diags = run_cop_full_with_config(&FirstArrayElementIndentation, src, config.clone());
        assert!(
            diags.is_empty(),
            "consistent style + multi-pair hash should accept hash-key-relative: {:?}",
            diags
        );
    }
}
