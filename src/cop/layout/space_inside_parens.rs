use crate::cop::shared::node_type::{
    ARRAY_PATTERN_NODE, BLOCK_PARAMETERS_NODE, CALL_NODE, DEF_NODE, DEFINED_NODE,
    FIND_PATTERN_NODE, HASH_PATTERN_NODE, MULTI_TARGET_NODE, MULTI_WRITE_NODE, PARENTHESES_NODE,
    PINNED_EXPRESSION_NODE, SUPER_NODE, YIELD_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-20)
///
/// Corpus oracle reported FP=1, FN=4,120. Local
/// `verify-cop-locations.py Layout/SpaceInsideParens` now shows the lone CI FP
/// already fixed, so the remaining work is entirely FN.
///
/// The missing detections came from two implementation gaps:
///
/// 1. **Method/lambda parameter parens were never inspected.** The previous
///    code only checked `CallNode` and `ParenthesesNode`, so definitions like
///    `def initialize( options )` and lambda params with `->( value ) { ... }`
///    were invisible.
/// 2. **Multiline parens were skipped wholesale.** RuboCop checks each side of
///    a paren pair independently using adjacent tokens. `deliver( payload,\n`
///    should still flag the opening side, and `format: :json )` should still
///    flag the closing side. The previous `open_line != close_line => skip`
///    shortcut missed both.
///
/// Fix: extract paren pairs from calls, grouping parens, defs, and
/// parenthesized block/lambda params, then apply side-specific same-line
/// checks that mirror RuboCop's token-pair behavior (including comment and
/// empty-parens exceptions). Follow-up investigation against Twilio's
/// generated client code showed that RuboCop also accepts multiline empty
/// parens in `no_space` style (`call(\n)`), so the whitespace-only fast path
/// now preserves that form while still flagging `call( )`. A later acl9
/// reduction showed one more token-driven asymmetry: command-style argument
/// parens like `check ( value)` and `yield ( value)` ignore the opening side
/// entirely, but still check the closing side. We mirror that here with a
/// source-context guard on `ParenthesesNode` opening checks. Remaining live FN
/// investigation in webistrano/rufo also showed extra paren carriers:
/// ternary branches like `? ( value)` are ordinary grouping parens and must
/// not be skipped by that guard, while pattern pins (`^ ( 1 + 2 )`) and block
/// destructuring params (`| ( x ) , y |`) store their delimiters on
/// `PinnedExpressionNode` and `MultiTargetNode`. Constant patterns
/// (`Point( 1 )`, `SuperPoint( x: 1 )`) and parenthesized `yield(...)` calls
/// also need their Prism-specific nodes (`ArrayPatternNode`,
/// `HashPatternNode`, `YieldNode`).
///
/// ## Corpus investigation (2026-03-23)
///
/// Corpus oracle reported FP=35, FN=5.
///
/// FP=35: All from line-continuation backslash after opening paren space, e.g.
/// `method( \`. RuboCop's token-based approach sees the next token on the
/// following line, so it doesn't flag the space. Fixed by treating a trailing
/// `\` in `next_same_line_item` as no code on the same line.
///
/// FN=5: Parenthesized multi-write targets like `( x, y ) = foo`. Prism uses
/// `MultiWriteNode` (not `MultiTargetNode`) for the outer parens. Fixed by
/// adding `MULTI_WRITE_NODE` to interested nodes and extracting lparen/rparen
/// from `MultiWriteNode` in `paren_offsets()`.
///
/// ## Corpus investigation (2026-03-28)
///
/// The workflow prompt still cited FP=8/FN=2, but
/// `verify_cop_locations.py Layout/SpaceInsideParens` on this branch showed
/// those exact corpus FP locations already fixed. Focused fixtures exposed the
/// remaining shared root cause instead:
///
/// 1. **Command-form opening-side suppression was text-based and too narrow.**
///    `command_form_prefix` only accepted bare method names before `(`, so
///    receiver calls like `JSON.generate ( { ... })` and
///    `BSON::Binary.new ( value )` still flagged the opening side even though
///    RuboCop ignores it.
/// 2. **The same heuristic was too broad for boolean operators.** It treated
///    `and (` like a command-form call, so grouped expressions such as
///    `obj and ( cond )` skipped the opening-side check entirely, matching the
///    two live FN fixture cases.
///
/// Fix: keep the opening-side exemption source-based, but widen it only enough
/// to treat receiver method calls (`foo.bar ( value)`, `foo&.bar ( value)`) as
/// command form while explicitly denying boolean keywords like `and`/`or`.
/// That preserves `check ( value )`, fixes receiver-call FPs, and keeps
/// boolean grouping parens checked.
///
/// ## Corpus investigation (2026-03-31)
///
/// The workflow packet still cited four remaining FPs, but fresh verification
/// showed the live isolated detection bug was narrower: a trailing `?\ `
/// space-character literal before `)` (for example `foo(?\ )`,
/// `expr.index(?\ )`, or `join(?\ )`). The close-side scanner walked backward
/// over the literal's source-space byte as if it were separator whitespace and
/// reported an extraneous space. Fix: treat that byte as part of the
/// character-literal token when identifying the previous same-line code item,
/// which preserves `no_space` behavior without weakening legitimate close-side
/// checks.
///
/// ## Corpus investigation (2026-03-31, second pass)
///
/// FP=2: both from parenthesized expressions used as hash values in
/// label syntax, e.g. `max: ( plan.to_i * 1024)` or
/// `status: ( @type == 'error') ? 400 : 200`. The corpus oracle uses
/// `TargetRubyVersion: 4.0`, under which RuboCop's parser assigns
/// the `(` a different token type that `left_parens?` does not match,
/// so the opening-side space check is skipped (but the closing side
/// still applies). Fix: added `label_value_paren()` which detects
/// the `identifier: (` pattern by walking backward from `(` and
/// checking for a `:` immediately preceded by an identifier character
/// (excluding `::` constant resolution). This feeds into
/// `ignores_open_side()` alongside the existing command-form check.
///
/// ## Corpus investigation (2026-04-16)
///
/// The remaining `compact`-style misses came from treating raw `(` / `)`
/// bytes as if they always represented paren tokens. RuboCop only collapses
/// true consecutive paren tokens:
///
/// 1. `g( ( 3 + 5 ) * x )` should remove the space between `(` and `(`.
/// 2. `g( f( x ) )` / `uri_parse( to_absolute(...) )` should remove the space
///    between `)` and `)`.
/// 3. `warning(%(\n...\n))` must still require a space before the outer `)`
///    because the inner `)` belongs to a percent-string delimiter, not a
///    nested paren token.
///
/// Fix: for `compact`, opening-side collapse now only allows adjacent `((`
/// and reports `"( ("` as extraneous space, while closing-side collapse only
/// treats adjacent/spaced `))` as collapsible when the last inner child is
/// itself a paren-carrying node. That matches RuboCop's token-based behavior
/// without weakening ordinary string/literal cases.
///
/// ## Corpus investigation (2026-04-16, follow-up)
///
/// Two variant mismatches remained:
///
/// 1. Prism parses `Point(*, 1, *a)` pattern parens as `FindPatternNode`, not
///    `ArrayPatternNode`, so both `space` and `compact` missed those offenses.
/// 2. RuboCop tokenizes multiline plain quoted strings like
///    `select("...\n...")` as a single `tSTRING` token. The opening `(` still
///    checks against that token, but the closing `)` is ignored because the
///    token started on a previous line. Percent strings like `%(\n...\n)` still
///    expose a same-line `tSTRING_END`, so their closing side remains checked.
///
/// Fix: add `FindPatternNode` paren extraction, and skip close-side
/// missing-space checks only when the final inner node is a multiline plain
/// quoted `StringNode` whose closing quote is immediately before the outer `)`.
///
/// ## Corpus investigation (2026-04-19)
///
/// The `space` variant still showed ~100 FN from binstub-style sources like
/// `abort("Your bin/bundle...\nReplace ... again.")` — a plain double-quoted
/// string with a real embedded newline inside `()`. The multiline-plain-string
/// exemption above was too broad: it skipped close-side checks for every
/// multiline `"..."` and `'...'`, but RuboCop only ignores the close side when
/// Parser emits a single combined `tSTRING` token. Parser only does that for
/// double-quoted strings where every newline is preceded by an odd number of
/// backslashes (a `\<newline>` line continuation). Any bare newline — or any
/// single-quoted multiline string — produces separate `tSTRING_BEG`/
/// `tSTRING_END` tokens, and the close side fires on the `)` line.
///
/// Fix: restrict the exemption to double-quoted strings whose interior
/// contains only line-continuation newlines.
///
/// ## Corpus investigation (2026-04-20)
///
/// The remaining `compact` variant drift came from two token-pair quirks in
/// RuboCop's implementation:
///
/// 1. Consecutive close-paren handling is based on the last token, not the
///    immediate AST child. Wrapper nodes like `&method(...)`, keyword hashes,
///    `=>` pairs, and `||=` writes still end in a nested `)`, so forms like
///    `http.get( url, &method(...) )` and `wrap( outer: inner(...))` must use
///    the `))` compact rule.
/// 2. RuboCop only removes an exact single ASCII space between consecutive
///    paren tokens. It intentionally ignores `((`/`))` separated by two spaces
///    or tabs.
///
/// Fix: compact close-side detection now walks through trailing wrapper nodes
/// to find a nested paren token, and both compact open/close collapse paths now
/// require the gap to be exactly `" "`.
pub struct SpaceInsideParens;

