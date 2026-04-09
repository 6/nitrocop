use std::collections::HashSet;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Detects redundant line continuations (`\` at end of line).
///
/// This cop mirrors RuboCop's baseline behavior by removing each trailing `\`
/// and reparsing the file instead of relying on a fixed operator allowlist.
/// We preserve narrow cases where RuboCop requires `\`: string concatenation,
/// arithmetic-leading next lines, unparenthesized method arguments, and
/// leading-dot chains separated by a blank line.
///
/// ## Fixes applied
///
/// - **Keyword exclusion**: Ruby keywords (`or`, `and`, `if`, `unless`, `while`,
///   `until`, etc.) at the end of a line before `\` were incorrectly treated as
///   method names that could take arguments on the next line. This caused FNs for
///   patterns like `expr or \`, `expr if \`, `expr unless \`. Fixed by excluding
///   Ruby keywords from `last_token_can_take_argument` via `is_keyword_not_method`,
///   which also avoids false-triggering on method calls like `.and` and `.or`.
///
/// - **Ternary branch detection**: Lines starting with `? ` (ternary "then"
///   branch, e.g. `? self.refs \`) were incorrectly treated as method-with-argument
///   by `method_with_argument`. Fixed by detecting ternary branch context.
///
/// - **Heredoc argument recognition**: `next_line_starts_with_argument` did not
///   recognize heredoc delimiters (`<<-file`, `<<~RUBY`) as arguments, causing
///   FPs for `method \` + `<<-heredoc` patterns. Fixed by adding `<<` to the
///   multi-char prefix check.
///
/// - **Interpolated string continuations**: Prism stores a bare `\` + newline
///   between interpolated string segments as a `StringNode` part whose content
///   ends with `\\\n`. `code_map.is_code()` treated those offsets as non-code,
///   causing FNs for patterns like `"#{a}\` + `#{b}"`. Fixed by collecting
///   Prism string-part offsets that end with `\\\n` and allowing the line scan
///   there. This also handles parts with prefix content like `" from \` + `#{b}"`.
///
/// - **Union/pipe operator**: `continues_union_rhs` matched `||` (logical OR)
///   and block parameter delimiters (`do |x|`, `{ |x|`), causing FNs when `\`
///   appeared after `||` or at the end of a block-param line. Fixed by excluding
///   `||` patterns and detecting block parameter context.
///
/// - **Regex vs division**: `starts_with_arithmetic_operator` treated `/` as
///   division, causing FNs when `\` preceded a regex literal on the next line.
///   At line start, `/` is almost always a regex. Removed `/` from the check;
///   the reparse fallback correctly handles actual division.
///
/// - **Assignment to operator chain**: `assignment_to_simple_operator_chain`
///   protects `var = \` when the RHS is a multi-line operator chain with simple
///   (non-parenthesized) operands, matching RuboCop's `argument_newline?` AST
///   check. Chains with parenthesized method calls are not protected.
///
/// - **Keyword operators on next line**: `next_line_starts_with_argument` treated
///   `and`, `or`, `not` keywords at the start of the next line as method arguments,
///   causing FNs for patterns like `(expr \` + `or other_expr)`. Fixed by excluding
///   keyword operators (followed by non-identifier char) from argument detection.
///   The `is_redundant_continuation` reparse fallback correctly determines whether
///   removal is safe (e.g., inside parens it is, at top level it isn't).
///
/// - **Percent literals, control-flow keywords, and `\\`-ending literals**:
///   `%w(...)` and `%i[...]` lines were misread as modulo operations because any
///   next line starting with `%` counted as arithmetic. Control-flow keywords on
///   the next line (`begin`, `then`, `else`, `end`, etc.) were also treated as
///   method arguments, and the blanket doubled-`\` skip hid real offenses in
///   character literals (`?\\`) and percent-array elements like `%W(... \\)`.
///   Fixed by only treating `%` as arithmetic when followed by whitespace,
///   excluding non-argument Ruby keywords at the start of the next line, detecting
///   ternary `? expr \` branches before `:`, and allowing those narrow literal
///   contexts to participate in the raw line scan.
///
/// - **`=begin`/`=end` block scanning**: RuboCop scans raw source including
///   multi-line comment blocks (`=begin`/`=end`), but our `code_map.is_code()`
///   returns false for content inside these blocks, causing FNs. Fixed by
///   tracking `=begin`/`=end` regions in the line scan and directly flagging
///   `\` at end of line inside them (skipping only string concatenation, to
///   match RuboCop's behavior). The `=begin` and `=end` markers must start at
///   column 0 per Ruby syntax.
///
/// - **AST range for embdoc blocks**: RuboCop only scans for `\\\n` within
///   `processed_source.ast.source_range`, so `=begin`/`=end` blocks that appear
///   before or after all code statements are outside the AST range and are not
///   checked. We now compute `ast_statements_range` from Prism's program node
///   and skip embdoc blocks whose start offset falls outside this range. This
///   resolved 33 FPs across several corpus repos.
///
/// - **`def` as valid argument type**: RuboCop's `ARGUMENT_TYPES` includes
///   `kDEF`, meaning `method \` + `def foo` is treated as a method call with
///   an argument (e.g., `helper_method def ordergroups_for_adding`). We were
///   incorrectly listing `def` in `is_non_argument_keyword`, causing FPs.
///   Removed `def` from that list so it is now treated as a valid argument.
///
/// - **Reparse error count comparison**: `is_redundant_continuation` now compares
///   parse error counts (`<= original_error_count`) instead of requiring zero
///   errors. This allows the reparse check to work correctly on files that have
///   pre-existing Prism parse errors unrelated to the line continuation.
///
/// - **Next-line multiline call expressions**: RuboCop keeps `\` before a
///   multiline RHS/argument expression when the expression starting on the next
///   line is itself a call/operator whose first argument begins on a later line
///   (Rails/Gon/Rubber corpus cases like `lhs = \` + `foo -` + `bar`). Our raw
///   line heuristics missed these and produced 9 FPs. Fixed by recording the
///   first `CallNode` that starts on each line and mirroring RuboCop's
///   `argument_newline?` logic on that node.
///
/// ## Remaining gaps
///
/// - **CRLF reparse compat**: `trim_end` strips `\r` so CRLF files are
///   detected. The Parser gem normalizes `\r\n` → `\n` in its buffer, so
///   RuboCop's `range.begin_pos` is a normalized offset into shorter source.
///   But `redundant_line_continuation?` does its reparse on `raw_source`
///   (CRLF) using that normalized offset, replacing wrong bytes. We replicate
///   this exact mismatch in `is_redundant_continuation` to match RuboCop 1:1.
pub struct RedundantLineContinuation;

impl Cop for RedundantLineContinuation {
    fn name(&self) -> &'static str {
        "Style/RedundantLineContinuation"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        code_map: &crate::parse::codemap::CodeMap,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let lines: Vec<&[u8]> = source.lines().collect();
        let source_bytes = source.as_bytes();
        let interpolated_literal_continuations =
            interpolated_literal_continuation_offsets(parse_result, source_bytes);
        let string_like_literal_continuations =
            string_like_literal_continuation_offsets(parse_result, source_bytes);
        let argument_newline_lines = argument_newline_lines(source, parse_result);

        // Determine the AST statements range. RuboCop only scans for `\\\n`
        // within `processed_source.ast.source_range`, which corresponds to
        // the byte range of the actual code statements. Embdoc blocks
        // (`=begin`/`=end`) that are before or after all code are outside
        // this range and should not be flagged.
        let (ast_start, ast_end) = ast_statements_range(parse_result);

        let original_error_count = parse_result.errors().count();

        let mut in_embdoc = false;
        let mut embdoc_start_offset: usize = 0;
        for (i, line) in lines.iter().enumerate() {
            // Track =begin/=end embedded document blocks.
            // =begin must start at column 0 (no leading whitespace).
            if line.starts_with(b"=begin")
                && line
                    .get(6)
                    .is_none_or(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
            {
                in_embdoc = true;
                embdoc_start_offset = source.line_start_offset(i + 1);
                continue;
            }
            if in_embdoc {
                if line.starts_with(b"=end")
                    && line
                        .get(4)
                        .is_none_or(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\n')
                {
                    in_embdoc = false;
                } else if embdoc_start_offset >= ast_start && embdoc_start_offset < ast_end {
                    // Inside =begin/=end within the AST range: \ is always
                    // redundant (it's in a comment) unless it looks like
                    // string concatenation (to match RuboCop).
                    let trimmed = trim_end(line);
                    if trimmed.ends_with(b"\\")
                        && !(trimmed.len() >= 2 && trimmed[trimmed.len() - 2] == b'\\')
                    {
                        let before_backslash = trim_end(&trimmed[..trimmed.len() - 1]);
                        if !string_concatenation(before_backslash) {
                            let col = trimmed.len() - 1;
                            diagnostics.push(self.diagnostic(
                                source,
                                i + 1,
                                col,
                                "Redundant line continuation.".to_string(),
                            ));
                        }
                    }
                }
                continue;
            }

            let trimmed = trim_end(line);
            if !trimmed.ends_with(b"\\") {
                continue;
            }

            // Compute the absolute offset of the backslash to check if it's in code.
            let line_start = source.line_start_offset(i + 1);
            let backslash_offset = line_start + trimmed.len() - 1;

            // A doubled `\\` at end of line is usually just a literal backslash,
            // but RuboCop still flags it in narrow contexts like `?\\` and `%W(... \\)`.
            if trimmed.len() >= 2
                && trimmed[trimmed.len() - 2] == b'\\'
                && !string_like_literal_continuations.contains(&backslash_offset)
            {
                continue;
            }

            // Use code_map to verify the backslash is in a code region
            // (not inside a string, heredoc, or comment)
            if !code_map.is_code(backslash_offset)
                && !interpolated_literal_continuations.contains(&backslash_offset)
                && !string_like_literal_continuations.contains(&backslash_offset)
            {
                continue;
            }

            let before_backslash = trim_end(&trimmed[..trimmed.len() - 1]);

            if continuation_is_required(before_backslash, i, &lines) {
                continue;
            }

            if argument_newline_lines.contains(&(i + 2)) {
                continue;
            }

            let next_trimmed = lines.get(i + 1).map(|next_line| trim_start(next_line));

            if next_trimmed.is_some_and(starts_with_boolean_operator)
                || is_redundant_continuation(source_bytes, backslash_offset, original_error_count)
            {
                let col = trimmed.len() - 1;
                diagnostics.push(self.diagnostic(
                    source,
                    i + 1,
                    col,
                    "Redundant line continuation.".to_string(),
                ));
            }
        }
    }
}

fn trim_end(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    while end > 0 && (bytes[end - 1] == b' ' || bytes[end - 1] == b'\t' || bytes[end - 1] == b'\r')
    {
        end -= 1;
    }
    &bytes[..end]
}

fn trim_start(bytes: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < bytes.len() && (bytes[start] == b' ' || bytes[start] == b'\t') {
        start += 1;
    }
    &bytes[start..]
}

fn continuation_is_required(before_backslash: &[u8], line_idx: usize, lines: &[&[u8]]) -> bool {
    let trimmed = trim_end(before_backslash);
    if trimmed.is_empty() {
        return false;
    }

    if string_concatenation(trimmed) {
        return true;
    }

    if leading_dot_method_chain_with_blank_line(trimmed, line_idx, lines) {
        return true;
    }

    let Some(next_line) = lines.get(line_idx + 1) else {
        return false;
    };
    let next_trimmed = trim_start(next_line);

    assignment_to_multiline_rhs(trimmed, next_trimmed)
        || assignment_to_simple_operator_chain(trimmed, line_idx, lines)
        || starts_with_arithmetic_operator(next_trimmed)
        || method_with_argument(trimmed, next_trimmed)
}

/// Check if `var = \` is followed by a simple multi-line operator chain
/// (e.g., `a.value - \ a.value - \ a.value`). RuboCop's `argument_newline?`
/// considers this non-redundant when the chain's final operand is a simple
/// expression (no parenthesized method calls), but redundant when the final
/// operand has parenthesized arguments.
fn assignment_to_simple_operator_chain(
    before_backslash: &[u8],
    line_idx: usize,
    lines: &[&[u8]],
) -> bool {
    if !ends_with_assignment_operator(before_backslash) {
        return false;
    }

    // Check if the next line ends with `operator \`
    let Some(next_line) = lines.get(line_idx + 1) else {
        return false;
    };
    let next_end = trim_end(trim_start(next_line));
    if !next_end.ends_with(b"\\") {
        return false;
    }
    let before_cont = trim_end(&next_end[..next_end.len() - 1]);
    if !matches!(before_cont.last(), Some(b'+' | b'-' | b'*' | b'/' | b'%')) {
        return false;
    }

    // Find the last line of the operator chain (first line without `\`)
    let mut idx = line_idx + 2;
    while let Some(line) = lines.get(idx) {
        let t = trim_end(line);
        if !t.ends_with(b"\\") {
            break;
        }
        idx += 1;
    }

    // Check if the final line has parenthesized method calls.
    // If it does, the continuation IS redundant (RuboCop flags it).
    // If it doesn't, the chain is simple and the continuation is required.
    let Some(last_line) = lines.get(idx) else {
        return true;
    };
    let last_trimmed = trim_start(last_line);
    !last_trimmed.contains(&b'(')
}

fn is_redundant_continuation(
    source: &[u8],
    backslash_offset: usize,
    original_error_count: usize,
) -> bool {
    // RuboCop's reparse check has a CRLF offset mismatch: the Parser gem
    // normalizes \r\n → \n in its buffer, so `range.begin_pos` is an offset
    // into the shorter (LF) source. But `processed_source.raw_source` is the
    // original CRLF content. RuboCop's reparse does:
    //   source[range.begin_pos, 2] = "\n"
    // replacing 2 bytes at the normalized offset with "\n". On CRLF files this
    // replaces the wrong bytes, causing some continuations to appear required.
    // We replicate this exactly to match RuboCop's behavior.
    let cr_count = source[..backslash_offset]
        .iter()
        .enumerate()
        .filter(|&(i, &b)| b == b'\r' && source.get(i + 1) == Some(&b'\n'))
        .count();
    let normalized_offset = backslash_offset - cr_count;

    let mut modified = source.to_vec();
    // Replicate RuboCop's `source[pos, 2] = "\n"`: replace byte at pos with \n,
    // remove byte at pos+1.
    if normalized_offset + 1 < modified.len() {
        modified[normalized_offset] = b'\n';
        modified.remove(normalized_offset + 1);
    } else if normalized_offset < modified.len() {
        modified[normalized_offset] = b'\n';
    }
    ruby_prism::parse(&modified).errors().count() <= original_error_count
}

/// Get the byte range of the AST statements, excluding leading/trailing
/// comments and embdoc blocks. RuboCop scans `processed_source.ast.source_range`
/// which only covers actual code, so embdocs outside this range are not checked.
fn ast_statements_range(parse_result: &ruby_prism::ParseResult<'_>) -> (usize, usize) {
    let node = parse_result.node();
    if let Some(program) = node.as_program_node() {
        let stmts = program.statements();
        let body: Vec<ruby_prism::Node<'_>> = stmts.body().iter().collect();
        if body.is_empty() {
            return (0, 0);
        }
        let loc = stmts.location();
        (loc.start_offset(), loc.end_offset())
    } else {
        let loc = node.location();
        (loc.start_offset(), loc.end_offset())
    }
}

fn string_concatenation(trimmed: &[u8]) -> bool {
    matches!(trimmed.last(), Some(b'"' | b'\''))
}

fn assignment_to_multiline_rhs(before_backslash: &[u8], next_trimmed: &[u8]) -> bool {
    ends_with_assignment_operator(before_backslash)
        && (continues_union_rhs(next_trimmed) || continues_string_concat_rhs(next_trimmed))
}

fn ends_with_assignment_operator(trimmed: &[u8]) -> bool {
    if !trimmed.ends_with(b"=") {
        return false;
    }

    !matches!(
        trimmed.get(trimmed.len().saturating_sub(2)).copied(),
        Some(b'=' | b'!' | b'<' | b'>')
    )
}

fn continues_union_rhs(line: &[u8]) -> bool {
    let trimmed = trim_end(line);
    if !trimmed.ends_with(b"|") || trimmed.ends_with(b"||") {
        return false;
    }
    // Exclude block parameter patterns like "do |param|" or "{ |param|"
    // These end with | but are not pipe/union operators
    if trimmed.len() >= 2 {
        let before_pipe = trimmed[trimmed.len() - 2];
        if before_pipe.is_ascii_alphanumeric() || before_pipe == b'_' {
            // Looks like |identifier| — check for a matching | after a block opener
            if let Some(pos) = trimmed[..trimmed.len() - 1]
                .iter()
                .rposition(|&b| b == b'|')
            {
                let before_first_pipe = trim_end(&trimmed[..pos]);
                if before_first_pipe.ends_with(b"do") || before_first_pipe.ends_with(b"{") {
                    return false;
                }
            }
        }
    }
    true
}

fn continues_string_concat_rhs(line: &[u8]) -> bool {
    let trimmed = trim_end(line);
    starts_with_string_literal(trim_start(trimmed)) && trimmed.ends_with(b"+")
}

fn starts_with_string_literal(trimmed: &[u8]) -> bool {
    matches!(trimmed.first(), Some(b'"' | b'\'' | b'`'))
}

fn leading_dot_method_chain_with_blank_line(
    before_backslash: &[u8],
    line_idx: usize,
    lines: &[&[u8]],
) -> bool {
    let trimmed = trim_start(before_backslash);
    if !(trimmed.starts_with(b".") || trimmed.starts_with(b"&.")) {
        return false;
    }

    lines
        .get(line_idx + 1)
        .is_some_and(|next_line| trim_start(next_line).is_empty())
}

fn starts_with_arithmetic_operator(next_trimmed: &[u8]) -> bool {
    next_trimmed.starts_with(b"**")
        || matches!(next_trimmed.first(), Some(b'*' | b'+' | b'-'))
        || (next_trimmed.starts_with(b"%")
            && next_trimmed
                .get(1)
                .is_some_and(|b| matches!(b, b' ' | b'\t')))
}

fn starts_with_boolean_operator(next_trimmed: &[u8]) -> bool {
    next_trimmed.starts_with(b"&&") || next_trimmed.starts_with(b"||")
}

/// Check if line starts with a Ruby keyword operator (`and`, `or`, `not`)
/// followed by a non-identifier character (so `and_value` is not matched).
fn starts_with_keyword_operator(trimmed: &[u8]) -> bool {
    starts_with_exact_keyword(trimmed, b"and")
        || starts_with_exact_keyword(trimmed, b"or")
        || starts_with_exact_keyword(trimmed, b"not")
}

fn starts_with_non_argument_keyword(trimmed: &[u8]) -> bool {
    leading_identifier(trimmed).is_some_and(is_non_argument_keyword)
}

/// Keywords that are NOT valid method arguments in RuboCop's `ARGUMENT_TYPES`.
/// Excludes `def` because RuboCop treats `kDEF` as a valid argument type
/// (e.g., `helper_method def foo; end` is valid Ruby).
fn is_non_argument_keyword(token: &[u8]) -> bool {
    matches!(
        token,
        b"and"
            | b"begin"
            | b"case"
            | b"class"
            | b"do"
            | b"else"
            | b"elsif"
            | b"end"
            | b"ensure"
            | b"for"
            | b"if"
            | b"in"
            | b"module"
            | b"not"
            | b"or"
            | b"redo"
            | b"rescue"
            | b"retry"
            | b"then"
            | b"unless"
            | b"until"
            | b"when"
            | b"while"
    )
}

fn starts_with_exact_keyword(trimmed: &[u8], keyword: &[u8]) -> bool {
    trimmed.starts_with(keyword)
        && trimmed
            .get(keyword.len())
            .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_')
}

fn method_with_argument(before_backslash: &[u8], next_trimmed: &[u8]) -> bool {
    // A ternary "then" branch (line starts with `? `) is not a method call
    let trimmed = trim_start(before_backslash);
    if trimmed.len() >= 2 && trimmed[0] == b'?' && (trimmed[1] == b' ' || trimmed[1] == b'\t') {
        return false;
    }
    // `cond ? expr \` + `: other` is also a ternary branch, not a method call.
    if next_trimmed.starts_with(b":")
        && before_backslash
            .windows(2)
            .any(|window| window == b"? " || window == b"?\t")
    {
        return false;
    }

    last_token_can_take_argument(before_backslash) && next_line_starts_with_argument(next_trimmed)
}

fn last_token_can_take_argument(before_backslash: &[u8]) -> bool {
    let Some(token) = trailing_identifier(before_backslash) else {
        return false;
    };

    matches!(token, b"break" | b"next" | b"return" | b"super" | b"yield")
        || (token
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || *b == b'_')
            && !is_keyword_not_method(before_backslash, token))
}

