use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;
use std::collections::HashSet;
use std::ops::Range;

/// Layout/ExtraSpacing: flags unnecessary whitespace between tokens.
///
/// ## Investigation findings (2026-03-15)
///
/// Root causes of FNs (671 in corpus baseline):
/// 1. **Mode 2 alignment bug (fixed)**: `check_alignment` had a "same character"
///    mode that checked if the same byte appeared at the same column on an adjacent
///    line, without requiring it to be preceded by whitespace. This allowed
///    coincidental character alignment (e.g., `d` in `do` aligning with `d` at the
///    end of `_______________________d`) to suppress offense reports. RuboCop's
///    `aligned_words?` requires either `\s\S` (space-then-nonspace) at the column
///    or an exact full-token match. Removed Mode 2 to match RuboCop behavior.
///
/// 2. **Overly broad equals alignment (tightened)**: `check_equals_alignment`
///    matched any `=` character on the adjacent line without verifying it was
///    actually part of an assignment operator. Added a check requiring the `=` to
///    be preceded by whitespace or an operator character (like `+`, `|`, etc.),
///    preventing false alignment with `=` embedded in other contexts.
///
/// Root causes of FPs (526 in corpus baseline):
/// 3. **Missing exact token match in alignment check (fixed 2026-03-18)**:
///    RuboCop's `aligned_words?` has two modes: (1) `\s\S` at col-1 (token boundary)
///    and (2) exact token text match at the same column. Our `check_alignment` only
///    had mode 1. This caused FPs where intentional alignment used tokens not preceded
///    by space (e.g., `.divmod` aligning with `.divmod` where `.` follows `)` directly).
///    Added `extract_token_at` to extract a word/operator at the column and compare.
///
/// Root causes of FNs (503 in corpus baseline):
/// 4. **Alignment check leaking into comment text (fixed 2026-03-18)**: When the
///    extra space was before a trailing `#` comment, `is_aligned_with_adjacent` checked
///    alignment at the column of `#` on adjacent lines. If the adjacent line also had
///    a comment starting at a different column, the `\s\S` pattern inside comment text
///    (e.g., `# c` in `# comment`) would falsely match as a token boundary, suppressing
///    the offense. RuboCop only checks `@aligned_comments` for comment tokens, never
///    `aligned_with_something?`. Fixed by separating comment and non-comment alignment
///    paths: comments only use `aligned_comment_lines`, non-comments use the full
///    `is_aligned_with_adjacent` check.
///
/// ## Investigation findings (2026-03-19)
///
/// 5. **%w()/%i() word/symbol array FPs (fixed)**: Extra spaces inside `%w()`, `%W()`,
///    `%i()`, `%I()` arrays are element separators, not extra spacing. RuboCop's
///    token-based approach doesn't see these as code gaps because the entire array
///    content is tokenized differently. Added collection of word/symbol array interior
///    ranges (similar to hash pair ranges) and skip them during scanning.
///    Fixes ~20 FPs from rouge-ruby and similar repos.
///
/// 6. **Multibyte alignment FPs (fixed)**: Lines with multibyte characters (CJK, etc.)
///    have different byte offsets for the same visual column. The alignment check used
///    byte positions, so tokens visually aligned at the same column but at different
///    byte offsets were not recognized as aligned. Changed alignment detection to use
///    character-count-based columns (counting each byte-level position through chars)
///    so that multibyte characters are properly accounted for.
///    Fixes ~24 FPs from shopqi and similar repos with CJK text.
///
/// 7. **Tab-based spacing FNs (fixed)**: Tabs between tokens (not as indentation) were
///    completely ignored because the scanner only looked for space characters. RuboCop's
///    token-based approach counts any gap > 1 character between tokens as extra spacing,
///    regardless of whether it's spaces or tabs. Extended the scanner to detect whitespace
///    runs containing tabs (after skipping indentation) and flag them.
///    Fixes ~30 FNs from coderwall, fog, jruby and similar repos.
///
/// ## Key design notes
/// - Works with raw text scanning (not tokens), using CodeMap to skip non-code regions
/// - Alignment detection mirrors RuboCop's PrecedingFollowingAlignment mixin:
///   Pass 1 checks nearest non-blank non-comment line, Pass 2 checks nearest
///   line with same indentation
/// - Hash pair ranges in multiline hashes are ignored (handled by Layout/HashAlignment)
/// - Word/symbol array ranges (%w/%i/%W/%I) are ignored (spacing is element separation)
/// - ForceEqualSignAlignment is read from config but not yet implemented (produces
///   a different offense message)
///
/// ## Investigation findings (2026-03-23)
///
/// 8. **Single-tab FPs (fixed)**: The scanner flagged any whitespace run containing
///    a tab character, even single tabs. RuboCop's token-based approach counts the
///    number of characters in the gap between tokens, not visual column width. A
///    single tab is 1 character of whitespace and is NOT extra spacing. Changed
///    the condition from `space_count > 1 || has_tab` to just `space_count > 1`.
///    Fixes ~80 FPs from repos using tabs for alignment (louismullie__treat,
///    pluosi__app-host, github-linguist__linguist, zammad, etc.).
///
/// 9. **Empty word/symbol array FNs (fixed)**: The `%w(  )` and `%i(  )` interior
///    ranges were unconditionally ignored, even for empty arrays where the spaces
///    are NOT element separators. Added a check that only ignores non-empty arrays.
///    Fixes ~8 FNs from browsermedia__browsercms and ruby-formatter__rufo.
///
/// 10. **Quote-character alignment FNs (fixed)**: `extract_token_at` returned only
///     a single `"` or `'` character for string delimiters, causing coincidental
///     Mode 2 alignment matches. RuboCop's `range.source` returns the full string
///     token. Extended `extract_token_at` to extract the full quoted string, and
///     also improved `.method_name` extraction for dot-method calls.
///
/// ## Investigation findings (2026-03-24)
///
/// 11. **Missing `=`/`<<` cross-alignment (fixed)**: RuboCop's
///     `aligned_with_append_operator?` treats `<<` and `=` as cross-alignable by
///     last column. When a variable is assigned with `=` on one line and appended
///     with `<<` on an adjacent line, the extra spaces before `=` are allowed if
///     the `=` aligns with the second `<` of `<<`. Our `check_equals_alignment`
///     only checked `=` vs `=`. Added cross-alignment checks for `=`/`<<` and
///     `<<`/`=` using last-column matching, plus a `find_lshift_col` helper.
///     Fixes 46 FPs from Arachni, ManageIQ, thinking-sphinx, etc.
///
/// 12. **Single-character operator/sigil alignment FNs (fixed)**: `extract_token_at`
///     returned single characters for operators (`|`, `<`, `>`, `&`) and sigils
///     (`@`, `$`), causing coincidental Mode 2 alignment matches (e.g., `@` in
///     `@fake_stderr` matching `@` in `@called`). Extended `extract_token_at` to
///     extract full variable names for `@foo`, `@@foo`, `$foo` sigils and
///     multi-character operators (`||`, `&&`, `<<`, `>>`, `||=`, etc.).
///     Fixes several FNs from gli, Arachni, and similar repos.
///
/// ## Investigation findings (2026-03-30)
///
/// 13. **CRLF trailing-space FPs (fixed 2026-03-31)**: `SourceFile::lines()`
///     splits on `\n` and leaves a terminal `\r` on CRLF lines. The scanner
///     treated spaces before that `\r` as spacing before another token, so old
///     Windows-style files produced false positives on trailing whitespace like
///     `end  \r\n` or `render ...    \r\n`. RuboCop leaves those to
///     `Layout/TrailingWhitespace`. Fixed by normalizing per-line slices before
///     scanning/alignment and by using `SourceFile::line_start_offset` for byte
///     offsets instead of manual newline-width accounting.
///
/// 14. **Heredoc interpolation FNs (fixed 2026-03-31)**: CodeMap marks whole
///     heredoc bodies as non-code, so raw scanners must opt back into `#{...}`
///     ranges explicitly. ExtraSpacing only checked `is_code()`, which skipped
///     real gaps such as `#{foo(1,  2)}` inside heredocs and left the Skyline
///     helper-browser examples undetected. Fixed by scanning heredoc
///     interpolation offsets while still excluding nested string/regex/symbol
///     literals within those interpolations.
///
/// 15. **Numeric literal alignment FNs (fixed 2026-04-01)**: `extract_token_at`
///     treated numeric literals like `0.4725` and `-1.0710` as single-character
///     tokens (`0` or `-`). That made unrelated floats in aligned matrix data
///     look like exact token matches, so extra spaces before later values were
///     incorrectly allowed. Extract full numeric literals, including signs,
///     decimal parts, exponents, and common Ruby suffixes, before comparing
///     exact-token alignment.
///
/// 16. **Heredoc opener FNs (fixed 2026-04-01)**: `check_equals_alignment`
///     treated every `<<` as an append operator, so heredoc openers like
///     `let(:x) {      <<-EOT` were incorrectly considered aligned with `=` on
///     adjacent lines. Track actual heredoc opener offsets from Prism and
///     exclude them from `=`/`<<` cross-alignment.
///
/// 17. **Leading-space `%w/%i/%W/%I` FNs (fixed 2026-04-01)**: non-empty
///     word/symbol arrays previously ignored their entire interior, which hid
///     extra spaces immediately after the opener in forms like
///     `%w[  id lock_version]`. Ignore only separator spans between elements
///     plus the trailing span before the closing delimiter.
///
/// 18. **Symbol/label token extraction FNs (fixed 2026-04-02)**: the exact-token
///     alignment fallback still reduced `:symbol`, `::Const`, and `label:` tokens
///     to `:` or a bare identifier. In real files that let coincidental `:` or
///     identifier text on adjacent lines suppress offenses that RuboCop still
///     reports, including `loopback  :localtick`, `[:posixclass,    :word]`,
///     and `text ...,   layout: :title`. Extract full symbol/path/label tokens
///     so alignment only succeeds for genuinely matching tokens.
///
/// 19. **Multiline receiver chaining FPs (fixed 2026-04-02)**: RuboCop does not
///     report extra spaces before a chained `.` when the receiver itself spans
///     multiple lines, such as `expect { ... }          .to ...` or
///     `{\n  a: 1\n}  .transform_values`. Raw scanning saw those as ordinary
///     gaps and flagged them. Ignore only the whitespace range between a
///     multiline receiver's end and its chained `.` operator.
///
/// 20. **Heredoc opener/block-close FPs (fixed 2026-04-02)**: RuboCop allows
///     `let(:x) { <<~TEXT  }` style spacing before the same-line `}` that closes
///     a block containing a heredoc body. Raw scanning still treated the opener
///     line as normal code and flagged the gap. Ignore only the whitespace range
///     between a heredoc opener and that same-line block closer.
///
/// ## Investigation findings (2026-04-04)
///
/// 21. **`extract_token_at` returning single `=` for `==`/`===`/`=~`/`=>` and
///     `!` for `!=`/`!~` (fixed)**: Mode 2 alignment compared only the first
///     character of compound operators, causing coincidental alignment matches
///     (e.g., the `=` of `==` matching a plain `=` on an adjacent line).
///     Extended `extract_token_at` to extract full multi-character `=`-based
///     and `!`-based operators.
///
/// 22. **`check_equals_alignment` using first `=` column (fixed)**: Even after
///     fixing `extract_token_at`, the separate equals-alignment check still
///     matched the first `=` of `==` on the offense line with an interior `=`
///     of `==` on the adjacent line. Changed to `find_last_equals_col` which
///     returns the rightmost `=` position, and added a terminal-equals check
///     ensuring the adjacent `=` is not followed by another `=`.
///
/// 23. **Heredoc trailing content FPs (fixed)**: RuboCop's token stream jumps
///     from the heredoc opener to the heredoc body, so trailing content on the
///     opener's line (comments like `# :nodoc:`, `if` modifiers, block closers)
///     is never token-paired with the opener. Raw scanning still checked these
///     gaps and produced FPs. Ignore the range from each heredoc opener's end
///     to the end of its line. Fixes ~8 FPs from rubychan, sidekiq, mcorino,
///     opal, ruby__optparse, and volanja.
pub struct ExtraSpacing;