const MSG: &str = "Space inside parentheses detected.";
const MSG_NO_SPACE: &str = "No space inside parentheses detected.";

impl Cop for SpaceInsideParens {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideParens"
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ARRAY_PATTERN_NODE,
            BLOCK_PARAMETERS_NODE,
            CALL_NODE,
            DEF_NODE,
            DEFINED_NODE,
            FIND_PATTERN_NODE,
            HASH_PATTERN_NODE,
            MULTI_TARGET_NODE,
            MULTI_WRITE_NODE,
            PARENTHESES_NODE,
            PINNED_EXPRESSION_NODE,
            SUPER_NODE,
            YIELD_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "no_space");
        let bytes = source.as_bytes();

        let Some((open_start, open_end, close_start)) = paren_offsets(node, bytes) else {
            return;
        };

        if close_start <= open_end {
            return;
        }

        let interior = &bytes[open_end..close_start];
        if interior.is_empty() {
            return;
        }

        // Empty parens always want `()`, even in `space` / `compact`.
        if interior.iter().all(|&b| is_paren_whitespace(b)) {
            if style == "no_space" && interior.contains(&b'\n') {
                return;
            }

            if !interior.is_empty() {
                push_remove_offense(
                    self,
                    source,
                    diagnostics,
                    &mut corrections,
                    open_end,
                    close_start,
                    MSG,
                );
            }
            return;
        }

        let open_side = next_same_line_item(bytes, open_end);
        let close_side = previous_same_line_code(bytes, close_start);

        let ignore_open_side = ignores_open_side(node, bytes, open_start);

        match style {
            "space" => {
                if !ignore_open_side {
                    check_missing_open_space(
                        self,
                        source,
                        diagnostics,
                        &mut corrections,
                        bytes,
                        open_side,
                        false,
                    );
                }
                check_missing_close_space(
                    self,
                    source,
                    diagnostics,
                    &mut corrections,
                    node,
                    bytes,
                    close_side,
                    close_start,
                    false,
                );
            }
            "compact" => {
                if !ignore_open_side {
                    check_compact_open_space(
                        self,
                        source,
                        diagnostics,
                        &mut corrections,
                        open_end,
                        bytes,
                        open_side,
                    );
                }
                check_compact_close_space(
                    self,
                    source,
                    diagnostics,
                    &mut corrections,
                    node,
                    bytes,
                    close_side,
                    close_start,
                );
            }
            _ => {
                if !ignore_open_side {
                    check_extraneous_open_space(
                        self,
                        source,
                        diagnostics,
                        &mut corrections,
                        open_end,
                        open_side,
                    );
                }
                check_extraneous_close_space(
                    self,
                    source,
                    diagnostics,
                    &mut corrections,
                    close_start,
                    close_side,
                );
            }
        }
    }
}