/// Check if a trailing token is a Ruby keyword used as a keyword (not a method call).
/// Keywords preceded by `.` or `&.` are method names (e.g., `.and`, `.or`).
fn is_keyword_not_method(before_backslash: &[u8], token: &[u8]) -> bool {
    if !is_ruby_keyword(token) {
        return false;
    }
    // If preceded by `.` or `&.`, it's a method call, not a keyword
    let prefix = trim_end(&before_backslash[..before_backslash.len() - token.len()]);
    !prefix.ends_with(b".") && !prefix.ends_with(b"&.")
}

fn is_ruby_keyword(token: &[u8]) -> bool {
    matches!(
        token,
        b"and"
            | b"begin"
            | b"case"
            | b"class"
            | b"def"
            | b"do"
            | b"else"
            | b"elsif"
            | b"end"
            | b"ensure"
            | b"for"
            | b"if"
            | b"in"
            | b"module"
            | b"not"
            | b"or"
            | b"redo"
            | b"rescue"
            | b"retry"
            | b"then"
            | b"unless"
            | b"until"
            | b"when"
            | b"while"
    )
}

fn trailing_identifier(bytes: &[u8]) -> Option<&[u8]> {
    let end = bytes.len();
    if end > 0 {
        let b = bytes[end - 1];
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'?' | b'!') {
            // Continue below to collect the full trailing identifier token.
        } else {
            return None;
        }
    }

    let mut start = end;
    while start > 0 {
        let b = bytes[start - 1];
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'?' | b'!') {
            start -= 1;
        } else {
            break;
        }
    }

    (start < end).then_some(&bytes[start..end])
}

