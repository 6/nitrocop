use ruby_prism::Visit;

/// A sorted list of non-code byte ranges (comments, strings, regexps, symbols).
///
/// Used by `check_source` cops to skip non-code regions when scanning raw bytes,
/// avoiding false positives on commas/semicolons/etc inside strings or comments.
pub struct CodeMap {
    /// Sorted, non-overlapping (start, end) byte ranges of non-code regions.
    ranges: Vec<(usize, usize)>,
    /// Sorted, non-overlapping (start, end) byte ranges of string/heredoc/regex
    /// regions ONLY (excludes comments). Used by cops that inspect comments but
    /// need to skip string content (e.g. Layout/EmptyComment, Lint/TripleQuotes).
    string_ranges: Vec<(usize, usize)>,
    /// Sorted, non-overlapping (start, end) byte ranges of heredoc content only.
    /// Used by cops that need to distinguish heredoc content from regular strings.
    heredoc_ranges: Vec<(usize, usize)>,
    /// Sorted, non-overlapping (start, end) byte ranges of regex literal content.
    /// Used by cops that need to skip content inside regex literals specifically.
    regex_ranges: Vec<(usize, usize)>,
    /// Sorted, non-overlapping (start, end) byte ranges of xstring (backtick)
    /// literal content. Used by cops like Layout/IndentationStyle that need to
    /// NOT skip xstring content (RuboCop's string_literal_ranges excludes xstr).
    xstring_ranges: Vec<(usize, usize)>,
    /// Sorted (but NOT merged) heredoc ranges for boundary detection.
    /// Adjacent heredocs need distinct boundaries, and nested heredocs can
    /// overlap. This field preserves the raw per-heredoc ranges so callers can
    /// resolve the correct enclosing boundary for closing-delimiter checks.
    raw_heredoc_ranges: Vec<(usize, usize)>,
    /// Sorted, non-overlapping (start, end) byte ranges of `#{}` interpolation
    /// content inside heredocs. These regions are marked as non-code by the main
    /// `ranges` (heredocs mark everything non-code), but cops like SpaceAfterComma
    /// need to inspect code inside heredoc interpolation.
    heredoc_interpolation_ranges: Vec<(usize, usize)>,
    /// Sorted, non-overlapping (start, end) byte ranges of string/regex/symbol
    /// literals that are nested inside heredoc interpolation blocks. For example,
    /// in `<<~STR\n  #{method('arg,')}\nSTR`, the `'arg,'` range is in this set.
    /// Used to exclude non-code content when inspecting heredoc interpolation.
    heredoc_interpolation_non_code_ranges: Vec<(usize, usize)>,
}

