use crate::diagnostic::{Diagnostic, Location, Severity};
use crate::parse::source::SourceFile;

/// Count body lines between start and end offsets (exclusive of keyword lines).
/// Skips blank lines. Optionally skips comment-only lines.
pub fn count_body_lines(
    source: &SourceFile,
    start_offset: usize,
    end_offset: usize,
    count_comments: bool,
) -> usize {
    count_body_lines_ex(source, start_offset, end_offset, count_comments, &[])
}

/// Count body lines with foldable line ranges.
/// `foldable_ranges` contains (start_line, end_line) pairs (1-indexed) of multiline
/// constructs that should count as a single line instead of their actual line count.
pub fn count_body_lines_ex(
    source: &SourceFile,
    start_offset: usize,
    end_offset: usize,
    count_comments: bool,
    foldable_ranges: &[(usize, usize)],
) -> usize {
    count_body_lines_impl(
        source,
        start_offset,
        end_offset,
        count_comments,
        foldable_ranges,
        &[],
        false,
    )
}

/// Count body lines with foldable line ranges and fully excluded line ranges.
/// `foldable_ranges` contains (start_line, end_line) pairs (1-indexed) of multiline
/// constructs that should count as a single line instead of their actual line count.
/// `excluded_ranges` contains (start_line, end_line) pairs (1-indexed) of lines to
/// completely exclude from the count (e.g., inner class/module definitions).
pub fn count_body_lines_full(
    source: &SourceFile,
    start_offset: usize,
    end_offset: usize,
    count_comments: bool,
    foldable_ranges: &[(usize, usize)],
    excluded_ranges: &[(usize, usize)],
) -> usize {
    count_body_lines_impl(
        source,
        start_offset,
        end_offset,
        count_comments,
        foldable_ranges,
        excluded_ranges,
        true,
    )
}

/// Internal implementation for counting body lines.
///
/// When `classlike` is true, replicates RuboCop's off-by-one in
/// `CodeLengthCalculator#classlike_code_length`: for each line number `ln` in the
/// body range, RuboCop accesses `@processed_source[ln]` (0-indexed array lookup on
/// 1-indexed line numbers), which effectively examines the content of line `ln+1`.
/// This skips the first body line and instead counts the `end` keyword line.
/// For most modules/classes the two effects cancel out, but they diverge when the
/// first body line is blank/comment (FN) or near inner class/module boundaries (FP).
fn count_body_lines_impl(
    source: &SourceFile,
    start_offset: usize,
    end_offset: usize,
    count_comments: bool,
    foldable_ranges: &[(usize, usize)],
    excluded_ranges: &[(usize, usize)],
    classlike: bool,
) -> usize {
    let (start_line, _) = source.offset_to_line_col(start_offset);
    let (end_line, _) = source.offset_to_line_col(end_offset);

    // Build a set of lines that are "folded away" (continuation lines of foldable constructs)
    let mut folded_lines = std::collections::HashSet::new();
    for &(fold_start, fold_end) in foldable_ranges {
        // The first line counts as 1, additional lines are folded
        for line in (fold_start + 1)..=fold_end {
            folded_lines.insert(line);
        }
    }

    // Build a set of fully excluded lines (inner class/module definitions)
    let mut excluded_lines = std::collections::HashSet::new();
    for &(excl_start, excl_end) in excluded_ranges {
        for line in excl_start..=excl_end {
            excluded_lines.insert(line);
        }
    }

    // Count lines between (exclusive of def/end lines)
    let lines: Vec<&[u8]> = source.lines().collect();
    let mut count = 0;

    // Lines between start_line and end_line (exclusive)
    // start_line and end_line are 1-indexed
    for line_num in (start_line + 1)..end_line {
        if line_num > lines.len() {
            break;
        }

        // Skip fully excluded lines (inner class/module)
        if excluded_lines.contains(&line_num) {
            continue;
        }

        // Skip folded continuation lines
        if folded_lines.contains(&line_num) {
            continue;
        }

        // When classlike is true, use lines[line_num] (shifted by +1) to match
        // RuboCop's off-by-one in processed_source[ln] indexing.
        // Otherwise use lines[line_num - 1] (correct 0-indexed access).
        let line_index = if classlike { line_num } else { line_num - 1 };
        if line_index >= lines.len() {
            break;
        }
        let line = lines[line_index];

        // NOTE: =begin/=end multi-line comments are NOT excluded here.
        // RuboCop's `comment_line?` only matches `#`-style comments (regex /^\s*#/),
        // so =begin/=end content is always counted as body lines regardless of
        // the `CountComments` setting. This matches RuboCop behavior.

        let trimmed = trim_bytes(line);

        // Skip blank lines
        if trimmed.is_empty() {
            continue;
        }

        // Optionally skip comment-only lines
        if !count_comments && trimmed.starts_with(b"#") {
            continue;
        }

        count += 1;
    }

    count
}

/// Recursively collect line ranges of all inner class/module/sclass definitions
/// within a body node. Returns (start_line, end_line) pairs (1-indexed).
///
/// RuboCop uses `each_descendant(:module, :class)` which traverses the entire AST,
/// finding class/module nodes nested inside `begin`, `if`, `rescue`, `class << self`,
/// etc. This visitor replicates that behavior.
pub fn inner_classlike_ranges(
    source: &SourceFile,
    body: &ruby_prism::Node<'_>,
) -> Vec<(usize, usize)> {
    use ruby_prism::Visit;

    struct ClasslikeFinder<'a> {
        source: &'a SourceFile,
        ranges: Vec<(usize, usize)>,
    }

    impl<'pr> Visit<'pr> for ClasslikeFinder<'_> {
        fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
            let loc = node.location();
            let (start, _) = self.source.offset_to_line_col(loc.start_offset());
            let end_off = loc.end_offset().saturating_sub(1).max(loc.start_offset());
            let (end, _) = self.source.offset_to_line_col(end_off);
            self.ranges.push((start, end));
            // Do NOT recurse into class body — RuboCop's each_descendant finds
            // nested classes but the outer class's exclusion already covers them.
        }

        fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'pr>) {
            let loc = node.location();
            let (start, _) = self.source.offset_to_line_col(loc.start_offset());
            let end_off = loc.end_offset().saturating_sub(1).max(loc.start_offset());
            let (end, _) = self.source.offset_to_line_col(end_off);
            self.ranges.push((start, end));
            // Do NOT recurse — same as class above.
        }

        // NOTE: visit_singleton_class_node (class << self) is intentionally NOT handled here.
        // RuboCop's line_numbers_of_inner_nodes only excludes :module and :class, NOT :sclass.
        // Singleton class lines count toward the enclosing module/class length.
    }

    let mut finder = ClasslikeFinder {
        source,
        ranges: Vec::new(),
    };
    finder.visit(body);
    finder.ranges
}