impl Cop for ExtraSpacing {
    fn name(&self) -> &'static str {
        "Layout/ExtraSpacing"
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
        mut corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let allow_for_alignment = config.get_bool("AllowForAlignment", true);
        let allow_before_trailing_comments = config.get_bool("AllowBeforeTrailingComments", false);
        let _force_equal_sign_alignment = config.get_bool("ForceEqualSignAlignment", false);

        let lines: Vec<&[u8]> = source.lines().map(trim_terminal_cr).collect();
        let src_bytes = source.as_bytes();

        // Collect multiline hash pair ranges to ignore (key..value spacing
        // is handled by Layout/HashAlignment, not this cop).
        let mut ignored_ranges = collect_hash_pair_ranges(parse_result, src_bytes);

        // Collect word/symbol array interior ranges to ignore (%w, %W, %i, %I).
        // Spaces inside these arrays are element separators, not extra spacing.
        ignored_ranges.extend(collect_word_array_ranges(parse_result));

        // RuboCop allows spaces before a chained `.` when the receiver spans
        // multiple lines (for example a multiline block or hash literal).
        ignored_ranges.extend(collect_multiline_receiver_chain_ranges(
            parse_result,
            source,
            src_bytes,
        ));

        // RuboCop also allows spaces between a heredoc opener and the same-line
        // `}` that closes the surrounding block.
        ignored_ranges.extend(collect_heredoc_block_closer_ranges(parse_result, src_bytes));

        // RuboCop's token stream jumps from the heredoc opener to the heredoc
        // body, so any text after the opener on the same line (trailing comments,
        // `if` modifiers, block closers) is never paired with the opener token.
        // Ignore the gap from each heredoc opener's end to the end of its line.
        ignored_ranges.extend(collect_heredoc_trailing_ranges(parse_result, src_bytes));

        // Track actual heredoc opener offsets so `<<` heredoc delimiters are
        // not mistaken for append operators during alignment checks.
        let heredoc_opener_starts = collect_heredoc_opener_starts(parse_result);

        // Build the set of aligned comment lines (1-indexed). Two consecutive
        // comments that start at the same column are both considered "aligned".
        let aligned_comment_lines = build_aligned_comment_lines(parse_result, source);

        // Identify comment-only lines (0-indexed) for skipping during alignment search
        let comment_only_lines = build_comment_only_lines(&lines);

        for (line_idx, &line) in lines.iter().enumerate() {
            let line_num = line_idx + 1;
            let line_start_offset = source.line_start_offset(line_num);
            let mut i = 0;

            // Skip leading whitespace (indentation)
            while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
                i += 1;
            }