fn leading_identifier(bytes: &[u8]) -> Option<&[u8]> {
    let &first = bytes.first()?;
    if !first.is_ascii_alphabetic() && first != b'_' {
        return None;
    }

    let mut end = 1;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'?' | b'!') {
            end += 1;
        } else {
            break;
        }
    }

    Some(&bytes[..end])
}

fn next_line_starts_with_argument(next_trimmed: &[u8]) -> bool {
    if next_trimmed.is_empty() {
        return false;
    }

    if starts_with_boolean_operator(next_trimmed) {
        return false;
    }

    // Don't treat keyword boolean operators (and, or, not) as arguments.
    // These are operators, not method arguments. The reparse fallback will
    // correctly determine if removing \ is safe (e.g., inside parens it is).
    if starts_with_keyword_operator(next_trimmed) {
        return false;
    }

    if starts_with_non_argument_keyword(next_trimmed) {
        return false;
    }

    if next_trimmed.starts_with(b"...")
        || next_trimmed.starts_with(b"..")
        || next_trimmed.starts_with(b"->")
        || next_trimmed.starts_with(b"**")
        || next_trimmed.starts_with(b"::")
        || next_trimmed.starts_with(b"<<")
    {
        return true;
    }

    matches!(
        next_trimmed[0],
        b'"'
            | b'\''
            | b'`'
            | b':'
            | b'?'
            | b'!'
            | b'~'
            | b'['
            | b'{'
            | b'('
            | b'|'
            | b'/'
            | b'*'
            | b'&'
            | b'+'
            | b'-'
            | b'%'
            | b'@'
            | b'$'
            | b'0'..=b'9'
            | b'A'..=b'Z'
            | b'_'
            | b'a'..=b'z'
    )
}