/// Collect line ranges of heredoc bodies within a node.
/// Returns pairs of (start_line, end_line) (1-indexed) for multiline heredoc nodes.
///
/// Used when `CountAsOne` includes "heredoc" to fold each heredoc to a single
/// line. In Prism, InterpolatedStringNode.location() only covers the opening
/// delimiter (e.g. `<<~HEREDOC`), but closing_loc() gives the end of the
/// heredoc content. This function uses closing_loc() to get the correct range.
pub fn collect_heredoc_ranges(
    source: &SourceFile,
    body_node: &ruby_prism::Node<'_>,
) -> Vec<(usize, usize)> {
    use ruby_prism::Visit;
    let mut visitor = HeredocVisitor {
        source,
        ranges: Vec::new(),
    };
    visitor.visit(body_node);
    visitor.ranges
}

struct HeredocVisitor<'a> {
    source: &'a SourceFile,
    ranges: Vec<(usize, usize)>,
}

impl<'pr> ruby_prism::Visit<'pr> for HeredocVisitor<'_> {
    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        // Check if this is a heredoc by looking at the opening — heredocs have
        // opening_loc that starts with <<
        if let Some(opening) = node.opening_loc() {
            let bytes = &self.source.as_bytes()[opening.start_offset()..opening.end_offset()];
            if bytes.starts_with(b"<<") {
                // In Prism, heredoc location() only covers the opening delimiter.
                // The actual content and closing delimiter are found via closing_loc().
                let (start_line, _) = self.source.offset_to_line_col(opening.start_offset());
                if let Some(closing) = node.closing_loc() {
                    let end_off = closing
                        .end_offset()
                        .saturating_sub(1)
                        .max(closing.start_offset());
                    let (end_line, _) = self.source.offset_to_line_col(end_off);
                    if end_line > start_line {
                        self.ranges.push((start_line, end_line));
                    }
                }
                return; // Don't recurse into heredoc
            }
        }
        ruby_prism::visit_interpolated_string_node(self, node);
    }

    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if let Some(opening) = node.opening_loc() {
            let bytes = &self.source.as_bytes()[opening.start_offset()..opening.end_offset()];
            if bytes.starts_with(b"<<") {
                let (start_line, _) = self.source.offset_to_line_col(opening.start_offset());
                if let Some(closing) = node.closing_loc() {
                    let end_off = closing
                        .end_offset()
                        .saturating_sub(1)
                        .max(closing.start_offset());
                    let (end_line, _) = self.source.offset_to_line_col(end_off);
                    if end_line > start_line {
                        self.ranges.push((start_line, end_line));
                    }
                }
                return; // Don't recurse
            }
        }
        ruby_prism::visit_string_node(self, node);
    }
}

/// Collect line ranges of foldable constructs within a node.
/// `count_as_one` contains type names like "array", "hash", "heredoc", "method_call".
/// Returns pairs of (start_line, end_line) (1-indexed) for multiline foldable constructs.
pub fn collect_foldable_ranges(
    source: &SourceFile,
    body_node: &ruby_prism::Node<'_>,
    count_as_one: &[String],
) -> Vec<(usize, usize)> {
    use ruby_prism::Visit;
    let mut visitor = FoldableVisitor {
        source,
        count_as_one,
        ranges: Vec::new(),
    };
    visitor.visit(body_node);
    visitor.ranges
}

struct FoldableVisitor<'a> {
    source: &'a SourceFile,
    count_as_one: &'a [String],
    ranges: Vec<(usize, usize)>,
}

impl<'pr> ruby_prism::Visit<'pr> for FoldableVisitor<'_> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        let is_foldable = match &node {
            ruby_prism::Node::ArrayNode { .. } => self.count_as_one.iter().any(|s| s == "array"),
            ruby_prism::Node::HashNode { .. } => self.count_as_one.iter().any(|s| s == "hash"),
            ruby_prism::Node::InterpolatedStringNode { .. } => {
                // Heredocs are handled separately by collect_heredoc_ranges()
                // because InterpolatedStringNode.location() only covers the
                // opening delimiter in Prism, not the heredoc content/closing.
                // collect_heredoc_ranges uses closing_loc() to get the full range.
                false
            }
            ruby_prism::Node::CallNode { .. } => {
                self.count_as_one.iter().any(|s| s == "method_call")
            }
            _ => false,
        };

        if is_foldable {
            let loc = node.location();
            let (start_line, _) = self.source.offset_to_line_col(loc.start_offset());
            let end_off = loc.end_offset().saturating_sub(1).max(loc.start_offset());
            let (end_line, _) = self.source.offset_to_line_col(end_off);
            if end_line > start_line {
                self.ranges.push((start_line, end_line)); // Don't recurse into foldable construct
            }
        }
    }
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let start = b
        .iter()
        .position(|&c| c != b' ' && c != b'\t' && c != b'\r');
    match start {
        Some(s) => {
            let end = b
                .iter()
                .rposition(|&c| c != b' ' && c != b'\t' && c != b'\r')
                .unwrap();
            &b[s..=end]
        }
        None => &[],
    }
}

/// Check if a name is snake_case (lowercase + digits + underscores, not starting with uppercase).
/// Non-ASCII characters are allowed only if they are Unicode lowercase letters (matching
/// Ruby's `[[:lower:]]` POSIX class). CJK characters, emoji, and other non-lowercase
/// Unicode characters are rejected.
pub fn is_snake_case(name: &[u8]) -> bool {
    if name.is_empty() {
        return true;
    }
    let s = match std::str::from_utf8(name) {
        Ok(s) => s,
        Err(_) => return false,
    };
    for ch in s.chars() {
        if ch.is_ascii() {
            if ch.is_ascii_uppercase() {
                return false;
            }
            if !(ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || ch == '_'
                || ch == '?'
                || ch == '!'
                || ch == '=')
            {
                return false;
            }
        } else if !ch.is_lowercase() {
            return false;
        }
    }
    true
}

/// Check if a name is SCREAMING_SNAKE_CASE (uppercase + digits + underscores).
pub fn is_screaming_snake_case(name: &[u8]) -> bool {
    if name.is_empty() {
        return true;
    }
    for &b in name {
        if b.is_ascii_lowercase() {
            return false;
        }
        if !(b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') {
            return false;
        }
    }
    true
}

/// Check if a name is CamelCase (starts with letter, no underscores in body).
/// RuboCop's camelCase regex allows lowercase-starting names (e.g., `userName`),
/// so we accept both lowercase and uppercase first characters.
pub fn is_camel_case(name: &[u8]) -> bool {
    if name.is_empty() {
        return false;
    }
    // RuboCop's camelCase regex: /^@{0,2}(?:_|_?[[:lower:]][\d[:lower:][:upper:]]*)[!?=]?$/
    // After optional @/@@ prefix, it allows:
    //   - Just underscore: _
    //   - Optional underscore + lowercase start + alphanumerics: _?lower+alnum*
    // So first char can be lowercase (userName) or the underscore itself
    if name[0].is_ascii() && !name[0].is_ascii_alphabetic() && name[0] != b'_' {
        return false;
    }
    for &b in &name[1..] {
        if b == b'_' {
            return false;
        }
        // Allow non-ASCII bytes (UTF-8 multibyte characters) and ASCII alphanumerics
        if b.is_ascii() && !b.is_ascii_alphanumeric() {
            return false;
        }
    }
    true
}

