/// Layout/SpaceInsideArrayLiteralBrackets
///
/// Investigation notes (2026-03-24, FP=0 FN=58):
/// - The 58 FNs were caused by two issues:
///   1. Empty bracket detection only handled exactly 0 or 1 space between brackets.
///      RuboCop treats any bracket pair with only whitespace/newlines between them
///      as "empty", including `[   ]` and `[\n]`. Fixed by scanning for non-whitespace
///      between brackets.
///   2. Autocorrect for `no_space` style only removed a single space character.
///      When multiple spaces exist (e.g., `[  1, 2, 3   ]`), all contiguous spaces
///      adjacent to the bracket must be removed. Fixed by scanning for the full
///      whitespace run.
///   3. `ARRAY_PATTERN_NODE` (pattern matching `in [a, b]`) was not handled.
///      RuboCop aliases `on_array_pattern` to `on_array`. Added support.
/// - FP=6 fix (2026-04-02): RuboCop skips this cop entirely for array patterns
///   that end with a trailing comma (`in [a, ]`, `in Foo[ a, ]`), and it also
///   accepts multiline arrays with `[ <spaces>\n  # comment` after the opening
///   bracket. Fixed by detecting the trailing-comma array-pattern context and by
///   treating comment-on-next-line as an allowed multiline opening-bracket case.
/// - compact fix (2026-04-16): RuboCop collapses adjacent bracket tokens even
///   when they are separated by newlines inside the same array node. That means
///   multiline `[\n  [ ... ]\n]` needs outer `[[`/`]]`, and a closing `]` from
///   an index/reference expression like `html_options[:class]` also collapses
///   against the outer closing bracket. Fixed by letting compact adjacency scans
///   skip newline whitespace while still excluding `%w/%i/%W/%I` delimiters.
/// - compact follow-up (2026-04-16): RuboCop never reaches this cop in
///   non-UTF-8 regex-escape files such as jruby's `test_windows_1252.rb`; it
///   emits only `Lint/Syntax` there. nitrocop still parsed the trailing array
///   literals and reported compact-style offenses. Fixed by skipping files with
///   a non-UTF-8 encoding comment and regex literals containing invalid high-bit
///   `\xHH` escape runs.
/// - compact follow-up (2026-04-17): the raw-byte compact `]]` scan treated
///   comment text and `%[...]` string delimiters as real array brackets, causing
///   FPs on commented-out array lines and FNs/FPs around multiline percent
///   literals. RuboCop only considers code tokens, so compact adjacency checks
///   now consult `CodeMap` and ignore non-code brackets. Also, Prism-style named
///   constant patterns like `Prism::ArgumentsNode[arguments: [ ... ]]` are
///   bracket-owned by the enclosing `HashPatternNode`, not by the inner
///   `arguments: [ ... ]` array pattern. Match RuboCop's
///   `find_node_with_brackets` by remapping array/array-pattern checks to the
///   nearest enclosing bracketed hash pattern.
use crate::cop::shared::node_type::{ARRAY_NODE, ARRAY_PATTERN_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::{codemap::CodeMap, source::SourceFile};
use ruby_prism::Visit;

pub struct SpaceInsideArrayLiteralBrackets;

impl Cop for SpaceInsideArrayLiteralBrackets {
    fn name(&self) -> &'static str {
        "Layout/SpaceInsideArrayLiteralBrackets"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[ARRAY_NODE, ARRAY_PATTERN_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        if rubocop_skips_non_utf8_regex_escape_file(source) {
            return;
        }

        let mut visitor = SpaceInsideArrayLiteralBracketsVisitor {
            cop: self,
            source,
            code_map,
            config,
            diagnostics,
            corrections,
            hash_pattern_stack: Vec::new(),
            pushed_hash_pattern_stack: Vec::new(),
        };
        visitor.visit(&parse_result.node());
    }
}

impl SpaceInsideArrayLiteralBrackets {
    #[allow(clippy::too_many_arguments)]
    fn check_brackets(
        &self,
        source: &SourceFile,
        open_start: usize,
        open_end: usize,
        close_start: usize,
        is_array_pattern: bool,
        code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let bytes = source.as_bytes();

        if is_array_pattern && prev_non_whitespace(bytes, close_start) == Some(b',') {
            return;
        }

        let empty_style = config.get_str("EnforcedStyleForEmptyBrackets", "no_space");

        // Check if the array is empty: only whitespace/newlines between brackets
        let is_empty = is_only_whitespace(bytes, open_end, close_start);

        if is_empty {
            if close_start == open_end {
                // Truly empty: []
                if empty_style == "space" {
                    let (line, column) = source.offset_to_line_col(open_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside empty array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            } else {
                // Has whitespace between brackets: [ ], [   ], [\n]
                let is_single_space =
                    close_start == open_end + 1 && bytes.get(open_end) == Some(&b' ');
                match empty_style {
                    "no_space" => {
                        let (line, column) = source.offset_to_line_col(open_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside empty array literal brackets detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: close_start,
                                replacement: String::new(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                    "space" if !is_single_space => {
                        // Multiple spaces or newline: correct to single space
                        let (line, column) = source.offset_to_line_col(open_start);
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            "Space inside empty array literal brackets missing.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            corr.push(crate::correction::Correction {
                                start: open_end,
                                end: close_start,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                    _ => {}
                }
            }
            return;
        }

        let enforced = config.get_str("EnforcedStyle", "no_space");

        // For multiline arrays, determine which bracket sides to skip.
        let (open_line, _) = source.offset_to_line_col(open_start);
        let (close_line, _) = source.offset_to_line_col(close_start);
        let is_multiline = open_line != close_line;

        let start_ok = if is_multiline {
            match enforced {
                "no_space" => {
                    next_to_comment(bytes, open_end)
                        || next_line_starts_with_comment(bytes, open_end)
                }
                _ => next_to_newline(bytes, open_end),
            }
        } else {
            false
        };

        let end_ok = if is_multiline {
            begins_its_line_raw(bytes, close_start)
        } else {
            false
        };

        let space_after_open = matches!(bytes.get(open_end), Some(b' ' | b'\t'));
        let space_before_close =
            close_start > 0 && matches!(bytes.get(close_start - 1), Some(b' ' | b'\t'));

        match enforced {
            "no_space" => {
                if !start_ok && space_after_open {
                    let space_end = scan_space_forward(bytes, open_end);
                    let (line, column) = source.offset_to_line_col(open_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: space_end,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                if !end_ok && space_before_close {
                    let space_start = scan_space_backward(bytes, close_start);
                    let (line, column) = source.offset_to_line_col(close_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: space_start,
                            end: close_start,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            "space" => {
                if !start_ok && !space_after_open {
                    let (line, column) = source.offset_to_line_col(open_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
                if !end_ok && !space_before_close {
                    let (line, column) = source.offset_to_line_col(close_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: close_start,
                            end: close_start,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            "compact" => {
                let multi_dim_left = is_adjacent_bracket_forward(bytes, code_map, open_end);
                let multi_dim_right = is_adjacent_bracket_backward(bytes, code_map, close_start);

                // Left side: whitespace check includes newlines for compact collapse
                let ws_after_open =
                    matches!(bytes.get(open_end), Some(b' ' | b'\t' | b'\n' | b'\r'));

                if multi_dim_left && ws_after_open {
                    // Space (or newline) before nested [[ should be collapsed
                    let ws_end = scan_all_whitespace_forward(bytes, open_end);
                    let (line, column) = source.offset_to_line_col(open_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: ws_end,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                } else if !multi_dim_left && !start_ok && !space_after_open {
                    // Non-nested: require space (like space style)
                    let (line, column) = source.offset_to_line_col(open_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: open_end,
                            end: open_end,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }

                // Right side: whitespace check includes newlines for compact collapse
                let ws_before_close = close_start > 0
                    && matches!(
                        bytes.get(close_start - 1),
                        Some(b' ' | b'\t' | b'\n' | b'\r')
                    );

                if multi_dim_right && ws_before_close {
                    // Space (or newline) after nested ]] should be collapsed
                    let ws_start = scan_all_whitespace_backward(bytes, close_start);
                    let (line, column) = source.offset_to_line_col(close_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets detected.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: ws_start,
                            end: close_start,
                            replacement: String::new(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                } else if !multi_dim_right && !end_ok && !space_before_close {
                    // Non-nested: require space (like space style)
                    let (line, column) = source.offset_to_line_col(close_start);
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        "Space inside array literal brackets missing.".to_string(),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: close_start,
                            end: close_start,
                            replacement: " ".to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy)]
struct BracketSpan {
    open_start: usize,
    open_end: usize,
    close_start: usize,
    is_array_pattern: bool,
}

impl BracketSpan {
    fn from_locations(
        opening: &ruby_prism::Location<'_>,
        closing: &ruby_prism::Location<'_>,
        is_array_pattern: bool,
    ) -> Self {
        Self {
            open_start: opening.start_offset(),
            open_end: opening.end_offset(),
            close_start: closing.start_offset(),
            is_array_pattern,
        }
    }
}

struct SpaceInsideArrayLiteralBracketsVisitor<'a> {
    cop: &'a SpaceInsideArrayLiteralBrackets,
    source: &'a SourceFile,
    code_map: &'a CodeMap,
    config: &'a CopConfig,
    diagnostics: &'a mut Vec<Diagnostic>,
    corrections: Option<&'a mut Vec<crate::correction::Correction>>,
    hash_pattern_stack: Vec<BracketSpan>,
    pushed_hash_pattern_stack: Vec<bool>,
}

impl SpaceInsideArrayLiteralBracketsVisitor<'_> {
    fn current_owner_or(&self, own: BracketSpan) -> BracketSpan {
        self.hash_pattern_stack.last().copied().unwrap_or(own)
    }

    fn check_span(&mut self, span: BracketSpan) {
        let corrections = self.corrections.as_mut().map(|corr| &mut **corr);
        self.cop.check_brackets(
            self.source,
            span.open_start,
            span.open_end,
            span.close_start,
            span.is_array_pattern,
            self.code_map,
            self.config,
            self.diagnostics,
            corrections,
        );
    }
}

impl<'pr> Visit<'pr> for SpaceInsideArrayLiteralBracketsVisitor<'_> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        if let Some(pattern) = node.as_hash_pattern_node() {
            if let (Some(opening), Some(closing)) = (pattern.opening_loc(), pattern.closing_loc()) {
                if opening.as_slice() == b"[" && closing.as_slice() == b"]" {
                    self.hash_pattern_stack
                        .push(BracketSpan::from_locations(&opening, &closing, false));
                    self.pushed_hash_pattern_stack.push(true);
                    return;
                }
            }
        }

        self.pushed_hash_pattern_stack.push(false);
    }

    fn visit_branch_node_leave(&mut self) {
        if self.pushed_hash_pattern_stack.pop() == Some(true) {
            self.hash_pattern_stack.pop();
        }
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        if let (Some(opening), Some(closing)) = (node.opening_loc(), node.closing_loc()) {
            if opening.as_slice() == b"[" && closing.as_slice() == b"]" {
                let owner =
                    self.current_owner_or(BracketSpan::from_locations(&opening, &closing, false));
                self.check_span(owner);
            }
        }

        ruby_prism::visit_array_node(self, node);
    }

    fn visit_array_pattern_node(&mut self, node: &ruby_prism::ArrayPatternNode<'pr>) {
        if let (Some(opening), Some(closing)) = (node.opening_loc(), node.closing_loc()) {
            if opening.as_slice() == b"[" && closing.as_slice() == b"]" {
                let owner =
                    self.current_owner_or(BracketSpan::from_locations(&opening, &closing, true));
                self.check_span(owner);
            }
        }

        ruby_prism::visit_array_pattern_node(self, node);
    }
}

/// Check if bytes between `start` and `end` contain only whitespace (spaces, tabs, newlines).
fn is_only_whitespace(bytes: &[u8], start: usize, end: usize) -> bool {
    bytes[start..end]
        .iter()
        .all(|&b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r')
}

/// Find the previous non-whitespace byte before `pos`.
fn prev_non_whitespace(bytes: &[u8], pos: usize) -> Option<u8> {
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            byte => return Some(byte),
        }
    }
    None
}

/// Scan forward from `pos` past contiguous spaces/tabs. Returns the offset after the run.
fn scan_space_forward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }
    i
}

/// Scan backward from `pos` past contiguous spaces/tabs. Returns the offset at the start of the run.
fn scan_space_backward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    i
}

/// Check if the next non-whitespace character after `pos` is on a different line.
fn next_to_newline(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'\n' | b'\r' => return true,
            _ => return false,
        }
    }
    true // end of file
}

/// Check if the next non-whitespace character after `pos` is a `#` comment.
fn next_to_comment(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b'#' => return true,
            _ => return false,
        }
    }
    false
}

/// Check if optional spaces/tabs are followed by a newline whose next line starts with `#`.
fn next_line_starts_with_comment(bytes: &[u8], pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }

    if i >= bytes.len() {
        return false;
    }

    match bytes[i] {
        b'\n' => i += 1,
        b'\r' => {
            i += 1;
            if i < bytes.len() && bytes[i] == b'\n' {
                i += 1;
            }
        }
        _ => return false,
    }

    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
        i += 1;
    }

    bytes.get(i) == Some(&b'#')
}

/// Check if the next non-whitespace character after `pos` is `[`.
/// Compact style skips newline tokens between adjacent brackets.
fn is_adjacent_bracket_forward(bytes: &[u8], code_map: &CodeMap, pos: usize) -> bool {
    let mut i = pos;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'[' if code_map.is_code(i) => return true,
            _ => return false,
        }
    }
    false
}

/// Check if the previous non-whitespace character before `pos` is `]`
/// from a real bracket array/pattern in code, not from comment or string text.
/// Compact style skips newline tokens between adjacent brackets.
fn is_adjacent_bracket_backward(bytes: &[u8], code_map: &CodeMap, pos: usize) -> bool {
    let mut i = pos;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => continue,
            b']' => {
                if !code_map.is_code(i) {
                    return false;
                }

                // Found a code `]` — find its matching code `[` via bracket counting.
                let mut depth: usize = 1;
                let mut j = i;
                while j > 0 && depth > 0 {
                    j -= 1;
                    if !code_map.is_code(j) {
                        continue;
                    }
                    match bytes[j] {
                        b']' => depth += 1,
                        b'[' => depth -= 1,
                        _ => {}
                    }
                }
                if depth == 0
                    && j >= 2
                    && bytes[j - 2] == b'%'
                    && matches!(bytes[j - 1], b'w' | b'i' | b'W' | b'I')
                {
                    return false;
                }
                return depth == 0;
            }
            _ => return false,
        }
    }
    false
}

/// Scan forward from `pos` past all whitespace including newlines.
fn scan_all_whitespace_forward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// Scan backward from `pos` past all whitespace including newlines.
fn scan_all_whitespace_backward(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r') {
        i -= 1;
    }
    i
}

/// Check if the position is the first non-whitespace on its line (raw byte scan).
fn begins_its_line_raw(bytes: &[u8], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let mut i = pos - 1;
    loop {
        match bytes[i] {
            b' ' | b'\t' => {
                if i == 0 {
                    return true;
                }
                i -= 1;
            }
            b'\n' => return true,
            _ => return false,
        }
    }
}

fn rubocop_skips_non_utf8_regex_escape_file(source: &SourceFile) -> bool {
    let lines: Vec<&[u8]> = source.lines().collect();
    has_non_utf8_encoding_comment(&lines)
        && lines
            .iter()
            .copied()
            .any(line_contains_high_hex_escape_in_regex_literal)
}

fn has_non_utf8_encoding_comment(lines: &[&[u8]]) -> bool {
    let mut idx = 0;

    if idx < lines.len() && starts_with_shebang(lines[idx]) {
        idx += 1;
    }

    while idx < lines.len() && is_blank_line(lines[idx]) {
        idx += 1;
    }

    let Some(line) = lines.get(idx) else {
        return false;
    };
    if !is_encoding_comment(line) {
        return false;
    }

    let lower = normalized_ascii_string(line).to_ascii_lowercase();
    let Some(keyword_idx) = lower.find("encoding").or_else(|| lower.find("coding")) else {
        return false;
    };
    let after_keyword = &lower[keyword_idx..];
    let value = after_keyword
        .split_once([':', '='])
        .map(|(_, rhs)| rhs.trim_start())
        .unwrap_or("");
    let enc_name: String = value
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .collect();

    if enc_name.is_empty() {
        return false;
    }

    !(enc_name == "utf"
        || enc_name == "utf8"
        || enc_name.starts_with("utf-8")
        || enc_name.starts_with("utf_8")
        || enc_name == "binary"
        || enc_name.starts_with("ascii-8bit")
        || enc_name.starts_with("ascii_8bit")
        || enc_name == "us-ascii"
        || enc_name == "ascii")
}

fn starts_with_shebang(line: &[u8]) -> bool {
    normalized_ascii_bytes(line).starts_with(b"#!")
}

fn is_blank_line(line: &[u8]) -> bool {
    first_non_padding_byte(line).is_none()
}

fn first_non_padding_byte(line: &[u8]) -> Option<u8> {
    line.iter()
        .copied()
        .filter(|&b| b != 0x00)
        .find(|&b| b != b' ' && b != b'\t' && b != b'\r')
}

fn is_encoding_comment(line: &[u8]) -> bool {
    let s = normalized_ascii_string(line);
    let trimmed = s.trim_start_matches([' ', '\t']);
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("# encoding:") || lower.starts_with("# coding:") {
        return true;
    }
    if lower.starts_with("# -*-") {
        return lower.contains("encoding") || lower.contains("coding");
    }
    false
}

fn normalized_ascii_string(line: &[u8]) -> String {
    String::from_utf8(normalized_ascii_bytes(line)).expect("ASCII normalization must stay ASCII")
}

fn normalized_ascii_bytes(line: &[u8]) -> Vec<u8> {
    line.iter()
        .copied()
        .filter(|&b| b != 0x00 && b.is_ascii())
        .collect()
}

fn line_contains_high_hex_escape_in_regex_literal(line: &[u8]) -> bool {
    let mut start = 0;

    while start < line.len() {
        let Some(open_idx) = line[start..].iter().position(|&byte| byte == b'/') else {
            return false;
        };
        let slash_idx = start + open_idx;
        if !looks_like_regex_open(line, slash_idx) {
            start = slash_idx + 1;
            continue;
        }

        let body_start = slash_idx + 1;
        let mut idx = body_start;
        let mut escaped = false;

        while idx < line.len() {
            let byte = line[idx];

            if escaped {
                escaped = false;
                idx += 1;
                continue;
            }

            if byte == b'\\' {
                escaped = true;
                idx += 1;
                continue;
            }

            if byte == b'/' {
                if contains_non_utf8_hex_escape(&line[body_start..idx]) {
                    return true;
                }
                start = idx + 1;
                break;
            }

            idx += 1;
        }

        if idx >= line.len() {
            return false;
        }
    }

    false
}

fn looks_like_regex_open(line: &[u8], slash_idx: usize) -> bool {
    let prev = line[..slash_idx]
        .iter()
        .rfind(|&&byte| !byte.is_ascii_whitespace())
        .copied();

    !matches!(
        prev,
        Some(
            b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'_'
            | b')'
            | b']'
            | b'}'
            | b'"'
            | b'\''
            | b'/',
        )
    )
}

fn contains_non_utf8_hex_escape(body: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 < body.len() {
        if body[i] == b'\\' && body[i + 1] == b'x' {
            let (d1, d2) = (body[i + 2], body[i + 3]);
            if d1.is_ascii_hexdigit() && d2.is_ascii_hexdigit() {
                let byte = hex_pair_to_byte(d1, d2);
                if byte >= 0x80 {
                    let mut bytes = vec![byte];
                    let mut j = i + 4;
                    while j + 3 < body.len()
                        && body[j] == b'\\'
                        && body[j + 1] == b'x'
                        && body[j + 2].is_ascii_hexdigit()
                        && body[j + 3].is_ascii_hexdigit()
                    {
                        let next = hex_pair_to_byte(body[j + 2], body[j + 3]);
                        if next >= 0x80 {
                            bytes.push(next);
                            j += 4;
                        } else {
                            break;
                        }
                    }
                    if std::str::from_utf8(&bytes).is_err() {
                        return true;
                    }
                    i = j;
                    continue;
                }
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    false
}

fn hex_pair_to_byte(h1: u8, h2: u8) -> u8 {
    hex_digit_val(h1) * 16 + hex_digit_val(h2)
}

fn hex_digit_val(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(
        SpaceInsideArrayLiteralBrackets,
        "cops/layout/space_inside_array_literal_brackets"
    );
    crate::cop_autocorrect_fixture_tests!(
        SpaceInsideArrayLiteralBrackets,
        "cops/layout/space_inside_array_literal_brackets"
    );

    #[test]
    fn empty_brackets_space_style_flags_no_space() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyleForEmptyBrackets".into(),
                serde_yml::Value::String("space".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"x = []\n";
        let diags = run_cop_full_with_config(&SpaceInsideArrayLiteralBrackets, src, config);
        assert_eq!(
            diags.len(),
            1,
            "space style should flag empty [] without space"
        );
    }

    #[test]
    fn empty_brackets_no_space_is_default() {
        use crate::testutil::run_cop_full;

        let src = b"x = []\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert!(diags.is_empty(), "Default no_space should accept []");
    }

    #[test]
    fn array_pattern_no_space_flags_spaces() {
        use crate::testutil::run_cop_full;

        let src = b"case foo\nin [ bar, baz ]\nend\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert_eq!(
            diags.len(),
            2,
            "array pattern with spaces should be flagged"
        );
    }

    #[test]
    fn array_pattern_no_space_accepts_no_spaces() {
        use crate::testutil::run_cop_full;

        let src = b"case foo\nin [bar, baz]\nend\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert!(
            diags.is_empty(),
            "array pattern without spaces should not be flagged"
        );
    }

    #[test]
    fn empty_brackets_multiple_spaces_no_space_style() {
        use crate::testutil::run_cop_full;

        let src = b"x = [     ]\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert_eq!(
            diags.len(),
            1,
            "empty brackets with multiple spaces should be flagged"
        );
        assert!(diags[0].message.contains("empty"));
    }

    #[test]
    fn multiline_empty_brackets_no_space_style() {
        use crate::testutil::run_cop_full;

        let src = b"x = [\n]\n";
        let diags = run_cop_full(&SpaceInsideArrayLiteralBrackets, src);
        assert_eq!(diags.len(), 1, "multiline empty brackets should be flagged");
        assert!(diags[0].message.contains("empty"));
    }

    fn compact_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("compact".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn compact_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &SpaceInsideArrayLiteralBrackets,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_array_literal_brackets/compact_offense.rb"
            ),
            compact_config(),
        );
    }

    #[test]
    fn compact_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &SpaceInsideArrayLiteralBrackets,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_array_literal_brackets/compact_no_offense.rb"
            ),
            compact_config(),
        );
    }

    #[test]
    fn compact_corpus_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &SpaceInsideArrayLiteralBrackets,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/space_inside_array_literal_brackets/offense.compact.rb"
            ),
            compact_config(),
        );
    }

    #[test]
    fn compact_collapses_adjacent_brackets_across_newlines() {
        use crate::testutil::run_cop_full_with_config;

        let src = b"multiline = [\n  [ 1, 2, 3, 4 ],\n  [ 3, 4, 5, 6 ]]\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideArrayLiteralBrackets, src, compact_config());
        assert!(
            diags.iter().any(|d| d.message.contains("detected")),
            "multiline [ \\n [ should collapse under compact style"
        );

        let src = b"css_classes = [\n  html_options[:class]\n]\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideArrayLiteralBrackets, src, compact_config());
        assert!(
            diags.iter().any(|d| d.message.contains("detected")),
            "multiline ] after a reference bracket should collapse under compact style"
        );
    }

    #[test]
    fn compact_skips_non_utf8_regex_escape_files() {
        use crate::testutil::run_cop_full_with_config;

        let src = b"# encoding:windows-1252\nassert_match(/^[\\xdfz]+$/i, \"sszzsszz\")\na = [0x8a, 0x8c]\n";
        let diags =
            run_cop_full_with_config(&SpaceInsideArrayLiteralBrackets, src, compact_config());
        assert!(
            diags.is_empty(),
            "RuboCop only emits Lint/Syntax for non-UTF-8 regex escape files"
        );
    }
}