fn argument_newline_lines(
    source: &SourceFile,
    parse_result: &ruby_prism::ParseResult<'_>,
) -> HashSet<usize> {
    let mut collector = ArgumentNewlineLineCollector {
        source,
        seen_lines: HashSet::new(),
        argument_newline_lines: HashSet::new(),
    };
    collector.visit(&parse_result.node());
    collector.argument_newline_lines
}

fn call_argument_newline(call: &ruby_prism::CallNode<'_>, source: &SourceFile) -> bool {
    if call.opening_loc().is_some() {
        return false;
    }

    let Some(first_argument) = first_call_argument(call) else {
        return false;
    };

    if let Some(inner_call) = first_argument.as_call_node() {
        if first_call_argument(&inner_call).is_some() {
            return call_argument_newline(&inner_call, source);
        }
    }

    let selector_line = call
        .message_loc()
        .map(|loc| source.offset_to_line_col(loc.start_offset()).0)
        .unwrap_or_else(|| source.offset_to_line_col(call.location().start_offset()).0);
    let first_argument_line = source
        .offset_to_line_col(first_argument.location().start_offset())
        .0;

    selector_line != first_argument_line
}

fn first_call_argument<'pr>(call: &'pr ruby_prism::CallNode<'pr>) -> Option<ruby_prism::Node<'pr>> {
    call.arguments()?.arguments().iter().next()
}