/// Check if all bytes in a name are ASCII.
pub fn is_ascii_name(name: &[u8]) -> bool {
    name.iter().all(|b| b.is_ascii())
}

/// Info about a 2-method chain: `receiver.inner_method(...).outer_method(...)`.
pub struct MethodChain<'a> {
    /// The inner CallNode (the receiver of the outer call).
    pub inner_call: ruby_prism::CallNode<'a>,
    /// The method name of the inner call.
    pub inner_method: &'a [u8],
    /// The method name of the outer call.
    pub outer_method: &'a [u8],
}

/// Extract a 2-method chain from a node.
///
/// If `node` is a CallNode `x.outer()` whose receiver is also a CallNode `y.inner()`,
/// returns `Some(MethodChain { inner_call, inner_method, outer_method })`.
pub fn as_method_chain<'a>(node: &ruby_prism::Node<'a>) -> Option<MethodChain<'a>> {
    let outer_call = node.as_call_node()?;
    let outer_method = outer_call.name().as_slice();
    let receiver = outer_call.receiver()?;
    let inner_call = receiver.as_call_node()?;
    let inner_method = inner_call.name().as_slice();
    Some(MethodChain {
        inner_call,
        inner_method,
        outer_method,
    })
}

/// Check if the line above a node's start offset is a comment line.
pub fn preceding_comment_line(source: &SourceFile, node_start_offset: usize) -> bool {
    let (node_line, _) = source.offset_to_line_col(node_start_offset);
    if node_line <= 1 {
        return false;
    }
    let lines: Vec<&[u8]> = source.lines().collect();
    let prev_line = lines.get(node_line - 2); // node_line is 1-indexed, prev is node_line-1, 0-indexed = node_line-2
    match prev_line {
        Some(line) => {
            let trimmed = trim_bytes(line);
            trimmed.starts_with(b"#")
        }
        None => false,
    }
}

/// Check if a node spans exactly one line in the source.
pub fn node_on_single_line(source: &SourceFile, loc: &ruby_prism::Location<'_>) -> bool {
    let (start_line, _) = source.offset_to_line_col(loc.start_offset());
    let end_offset = loc.end_offset().saturating_sub(1).max(loc.start_offset());
    let (end_line, _) = source.offset_to_line_col(end_offset);
    start_line == end_line
}

/// Compute the expected indentation column for body statements
/// given the keyword's column and the configured width.
pub fn expected_indent_for_body(keyword_column: usize, width: usize) -> usize {
    keyword_column + width
}

/// If the keyword at `keyword_offset` is on the RHS of an assignment
/// (e.g., `x = if ...`), returns the column of the first non-whitespace
/// character on that line (the LHS variable). Returns None otherwise.
pub fn assignment_context_base_col(source: &SourceFile, keyword_offset: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    // Find the start of the line containing keyword_offset
    let mut line_start = keyword_offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    // Slice from line start to keyword position
    let before_keyword = &bytes[line_start..keyword_offset];

    // Scan for bare `=` (not part of ==, !=, <=, >=, =~, =>)
    let mut i = 0;
    while i < before_keyword.len() {
        if before_keyword[i] == b'=' {
            // Check following char: skip ==, =~, =>
            let next = before_keyword.get(i + 1).copied().unwrap_or(b' ');
            if next == b'=' || next == b'~' || next == b'>' {
                i += 2;
                continue;
            }
            // Check preceding char: skip !=, <=, >=, ==
            if i > 0 {
                let prev = before_keyword[i - 1];
                if prev == b'!' || prev == b'<' || prev == b'>' || prev == b'=' {
                    i += 1;
                    continue;
                }
            }
            // Found a bare assignment `=`
            // Return column of first non-whitespace on the line
            return before_keyword.iter().position(|&b| b != b' ' && b != b'\t');
        }
        i += 1;
    }

    // Also check for `<<` operator before keyword (e.g., `html << if cond`).
    // RuboCop treats `<<` as assignment context for alignment purposes.
    let mut j = 0;
    while j + 1 < before_keyword.len() {
        if before_keyword[j] == b'<' && before_keyword[j + 1] == b'<' {
            // Skip <<= (compound assignment) and <<~ <<- (heredoc markers)
            let next = before_keyword.get(j + 2).copied().unwrap_or(b' ');
            if next == b'=' || next == b'~' || next == b'-' {
                j += 3;
                continue;
            }
            // Found `<<` — return column of first non-whitespace on line
            return before_keyword.iter().position(|&b| b != b' ' && b != b'\t');
        }
        j += 1;
    }

    None
}

/// Get the line content at a given 1-indexed line number.
pub fn line_at(source: &SourceFile, line_number: usize) -> Option<&[u8]> {
    source.lines().nth(line_number - 1)
}

/// Get the indentation (number of leading spaces) for a byte slice.
pub fn indentation_of(line: &[u8]) -> usize {
    line.iter().take_while(|&&b| b == b' ').count()
}

/// Find the first non-whitespace byte offset starting from `start`.
///
/// All ASCII whitespace (space, tab, newline, \r, etc.) is skipped.
/// Returns the absolute byte offset, or `None` if only whitespace remains.
pub fn first_non_whitespace_offset(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|pos| start + pos)
}

/// Find the first non-space/tab byte offset on the current line starting from `offset`.
///
/// Unlike [`first_non_whitespace_offset`], this treats newlines as a boundary:
/// returns `None` if a newline or end-of-bytes is reached before any visible content.
pub fn first_non_space_on_line(bytes: &[u8], mut offset: usize) -> Option<usize> {
    while offset < bytes.len() && (bytes[offset] == b' ' || bytes[offset] == b'\t') {
        offset += 1;
    }
    if offset < bytes.len() && bytes[offset] != b'\n' && bytes[offset] != b'\r' {
        Some(offset)
    } else {
        None
    }
}

/// Check if there is a trailing comma between last_element_end and closing_start.
pub fn has_trailing_comma(
    source_bytes: &[u8],
    last_element_end: usize,
    closing_start: usize,
) -> bool {
    if last_element_end >= closing_start || closing_start > source_bytes.len() {
        return false;
    }
    // Scan the region between the last element and the closing delimiter,
    // skipping content inside comments (# to end of line) and string literals.
    let region = &source_bytes[last_element_end..closing_start];
    let mut in_comment = false;
    for &b in region {
        if in_comment {
            if b == b'\n' {
                in_comment = false;
            }
            continue;
        }
        if b == b'#' {
            in_comment = true;
            continue;
        }
        if b == b',' {
            return true;
        }
    }
    false
}

// ── Shared cop logic helpers ──────────────────────────────────────────

/// Check if a line is blank (completely empty, no content at all).
///
/// RuboCop considers a line blank only if it has zero content — lines with
/// trailing whitespace (spaces/tabs) are NOT blank.  Those are handled by
/// `Layout/TrailingWhitespace` instead.  Having two cops register offenses
/// for the same whitespace-only line would cause havoc in autocorrection.
pub fn is_blank_line(line: &[u8]) -> bool {
    line.is_empty() || line == b"\r"
}