fn paren_offsets(node: &ruby_prism::Node<'_>, bytes: &[u8]) -> Option<(usize, usize, usize)> {
    if let Some(parens) = node.as_parentheses_node() {
        let open = parens.opening_loc();
        let close = parens.closing_loc();
        return Some((open.start_offset(), open.end_offset(), close.start_offset()));
    }

    if let Some(call) = node.as_call_node() {
        let open = call.opening_loc()?;
        let close = call.closing_loc()?;
        if open.as_slice() == b"(" && close.as_slice() == b")" {
            return Some((open.start_offset(), open.end_offset(), close.start_offset()));
        }
    }

    if let Some(yield_node) = node.as_yield_node() {
        let open = yield_node.lparen_loc()?;
        let close = yield_node.rparen_loc()?;
        return Some((open.start_offset(), open.end_offset(), close.start_offset()));
    }

    if let Some(def) = node.as_def_node() {
        let open = def.lparen_loc()?;
        let close = def.rparen_loc()?;
        return Some((open.start_offset(), open.end_offset(), close.start_offset()));
    }

    if let Some(multi_target) = node.as_multi_target_node() {
        let open = multi_target.lparen_loc()?;
        let close = multi_target.rparen_loc()?;
        return Some((open.start_offset(), open.end_offset(), close.start_offset()));
    }

    if let Some(multi_write) = node.as_multi_write_node() {
        let open = multi_write.lparen_loc()?;
        let close = multi_write.rparen_loc()?;
        return Some((open.start_offset(), open.end_offset(), close.start_offset()));
    }

    if let Some(array_pattern) = node.as_array_pattern_node() {
        let open = array_pattern.opening_loc()?;
        let close = array_pattern.closing_loc()?;
        if open.as_slice() == b"(" && close.as_slice() == b")" {
            return Some((open.start_offset(), open.end_offset(), close.start_offset()));
        }
    }

    if let Some(hash_pattern) = node.as_hash_pattern_node() {
        let open = hash_pattern.opening_loc()?;
        let close = hash_pattern.closing_loc()?;
        if open.as_slice() == b"(" && close.as_slice() == b")" {
            return Some((open.start_offset(), open.end_offset(), close.start_offset()));
        }
    }

    if let Some(find_pattern) = node.as_find_pattern_node() {
        let open = find_pattern.opening_loc()?;
        let close = find_pattern.closing_loc()?;
        if open.as_slice() == b"(" && close.as_slice() == b")" {
            return Some((open.start_offset(), open.end_offset(), close.start_offset()));
        }
    }

    if let Some(pinned_expression) = node.as_pinned_expression_node() {
        let open = pinned_expression.lparen_loc();
        let close = pinned_expression.rparen_loc();
        return Some((open.start_offset(), open.end_offset(), close.start_offset()));
    }

    if let Some(super_node) = node.as_super_node() {
        let open = super_node.lparen_loc()?;
        let close = super_node.rparen_loc()?;
        return Some((open.start_offset(), open.end_offset(), close.start_offset()));
    }

    if node.as_defined_node().is_some() {
        let loc = node.location();
        let slice = &bytes[loc.start_offset()..loc.end_offset()];
        if !slice.starts_with(b"defined?") {
            return None;
        }

        let mut open_start = loc.start_offset() + b"defined?".len();
        while open_start < loc.end_offset() && matches!(bytes[open_start], b' ' | b'\t' | b'\r') {
            open_start += 1;
        }
        if open_start >= loc.end_offset() || bytes[open_start] != b'(' {
            return None;
        }

        let close_start = slice.iter().rposition(|&b| b == b')')? + loc.start_offset();
        if close_start <= open_start {
            return None;
        }

        return Some((open_start, open_start + 1, close_start));
    }

    if let Some(block_params) = node.as_block_parameters_node() {
        let open = block_params.opening_loc()?;
        let close = block_params.closing_loc()?;
        if open.as_slice() == b"(" && close.as_slice() == b")" {
            return Some((open.start_offset(), open.end_offset(), close.start_offset()));
        }
    }

    None
}