struct ArgumentNewlineLineCollector<'a> {
    source: &'a SourceFile,
    seen_lines: HashSet<usize>,
    argument_newline_lines: HashSet<usize>,
}

impl<'pr> Visit<'pr> for ArgumentNewlineLineCollector<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let line = self
            .source
            .offset_to_line_col(node.location().start_offset())
            .0;
        if self.seen_lines.insert(line) && call_argument_newline(node, self.source) {
            self.argument_newline_lines.insert(line);
        }

        ruby_prism::visit_call_node(self, node);
    }
}

fn interpolated_literal_continuation_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
    source: &[u8],
) -> HashSet<usize> {
    let mut collector = InterpolatedStringContinuationCollector {
        source,
        offsets: HashSet::new(),
        interpolated_string_depth: 0,
        embedded_depth: 0,
    };
    collector.visit(&parse_result.node());
    collector.offsets
}

fn string_like_literal_continuation_offsets(
    parse_result: &ruby_prism::ParseResult<'_>,
    source: &[u8],
) -> HashSet<usize> {
    let mut collector = StringLikeLiteralContinuationCollector {
        source,
        offsets: HashSet::new(),
        percent_array_depth: 0,
    };
    collector.visit(&parse_result.node());
    collector.offsets
}

struct InterpolatedStringContinuationCollector<'a> {
    source: &'a [u8],
    offsets: HashSet<usize>,
    interpolated_string_depth: usize,
    embedded_depth: usize,
}