            // Now scan for extra whitespace within the line.
            // We detect runs of 2+ whitespace characters (spaces and/or tabs).
            // A single space or tab between tokens is normal; only multi-char
            // gaps are extra spacing, matching RuboCop's character-count approach.
            while i < line.len() {
                if line[i] == b' ' || line[i] == b'\t' {
                    let space_start = i;
                    while i < line.len() && (line[i] == b' ' || line[i] == b'\t') {
                        i += 1;
                    }
                    let space_count = i - space_start;

                    // Flag if: multiple whitespace characters (2+ spaces/tabs).
                    // A single tab is 1 whitespace character and is NOT extra spacing,
                    // matching RuboCop's token-based approach which counts characters
                    // in the gap between tokens, not visual column width.
                    if space_count > 1 && i < line.len() {
                        // Skip spacing before backslash line continuation at end of line.
                        // RuboCop's token-based approach doesn't see `\` as a token, so
                        // the space between the last token and `\` is never flagged.
                        if line[i] == b'\\'
                            && line[i + 1..]
                                .iter()
                                .all(|&b| b == b' ' || b == b'\t' || b == b'\n' || b == b'\r')
                        {
                            continue;
                        }

                        // Get the byte offset in the full source
                        let abs_offset = line_start_offset + space_start;

                        // Skip if inside string/comment, except for code inside
                        // #{...} interpolation within heredocs.
                        if !code_map.is_code(abs_offset)
                            && (!code_map.is_heredoc_interpolation(abs_offset)
                                || code_map.is_non_code_in_heredoc_interpolation(abs_offset))
                        {
                            continue;
                        }

                        // Skip if inside a multiline hash pair (key => value
                        // or key: value) -- handled by Layout/HashAlignment
                        if is_in_ignored_range(&ignored_ranges, abs_offset) {
                            continue;
                        }

                        // Skip if before trailing comment and that's allowed
                        if allow_before_trailing_comments && line[i] == b'#' {
                            continue;
                        }

                        if line[i] == b'#' {
                            // For trailing comments: check ONLY if the comment is
                            // aligned with other comments at the same column.
                            // RuboCop's aligned_tok? for comment tokens only checks
                            // @aligned_comments, never aligned_with_something?.
                            // Checking is_aligned_with_adjacent here would cause
                            // false negatives by matching `\s\S` patterns inside
                            // comment text on adjacent lines.
                            if allow_for_alignment && aligned_comment_lines.contains(&line_num) {
                                continue;
                            }
                        } else {
                            // For non-comment tokens: check alignment with adjacent code
                            if allow_for_alignment
                                && is_aligned_with_adjacent(
                                    &lines,
                                    line_idx,
                                    i,
                                    line_start_offset,
                                    source,
                                    &heredoc_opener_starts,
                                    &comment_only_lines,
                                )
                            {
                                continue;
                            }
                        }

                        let mut diag = self.diagnostic(
                            source,
                            line_num,
                            space_start, // point to the start of the extra space run
                            "Unnecessary spacing detected.".to_string(),
                        );
                        if let Some(ref mut corr) = corrections {
                            // Replace multi-space run with single space
                            corr.push(crate::correction::Correction {
                                start: abs_offset,
                                end: abs_offset + space_count,
                                replacement: " ".to_string(),
                                cop_name: self.name(),
                                cop_index: 0,
                            });
                            diag.corrected = true;
                        }
                        diagnostics.push(diag);
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
}

// -- Multiline hash pair ignored ranges --

/// Collect byte ranges between keys and values in multiline hash pairs.
fn collect_hash_pair_ranges(
    parse_result: &ruby_prism::ParseResult<'_>,
    src_bytes: &[u8],
) -> Vec<Range<usize>> {
    let mut collector = HashPairCollector {
        ranges: Vec::new(),
        src_bytes,
    };
    collector.visit(&parse_result.node());
    collector.ranges
}

struct HashPairCollector<'a> {
    ranges: Vec<Range<usize>>,
    src_bytes: &'a [u8],
}

impl<'pr> Visit<'pr> for HashPairCollector<'_> {
    fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'pr>) {
        self.collect_multiline_pairs(node.elements().iter(), &node.location());
        ruby_prism::visit_hash_node(self, node);
    }

    fn visit_keyword_hash_node(&mut self, node: &ruby_prism::KeywordHashNode<'pr>) {
        self.collect_multiline_pairs(node.elements().iter(), &node.location());
        ruby_prism::visit_keyword_hash_node(self, node);
    }
}

impl HashPairCollector<'_> {
    fn collect_multiline_pairs<'a>(
        &mut self,
        elements: impl Iterator<Item = ruby_prism::Node<'a>>,
        parent_loc: &ruby_prism::Location<'_>,
    ) {
        let start = parent_loc.start_offset();
        let end = parent_loc.end_offset().min(self.src_bytes.len());
        let is_multiline = self.src_bytes[start..end].contains(&b'\n');
        if !is_multiline {
            return;
        }
        for element in elements {
            if let Some(assoc) = element.as_assoc_node() {
                let key_end = assoc.key().location().end_offset();
                let val_start = assoc.value().location().start_offset();
                if val_start > key_end {
                    self.ranges.push(key_end..val_start);
                }
            }
        }
    }
}

fn is_in_ignored_range(ranges: &[Range<usize>], offset: usize) -> bool {
    ranges.iter().any(|r| r.contains(&offset))
}

// -- Multiline receiver chained-call ignored ranges --

/// Collect byte ranges between a multiline receiver and its chained `.` call
/// operator. RuboCop does not treat these as extra spacing.
fn collect_multiline_receiver_chain_ranges(
    parse_result: &ruby_prism::ParseResult<'_>,
    source: &SourceFile,
    src_bytes: &[u8],
) -> Vec<Range<usize>> {
    let mut collector = MultilineReceiverChainCollector {
        ranges: Vec::new(),
        source,
        src_bytes,
    };
    collector.visit(&parse_result.node());
    collector.ranges
}

struct MultilineReceiverChainCollector<'a> {
    ranges: Vec<Range<usize>>,
    source: &'a SourceFile,
    src_bytes: &'a [u8],
}

impl<'pr> Visit<'pr> for MultilineReceiverChainCollector<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        self.collect(node.receiver(), node.call_operator_loc());
        ruby_prism::visit_call_node(self, node);
    }
}

impl MultilineReceiverChainCollector<'_> {
    fn collect(
        &mut self,
        receiver: Option<ruby_prism::Node<'_>>,
        operator: Option<ruby_prism::Location<'_>>,
    ) {
        let Some(receiver) = receiver else {
            return;
        };
        let Some(operator) = operator else {
            return;
        };

        if operator.as_slice() != b"." {
            return;
        }

        let receiver_loc = receiver.location();
        if !location_spans_multiple_lines(self.source, &receiver_loc) {
            return;
        }

        let gap_start = receiver_loc.end_offset();
        let gap_end = operator.start_offset();
        if gap_end <= gap_start {
            return;
        }

        if self.src_bytes[gap_start..gap_end]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
        {
            self.ranges.push(gap_start..gap_end);
        }
    }
}

// -- Heredoc opener/block-close ignored ranges --

/// Collect byte ranges between a heredoc opener and the same-line `}` that
/// closes the surrounding block. RuboCop allows this form.
fn collect_heredoc_block_closer_ranges(
    parse_result: &ruby_prism::ParseResult<'_>,
    src_bytes: &[u8],
) -> Vec<Range<usize>> {
    let mut collector = HeredocBlockCloserCollector {
        ranges: Vec::new(),
        src_bytes,
    };
    collector.visit(&parse_result.node());
    collector.ranges
}

struct HeredocBlockCloserCollector<'a> {
    ranges: Vec<Range<usize>>,
    src_bytes: &'a [u8],
}

impl<'pr> Visit<'pr> for HeredocBlockCloserCollector<'_> {
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        self.collect(node);
        ruby_prism::visit_block_node(self, node);
    }
}

impl HeredocBlockCloserCollector<'_> {
    fn collect(&mut self, node: &ruby_prism::BlockNode<'_>) {
        let Some(body) = node.body().and_then(|body| body.as_statements_node()) else {
            return;
        };

        let mut statements = body.body().iter();
        let Some(statement) = statements.next() else {
            return;
        };

        if statements.next().is_some() {
            return;
        }

        let Some(opener_end) = heredoc_opening_end(statement) else {
            return;
        };

        let gap_end = node.closing_loc().start_offset();
        if gap_end <= opener_end {
            return;
        }

        if self.src_bytes[opener_end..gap_end]
            .iter()
            .all(|&b| b == b' ' || b == b'\t')
        {
            self.ranges.push(opener_end..gap_end);
        }
    }
}