fn ignores_open_side(node: &ruby_prism::Node<'_>, bytes: &[u8], open_start: usize) -> bool {
    if node.as_parentheses_node().is_none() {
        return false;
    }

    command_form_prefix(bytes, open_start).is_some() || label_value_paren(bytes, open_start)
}

/// Detects when `(` is used as a hash value in label syntax, e.g. `key: (expr)`.
///
/// In Ruby 4.0 parsing mode, RuboCop's tokenizer assigns a different token type
/// to `(` in this context, so `left_parens?` returns false and the opening-side
/// space check is skipped. The closing side is still checked because `)` always
/// matches `right_parens?`.
fn label_value_paren(bytes: &[u8], open_start: usize) -> bool {
    if open_start < 3 {
        return false;
    }

    // Walk backward past whitespace before `(`
    let mut idx = open_start;
    while idx > 0 && matches!(bytes[idx - 1], b' ' | b'\t' | b'\r') {
        idx -= 1;
    }
    if idx == open_start {
        return false; // No space before `(` — not a command-form label value
    }

    // Expect `:`
    if idx == 0 || bytes[idx - 1] != b':' {
        return false;
    }
    idx -= 1;

    // Reject `::` (constant resolution)
    if idx > 0 && bytes[idx - 1] == b':' {
        return false;
    }

    // Require an identifier character before `:`
    if idx == 0 || !is_identifier_tail(bytes[idx - 1]) {
        return false;
    }

    true
}