/// Check if a line is blank or whitespace-only.
///
/// This is used by spacing/separation cops that mirror RuboCop's `blank?`
/// behavior when deciding whether an empty separator line exists.
pub fn is_blank_or_whitespace_line(line: &[u8]) -> bool {
    line.iter()
        .all(|&b| b == b' ' || b == b'\t' || b == b'\r' || b == b'\x0C' || b == b'\x0B')
}

/// Check for extra empty lines at the beginning/end of a body.
/// Used by EmptyLinesAround{Class,Module,Method,Block}Body.
pub fn check_empty_lines_around_body(
    cop_name: &'static str,
    source: &SourceFile,
    keyword_offset: usize,
    end_offset: usize,
    body_kind: &str,
) -> Vec<Diagnostic> {
    check_empty_lines_around_body_with_corrections(
        cop_name,
        source,
        keyword_offset,
        end_offset,
        body_kind,
        None,
    )
}

/// Like `check_empty_lines_around_body` but also generates corrections when `corrections` is Some.
pub fn check_empty_lines_around_body_with_corrections(
    cop_name: &'static str,
    source: &SourceFile,
    keyword_offset: usize,
    end_offset: usize,
    body_kind: &str,
    mut corrections: Option<&mut Vec<crate::correction::Correction>>,
) -> Vec<Diagnostic> {
    let (keyword_line, _) = source.offset_to_line_col(keyword_offset);
    let (end_line, _) = source.offset_to_line_col(end_offset);

    if keyword_line == end_line {
        return Vec::new();
    }

    let mut diagnostics = Vec::new();

    // Check for blank line after keyword
    let after_keyword = keyword_line + 1;
    if let Some(line) = line_at(source, after_keyword) {
        if is_blank_line(line) && after_keyword < end_line {
            let mut diag = Diagnostic {
                path: source.path_str().to_string(),
                location: Location {
                    line: after_keyword,
                    column: 0,
                },
                severity: Severity::Convention,
                cop_name: cop_name.to_string(),
                message: format!("Extra empty line detected at {body_kind} body beginning."),
                corrected: false,
            };
            if let Some(ref mut corr) = corrections {
                if let (Some(start), Some(end)) = (
                    source.line_col_to_offset(after_keyword, 0),
                    source.line_col_to_offset(after_keyword + 1, 0),
                ) {
                    corr.push(crate::correction::Correction {
                        start,
                        end,
                        replacement: String::new(),
                        cop_name,
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
            }
            diagnostics.push(diag);
        }
    }

    // Check for blank line before end
    if end_line > 1 {
        let before_end = end_line - 1;
        if before_end > keyword_line {
            if let Some(line) = line_at(source, before_end) {
                if is_blank_line(line) {
                    let mut diag = Diagnostic {
                        path: source.path_str().to_string(),
                        location: Location {
                            line: before_end,
                            column: 0,
                        },
                        severity: Severity::Convention,
                        cop_name: cop_name.to_string(),
                        message: format!("Extra empty line detected at {body_kind} body end."),
                        corrected: false,
                    };
                    if let Some(ref mut corr) = corrections {
                        if let (Some(start), Some(end)) = (
                            source.line_col_to_offset(before_end, 0),
                            source.line_col_to_offset(before_end + 1, 0),
                        ) {
                            corr.push(crate::correction::Correction {
                                start,
                                end,
                                replacement: String::new(),
                                cop_name,
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                    }
                    diagnostics.push(diag);
                }
            }
        }
    }

    diagnostics
}

/// Check for MISSING empty lines at the beginning/end of a body.
/// Used by EmptyLinesAround{Block,Class,Module}Body with "empty_lines" style.
pub fn check_missing_empty_lines_around_body(
    cop_name: &'static str,
    source: &SourceFile,
    keyword_offset: usize,
    end_offset: usize,
    body_kind: &str,
) -> Vec<Diagnostic> {
    check_missing_empty_lines_around_body_with_corrections(
        cop_name,
        source,
        keyword_offset,
        end_offset,
        body_kind,
        None,
    )
}

/// Like `check_missing_empty_lines_around_body` but also generates corrections when `corrections` is Some.
pub fn check_missing_empty_lines_around_body_with_corrections(
    cop_name: &'static str,
    source: &SourceFile,
    keyword_offset: usize,
    end_offset: usize,
    body_kind: &str,
    mut corrections: Option<&mut Vec<crate::correction::Correction>>,
) -> Vec<Diagnostic> {
    let (keyword_line, _) = source.offset_to_line_col(keyword_offset);
    let (end_line, _) = source.offset_to_line_col(end_offset);

    // Skip single-line or empty bodies
    if end_line <= keyword_line + 1 {
        return Vec::new();
    }

    let mut diagnostics = Vec::new();

    // Check for missing blank line after keyword
    let after_keyword = keyword_line + 1;
    if let Some(line) = line_at(source, after_keyword) {
        if !is_blank_line(line) && after_keyword < end_line {
            let mut diag = Diagnostic {
                path: source.path_str().to_string(),
                location: Location {
                    line: after_keyword,
                    column: 0,
                },
                severity: Severity::Convention,
                cop_name: cop_name.to_string(),
                message: format!("Empty line missing at {body_kind} body beginning."),
                corrected: false,
            };
            if let Some(ref mut corr) = corrections {
                if let Some(offset) = source.line_col_to_offset(after_keyword, 0) {
                    corr.push(crate::correction::Correction {
                        start: offset,
                        end: offset,
                        replacement: "\n".to_string(),
                        cop_name,
                        cop_index: 0,
                    });
                    diag.corrected = true;
                }
            }
            diagnostics.push(diag);
        }
    }

    // Check for missing blank line before end
    if end_line > 1 {
        let before_end = end_line - 1;
        if before_end > keyword_line {
            if let Some(line) = line_at(source, before_end) {
                if !is_blank_line(line) {
                    // RuboCop reports "missing at body end" on the end/} line itself
                    let mut diag = Diagnostic {
                        path: source.path_str().to_string(),
                        location: Location {
                            line: end_line,
                            column: 0,
                        },
                        severity: Severity::Convention,
                        cop_name: cop_name.to_string(),
                        message: format!("Empty line missing at {body_kind} body end."),
                        corrected: false,
                    };
                    if let Some(ref mut corr) = corrections {
                        // Insert \n before the end line
                        if let Some(offset) = source.line_col_to_offset(end_line, 0) {
                            corr.push(crate::correction::Correction {
                                start: offset,
                                end: offset,
                                replacement: "\n".to_string(),
                                cop_name,
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                    }
                    diagnostics.push(diag);
                }
            }
        }
    }

    diagnostics
}

/// Check that `end` is aligned with the opening keyword.
/// Used by DefEndAlignment, EndAlignment, ElseAlignment.
pub fn check_keyword_end_alignment(
    cop_name: &str,
    source: &SourceFile,
    keyword_name: &str,
    keyword_offset: usize,
    end_offset: usize,
) -> Vec<Diagnostic> {
    // Use the column of the first non-whitespace character on the line containing the
    // keyword. This handles modifiers like `private_class_method def ...` (align with
    // `private_class_method`) and also correctly handles tab indentation and BOM markers,
    // since we use offset_to_line_col for proper character-based column computation.
    let first_non_ws_col = {
        let bytes = source.as_bytes();
        let mut line_start = keyword_offset;
        while line_start > 0 && bytes[line_start - 1] != b'\n' {
            line_start -= 1;
        }
        let mut pos = line_start;
        while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
            pos += 1;
        }
        let (_, col) = source.offset_to_line_col(pos);
        col
    };
    let (end_line, end_col) = source.offset_to_line_col(end_offset);

    if end_col != first_non_ws_col {
        return vec![Diagnostic {
            path: source.path_str().to_string(),
            location: Location {
                line: end_line,
                column: end_col,
            },
            severity: Severity::Convention,
            cop_name: cop_name.to_string(),
            message: format!("Align `end` with `{keyword_name}`."),

            corrected: false,
        }];
    }

    Vec::new()
}

/// Check if the given byte offset is the first non-whitespace character on its line.
/// Matches RuboCop's `begins_its_line?` helper.
pub fn begins_its_line(source: &SourceFile, offset: usize) -> bool {
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

/// Check first element indentation relative to an opening delimiter.
/// Used by FirstArgument/Array/HashElementIndentation.
pub fn check_first_element_indentation(
    cop_name: &str,
    source: &SourceFile,
    width: usize,
    opening_offset: usize,
    first_element_offset: usize,
) -> Vec<Diagnostic> {
    let (open_line, _) = source.offset_to_line_col(opening_offset);
    let (elem_line, elem_col) = source.offset_to_line_col(first_element_offset);

    // Skip if on same line as opener
    if elem_line == open_line {
        return Vec::new();
    }

    let open_line_bytes = source.lines().nth(open_line - 1).unwrap_or(b"");
    let open_indent = indentation_of(open_line_bytes);
    let expected = open_indent + width;

    if elem_col != expected {
        return vec![Diagnostic {
            path: source.path_str().to_string(),
            location: Location {
                line: elem_line,
                column: elem_col,
            },
            severity: Severity::Convention,
            cop_name: cop_name.to_string(),
            message: format!(
                "Use {} (not {}) spaces for indentation of the first element.",
                width,
                elem_col.saturating_sub(open_indent)
            ),
            corrected: false,
        }];
    }

    Vec::new()
}

// ── Rails-specific helpers ─────────────────────────────────────────────

/// Extract the superclass constant name from a ClassNode.
///
/// For `class Foo < ActiveRecord::Base`, returns `Some(b"ActiveRecord::Base")`.
/// Returns `None` if the class has no superclass or the superclass isn't a
/// simple constant or constant path.
pub fn parent_class_name<'a>(
    source: &'a SourceFile,
    class_node: &ruby_prism::ClassNode<'a>,
) -> Option<&'a [u8]> {
    let superclass = class_node.superclass()?;
    let loc = superclass.location();
    Some(&source.as_bytes()[loc.start_offset()..loc.end_offset()])
}

/// Check if a CallNode is a receiverless DSL-style call with the given method name.
///
/// Matches patterns like `has_many`, `validates`, `before_action` etc.
pub fn is_dsl_call(call: &ruby_prism::CallNode<'_>, name: &[u8]) -> bool {
    call.receiver().is_none() && call.name().as_slice() == name
}

/// Get all direct call statements from a class body's StatementsNode.
///
/// Returns an iterator over CallNode entries in the class body at the top level
/// (not nested inside methods).
pub fn class_body_calls<'a>(
    class_node: &ruby_prism::ClassNode<'a>,
) -> Vec<ruby_prism::CallNode<'a>> {
    let body = match class_node.body() {
        Some(b) => b,
        None => return Vec::new(),
    };
    let stmts = match body.as_statements_node() {
        Some(s) => s,
        None => return Vec::new(),
    };
    stmts
        .body()
        .iter()
        .filter_map(|node| node.as_call_node())
        .collect()
}

/// Check if a CallNode has a keyword argument with the given key name.
///
/// Looks for `key: value` in the call's argument list.
pub fn has_keyword_arg(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> bool {
    keyword_arg_value(call, key).is_some()
}

/// Get the value node of a keyword argument with the given key name.
///
/// For `has_many :items, dependent: :destroy`, `keyword_arg_value(call, b"dependent")`
/// returns the SymbolNode for `:destroy`.
pub fn keyword_arg_value<'a>(
    call: &ruby_prism::CallNode<'a>,
    key: &[u8],
) -> Option<ruby_prism::Node<'a>> {
    let args = call.arguments()?;
    for arg in args.arguments().iter() {
        // Direct keyword hash pairs in arguments
        if let Some(kw) = arg.as_keyword_hash_node() {
            for elem in kw.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == key {
                            return Some(assoc.value());
                        }
                    }
                }
            }
        }
        // Hash literal as last argument
        if let Some(hash) = arg.as_hash_node() {
            for elem in hash.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == key {
                            return Some(assoc.value());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Get the start offset of the AssocNode for a keyword argument in a call's arguments.
///
/// This is useful when you need to report a diagnostic at the `key: value` pair
/// rather than at the call node's start (e.g., for multiline associations where
/// the key may be on a different line than the method call).
pub fn keyword_arg_pair_start_offset(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> Option<usize> {
    let args = call.arguments()?;
    for arg in args.arguments().iter() {
        if let Some(kw) = arg.as_keyword_hash_node() {
            for elem in kw.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == key {
                            return Some(assoc.key().location().start_offset());
                        }
                    }
                }
            }
        }
        if let Some(hash) = arg.as_hash_node() {
            for elem in hash.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == key {
                            return Some(assoc.key().location().start_offset());
                        }
                    }
                }
            }
        }
    }
    None
}