impl CodeMap {
    /// Build a CodeMap from a parse result, collecting non-code regions from
    /// comments, string literals, regular expressions, symbols, and heredocs.
    pub fn from_parse_result(_source: &[u8], parse_result: &ruby_prism::ParseResult<'_>) -> Self {
        let mut string_ranges = Vec::new();
        let mut heredoc_ranges = Vec::new();
        let mut regex_ranges = Vec::new();
        let mut xstring_ranges = Vec::new();
        let mut heredoc_interpolation_ranges = Vec::new();

        // Walk AST to collect string/regex/symbol ranges
        let mut collector = NonCodeCollector {
            ranges: &mut string_ranges,
            heredoc_ranges: &mut heredoc_ranges,
            regex_ranges: &mut regex_ranges,
            xstring_ranges: &mut xstring_ranges,
            heredoc_interpolation_ranges: &mut heredoc_interpolation_ranges,
        };
        collector.visit(&parse_result.node());

        // Content after __END__ marker is not code (data section).
        // Add to string_ranges before merging so it's excluded from both
        // is_code() and is_not_string() queries.
        if let Some(data_loc) = parse_result.data_loc() {
            string_ranges.push((data_loc.start_offset(), data_loc.end_offset()));
        }

        // Sort string ranges (but don't merge yet — we need the un-merged list
        // to identify nested string literals inside heredoc interpolation).
        string_ranges.sort_unstable();

        // Sort and merge heredoc interpolation ranges early so we can use them
        // to identify nested non-code ranges before string_ranges gets merged.
        heredoc_interpolation_ranges.sort_unstable();
        let heredoc_interpolation_ranges = merge_ranges(heredoc_interpolation_ranges);

        // Compute non-code ranges within heredoc interpolation: string/regex/symbol
        // literals whose range is fully contained in a heredoc interpolation range.
        // Must be done before merging string_ranges (which absorbs nested strings
        // into the larger heredoc body range, making them indistinguishable).
        let mut heredoc_interpolation_non_code_ranges: Vec<(usize, usize)> = string_ranges
            .iter()
            .filter(|&&(s, e)| {
                heredoc_interpolation_ranges
                    .iter()
                    .any(|&(is, ie)| s >= is && e <= ie)
            })
            .copied()
            .collect();
        heredoc_interpolation_non_code_ranges.sort_unstable();
        let heredoc_interpolation_non_code_ranges =
            merge_ranges(heredoc_interpolation_non_code_ranges);

        // Now merge string ranges
        let string_ranges = merge_ranges(string_ranges);

        // Sort and merge heredoc ranges (keep raw copy for boundary detection)
        heredoc_ranges.sort_unstable();
        let raw_heredoc_ranges = heredoc_ranges.clone();
        let heredoc_ranges = merge_ranges(heredoc_ranges);

        // Sort and merge regex ranges
        regex_ranges.sort_unstable();
        let regex_ranges = merge_ranges(regex_ranges);

        // Sort and merge xstring ranges
        xstring_ranges.sort_unstable();
        let xstring_ranges = merge_ranges(xstring_ranges);

        // Full non-code ranges include comments + strings + __END__ data section
        let mut ranges = string_ranges.clone();
        for comment in parse_result.comments() {
            let loc = comment.location();
            ranges.push((loc.start_offset(), loc.end_offset()));
        }
        ranges.sort_unstable();
        let ranges = merge_ranges(ranges);

        CodeMap {
            ranges,
            string_ranges,
            heredoc_ranges,
            xstring_ranges,
            raw_heredoc_ranges,
            regex_ranges,
            heredoc_interpolation_ranges,
            heredoc_interpolation_non_code_ranges,
        }
    }

    /// Returns true if the given byte offset is in "code" (not inside a
    /// comment, string, regexp, or symbol literal).
    pub fn is_code(&self, offset: usize) -> bool {
        !Self::in_ranges(&self.ranges, offset)
    }

    /// Returns true if the given byte offset is NOT inside a string, heredoc,
    /// regexp, or symbol literal. Comments are NOT excluded — use this for cops
    /// that inspect comment content but need to skip string/heredoc regions.
    pub fn is_not_string(&self, offset: usize) -> bool {
        !Self::in_ranges(&self.string_ranges, offset)
    }

    /// Returns true if the given byte offset is inside a heredoc body.
    pub fn is_heredoc(&self, offset: usize) -> bool {
        Self::in_ranges(&self.heredoc_ranges, offset)
    }

    /// Returns true if the given byte offset is inside a regex literal.
    pub fn is_regex(&self, offset: usize) -> bool {
        Self::in_ranges(&self.regex_ranges, offset)
    }

    /// Returns true if the given byte offset is inside an xstring (backtick)
    /// literal. RuboCop's `string_literal_ranges` excludes `:xstr` nodes, so
    /// cops that mirror RuboCop's skip logic need this to avoid false negatives.
    pub fn is_xstring(&self, offset: usize) -> bool {
        Self::in_ranges(&self.xstring_ranges, offset)
    }

    /// Returns the end offset of the OUTERMOST raw heredoc range containing
    /// `offset`, or None if the offset is not inside a heredoc range.
    /// Uses raw (unmerged) ranges because nested heredocs overlap and adjacent
    /// heredocs abut; callers like Layout/IndentationStyle need the enclosing
    /// boundary, not an arbitrary inner one.
    pub fn heredoc_range_end(&self, offset: usize) -> Option<usize> {
        let mut outer_end = None;

        for &(_start, end) in self
            .raw_heredoc_ranges
            .iter()
            .take_while(|&&(start, _)| start <= offset)
        {
            if offset < end {
                outer_end = Some(outer_end.map_or(end, |current: usize| current.max(end)));
            }
        }

        outer_end
    }