fn command_form_prefix(bytes: &[u8], open_start: usize) -> Option<&[u8]> {
    if open_start == 0 {
        return None;
    }

    let line_start = bytes[..open_start]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);

    let mut word_end = open_start;
    while word_end > line_start && matches!(bytes[word_end - 1], b' ' | b'\t' | b'\r') {
        word_end -= 1;
    }
    if word_end == open_start || word_end == line_start {
        return None;
    }

    let mut word_start = word_end;
    while word_start > line_start && is_identifier_tail(bytes[word_start - 1]) {
        word_start -= 1;
    }
    if word_start == word_end {
        return None;
    }

    if word_start > line_start {
        let prev = bytes[word_start - 1];
        if is_identifier_tail(prev) || matches!(prev, b':' | b'@') {
            return None;
        }
    }

    let word = &bytes[word_start..word_end];
    if !matches!(word[0], b'a'..=b'z' | b'A'..=b'Z' | b'_') {
        return None;
    }
    if denied_command_prefix(word) {
        return None;
    }

    Some(word)
}

fn is_identifier_tail(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'?' | b'!')
}

fn denied_command_prefix(word: &[u8]) -> bool {
    matches!(
        word,
        b"and"
            | b"or"
            | b"if"
            | b"unless"
            | b"while"
            | b"until"
            | b"case"
            | b"for"
            | b"return"
            | b"break"
            | b"next"
            | b"redo"
            | b"retry"
            | b"then"
            | b"elsif"
            | b"when"
            | b"rescue"
            | b"super"
            | b"defined?"
    )
}

#[derive(Clone, Copy)]
enum NextSameLineItem {
    None,
    Comment,
    Code(usize),
}

fn next_same_line_item(bytes: &[u8], offset: usize) -> NextSameLineItem {
    let line_end = bytes[offset..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|idx| offset + idx)
        .unwrap_or(bytes.len());

    let mut idx = offset;
    while idx < line_end && matches!(bytes[idx], b' ' | b'\t' | b'\r') {
        idx += 1;
    }

    if idx >= line_end {
        NextSameLineItem::None
    } else if bytes[idx] == b'#' {
        NextSameLineItem::Comment
    } else if bytes[idx] == b'\\' && is_trailing_backslash(bytes, idx, line_end) {
        // Line continuation backslash — RuboCop treats this as no code on the
        // same line (the next token is on the following line).
        NextSameLineItem::None
    } else {
        NextSameLineItem::Code(idx)
    }
}

fn previous_same_line_code(bytes: &[u8], close_start: usize) -> Option<usize> {
    let line_start = bytes[..close_start]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);

    let mut idx = close_start;
    while idx > line_start && matches!(bytes[idx - 1], b' ' | b'\t' | b'\r') {
        if bytes[idx - 1] == b' ' && ends_space_character_literal(bytes, line_start, idx - 1) {
            return Some(idx - 1);
        }
        idx -= 1;
    }

    if idx == line_start {
        None
    } else {
        Some(idx - 1)
    }
}

fn ends_space_character_literal(bytes: &[u8], line_start: usize, space_offset: usize) -> bool {
    if bytes.get(space_offset) != Some(&b' ') || space_offset < line_start + 2 {
        return false;
    }

    if bytes[space_offset - 1] != b'\\' || bytes[space_offset - 2] != b'?' {
        return false;
    }

    if space_offset > line_start + 2 && is_identifier_tail(bytes[space_offset - 3]) {
        return false;
    }

    true
}

fn is_trailing_backslash(bytes: &[u8], idx: usize, line_end: usize) -> bool {
    let mut i = idx + 1;
    while i < line_end && matches!(bytes[i], b' ' | b'\t' | b'\r') {
        i += 1;
    }
    i >= line_end
}

fn check_extraneous_open_space(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    open_end: usize,
    open_side: NextSameLineItem,
) {
    if let NextSameLineItem::Code(code_start) = open_side {
        if code_start > open_end {
            push_remove_offense(
                cop,
                source,
                diagnostics,
                corrections,
                open_end,
                code_start,
                MSG,
            );
        }
    }
}

fn check_extraneous_close_space(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    close_start: usize,
    close_side: Option<usize>,
) {
    let Some(prev_code) = close_side else {
        return;
    };
    let space_start = prev_code + 1;
    if space_start < close_start {
        push_remove_offense(
            cop,
            source,
            diagnostics,
            corrections,
            space_start,
            close_start,
            MSG,
        );
    }
}