// ── String escape helpers ─────────────────────────────────────────────

/// Check if string content contains escape sequences that require double quotes.
///
/// Returns true if the content has `\X` where X is anything other than `\` or `"`.
/// Those two escapes (`\\`, `\"`) are representable in single-quoted strings, but
/// all others (`\n`, `\t`, `\#`, etc.) require double quotes.
///
/// Handles consecutive backslashes correctly: in `\\\\n`, there are 4 backslashes
/// (2 pairs of `\\`) followed by `n`, which does NOT require double quotes.
pub fn has_non_trivial_escape(content: &[u8]) -> bool {
    let mut i = 0;
    while i < content.len() {
        if content[i] == b'\\' {
            let run_start = i;
            while i < content.len() && content[i] == b'\\' {
                i += 1;
            }
            let run_len = i - run_start;
            let next = content.get(i).copied();
            // Odd number of backslashes means the last one escapes the next char.
            // Only \\ and \" are safe for single quotes; anything else needs double.
            if run_len % 2 == 1 && !matches!(next, Some(b'\\' | b'"')) {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// Check if string content requires double-quoted representation.
///
/// Returns true if the content contains a bare single quote (`'`) or escape
/// sequences that only work in double-quoted strings. This is the combined
/// check used by quoted_symbols, string_literals_in_interpolation, and
/// redundant_percent_q cops.
pub fn double_quotes_required(content: &[u8]) -> bool {
    content.contains(&b'\'') || has_non_trivial_escape(content)
}

// ── Conditional / modifier helpers ────────────────────────────────────

/// Check if an IfNode is a ternary operator (`x ? y : z`).
/// Ternaries have no `if` keyword in Prism — `if_keyword_loc` is None.
pub fn is_ternary(if_node: &ruby_prism::IfNode<'_>) -> bool {
    if_node.if_keyword_loc().is_none()
}

/// Check if an IfNode is a modifier form (`body if condition`).
/// Modifier conditionals have no `end` keyword and are not ternaries.
pub fn is_modifier_if(if_node: &ruby_prism::IfNode<'_>) -> bool {
    if_node.end_keyword_loc().is_none() && if_node.if_keyword_loc().is_some()
}

/// Check if an UnlessNode is a modifier form (`body unless condition`).
pub fn is_modifier_unless(unless_node: &ruby_prism::UnlessNode<'_>) -> bool {
    unless_node.end_keyword_loc().is_none()
}

// ── Shared node helpers ────────────────────────────────────────────────

/// Unwrap parentheses from a node, returning the inner expression.
/// Handles nested parentheses: `(expr)`, `((expr))`, etc.
/// Stops at empty parentheses or multi-statement bodies.
///
/// Note: Prism's ParenthesesNode body can be either a StatementsNode (for
/// `(expr)` in expression context) or a bare node (e.g., CallNode for
/// `def (receiver).method`). This function handles both shapes.
pub fn unwrap_parentheses<'a>(node: ruby_prism::Node<'a>) -> ruby_prism::Node<'a> {
    let mut current = node;
    while let Some(paren) = current.as_parentheses_node() {
        let Some(body) = paren.body() else {
            break;
        };
        if let Some(stmts) = body.as_statements_node() {
            let stmts_body = stmts.body();
            if stmts_body.len() == 1 {
                current = stmts_body.iter().next().unwrap();
                continue;
            }
            break;
        }
        // Body is a bare node (not wrapped in StatementsNode)
        current = body;
    }
    current
}

/// Check if a node is a single negation (`!expr`),
/// excluding double negation (`!!expr`) and safe-navigation `&.!`.
pub fn is_single_negation(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(call) = node.as_call_node() {
        if call.name().as_slice() == b"!" {
            // Skip safe-navigation `&.!` — rewriting is problematic
            if call.call_operator_loc().is_some() {
                return false;
            }
            // Check for double negation: `!!expr`
            if let Some(recv) = call.receiver() {
                if let Some(inner_call) = recv.as_call_node() {
                    if inner_call.name().as_slice() == b"!" {
                        return false;
                    }
                }
            }
            return true;
        }
    }
    false
}

/// Get the inner expression from a negation node (`!expr` → `expr`).
pub fn get_negation_inner<'a>(node: &ruby_prism::Node<'a>) -> Option<ruby_prism::Node<'a>> {
    if let Some(call) = node.as_call_node() {
        if call.name().as_slice() == b"!" {
            return call.receiver();
        }
    }
    None
}