fn heredoc_opening_end(node: ruby_prism::Node<'_>) -> Option<usize> {
    if let Some(string) = node.as_string_node() {
        let opening = string.opening_loc()?;
        if opening.as_slice().starts_with(b"<<") {
            return Some(opening.end_offset());
        }
    }

    if let Some(string) = node.as_interpolated_string_node() {
        let opening = string.opening_loc()?;
        if opening.as_slice().starts_with(b"<<") {
            return Some(opening.end_offset());
        }
    }

    None
}

// -- Heredoc trailing ranges --

/// In RuboCop's token stream, a heredoc opener causes the tokenizer to jump to
/// the heredoc body on the next line. Any text after the opener on the same
/// line (trailing comments like `# :nodoc:`, `if` modifiers, block closers)
/// is never token-paired with the opener. This collects ranges from each
/// heredoc opener's end to the end of its line, so the scanner ignores spaces
/// in those trailing segments.
fn collect_heredoc_trailing_ranges(
    parse_result: &ruby_prism::ParseResult<'_>,
    src_bytes: &[u8],
) -> Vec<Range<usize>> {
    let mut collector = HeredocTrailingCollector {
        ranges: Vec::new(),
        src_bytes,
    };
    collector.visit(&parse_result.node());
    collector.ranges
}

struct HeredocTrailingCollector<'a> {
    ranges: Vec<Range<usize>>,
    src_bytes: &'a [u8],
}

impl<'pr> Visit<'pr> for HeredocTrailingCollector<'_> {
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        self.collect_trailing(node.opening_loc());
        ruby_prism::visit_string_node(self, node);
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        self.collect_trailing(node.opening_loc());
        ruby_prism::visit_interpolated_string_node(self, node);
    }
}

impl HeredocTrailingCollector<'_> {
    fn collect_trailing(&mut self, opening: Option<ruby_prism::Location<'_>>) {
        let Some(opening) = opening else { return };
        if !opening.as_slice().starts_with(b"<<") {
            return;
        }
        let opener_end = opening.end_offset();
        // In RuboCop's token stream, the heredoc opener causes the tokenizer to
        // jump to the heredoc body. The gap between the opener and the NEXT token
        // on the same line is never checked by RuboCop's token-pair iteration.
        // But any subsequent token pairs on the same line (e.g., `, {key => val}`)
        // ARE checked because they are consecutive same-line tokens in the stream.
        //
        // So we only ignore from the opener end to the first non-whitespace
        // character on the same line, NOT the entire rest of the line.
        let line_end = self.src_bytes[opener_end..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| opener_end + p)
            .unwrap_or(self.src_bytes.len());
        // Find the first non-whitespace character after the opener on the same line
        let next_token_start = self.src_bytes[opener_end..line_end]
            .iter()
            .position(|&b| b != b' ' && b != b'\t')
            .map(|p| opener_end + p)
            .unwrap_or(line_end);
        if next_token_start > opener_end {
            self.ranges.push(opener_end..next_token_start);
        }
    }
}

// -- Word/symbol array ignored ranges --

/// Collect byte ranges inside word/symbol arrays (%w, %W, %i, %I) that should
/// be ignored by ExtraSpacing.
///
/// RuboCop allows separator spaces between elements and trailing spaces before
/// the closing delimiter, but still flags extra spaces immediately after the
/// opener in non-empty arrays (for example `%w[  id lock_version]`).
fn collect_word_array_ranges(parse_result: &ruby_prism::ParseResult<'_>) -> Vec<Range<usize>> {
    let mut collector = WordArrayCollector { ranges: Vec::new() };
    collector.visit(&parse_result.node());
    collector.ranges
}

struct WordArrayCollector {
    ranges: Vec<Range<usize>>,
}

impl<'pr> Visit<'pr> for WordArrayCollector {
    fn visit_array_node(&mut self, node: &ruby_prism::ArrayNode<'pr>) {
        if let Some(opening) = node.opening_loc() {
            let opener = opening.as_slice();
            // Check if this is a %w, %W, %i, or %I array
            if opener.starts_with(b"%w")
                || opener.starts_with(b"%W")
                || opener.starts_with(b"%i")
                || opener.starts_with(b"%I")
            {
                let elements: Vec<_> = node.elements().iter().collect();
                if let Some(first) = elements.first() {
                    // Ignore separator gaps between elements and the trailing
                    // span before the closing delimiter, but not the leading
                    // span after the opener.
                    let mut prev_end = first.location().end_offset();
                    for element in elements.iter().skip(1) {
                        let next_start = element.location().start_offset();
                        if next_start > prev_end {
                            self.ranges.push(prev_end..next_start);
                        }
                        prev_end = element.location().end_offset();
                    }

                    let closing_start = node
                        .closing_loc()
                        .map_or(node.location().end_offset(), |c| c.start_offset());
                    if closing_start > prev_end {
                        self.ranges.push(prev_end..closing_start);
                    }
                }
            }
        }
        ruby_prism::visit_array_node(self, node);
    }
}

// -- Heredoc opener tracking --

fn collect_heredoc_opener_starts(parse_result: &ruby_prism::ParseResult<'_>) -> HashSet<usize> {
    let mut collector = HeredocOpenerCollector {
        starts: HashSet::new(),
    };
    collector.visit(&parse_result.node());
    collector.starts
}

struct HeredocOpenerCollector {
    starts: HashSet<usize>,
}

impl<'pr> Visit<'pr> for HeredocOpenerCollector {
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        if let Some(opening) = node.opening_loc() {
            if opening.as_slice().starts_with(b"<<") {
                self.starts.insert(opening.start_offset());
            }
        }
        ruby_prism::visit_string_node(self, node);
    }

    fn visit_interpolated_string_node(&mut self, node: &ruby_prism::InterpolatedStringNode<'pr>) {
        if let Some(opening) = node.opening_loc() {
            if opening.as_slice().starts_with(b"<<") {
                self.starts.insert(opening.start_offset());
            }
        }
        ruby_prism::visit_interpolated_string_node(self, node);
    }
}

// -- Aligned comments --

/// Build a set of line numbers (1-indexed) where trailing comments are
/// aligned with adjacent comments at the same column.
fn build_aligned_comment_lines(
    parse_result: &ruby_prism::ParseResult<'_>,
    source: &SourceFile,
) -> HashSet<usize> {
    let mut comment_locs: Vec<(usize, usize)> = Vec::new();
    for comment in parse_result.comments() {
        let loc = comment.location();
        let (line, col) = source.offset_to_line_col(loc.start_offset());
        comment_locs.push((line, col));
    }
    comment_locs.sort_unstable();

    let mut aligned = HashSet::new();
    for pair in comment_locs.windows(2) {
        let (line1, col1) = pair[0];
        let (line2, col2) = pair[1];
        if col1 == col2 {
            aligned.insert(line1);
            aligned.insert(line2);
        }
    }
    aligned
}

// -- Comment-only lines --

fn build_comment_only_lines(lines: &[&[u8]]) -> HashSet<usize> {
    let mut set = HashSet::new();
    for (idx, line) in lines.iter().enumerate() {
        let first_non_ws = line.iter().position(|&b| b != b' ' && b != b'\t');
        if let Some(pos) = first_non_ws {
            if line[pos] == b'#' {
                set.insert(idx);
            }
        }
    }
    set
}

// -- Alignment detection --