fn check_missing_open_space(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    bytes: &[u8],
    open_side: NextSameLineItem,
    allow_consecutive_left_parens: bool,
) {
    let NextSameLineItem::Code(code_start) = open_side else {
        return;
    };
    if allow_consecutive_left_parens && bytes.get(code_start) == Some(&b'(') {
        return;
    }
    if code_start == 0 {
        return;
    }
    if bytes.get(code_start - 1) == Some(&b' ') {
        return;
    }
    push_insert_offense(
        cop,
        source,
        diagnostics,
        corrections,
        code_start,
        MSG_NO_SPACE,
    );
}

fn check_compact_open_space(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    open_end: usize,
    bytes: &[u8],
    open_side: NextSameLineItem,
) {
    let NextSameLineItem::Code(code_start) = open_side else {
        return;
    };

    if bytes.get(code_start) == Some(&b'(') {
        if code_start == open_end {
            return;
        }
        if !has_single_ascii_space(bytes, open_end, code_start) {
            return;
        }

        push_remove_offense(
            cop,
            source,
            diagnostics,
            corrections,
            open_end,
            code_start,
            MSG,
        );
        return;
    }

    if code_start == 0 {
        return;
    }
    if bytes.get(code_start - 1) == Some(&b' ') {
        return;
    }
    push_insert_offense(
        cop,
        source,
        diagnostics,
        corrections,
        code_start,
        MSG_NO_SPACE,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_missing_close_space(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    node: &ruby_prism::Node<'_>,
    bytes: &[u8],
    close_side: Option<usize>,
    close_start: usize,
    allow_consecutive_right_parens: bool,
) {
    let Some(prev_code) = close_side else {
        return;
    };
    if ignores_close_side_for_multiline_plain_string(node, bytes, prev_code) {
        return;
    }
    if allow_consecutive_right_parens && bytes.get(prev_code) == Some(&b')') {
        return;
    }
    if prev_code + 1 != close_start {
        return;
    }
    push_insert_offense(
        cop,
        source,
        diagnostics,
        corrections,
        close_start,
        MSG_NO_SPACE,
    );
}

#[allow(clippy::too_many_arguments)]
fn check_compact_close_space(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    node: &ruby_prism::Node<'_>,
    bytes: &[u8],
    close_side: Option<usize>,
    close_start: usize,
) {
    let Some(prev_code) = close_side else {
        return;
    };
    if ignores_close_side_for_multiline_plain_string(node, bytes, prev_code) {
        return;
    }

    if bytes.get(prev_code) == Some(&b')')
        && compact_allows_consecutive_close_paren(node, bytes, prev_code)
    {
        if prev_code + 1 == close_start {
            return;
        }
        if !has_single_ascii_space(bytes, prev_code + 1, close_start) {
            return;
        }

        push_remove_offense(
            cop,
            source,
            diagnostics,
            corrections,
            prev_code + 1,
            close_start,
            MSG,
        );
        return;
    }

    if prev_code + 1 != close_start {
        return;
    }

    push_insert_offense(
        cop,
        source,
        diagnostics,
        corrections,
        close_start,
        MSG_NO_SPACE,
    );
}

fn compact_allows_consecutive_close_paren(
    node: &ruby_prism::Node<'_>,
    bytes: &[u8],
    prev_code: usize,
) -> bool {
    let Some(last_inner) = compact_last_inner_node(node) else {
        return false;
    };

    trailing_paren_close_offset(&last_inner, bytes)
        .is_some_and(|inner_close| inner_close == prev_code)
}

fn has_single_ascii_space(bytes: &[u8], start: usize, end: usize) -> bool {
    start < end && &bytes[start..end] == b" "
}

fn compact_last_inner_node<'a>(node: &ruby_prism::Node<'a>) -> Option<ruby_prism::Node<'a>> {
    if let Some(paren) = node.as_parentheses_node() {
        let body = paren.body()?;
        if let Some(stmts) = body.as_statements_node() {
            return stmts.body().iter().last();
        }
        return Some(body);
    }

    if let Some(call) = node.as_call_node() {
        if let Some(block) = call.block() {
            return Some(block);
        }
        return call
            .arguments()
            .and_then(|args| args.arguments().iter().last());
    }

    if let Some(yield_node) = node.as_yield_node() {
        return yield_node
            .arguments()
            .and_then(|args| args.arguments().iter().last());
    }

    if let Some(super_node) = node.as_super_node() {
        return super_node
            .arguments()
            .and_then(|args| args.arguments().iter().last());
    }

    if let Some(pinned) = node.as_pinned_expression_node() {
        return Some(pinned.expression());
    }

    if let Some(defined) = node.as_defined_node() {
        return Some(defined.value());
    }

    None
}