/// Extract a 3-method chain from a node.
///
/// If `node` is a CallNode `x.c()` whose receiver is `y.b()` whose receiver is `z.a()`,
/// returns the three method names and call nodes.
pub struct MethodChain3<'a> {
    pub innermost_call: ruby_prism::CallNode<'a>,
    pub innermost_method: &'a [u8],
    pub middle_method: &'a [u8],
    pub outer_method: &'a [u8],
}

pub fn as_method_chain3<'a>(node: &ruby_prism::Node<'a>) -> Option<MethodChain3<'a>> {
    let outer_call = node.as_call_node()?;
    let outer_method = outer_call.name().as_slice();
    let mid_recv = outer_call.receiver()?;
    let mid_call = mid_recv.as_call_node()?;
    let middle_method = mid_call.name().as_slice();
    let inner_recv = mid_call.receiver()?;
    let innermost_call = inner_recv.as_call_node()?;
    let innermost_method = innermost_call.name().as_slice();
    Some(MethodChain3 {
        innermost_call,
        innermost_method,
        middle_method,
        outer_method,
    })
}

// ── RSpec-specific helpers ──────────────────────────────────────────────

/// RSpec example group methods.
pub const RSPEC_EXAMPLE_GROUPS: &[&str] = &[
    "describe",
    "context",
    "feature",
    "example_group",
    "xdescribe",
    "xcontext",
    "xfeature",
    "fdescribe",
    "fcontext",
    "ffeature",
    "shared_examples",
    "shared_examples_for",
    "shared_context",
];

/// RSpec focused (f-prefixed) methods.
pub const RSPEC_FOCUSED_METHODS: &[&str] = &[
    "fdescribe",
    "fcontext",
    "ffeature",
    "fit",
    "fspecify",
    "fexample",
    "fscenario",
    "focus",
];

/// RSpec example methods.
pub const RSPEC_EXAMPLES: &[&str] = &[
    "it",
    "specify",
    "example",
    "scenario",
    "its",
    "xit",
    "xspecify",
    "xexample",
    "xscenario",
    "fit",
    "fspecify",
    "fexample",
    "fscenario",
    "focus",
    "pending",
    "skip",
];

/// RSpec hook methods.
pub const RSPEC_HOOKS: &[&str] = &[
    "before",
    "after",
    "around",
    "prepend_before",
    "prepend_after",
    "append_before",
    "append_after",
];

/// RSpec let/subject methods.
pub const RSPEC_LETS: &[&str] = &["let", "let!"];
pub const RSPEC_SUBJECTS: &[&str] = &["subject", "subject!"];

/// All RSpec methods that define example groups or examples (for detecting RSpec context).
pub const RSPEC_ALL_METHODS: &[&str] = &[
    "describe",
    "context",
    "feature",
    "example_group",
    "xdescribe",
    "xcontext",
    "xfeature",
    "fdescribe",
    "fcontext",
    "ffeature",
    "shared_examples",
    "shared_examples_for",
    "shared_context",
    "it",
    "specify",
    "example",
    "scenario",
    "its",
    "xit",
    "xspecify",
    "xexample",
    "xscenario",
    "fit",
    "fspecify",
    "fexample",
    "fscenario",
    "pending",
    "skip",
    "focus",
    "before",
    "after",
    "around",
    "let",
    "let!",
    "subject",
    "subject!",
];

/// Shared group definition methods — NOT counted as nesting in NestedGroups.
pub const RSPEC_SHARED_GROUPS: &[&str] =
    &["shared_examples", "shared_examples_for", "shared_context"];