/// Check if the token at `col` aligns with a token on a nearby line.
///
/// Implements RuboCop's PrecedingFollowingAlignment:
/// 1. First pass: nearest non-blank, non-comment-only line in each direction.
/// 2. Second pass: nearest line with the same indentation in each direction.
fn is_aligned_with_adjacent(
    lines: &[&[u8]],
    line_idx: usize,
    col: usize,
    line_start_offset: usize,
    source: &SourceFile,
    heredoc_opener_starts: &HashSet<usize>,
    comment_only_lines: &HashSet<usize>,
) -> bool {
    let base_indent = line_indentation(lines[line_idx]);

    let current_line = lines[line_idx];

    // Pass 1: nearest non-blank, non-comment-only line
    if let Some(adj) = find_nearest_line(lines, line_idx, true, comment_only_lines, None) {
        if check_alignment(current_line, lines[adj], col)
            || check_equals_alignment(
                current_line,
                lines[adj],
                col,
                line_start_offset,
                source.line_start_offset(adj + 1),
                heredoc_opener_starts,
            )
        {
            return true;
        }
    }
    if let Some(adj) = find_nearest_line(lines, line_idx, false, comment_only_lines, None) {
        if check_alignment(current_line, lines[adj], col)
            || check_equals_alignment(
                current_line,
                lines[adj],
                col,
                line_start_offset,
                source.line_start_offset(adj + 1),
                heredoc_opener_starts,
            )
        {
            return true;
        }
    }

    // Pass 2: nearest line with same indentation
    if let Some(adj) =
        find_nearest_line(lines, line_idx, true, comment_only_lines, Some(base_indent))
    {
        if check_alignment(current_line, lines[adj], col)
            || check_equals_alignment(
                current_line,
                lines[adj],
                col,
                line_start_offset,
                source.line_start_offset(adj + 1),
                heredoc_opener_starts,
            )
        {
            return true;
        }
    }
    if let Some(adj) = find_nearest_line(
        lines,
        line_idx,
        false,
        comment_only_lines,
        Some(base_indent),
    ) {
        if check_alignment(current_line, lines[adj], col)
            || check_equals_alignment(
                current_line,
                lines[adj],
                col,
                line_start_offset,
                source.line_start_offset(adj + 1),
                heredoc_opener_starts,
            )
        {
            return true;
        }
    }

    false
}

/// Find the nearest non-blank, non-comment-only line in the given direction.
/// When `required_indent` is None, returns the very first non-blank, non-comment line.
/// When `required_indent` is Some, skips lines with different indentation (matching
/// RuboCop's PrecedingFollowingAlignment behavior which walks through all lines).
fn find_nearest_line(
    lines: &[&[u8]],
    start_idx: usize,
    going_up: bool,
    comment_only_lines: &HashSet<usize>,
    required_indent: Option<usize>,
) -> Option<usize> {
    let mut idx = start_idx;
    loop {
        if going_up {
            if idx == 0 {
                return None;
            }
            idx -= 1;
        } else {
            idx += 1;
            if idx >= lines.len() {
                return None;
            }
        }

        if comment_only_lines.contains(&idx) {
            continue;
        }

        let line = lines[idx];

        if line.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r') {
            continue;
        }

        if let Some(indent) = required_indent {
            let this_indent = line_indentation(line);
            if this_indent != indent {
                continue;
            }
        }

        return Some(idx);
    }
}

/// Check alignment: mirrors RuboCop's `aligned_words?` check.
///
/// Two modes:
/// 1. `\s\S` at `left_edge - 1` — space followed by non-space = token boundary
/// 2. Exact token match: the token text starting at `col` on the current line
///    appears at the same position on the adjacent line.
///
/// Mode 2 uses the full token text (not single characters) to avoid false
/// negatives from coincidental single-character alignment (e.g., `d` in `do`
/// aligning with trailing `d` in `_______________________d`).
///
/// Uses character-based column comparison to handle multibyte characters (CJK, etc.)
/// correctly. The byte position `col` on the current line is converted to a
/// character column, then the corresponding byte position on the adjacent line
/// is found for comparison.
fn check_alignment(current_line: &[u8], adj_line: &[u8], col: usize) -> bool {
    // Convert byte col on current line to character col, then find
    // the corresponding byte position on the adjacent line.
    let char_col = byte_to_char_col(current_line, col);
    let adj_col = match char_col_to_byte(adj_line, char_col) {
        Some(c) => c,
        None => return false,
    };

    if adj_col >= adj_line.len() {
        return false;
    }
    // Mode 1: space + non-space at the same character column (token boundary alignment)
    if adj_line[adj_col] != b' '
        && adj_line[adj_col] != b'\t'
        && adj_col > 0
        && (adj_line[adj_col - 1] == b' ' || adj_line[adj_col - 1] == b'\t')
    {
        return true;
    }
    // Mode 2: exact token match — extract the "token" starting at col on the
    // current line and check if it appears at the same position on the adjacent
    // line. This handles cases like `.divmod` aligning with `.divmod` where the
    // `.` on the adjacent line is not preceded by space but the alignment is
    // intentional.
    let token = extract_token_at(current_line, col);
    if !token.is_empty()
        && adj_col + token.len() <= adj_line.len()
        && &adj_line[adj_col..adj_col + token.len()] == token
    {
        return true;
    }
    false
}

/// Extract a "token-like" string starting at the given column.
/// This mirrors RuboCop's `range.source` for token comparison in `aligned_words?`.
///
/// - Alphanumeric/underscore: returns the full identifier.
/// - Labels: returns `label:` rather than just `label`.
/// - Numeric literals: returns the full numeric token, including sign/decimal/exponent.
/// - Symbols/constant paths: returns `:foo` / `::Foo` / `:@ivar` rather than just `:`.
/// - `@`, `@@`, `$` followed by identifier: returns the full variable name.
/// - `.` followed by a letter/underscore: returns `.method_name` (method call).
/// - `"` or `'`: returns the full quoted string to avoid coincidental single-char matches.
/// - Multi-character operators (`||`, `&&`, `<<`, `>>`, `||=`, etc.): returns the full operator.
/// - Other operator/punctuation: returns just that character.
fn extract_token_at(line: &[u8], col: usize) -> &[u8] {
    if col >= line.len() {
        return &[];
    }
    let ch = line[col];
    if is_numeric_literal_start(line, col) {
        let end = numeric_literal_end(line, col);
        &line[col..end]
    } else if ch == b':' {
        extract_colon_token(line, col)
    } else if ch.is_ascii_alphanumeric() || ch == b'_' {
        // Identifier: take consecutive word characters
        let mut end = identifier_like_end(line, col);
        if end < line.len() && line[end] == b':' && (end + 1 >= line.len() || line[end + 1] != b':')
        {
            end += 1;
        }
        &line[col..end]
    } else if ch == b' ' || ch == b'\t' {
        &[]
    } else if (ch == b'@' || ch == b'$')
        && col + 1 < line.len()
        && (line[col + 1].is_ascii_alphabetic()
            || line[col + 1] == b'_'
            || (ch == b'@' && line[col + 1] == b'@'))
    {
        // Instance variable (@foo), class variable (@@foo), or global variable ($foo).
        // Include the sigil(s) and the full identifier to avoid coincidental
        // single-character alignment on `@`, `@@`, or `$`.
        let ident_start = if ch == b'@' && col + 1 < line.len() && line[col + 1] == b'@' {
            col + 2 // @@
        } else {
            col + 1 // @ or $
        };
        let end = identifier_like_end(line, ident_start);
        &line[col..end]
    } else if ch == b'.' {
        // Dot (call operator): return just `.`, matching RuboCop's tokenization
        // where `.` is a separate token from the method name that follows.
        // Previously extracted `.method_name` which caused FPs when different
        // method names at the same column failed to match (e.g., `.to` vs `.not_to`).
        &line[col..col + 1]
    } else if ch == b'"' || ch == b'\'' {
        // String delimiter: extract the full quoted string to avoid coincidental
        // single-character alignment. This matches RuboCop's behavior where
        // `range.source` for a string token is the full string text.
        if let Some(close_pos) = line[col + 1..].iter().position(|&b| b == ch) {
            &line[col..col + 1 + close_pos + 1]
        } else {
            // No closing quote found on same line — return just the quote
            &line[col..col + 1]
        }
    } else if ch == b'|' || ch == b'&' || ch == b'<' || ch == b'>' {
        // Multi-character operators: ||, &&, <<, >>, ||=, &&=, <<=, >>=, <=, >=
        // Extract the full operator to avoid coincidental single-character alignment
        // (e.g., `|` in `||=` matching `|` in `||=` at a different position).
        let mut end = col + 1;
        // Second character of same type (||, &&, <<, >>)
        if end < line.len() && line[end] == ch {
            end += 1;
        }
        // Trailing = (||=, &&=, <<=, >>=, <=, >=)
        if end < line.len() && line[end] == b'=' {
            end += 1;
        }
        &line[col..end]
    } else if ch == b'=' {
        // Equals-based operators: =, ==, ===, =~, =>
        // Extract the full operator to avoid coincidental single-character alignment
        // (e.g., `=` from `==` matching the second `=` of `==` at a shifted column).
        let mut end = col + 1;
        if end < line.len() {
            match line[end] {
                b'=' => {
                    end += 1;
                    // Handle === (triple equals)
                    if end < line.len() && line[end] == b'=' {
                        end += 1;
                    }
                }
                b'~' | b'>' => {
                    end += 1;
                }
                _ => {}
            }
        }
        &line[col..end]
    } else if ch == b'!' {
        // Bang operators: !, !=, !~
        let mut end = col + 1;
        if end < line.len() && (line[end] == b'=' || line[end] == b'~') {
            end += 1;
        }
        &line[col..end]
    } else {
        // Other operator/punctuation: just the single character
        &line[col..col + 1]
    }
}