    /// Returns true if the given byte offset is inside `#{}` interpolation
    /// within a heredoc. These offsets are marked non-code by `is_code()` (since
    /// the entire heredoc body is non-code), but contain actual Ruby expressions
    /// that some cops (e.g. SpaceAfterComma) need to inspect.
    pub fn is_heredoc_interpolation(&self, offset: usize) -> bool {
        Self::in_ranges(&self.heredoc_interpolation_ranges, offset)
    }

    /// Returns true if the given byte offset is inside a string/regex/symbol
    /// literal that is nested within heredoc interpolation. For example,
    /// `'arg,'` inside `<<~STR\n  #{method('arg,')}\nSTR`.
    pub fn is_non_code_in_heredoc_interpolation(&self, offset: usize) -> bool {
        Self::in_ranges(&self.heredoc_interpolation_non_code_ranges, offset)
    }

    fn in_ranges(ranges: &[(usize, usize)], offset: usize) -> bool {
        ranges
            .binary_search_by(|&(start, end)| {
                if offset < start {
                    std::cmp::Ordering::Greater
                } else if offset >= end {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

struct NonCodeCollector<'a> {
    ranges: &'a mut Vec<(usize, usize)>,
    heredoc_ranges: &'a mut Vec<(usize, usize)>,
    regex_ranges: &'a mut Vec<(usize, usize)>,
    xstring_ranges: &'a mut Vec<(usize, usize)>,
    heredoc_interpolation_ranges: &'a mut Vec<(usize, usize)>,
}

impl<'pr> Visit<'pr> for NonCodeCollector<'_> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.collect_from_node(&node);
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.collect_from_node(&node);
    }
}

impl NonCodeCollector<'_> {
    fn collect_from_node(&mut self, node: &ruby_prism::Node<'_>) {
        // Collect the full range of string/regex/symbol nodes.
        // This marks the entire literal (including delimiters) as non-code.
        match node {
            ruby_prism::Node::StringNode { .. } => {
                let sn = node.as_string_node().unwrap();
                let loc = node.location();
                self.ranges.push((loc.start_offset(), loc.end_offset()));
                // For heredocs, the location only covers the opening delimiter (<<~FOO).
                // We need to also cover the content and closing terminator.
                if let Some(open) = sn.opening_loc() {
                    if open.as_slice().starts_with(b"<<") {
                        let content_loc = sn.content_loc();
                        if let Some(close) = sn.closing_loc() {
                            let range = (content_loc.start_offset(), close.end_offset());
                            self.ranges.push(range);
                            self.heredoc_ranges.push(range);
                        } else {
                            let range = (content_loc.start_offset(), content_loc.end_offset());
                            self.ranges.push(range);
                            self.heredoc_ranges.push(range);
                        }
                    }
                }
            }
            ruby_prism::Node::InterpolatedStringNode { .. } => {
                let isn = node.as_interpolated_string_node().unwrap();
                let is_heredoc = isn
                    .opening_loc()
                    .is_some_and(|o| o.as_slice().starts_with(b"<<"));

                if is_heredoc {
                    // For heredocs, mark the entire content as non-code (interpolation
                    // in heredocs is different — it's part of the text body).
                    let loc = node.location();
                    self.ranges.push((loc.start_offset(), loc.end_offset()));
                    let parts = isn.parts();
                    if !parts.is_empty() {
                        let first_start = parts.iter().next().unwrap().location().start_offset();
                        if let Some(close) = isn.closing_loc() {
                            let range = (first_start, close.end_offset());
                            self.ranges.push(range);
                            self.heredoc_ranges.push(range);
                        } else {
                            let last = parts.iter().last().unwrap();
                            let range = (first_start, last.location().end_offset());
                            self.ranges.push(range);
                            self.heredoc_ranges.push(range);
                        }
                    }
                    // Track interpolation regions inside heredocs so cops can
                    // optionally inspect code within #{...} in heredoc bodies.
                    for part in isn.parts().iter() {
                        if let Some(esn) = part.as_embedded_statements_node() {
                            // The EmbeddedStatementsNode covers `#{...}` — record
                            // just the statements inside (excluding `#{` and `}`).
                            if let Some(stmts) = esn.statements() {
                                let sloc = stmts.location();
                                self.heredoc_interpolation_ranges
                                    .push((sloc.start_offset(), sloc.end_offset()));
                            }
                        }
                    }
                } else {
                    // For regular interpolated strings ("...#{...}..."), mark only
                    // the non-interpolated parts as non-code. The opening delimiter,
                    // string literal parts, and closing delimiter are non-code, but
                    // the content inside #{...} (EmbeddedStatementsNode) is code.
                    if let Some(open) = isn.opening_loc() {
                        self.ranges.push((open.start_offset(), open.end_offset()));
                    }
                    for part in isn.parts().iter() {
                        if part.as_embedded_statements_node().is_none()
                            && part.as_interpolated_string_node().is_none()
                        {
                            // StringNode part — literal text, not code.
                            // Skip InterpolatedStringNode parts (from string continuation
                            // like "..." \ "#{...}") — the recursive visitor handles them.
                            let ploc = part.location();
                            self.ranges.push((ploc.start_offset(), ploc.end_offset()));
                        }
                    }
                    if let Some(close) = isn.closing_loc() {
                        self.ranges.push((close.start_offset(), close.end_offset()));
                    }
                }
            }
            ruby_prism::Node::RegularExpressionNode { .. } => {
                let loc = node.location();
                self.ranges.push((loc.start_offset(), loc.end_offset()));
                self.regex_ranges
                    .push((loc.start_offset(), loc.end_offset()));
            }
            ruby_prism::Node::XStringNode { .. } => {
                let loc = node.location();
                self.ranges.push((loc.start_offset(), loc.end_offset()));
                self.xstring_ranges
                    .push((loc.start_offset(), loc.end_offset()));
            }
            ruby_prism::Node::SymbolNode { .. } => {
                let loc = node.location();
                self.ranges.push((loc.start_offset(), loc.end_offset()));
            }
            // For interpolated regex/xstring/symbol, mark only non-interpolated parts
            ruby_prism::Node::InterpolatedRegularExpressionNode { .. } => {
                let irn = node.as_interpolated_regular_expression_node().unwrap();
                let open = irn.opening_loc();
                self.ranges.push((open.start_offset(), open.end_offset()));
                self.regex_ranges
                    .push((open.start_offset(), open.end_offset()));
                for part in irn.parts().iter() {
                    if part.as_embedded_statements_node().is_none() {
                        let ploc = part.location();
                        self.ranges.push((ploc.start_offset(), ploc.end_offset()));
                        self.regex_ranges
                            .push((ploc.start_offset(), ploc.end_offset()));
                    }
                }
                let close = irn.closing_loc();
                self.ranges.push((close.start_offset(), close.end_offset()));
                self.regex_ranges
                    .push((close.start_offset(), close.end_offset()));
            }
            ruby_prism::Node::InterpolatedXStringNode { .. } => {
                let ixn = node.as_interpolated_x_string_node().unwrap();
                let open = ixn.opening_loc();
                self.ranges.push((open.start_offset(), open.end_offset()));
                self.xstring_ranges
                    .push((open.start_offset(), open.end_offset()));
                for part in ixn.parts().iter() {
                    if part.as_embedded_statements_node().is_none() {
                        let ploc = part.location();
                        self.ranges.push((ploc.start_offset(), ploc.end_offset()));
                        self.xstring_ranges
                            .push((ploc.start_offset(), ploc.end_offset()));
                    }
                }
                let close = ixn.closing_loc();
                self.ranges.push((close.start_offset(), close.end_offset()));
                self.xstring_ranges
                    .push((close.start_offset(), close.end_offset()));
            }
            ruby_prism::Node::InterpolatedSymbolNode { .. } => {
                let isn = node.as_interpolated_symbol_node().unwrap();
                if let Some(open) = isn.opening_loc() {
                    self.ranges.push((open.start_offset(), open.end_offset()));
                }
                for part in isn.parts().iter() {
                    if part.as_embedded_statements_node().is_none() {
                        let ploc = part.location();
                        self.ranges.push((ploc.start_offset(), ploc.end_offset()));
                    }
                }
                if let Some(close) = isn.closing_loc() {
                    self.ranges.push((close.start_offset(), close.end_offset()));
                }
            }
            _ => {}
        }
    }
}

/// Merge sorted, possibly overlapping ranges into non-overlapping ranges.
fn merge_ranges(sorted: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in sorted {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_source;

    #[test]
    fn string_continuation_interpolation_is_code() {
        // String continuation: "x #{foo(1,2)}" \
        //   "more"
        // Prism produces an outer InterpolatedStringNode (no opening/closing)
        // wrapping the two continued strings. The inner InterpolatedStringNode
        // contains #{} interpolation whose content should be code.
        let source = b"x = \"x #{foo(1,2)}\" \\\n  \"more\"\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        let comma = source.iter().position(|&b| b == b',').unwrap();
        assert!(
            cm.is_code(comma),
            "Comma inside #{{}} in continuation string should be code"
        );
    }

    #[test]
    fn empty_source() {
        let source = b"";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);
        assert!(cm.ranges.is_empty());
    }

    #[test]
    fn comments_are_non_code() {
        let source = b"x = 1 # comment\ny = 2\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        // "x" at offset 0 is code
        assert!(cm.is_code(0));
        // "#" starts at offset 6
        assert!(!cm.is_code(6));
        // "c" in comment
        assert!(!cm.is_code(8));
        // "y" at start of next line is code
        let y_offset = source.iter().position(|&b| b == b'y').unwrap();
        assert!(cm.is_code(y_offset));
    }

    #[test]
    fn strings_are_non_code() {
        let source = b"x = \"hello, world\"\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        // "x" is code
        assert!(cm.is_code(0));
        // The comma inside the string is NOT code
        let comma_offset = source.iter().position(|&b| b == b',').unwrap();
        assert!(!cm.is_code(comma_offset));
    }

    #[test]
    fn regex_is_non_code() {
        let source = b"x = /a,b/\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        let comma_offset = source.iter().position(|&b| b == b',').unwrap();
        assert!(!cm.is_code(comma_offset));
    }

    #[test]
    fn code_between_strings() {
        let source = b"a = \"x\", \"y\"\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        // The comma between the two strings IS code
        // Find the comma that's between the strings
        let comma_offset = source.windows(2).position(|w| w == b"\",").unwrap() + 1;
        assert!(cm.is_code(comma_offset));
    }

    #[test]
    fn is_code_at_boundaries() {
        let source = b"# comment\nx = 1\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        // Offset 0 = '#', start of comment — non-code
        assert!(!cm.is_code(0));
        // 'x' on the next line is code
        assert!(cm.is_code(10));
    }

    #[test]
    fn merge_overlapping_ranges() {
        let merged = merge_ranges(vec![(0, 5), (3, 8), (10, 15)]);
        assert_eq!(merged, vec![(0, 8), (10, 15)]);
    }

    #[test]
    fn merge_adjacent_ranges() {
        let merged = merge_ranges(vec![(0, 5), (5, 10)]);
        assert_eq!(merged, vec![(0, 10)]);
    }

    #[test]
    fn merge_no_overlap() {
        let merged = merge_ranges(vec![(0, 3), (5, 8)]);
        assert_eq!(merged, vec![(0, 3), (5, 8)]);
    }

    #[test]
    fn heredoc_content_is_non_code() {
        let source = b"x = <<~FOO\n  font-weight: 500;\nFOO\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        // The semicolon inside the heredoc is NOT code
        let semi_offset = source.iter().position(|&b| b == b';').unwrap();
        assert!(
            !cm.is_code(semi_offset),
            "Semicolon inside heredoc should be non-code, offset={semi_offset}"
        );
    }

    #[test]
    fn heredoc_with_method_is_non_code() {
        let source = b"x = <<~FOO.squish\n  font-weight: 500;\nFOO\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        let semi_offset = source.iter().position(|&b| b == b';').unwrap();
        assert!(
            !cm.is_code(semi_offset),
            "Semicolon inside heredoc.squish should be non-code, offset={semi_offset}"
        );
    }

    #[test]
    fn symbol_is_non_code() {
        let source = b"x = :\"hello, world\"\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        let comma_offset = source.iter().position(|&b| b == b',').unwrap();
        assert!(!cm.is_code(comma_offset));
    }

    #[test]
    fn end_marker_content_is_non_code() {
        let source = b"x = 1\n__END__\nfoo; bar\na + b\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        // "x" at offset 0 is code
        assert!(cm.is_code(0));

        // The __END__ marker itself should be non-code
        let end_offset = source.windows(7).position(|w| w == b"__END__").unwrap();
        assert!(
            !cm.is_code(end_offset),
            "__END__ marker at offset {} should be non-code",
            end_offset
        );

        // The semicolon after __END__ should be non-code
        let semi_offset = source.iter().position(|&b| b == b';').unwrap();
        assert!(
            !cm.is_code(semi_offset),
            "Semicolon after __END__ at offset {} should be non-code",
            semi_offset
        );

        // The "+" after __END__ should be non-code
        let plus_offset = source.iter().position(|&b| b == b'+').unwrap();
        assert!(
            !cm.is_code(plus_offset),
            "Plus sign after __END__ at offset {} should be non-code",
            plus_offset
        );
    }

    #[test]
    fn no_end_marker_all_code() {
        let source = b"x = 1; y = 2\n";
        let pr = parse_source(source);
        let cm = CodeMap::from_parse_result(source, &pr);

        // Semicolon is code when there is no __END__
        let semi_offset = source.iter().position(|&b| b == b';').unwrap();
        assert!(
            cm.is_code(semi_offset),
            "Semicolon at offset {} should be code without __END__",
            semi_offset
        );
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        /// Generate a vec of sorted (start, end) ranges where start < end,
        /// capped at a reasonable universe size.
        fn sorted_ranges_strategy() -> impl Strategy<Value = Vec<(usize, usize)>> {
            prop::collection::vec((0usize..500, 1usize..100), 0..30).prop_map(|pairs| {
                let mut ranges: Vec<(usize, usize)> = pairs
                    .into_iter()
                    .map(|(start, len)| (start, start + len))
                    .collect();
                ranges.sort_unstable();
                ranges
            })
        }

        proptest! {
            #[test]
            fn merge_output_is_sorted(ranges in sorted_ranges_strategy()) {
                let merged = merge_ranges(ranges);
                for pair in merged.windows(2) {
                    prop_assert!(pair[0].0 < pair[1].0,
                        "merged starts not sorted: {:?} >= {:?}", pair[0], pair[1]);
                }
            }

            #[test]
            fn merge_output_is_non_overlapping(ranges in sorted_ranges_strategy()) {
                let merged = merge_ranges(ranges);
                for pair in merged.windows(2) {
                    prop_assert!(pair[0].1 <= pair[1].0,
                        "merged ranges overlap: {:?} and {:?}", pair[0], pair[1]);
                }
            }

            #[test]
            fn merge_preserves_coverage(ranges in sorted_ranges_strategy()) {
                let merged = merge_ranges(ranges.clone());
                // Every point in any input range must be in some merged range
                for &(start, end) in &ranges {
                    for pt in start..end {
                        prop_assert!(
                            merged.iter().any(|&(ms, me)| pt >= ms && pt < me),
                            "point {} from input range ({}, {}) not covered by merged output",
                            pt, start, end
                        );
                    }
                }
            }

            #[test]
            fn merge_does_not_expand_coverage(ranges in sorted_ranges_strategy()) {
                let merged = merge_ranges(ranges.clone());
                // Every point in a merged range must be in some input range
                for &(ms, me) in &merged {
                    for pt in ms..me {
                        prop_assert!(
                            ranges.iter().any(|&(s, e)| pt >= s && pt < e),
                            "point {} in merged range ({}, {}) not in any input range",
                            pt, ms, me
                        );
                    }
                }
            }

            #[test]
            fn merge_is_idempotent(ranges in sorted_ranges_strategy()) {
                let once = merge_ranges(ranges);
                let twice = merge_ranges(once.clone());
                prop_assert_eq!(once, twice);
            }

            #[test]
            fn in_ranges_consistent_with_merge(ranges in sorted_ranges_strategy()) {
                let merged = merge_ranges(ranges.clone());
                // Test a sample of offsets
                let max_offset = ranges.iter().map(|r| r.1).max().unwrap_or(0) + 10;
                for offset in (0..max_offset).step_by(3) {
                    let in_input = ranges.iter().any(|&(s, e)| offset >= s && offset < e);
                    let in_merged = CodeMap::in_ranges(&merged, offset);
                    prop_assert_eq!(in_input, in_merged,
                        "offset {} mismatch: in_input={}, in_merged={}", offset, in_input, in_merged);
                }
            }

            #[test]
            fn merge_output_length_leq_input(ranges in sorted_ranges_strategy()) {
                let merged = merge_ranges(ranges.clone());
                prop_assert!(merged.len() <= ranges.len() || ranges.is_empty());
            }
        }
    }
}