/// Check if a method name is an RSpec example group method.
pub fn is_rspec_example_group(name: &[u8]) -> bool {
    let s = std::str::from_utf8(name).unwrap_or("");
    RSPEC_EXAMPLE_GROUPS.contains(&s)
}

/// Check if a method name is an RSpec shared group definition
/// (shared_examples, shared_examples_for, shared_context).
/// These are top-level groups but do NOT count toward nesting depth.
pub fn is_rspec_shared_group(name: &[u8]) -> bool {
    let s = std::str::from_utf8(name).unwrap_or("");
    RSPEC_SHARED_GROUPS.contains(&s)
}

/// Check if a method name is an RSpec example method.
pub fn is_rspec_example(name: &[u8]) -> bool {
    let s = std::str::from_utf8(name).unwrap_or("");
    RSPEC_EXAMPLES.contains(&s)
}

/// Check if a method name is an RSpec hook method.
pub fn is_rspec_hook(name: &[u8]) -> bool {
    let s = std::str::from_utf8(name).unwrap_or("");
    RSPEC_HOOKS.contains(&s)
}

/// Check if a method name is a focused RSpec method (f-prefixed).
pub fn is_rspec_focused(name: &[u8]) -> bool {
    let s = std::str::from_utf8(name).unwrap_or("");
    RSPEC_FOCUSED_METHODS.contains(&s)
}

/// Check if a method name is an RSpec let or let!.
pub fn is_rspec_let(name: &[u8]) -> bool {
    name == b"let" || name == b"let!"
}

/// Check if a method name is an RSpec subject or subject!.
pub fn is_rspec_subject(name: &[u8]) -> bool {
    name == b"subject" || name == b"subject!"
}

/// Default include patterns for all RSpec cops — only run on spec files.
pub const RSPEC_DEFAULT_INCLUDE: &[&str] = &["**/*_spec.rb", "**/spec/**/*"];

/// Check if a CallNode has a keyword argument `focus: true` or symbol arg `:focus`.
pub fn has_rspec_focus_metadata(
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> Option<(usize, usize, usize, usize)> {
    let call = node.as_call_node()?;
    let args = call.arguments()?;
    for arg in args.arguments().iter() {
        // Check for `:focus` symbol argument
        if let Some(sym) = arg.as_symbol_node() {
            if sym.unescaped() == b"focus" {
                let loc = sym.location();
                let (line, col) = source.offset_to_line_col(loc.start_offset());
                let len = loc.end_offset() - loc.start_offset();
                return Some((line, col, loc.start_offset(), len));
            }
        }
        // Check for `focus: true` keyword argument
        if let Some(kw) = arg.as_keyword_hash_node() {
            for elem in kw.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == b"focus" {
                            let loc = elem.location();
                            let (line, col) = source.offset_to_line_col(loc.start_offset());
                            let len = loc.end_offset() - loc.start_offset();
                            return Some((line, col, loc.start_offset(), len));
                        }
                    }
                }
            }
        }
    }
    None
}

/// Get the first positional (non-keyword) argument from a call node.
pub fn first_positional_arg<'a>(call: &ruby_prism::CallNode<'a>) -> Option<ruby_prism::Node<'a>> {
    let args = call.arguments()?;
    for arg in args.arguments().iter() {
        // Skip keyword hash arguments
        if arg.as_keyword_hash_node().is_some() {
            continue;
        }
        return Some(arg);
    }
    None
}

/// Get the string content of a string node (returns owned Vec).
pub fn string_value(node: &ruby_prism::Node<'_>) -> Option<Vec<u8>> {
    if let Some(s) = node.as_string_node() {
        return Some(s.unescaped().to_vec());
    }
    None
}