fn extract_colon_token(line: &[u8], col: usize) -> &[u8] {
    if col + 1 >= line.len() {
        return &line[col..col + 1];
    }

    let mut next = col + 1;
    if line[next] == b':' {
        next += 1;
    }

    if next >= line.len() {
        return &line[col..next];
    }

    if line[next] == b'@' {
        next += 1;
        if next < line.len() && line[next] == b'@' {
            next += 1;
        }
    } else if line[next] == b'$' {
        next += 1;
    }

    if next < line.len() && (line[next].is_ascii_alphanumeric() || line[next] == b'_') {
        let end = identifier_like_end(line, next);
        &line[col..end]
    } else if col + 1 < line.len() && line[col + 1] == b':' {
        &line[col..col + 2]
    } else {
        &line[col..col + 1]
    }
}

fn identifier_like_end(line: &[u8], start: usize) -> usize {
    let mut end = line[start..]
        .iter()
        .position(|&b| !b.is_ascii_alphanumeric() && b != b'_')
        .map_or(line.len(), |p| start + p);

    if end < line.len() && (line[end] == b'?' || line[end] == b'!') {
        end += 1;
    }

    end
}

fn is_numeric_literal_start(line: &[u8], col: usize) -> bool {
    if col >= line.len() {
        return false;
    }

    let ch = line[col];
    if ch.is_ascii_digit() {
        return true;
    }

    (ch == b'+' || ch == b'-') && col + 1 < line.len() && line[col + 1].is_ascii_digit()
}

fn numeric_literal_end(line: &[u8], col: usize) -> usize {
    let mut i = col;

    if line[i] == b'+' || line[i] == b'-' {
        i += 1;
    }

    if i + 1 < line.len()
        && line[i] == b'0'
        && matches!(
            line[i + 1],
            b'b' | b'B' | b'd' | b'D' | b'o' | b'O' | b'x' | b'X'
        )
    {
        i += 2;
        while i < line.len() && is_base_prefixed_numeric_char(line[i]) {
            i += 1;
        }
    } else {
        while i < line.len() && (line[i].is_ascii_digit() || line[i] == b'_') {
            i += 1;
        }

        if i + 1 < line.len() && line[i] == b'.' && line[i + 1].is_ascii_digit() {
            i += 1;
            while i < line.len() && (line[i].is_ascii_digit() || line[i] == b'_') {
                i += 1;
            }
        }

        if i < line.len() && (line[i] == b'e' || line[i] == b'E') {
            let exp_start = i;
            i += 1;
            if i < line.len() && (line[i] == b'+' || line[i] == b'-') {
                i += 1;
            }

            let digits_start = i;
            while i < line.len() && (line[i].is_ascii_digit() || line[i] == b'_') {
                i += 1;
            }

            if digits_start == i {
                i = exp_start;
            }
        }
    }

    while i < line.len() && matches!(line[i], b'i' | b'r') {
        i += 1;
    }

    i
}

fn is_base_prefixed_numeric_char(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_'
}

/// Check if there's equals-sign alignment between the current line and
/// the adjacent line. For compound assignment operators like +=, -=, ||=,
/// &&=, the '=' sign should align with a '=' on the adjacent line.
///
/// Also handles cross-alignment between `=` and `<<` (append operator),
/// matching RuboCop's `aligned_with_append_operator?` which allows:
/// - `=` aligning with the last `<` of `<<` by last column
/// - `<<` aligning with `=` by last column
///
/// Both the current and adjacent line's `=` must look like an assignment
/// operator (preceded by space or an operator character like `+`, `|`, etc.)
/// to avoid matching `=` inside strings or other non-assignment contexts.
fn check_equals_alignment(
    current_line: &[u8],
    adj_line: &[u8],
    col: usize,
    current_line_start_offset: usize,
    adj_line_start_offset: usize,
    heredoc_opener_starts: &HashSet<usize>,
) -> bool {
    // Find the LAST '=' in the operator starting at col on the current line.
    // RuboCop compares operators by their last_column, so we use the rightmost `=`.
    let eq_col = find_last_equals_col(current_line, col);
    if let Some(eq_col) = eq_col {
        // Convert the byte position of the last '=' to a character column,
        // then find the corresponding byte position on the adjacent line.
        let eq_char_col = byte_to_char_col(current_line, eq_col);
        let adj_eq_col = match char_col_to_byte(adj_line, eq_char_col) {
            Some(c) => c,
            None => return false,
        };
        // Check if the adjacent line has '=' at the same character column,
        // and that it is the LAST '=' of its operator (not an interior '=' of '==').
        if adj_eq_col < adj_line.len()
            && adj_line[adj_eq_col] == b'='
            && is_assignment_equals(adj_line, adj_eq_col)
            && (adj_eq_col + 1 >= adj_line.len() || adj_line[adj_eq_col + 1] != b'=')
        {
            return true;
        }
        // Cross-alignment: current has `=` (or ends with `=`), adjacent has `<<`
        // whose last `<` is at the same column. RuboCop's aligned_with_append_operator?
        // checks: range.source[-1] == '=' && token.type == tLSHFT && last_column matches.
        //
        // RuboCop uses `detect` to find the FIRST assignment/comparison token on the
        // adjacent line. If there's an earlier `=`/`||=`/etc. before the `<<`, the `<<`
        // is not the first operator and should not be used for cross-alignment.
        if adj_eq_col < adj_line.len()
            && adj_line[adj_eq_col] == b'<'
            && adj_eq_col > 0
            && adj_line[adj_eq_col - 1] == b'<'
        {
            // Adjacent line has `<<` ending at eq_char_col — last `<` aligns with `=`
            // Verify the `<<` is preceded by space (i.e., it's an operator, not inside something)
            let lshift_start = adj_eq_col - 1;
            if lshift_start == 0
                || adj_line[lshift_start - 1] == b' '
                || adj_line[lshift_start - 1] == b'\t'
            {
                let adj_lshift_offset = adj_line_start_offset + lshift_start;
                if !heredoc_opener_starts.contains(&adj_lshift_offset)
                    && !has_earlier_assignment_op(adj_line, lshift_start)
                {
                    return true;
                }
            }
        }
    }

    // Cross-alignment: current has `<<`, adjacent has `=` at the same last column.
    // RuboCop's aligned_with_append_operator? checks:
    // range.source == '<<' && token.equal_sign? && last_column matches.
    let lshift_col = find_lshift_col(current_line, col);
    if let Some(lshift_col) = lshift_col {
        let current_lshift_offset = current_line_start_offset + lshift_col;
        if heredoc_opener_starts.contains(&current_lshift_offset) {
            return false;
        }

        // The last `<` of `<<` is at lshift_col + 1
        let last_char_col = byte_to_char_col(current_line, lshift_col + 1);
        let adj_last_col = match char_col_to_byte(adj_line, last_char_col) {
            Some(c) => c,
            None => return false,
        };
        // Check if adjacent line has `=` at the same column as last `<`
        // Only consider the `=` if it would be the first assignment/comparison
        // operator on the adjacent line (matching RuboCop's `detect` behavior).
        if adj_last_col < adj_line.len()
            && adj_line[adj_last_col] == b'='
            && is_assignment_equals(adj_line, adj_last_col)
            && !has_earlier_assignment_op(adj_line, adj_last_col)
        {
            return true;
        }
    }

    false
}