fn trailing_paren_close_offset(node: &ruby_prism::Node<'_>, bytes: &[u8]) -> Option<usize> {
    if let Some((_, _, close_start)) = paren_offsets(node, bytes) {
        return Some(close_start);
    }

    let next = trailing_inner_node(node)?;
    trailing_paren_close_offset(&next, bytes)
}

fn trailing_inner_node<'a>(node: &ruby_prism::Node<'a>) -> Option<ruby_prism::Node<'a>> {
    if let Some(block_arg) = node.as_block_argument_node() {
        return block_arg.expression();
    }

    if let Some(keyword_hash) = node.as_keyword_hash_node() {
        return keyword_hash.elements().iter().last();
    }

    if let Some(hash) = node.as_hash_node() {
        return hash.elements().iter().last();
    }

    if let Some(assoc) = node.as_assoc_node() {
        return Some(assoc.value());
    }

    if let Some(assoc_splat) = node.as_assoc_splat_node() {
        return assoc_splat.value();
    }

    if let Some(splat) = node.as_splat_node() {
        return splat.expression();
    }

    if let Some(write) = node.as_local_variable_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_instance_variable_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_class_variable_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_global_variable_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_path_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_local_variable_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_local_variable_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_local_variable_operator_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_instance_variable_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_instance_variable_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_instance_variable_operator_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_class_variable_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_class_variable_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_class_variable_operator_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_global_variable_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_global_variable_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_global_variable_operator_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_operator_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_path_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_path_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_constant_path_operator_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_index_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_index_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_index_operator_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_call_or_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_call_and_write_node() {
        return Some(write.value());
    }

    if let Some(write) = node.as_call_operator_write_node() {
        return Some(write.value());
    }

    None
}

fn ignores_close_side_for_multiline_plain_string(
    node: &ruby_prism::Node<'_>,
    bytes: &[u8],
    prev_code: usize,
) -> bool {
    let Some(last_inner) = last_inner_node(node) else {
        return false;
    };
    let Some(string) = last_inner.as_string_node() else {
        return false;
    };

    let Some(opening) = string.opening_loc() else {
        return false;
    };
    let Some(closing) = string.closing_loc() else {
        return false;
    };
    // Parser only folds multiple source lines into a single `tSTRING` token for
    // double-quoted strings where every newline is preceded by an odd number of
    // backslashes (a `\<newline>` line continuation). Any bare newline — or a
    // single-quoted string — produces separate `tSTRING_BEG`/`tSTRING_END`
    // tokens, and RuboCop's close-side check fires on the `)` line.
    if opening.as_slice() != b"\"" || closing.as_slice() != b"\"" {
        return false;
    }
    if closing.start_offset() != prev_code {
        return false;
    }

    let content = &bytes[opening.end_offset()..closing.start_offset()];
    if !content.contains(&b'\n') {
        return false;
    }

    only_line_continuation_newlines(content)
}

fn only_line_continuation_newlines(content: &[u8]) -> bool {
    for (idx, &byte) in content.iter().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let mut backslashes = 0usize;
        let mut back = idx;
        while back > 0 && content[back - 1] == b'\\' {
            backslashes += 1;
            back -= 1;
        }
        if backslashes % 2 == 0 {
            return false;
        }
    }
    true
}

fn last_inner_node<'a>(node: &ruby_prism::Node<'a>) -> Option<ruby_prism::Node<'a>> {
    if let Some(paren) = node.as_parentheses_node() {
        let body = paren.body()?;
        if let Some(stmts) = body.as_statements_node() {
            return stmts.body().iter().last();
        }
        return Some(body);
    }

    if let Some(call) = node.as_call_node() {
        return call
            .arguments()
            .and_then(|args| args.arguments().iter().last());
    }

    if let Some(yield_node) = node.as_yield_node() {
        return yield_node
            .arguments()
            .and_then(|args| args.arguments().iter().last());
    }

    if let Some(super_node) = node.as_super_node() {
        return super_node
            .arguments()
            .and_then(|args| args.arguments().iter().last());
    }

    if let Some(pinned) = node.as_pinned_expression_node() {
        return Some(pinned.expression());
    }

    if let Some(defined) = node.as_defined_node() {
        return Some(defined.value());
    }

    None
}