impl InterpolatedStringContinuationCollector<'_> {
    fn collect_string_part_offsets(&mut self, node: &ruby_prism::StringNode<'_>) {
        let loc = node.content_loc();
        let bytes = &self.source[loc.start_offset()..loc.end_offset()];
        if bytes.ends_with(b"\\\n") {
            let len = bytes.len();
            // Don't match escaped backslash (\\) before the newline
            if len >= 3 && bytes[len - 3] == b'\\' {
                return;
            }
            self.offsets.insert(loc.start_offset() + len - 2);
        }
    }
}

impl<'pr> Visit<'pr> for InterpolatedStringContinuationCollector<'_> {
    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        let was = self.interpolated_string_depth;
        self.interpolated_string_depth += 1;
        ruby_prism::visit_interpolated_regular_expression_node(self, node);
        self.interpolated_string_depth = was;
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        let was = self.interpolated_string_depth;
        let is_heredoc = node
            .opening_loc()
            .is_some_and(|opening| opening.as_slice().starts_with(b"<<"));
        if !is_heredoc {
            self.interpolated_string_depth += 1;
        }
        ruby_prism::visit_interpolated_string_node(self, node);
        self.interpolated_string_depth = was;
    }

    fn visit_embedded_statements_node(&mut self, node: &ruby_prism::EmbeddedStatementsNode<'pr>) {
        let was = self.embedded_depth;
        self.embedded_depth += 1;
        ruby_prism::visit_embedded_statements_node(self, node);
        self.embedded_depth = was;
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if self.interpolated_string_depth > 0 && self.embedded_depth == 0 {
            self.collect_string_part_offsets(node);
        }
        ruby_prism::visit_string_node(self, node);
    }
}