/// Count block body lines (statements in a block node).
pub fn block_body_line_count(source: &SourceFile, block: &ruby_prism::BlockNode<'_>) -> usize {
    let loc = block.location();
    let (start_line, _) = source.offset_to_line_col(loc.start_offset());
    let (end_line, _) = source.offset_to_line_col(loc.end_offset().saturating_sub(1));
    if end_line <= start_line + 1 {
        return 0;
    }
    end_line - start_line - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_snake_case() {
        assert!(is_snake_case(b"foo_bar"));
        assert!(is_snake_case(b"foo"));
        assert!(is_snake_case(b"_foo"));
        assert!(is_snake_case(b"foo_bar_baz"));
        assert!(is_snake_case(b"foo123"));
        assert!(is_snake_case(b"valid?"));
        assert!(is_snake_case(b"save!"));
        assert!(!is_snake_case(b"FooBar"));
        assert!(!is_snake_case(b"fooBar"));
        assert!(!is_snake_case(b"FOO"));
    }

    #[test]
    fn test_is_screaming_snake_case() {
        assert!(is_screaming_snake_case(b"FOO_BAR"));
        assert!(is_screaming_snake_case(b"FOO"));
        assert!(is_screaming_snake_case(b"MAX_SIZE"));
        assert!(is_screaming_snake_case(b"V2"));
        assert!(!is_screaming_snake_case(b"foo_bar"));
        assert!(!is_screaming_snake_case(b"FooBar"));
        assert!(!is_screaming_snake_case(b"Foo"));
    }

    #[test]
    fn test_is_camel_case() {
        assert!(is_camel_case(b"FooBar"));
        assert!(is_camel_case(b"Foo"));
        assert!(is_camel_case(b"FooBarBaz"));
        assert!(is_camel_case(b"Foo123"));
        assert!(!is_camel_case(b"foo_bar"));
        assert!(!is_camel_case(b"FOO_BAR"));
        assert!(!is_camel_case(b"Foo_Bar"));
        assert!(!is_camel_case(b""));
        // Non-ASCII (UTF-8) characters should be allowed
        assert!(is_camel_case("Ålpha".as_bytes()));
        assert!(is_camel_case("Ëxample".as_bytes()));
    }

    #[test]
    fn test_is_ascii_name() {
        assert!(is_ascii_name(b"foo_bar"));
        assert!(is_ascii_name(b"FooBar"));
        assert!(!is_ascii_name("café".as_bytes()));
        assert!(!is_ascii_name("naïve".as_bytes()));
    }

    #[test]
    fn test_has_trailing_comma() {
        let src = b"[1, 2, 3,]";
        // '3' ends at byte 8, ']' at byte 9
        assert!(has_trailing_comma(src, 8, 9));
        let src2 = b"[1, 2, 3]";
        // '3' ends at byte 8, ']' at byte 8 — no room for comma
        assert!(!has_trailing_comma(src2, 8, 8));
    }

    #[test]
    fn test_count_body_lines() {
        let source = SourceFile::from_bytes(
            "test.rb",
            b"def foo\n  x = 1\n  y = 2\n  # comment\n\n  z = 3\nend\n".to_vec(),
        );
        // def starts at offset 0 (line 1), end starts at offset 45 (line 7)
        // Lines 2-6: "  x = 1", "  y = 2", "  # comment", "", "  z = 3"
        // Without comments: 3 lines (x, y, z)
        assert_eq!(count_body_lines(&source, 0, 45, false), 3);
        // With comments: 4 lines (x, y, #comment, z)
        assert_eq!(count_body_lines(&source, 0, 45, true), 4);
    }

    #[test]
    fn test_assignment_context_base_col() {
        // Simple assignment: `x = if ...`
        let src = SourceFile::from_bytes("test.rb", b"x = if foo\n  bar\nend\n".to_vec());
        assert_eq!(assignment_context_base_col(&src, 4), Some(0)); // `if` at offset 4

        // Indented: `  x = if ...`
        let src = SourceFile::from_bytes("test.rb", b"  x = if foo\n    bar\nend\n".to_vec());
        assert_eq!(assignment_context_base_col(&src, 6), Some(2)); // `if` at offset 6

        // No assignment: `if ...` at line start
        let src = SourceFile::from_bytes("test.rb", b"if foo\n  bar\nend\n".to_vec());
        assert_eq!(assignment_context_base_col(&src, 0), None);

        // Compound assignment: `x ||= if ...`
        let src = SourceFile::from_bytes("test.rb", b"x ||= if foo\n  bar\nend\n".to_vec());
        assert_eq!(assignment_context_base_col(&src, 6), Some(0));

        // Comparison (not assignment): `x == if ...`
        let src = SourceFile::from_bytes("test.rb", b"x == if foo\n  bar\nend\n".to_vec());
        assert_eq!(assignment_context_base_col(&src, 5), None);

        // Not assignment: `x =~ /pattern/`
        let src = SourceFile::from_bytes("test.rb", b"x =~ if foo\n  bar\nend\n".to_vec());
        assert_eq!(assignment_context_base_col(&src, 5), None);

        // Hash rocket (not assignment): `x => if ...`
        let src = SourceFile::from_bytes("test.rb", b"x => if foo\n  bar\nend\n".to_vec());
        assert_eq!(assignment_context_base_col(&src, 5), None);
    }

    // ── string escape helper tests ─────────────────────────────────────

    #[test]
    fn has_non_trivial_escape_basic() {
        assert!(has_non_trivial_escape(b"hello\\nworld"));
        assert!(has_non_trivial_escape(b"\\t"));
        assert!(!has_non_trivial_escape(b"hello"));
        assert!(!has_non_trivial_escape(b"hello\\\\world")); // \\\\ = two literal backslashes
        assert!(!has_non_trivial_escape(b"hello\\\"world")); // \\\" = escaped quote
    }

    #[test]
    fn has_non_trivial_escape_consecutive_backslashes() {
        // 3 backslashes + n: odd count means last one escapes n → needs double quotes
        assert!(has_non_trivial_escape(b"\\\\\\n"));
        // 4 backslashes + n: even count means all are paired, n is literal
        assert!(!has_non_trivial_escape(b"\\\\\\\\n"));
    }

    #[test]
    fn double_quotes_required_bare_quote() {
        assert!(double_quotes_required(b"it's"));
        assert!(!double_quotes_required(b"hello"));
    }

    #[test]
    fn double_quotes_required_escapes() {
        assert!(double_quotes_required(b"hello\\nworld"));
        assert!(!double_quotes_required(b"hello\\\\world"));
    }

    // ── unwrap_parentheses tests ───────────────────────────────────────

    /// Parse Ruby source and return the first expression node.
    fn parse_first_expr(source: &str) -> (ruby_prism::ParseResult<'static>, ()) {
        let bytes = source.as_bytes().to_vec();
        let leaked = Box::leak(bytes.into_boxed_slice());
        (ruby_prism::parse(leaked), ())
    }

    fn first_node<'a>(result: &'a ruby_prism::ParseResult<'a>) -> ruby_prism::Node<'a> {
        result
            .node()
            .as_program_node()
            .unwrap()
            .statements()
            .body()
            .iter()
            .next()
            .unwrap()
    }

    #[test]
    fn unwrap_parentheses_simple_expression() {
        let (result, _) = parse_first_expr("(42)");
        let unwrapped = unwrap_parentheses(first_node(&result));
        assert!(unwrapped.as_integer_node().is_some());
    }

    #[test]
    fn unwrap_parentheses_nested() {
        let (result, _) = parse_first_expr("((42))");
        let unwrapped = unwrap_parentheses(first_node(&result));
        assert!(unwrapped.as_integer_node().is_some());
    }

    #[test]
    fn unwrap_parentheses_multi_statement_stops() {
        let (result, _) = parse_first_expr("(1; 2)");
        let unwrapped = unwrap_parentheses(first_node(&result));
        assert!(unwrapped.as_parentheses_node().is_some());
    }

    #[test]
    fn unwrap_parentheses_no_parens() {
        let (result, _) = parse_first_expr("42");
        let unwrapped = unwrap_parentheses(first_node(&result));
        assert!(unwrapped.as_integer_node().is_some());
    }

    #[test]
    fn unwrap_parentheses_empty_parens() {
        let (result, _) = parse_first_expr("()");
        let unwrapped = unwrap_parentheses(first_node(&result));
        assert!(unwrapped.as_parentheses_node().is_some());
    }

    // ── is_single_negation tests ───────────────────────────────────────

    #[test]
    fn is_single_negation_simple() {
        let (result, _) = parse_first_expr("!foo");
        assert!(is_single_negation(&first_node(&result)));
    }

    #[test]
    fn is_single_negation_double_negation() {
        let (result, _) = parse_first_expr("!!foo");
        assert!(!is_single_negation(&first_node(&result)));
    }

    #[test]
    fn is_single_negation_safe_nav() {
        let (result, _) = parse_first_expr("foo&.!");
        assert!(!is_single_negation(&first_node(&result)));
    }

    #[test]
    fn is_single_negation_non_negation() {
        let (result, _) = parse_first_expr("foo");
        assert!(!is_single_negation(&first_node(&result)));
    }

    #[test]
    fn is_single_negation_method_call() {
        let (result, _) = parse_first_expr("foo.bar");
        assert!(!is_single_negation(&first_node(&result)));
    }

    // ── get_negation_inner tests ───────────────────────────────────────

    #[test]
    fn get_negation_inner_returns_receiver() {
        // Use `foo = 1; !foo` so Prism parses `foo` as a local variable
        let (result, _) = parse_first_expr("foo = 1; !foo");
        let stmts = result.node().as_program_node().unwrap().statements().body();
        // The second statement is `!foo`
        let negation = stmts.iter().nth(1).unwrap();
        let inner = get_negation_inner(&negation);
        assert!(inner.is_some());
        assert!(inner.unwrap().as_local_variable_read_node().is_some());
    }

    #[test]
    fn get_negation_inner_non_negation() {
        let (result, _) = parse_first_expr("foo");
        assert!(get_negation_inner(&first_node(&result)).is_none());
    }

    #[test]
    fn get_negation_inner_double_negation() {
        // `!!foo` → inner is `!foo` (a CallNode, not a local var)
        let (result, _) = parse_first_expr("!!foo");
        let inner = get_negation_inner(&first_node(&result));
        assert!(inner.is_some());
        assert!(inner.unwrap().as_call_node().is_some());
    }
}