fn push_remove_offense(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    start: usize,
    end: usize,
    message: &str,
) {
    let (line, column) = source.offset_to_line_col(start);
    let mut diag = cop.diagnostic(source, line, column, message.to_string());
    if let Some(corrs) = corrections.as_deref_mut() {
        corrs.push(crate::correction::Correction {
            start,
            end,
            replacement: String::new(),
            cop_name: cop.name(),
            cop_index: 0,
        });
        diag.corrected = true;
    }
    diagnostics.push(diag);
}

fn push_insert_offense(
    cop: &SpaceInsideParens,
    source: &SourceFile,
    diagnostics: &mut Vec<Diagnostic>,
    corrections: &mut Option<&mut Vec<crate::correction::Correction>>,
    offset: usize,
    message: &str,
) {
    let (line, column) = source.offset_to_line_col(offset);
    let mut diag = cop.diagnostic(source, line, column, message.to_string());
    if let Some(corrs) = corrections.as_deref_mut() {
        corrs.push(crate::correction::Correction {
            start: offset,
            end: offset,
            replacement: " ".to_string(),
            cop_name: cop.name(),
            cop_index: 0,
        });
        diag.corrected = true;
    }
    diagnostics.push(diag);
}

fn is_paren_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(SpaceInsideParens, "cops/layout/space_inside_parens");
    crate::cop_variant_fixture_tests!(
        SpaceInsideParens,
        "cops/layout/space_inside_parens",
        compact,
        space
    );
    crate::cop_autocorrect_fixture_tests!(SpaceInsideParens, "cops/layout/space_inside_parens");

    #[test]
    fn space_style_flags_missing_spaces() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"x = (1 + 2)\n";
        let diags = run_cop_full_with_config(&SpaceInsideParens, src, config);
        assert_eq!(
            diags.len(),
            2,
            "space style should flag missing spaces inside parens"
        );
        assert!(diags[0].message.contains("No space"));
    }

    #[test]
    fn space_style_accepts_spaces() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"x = ( 1 + 2 )\n";
        assert_cop_no_offenses_full_with_config(&SpaceInsideParens, src, config);
    }

    #[test]
    fn space_style_ignores_close_side_for_multiline_plain_string() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"select(\"DISTINCT ON(LOWER(miq_reports.name), miq_report_results.miq_report_id) LOWER(miq_reports.name), \\\n      miq_report_results.miq_report_id\")\n";
        let diags = run_cop_full_with_config(&SpaceInsideParens, src, config);
        assert_eq!(
            diags.len(),
            1,
            "space style should only flag the opening side"
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 7);
        assert!(diags[0].message.contains("No space"));
    }

    #[test]
    fn compact_style_ignores_close_side_for_multiline_plain_string() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"select(\"DISTINCT ON(LOWER(miq_reports.name), miq_report_results.miq_report_id) LOWER(miq_reports.name), \\\n      miq_report_results.miq_report_id\")\n";
        let diags = run_cop_full_with_config(&SpaceInsideParens, src, config);
        assert_eq!(
            diags.len(),
            1,
            "compact style should only flag the opening side for multiline strings"
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 7);
        assert!(diags[0].message.contains("No space"));
    }

    #[test]
    fn compact_style_tracks_nested_close_parens_through_multiline_keyword_hash() {
        let src = b"wrap( outer: inner(\n  value\n))\n";
        let parse_result = crate::parse::parse_source(src);
        let root = parse_result.node();
        let program = root.as_program_node().unwrap();
        let node = program
            .statements()
            .body()
            .iter()
            .next()
            .expect("expected outer call");

        let last_inner = compact_last_inner_node(&node).expect("expected last inner node");
        let trailing_close =
            trailing_paren_close_offset(&last_inner, src).expect("expected nested close paren");
        let outer_close = src
            .iter()
            .rposition(|&b| b == b')')
            .expect("expected closing paren");

        assert_eq!(trailing_close, outer_close - 1);
    }

    #[test]
    fn space_style_command_form_only_requires_closing_space() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"check ( value)\n";
        let diags = run_cop_full_with_config(&SpaceInsideParens, src, config);
        assert_eq!(
            diags.len(),
            1,
            "command-form parens should only check the closing side"
        );
        assert_eq!(diags[0].location.column, 13);
        assert!(diags[0].message.contains("No space"));
    }
}