struct StringLikeLiteralContinuationCollector<'a> {
    source: &'a [u8],
    offsets: HashSet<usize>,
    percent_array_depth: usize,
}

impl StringLikeLiteralContinuationCollector<'_> {
    fn collect_string_end_backslash(&mut self, node: &ruby_prism::StringNode<'_>) {
        let loc = node.content_loc();
        let bytes = &self.source[loc.start_offset()..loc.end_offset()];
        if bytes.ends_with(b"\\\\") {
            self.offsets.insert(loc.end_offset() - 1);
        }
    }
}

impl<'pr> Visit<'pr> for StringLikeLiteralContinuationCollector<'_> {
    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        let was_percent_array = self.percent_array_depth;
        if let Some(opening) = node.opening_loc() {
            if opening.as_slice().starts_with(b"%") {
                self.percent_array_depth += 1;
            }
        }
        ruby_prism::visit_array_node(self, node);
        self.percent_array_depth = was_percent_array;
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        let is_character_literal = node
            .opening_loc()
            .is_some_and(|opening| opening.as_slice() == b"?");
        if is_character_literal || self.percent_array_depth > 0 {
            self.collect_string_end_backslash(node);
        }
        ruby_prism::visit_string_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        RedundantLineContinuation,
        "cops/style/redundant_line_continuation"
    );

    #[test]
    fn crlf_line_endings() {
        // CRLF file: `foo(1, \<CR><LF>  2)<CR><LF>` should detect the redundant continuation
        let source = b"foo(1, \\\r\n  2)\r\n";
        let diags = crate::testutil::run_cop_full(&RedundantLineContinuation, source);
        assert_eq!(
            diags.len(),
            1,
            "expected 1 offense on CRLF file, got {diags:?}"
        );
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 7);
    }

    #[test]
    fn crlf_no_offense_string_concat() {
        // String concatenation with CRLF is still required
        let source = b"x = \"hello\" \\\r\n  \"world\"\r\n";
        let diags = crate::testutil::run_cop_full(&RedundantLineContinuation, source);
        assert!(
            diags.is_empty(),
            "string concat should not be flagged: {diags:?}"
        );
    }
}