/// Check if `=` at the given column on a line looks like an assignment operator.
fn is_assignment_equals(line: &[u8], eq_col: usize) -> bool {
    if eq_col == 0 {
        return true; // `=` at start of line is always an assignment
    }
    let prev = line[eq_col - 1];
    prev == b' '
        || prev == b'\t'
        || prev == b'+'
        || prev == b'-'
        || prev == b'*'
        || prev == b'/'
        || prev == b'%'
        || prev == b'|'
        || prev == b'&'
        || prev == b'^'
        || prev == b'<'
        || prev == b'>'
        || prev == b'!'
        || prev == b'='
}

/// Check if there's an assignment/comparison operator (=, +=, ||=, ==, <<, etc.)
/// on the line at a column strictly before `before_col`.
///
/// RuboCop uses `detect` to find the FIRST assignment/comparison token on a line
/// for alignment. If this function returns true, it means the operator at
/// `before_col` is NOT the first such operator and should not be used for
/// cross-alignment.
fn has_earlier_assignment_op(line: &[u8], before_col: usize) -> bool {
    let mut i = 0;
    while i < before_col && i < line.len() {
        if line[i] == b'=' && is_assignment_equals(line, i) {
            return true;
        }
        // Check for `<<` in operator position (preceded by space or at start)
        if i + 1 < before_col
            && i + 1 < line.len()
            && line[i] == b'<'
            && line[i + 1] == b'<'
            && (i == 0 || line[i - 1] == b' ' || line[i - 1] == b'\t')
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Find the column of the LAST '=' in an assignment/comparison operator starting at col.
///
/// RuboCop aligns operators by their `last_column`, so `=`, `==`, `+=`, `||=` etc.
/// all align by the position of their rightmost `=`.  This prevents false alignment
/// between `=` (assignment) and the interior `=` of `==` (comparison) at a shifted
/// column.
///
/// Handles: =, ==, ===, !=, <=, >=, +=, -=, *=, /=, %=, **=, ||=, &&=, <<=, >>=
fn find_last_equals_col(line: &[u8], col: usize) -> Option<usize> {
    let mut last_eq = None;
    for offset in 0..4 {
        let c = col + offset;
        if c >= line.len() {
            break;
        }
        if line[c] == b'=' {
            last_eq = Some(c);
        }
        // Stop if we hit a space (we've gone past the token)
        if line[c] == b' ' || line[c] == b'\t' {
            break;
        }
    }
    last_eq
}

/// Find the starting column of `<<` (left-shift/append operator) at or near col.
/// Returns Some(col_of_first_<) if found.
fn find_lshift_col(line: &[u8], col: usize) -> Option<usize> {
    for offset in 0..3 {
        let c = col + offset;
        if c + 1 >= line.len() {
            break;
        }
        if line[c] == b'<' && line[c + 1] == b'<' {
            // Make sure it's not `<<<` or part of a heredoc
            if c + 2 < line.len() && line[c + 2] == b'<' {
                return None;
            }
            return Some(c);
        }
        if line[c] == b' ' || line[c] == b'\t' {
            break;
        }
    }
    None
}

fn line_indentation(line: &[u8]) -> usize {
    line.iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

fn location_spans_multiple_lines(source: &SourceFile, loc: &ruby_prism::Location<'_>) -> bool {
    let (start_line, _) = source.offset_to_line_col(loc.start_offset());
    let end_offset = loc.end_offset().saturating_sub(1);
    let (end_line, _) = source.offset_to_line_col(end_offset);
    start_line != end_line
}

fn trim_terminal_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Convert a byte offset to a character column (0-indexed).
/// Each UTF-8 character (regardless of byte length) counts as 1 column.
fn byte_to_char_col(line: &[u8], byte_col: usize) -> usize {
    let end = byte_col.min(line.len());
    let s = std::str::from_utf8(&line[..end]).unwrap_or("");
    s.chars().count()
}

/// Convert a character column to a byte offset on the given line.
/// Returns None if the line is shorter than the requested character column.
fn char_col_to_byte(line: &[u8], char_col: usize) -> Option<usize> {
    let s = std::str::from_utf8(line).unwrap_or("");
    let mut byte_offset = 0;
    for (i, ch) in s.chars().enumerate() {
        if i == char_col {
            return Some(byte_offset);
        }
        byte_offset += ch.len_utf8();
    }
    if char_col == s.chars().count() {
        Some(byte_offset)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(ExtraSpacing, "cops/layout/extra_spacing");
    crate::cop_autocorrect_fixture_tests!(ExtraSpacing, "cops/layout/extra_spacing");

    #[test]
    fn coincidental_alignment_not_preceded_by_space() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // The `d` in `do` aligns with `d` at end of `_______________________d`
        // but the `d` on the adjacent line is NOT preceded by space, so this
        // is coincidental alignment, not intentional. Should be an offense.
        let diags = run_cop_full(
            &cop,
            b"d_is_vertically_aligned  do\n  _______________________d\nend\n",
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 23);
    }

    #[test]
    fn aligned_assignments_allowed() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Aligned assignments should be allowed with AllowForAlignment=true (default)
        let diags = run_cop_full(&cop, b"website = \"example.org\"\nname    = \"Jill\"\n");
        assert!(
            diags.is_empty(),
            "Aligned assignments should not be flagged"
        );
    }

    #[test]
    fn single_line_hash_extra_spaces_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Single-line hash with extra spaces should be flagged
        let diags = run_cop_full(&cop, b"hash = {a:   1,  b:    2}\n");
        assert_eq!(diags.len(), 3, "Expected 3 offenses in single-line hash");
    }

    #[test]
    fn trailing_spaces_before_crlf_are_ignored() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Trailing spaces before CRLF line endings are handled by
        // Layout/TrailingWhitespace, not Layout/ExtraSpacing.
        let src = b"class A\r\n  def x\r\n    render :nothing => true, :status => :not_found    \r\n  end  \r\nend\r\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Trailing spaces before CRLF should not be flagged as extra spacing: {diags:?}"
        );
    }

    #[test]
    fn class_inheritance_extra_spaces() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        let diags = run_cop_full(&cop, b"class A   < String\nend\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 7);
    }

    #[test]
    fn token_not_preceded_by_space_not_alignment() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // RuboCop spec: "alignment with token not preceded by space"
        // The = and ( are on the same column, but ( is not preceded by space,
        // so this is NOT alignment - should be an offense.
        let diags = run_cop_full(&cop, b"website(\"example.org\")\nname   = \"Jill\"\n");
        assert_eq!(
            diags.len(),
            1,
            "Should report offense when aligned token is not preceded by space"
        );
    }

    #[test]
    fn aligning_with_same_character_allowed() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // RuboCop: "aligning with the same character" - allowed with AllowForAlignment=true
        let diags = run_cop_full(
            &cop,
            b"y, m = (year * 12 + (mon - 1) + n).divmod(12)\nm,   = (m + 1)                    .divmod(1)\n",
        );
        assert!(
            diags.is_empty(),
            "Alignment with same character should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn different_kinds_of_assignments_allowed() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // RuboCop: "lining up different kinds of assignments" - allowed
        let src = b"type_name ||= value.class.name if value\ntype_name   = type_name.to_s   if type_name\n\ntype_name  = value.class.name if     value\ntype_name += type_name.to_s   unless type_name\na  += 1\naa -= 2\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Different kinds of aligned assignments should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn aligning_comments_non_adjacent_allowed() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // RuboCop: "aligning comments on non-adjacent lines" - allowed
        let src = b"include_examples 'aligned',   'var = until',  'test'\n\ninclude_examples 'unaligned', \"var = if\",     'test'\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Aligned comments on non-adjacent should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn multiple_unaligned_comments_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // RuboCop spec: multiple comments at different columns - all flagged
        let src = b"class Foo\n  def require(p)  # comment\n  end\n\n  def load(p)  # comment\n  end\n\n  def join(*ps)  # comment\n  end\n\n  def exist?(*ps)  # comment\n  end\nend\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            4,
            "Should report 4 offenses for unaligned comments, got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn aligned_values_in_array_of_hashes() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Aligned values across multiple lines in array of hashes
        // The commas and values align vertically — should be allowed
        let src = b"[\n  {id: 1, name: 'short'  , code: 'equals'      },\n  {id: 2, name: 'longer' , code: 'greater_than'},\n  {id: 3, name: 'longest', code: 'less_than'   },\n]\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Aligned values in array of hashes should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!(
                    "L{}:C{} '{}'",
                    d.location.line, d.location.column, d.message
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn aligned_has_many_declarations() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Aligned Rails declarations - commas at same columns across lines
        let src = b"has_many :items  , dependent: :destroy\nhas_many :images , dependent: :destroy\nhas_many :options, dependent: :destroy\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Aligned has_many declarations should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn trailing_comments_not_aligned_should_flag() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Trailing comments at different columns - NOT aligned, should be flagged
        let src = b"check_a_pattern_result   # comment A\ncheck_b   # comment B\ncheck_c_patterns   # comment C\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            3,
            "Should flag 3 unaligned trailing comments, got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn aligned_trailing_comments_allowed() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Trailing comments at the same column - aligned, should be allowed
        // From the vendor spec: "exactly two comments aligned"
        let src = b"one  # comment one\ntwo  # comment two\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Aligned trailing comments should be allowed, got {} offenses",
            diags.len()
        );
    }

    #[test]
    fn tabs_between_tokens_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Multiple tabs between tokens should be flagged as extra spacing
        let src = b"filter_data('<KEY>')\t\t\t\t{ ENV['KEY'] }\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            1,
            "Should flag tabs between tokens, got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn space_plus_tabs_between_tokens_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Space followed by tabs should be flagged
        let src = b"data[\"cpu\"]    =  temp[\"VCPU\"] \tunless temp[\"VCPU\"].nil?\n";
        let diags = run_cop_full(&cop, src);
        // The `    ` before `=` (4 spaces), `  ` before `temp` (2 spaces),
        // and ` \t` before `unless` (space+tab) should all be offenses
        assert!(
            diags.len() >= 3,
            "Should flag space+tab runs, got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn word_array_spaces_not_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Spaces inside %w() are element separators, not extra spacing
        let src = b"builtins = %w(\n  foo  bar  baz\n  one  two  three\n)\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Should not flag spaces inside %w(), got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn symbol_array_spaces_not_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        let src = b"syms = %i(foo  bar  baz)\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Should not flag spaces inside %i(), got {} offenses",
            diags.len()
        );
    }

    #[test]
    fn empty_percent_i_array_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Empty %i(  ) should flag the extra spaces (not element separators)
        let src = b"x = 1\nsyms = %i(  )\ny = 2\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            1,
            "Should flag extra spaces in empty %i(), got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn multibyte_alignment_allowed() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // CJK characters take 3 bytes each but the commas should align visually
        let src = "[\n  {id: 1, name: '\u{5F88}\u{96BE}'    , code: 'a'},\n  {id: 2, name: '\u{9700}\u{8981}\u{5176}\u{5B83}', code: 'b'},\n]\n";
        let diags = run_cop_full(&cop, src.as_bytes());
        assert!(
            diags.is_empty(),
            "Aligned tokens with multibyte chars should not be flagged, got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn trailing_comment_aligned_with_empty_line_between() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // RuboCop spec: aligned tokens with empty line between
        // The comments are at the same column, separated by blank/code lines
        let src = b"unless nochdir\n  Dir.chdir \"/\"    # Release old working directory.\nend\n\nFile.umask 0000    # Ensure sensible umask.\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Aligned trailing comments with empty line between should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn equals_aligned_with_lshift_allowed() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Corpus FP: = aligned with << on adjacent line (Arachni vector_feed.rb)
        let src = b"pages  = pages.values\npages << page_buffer\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "= aligned with << should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn equals_and_lshift_three_line_alignment() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Corpus FP: three-line alignment with =, <<, = (Arachni sax.rb)
        let src = b"e.document     = @document\n@current_node << e\n@current_node  = e\n";
        let diags = run_cop_full(&cop, src);
        assert!(
            diags.is_empty(),
            "Three-line =/<< alignment should be allowed, got {} offenses: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ivar_sigil_not_coincidental_alignment() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Corpus FN: assert @fake_stderr should be flagged (gli test)
        // The @ sigil coincidentally aligns with @ on adjacent line but they are
        // different tokens (@fake_stderr vs @called).
        let src = b"assert  @fake_stderr.contained?(/flag/)\nassert !@called\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            1,
            "Extra space in 'assert  @fake_stderr' should be flagged, got: {:?}",
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn unaligned_compound_assignment_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Corpus FN: ||= with extra spaces, = signs at different columns
        let src = b"@signatures[pair_hash]      ||= {}\n@data_gathering[pair_hash] ||= {}\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            1,
            "Unaligned ||= with extra spaces should be flagged, got: {:?}",
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn heredoc_trailing_hash_arg_extra_space_flagged() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // FN #8: Extra space in hash argument after heredoc opener should be flagged.
        // RuboCop flags the extra space between `=>` and `"application/atom+xml"`.
        let src = b"self.post(\"/path\", <<-XML, {\"Content-Type\" =>  \"application/atom+xml\"})\n<?xml version='1.0' encoding='UTF-8'?>\n<entry/>\nXML\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            1,
            "Extra space in hash arg after heredoc opener should be flagged, got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!(
                    "L{}:C{} '{}'",
                    d.location.line, d.location.column, d.message
                ))
                .collect::<Vec<_>>()
        );
        // Verify the offense is at the right position (col 48 in the `=>  "` gap)
        assert_eq!(diags[0].location.line, 1);
    }

    #[test]
    fn block_trailing_space_before_close() {
        use crate::testutil::run_cop_full;
        let cop = ExtraSpacing;

        // Extra space before `}` closing a block should be flagged
        let src = b"let(:output_missing) { \"\"  }\n";
        let diags = run_cop_full(&cop, src);
        assert_eq!(
            diags.len(),
            1,
            "Extra space before block closer should be flagged, got {}: {:?}",
            diags.len(),
            diags
                .iter()
                .map(|d| format!("L{}:C{}", d.location.line, d.location.column))
                .collect::<Vec<_>>()
        );
    }
}
