use crate::cop::shared::node_type_groups;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;
use std::collections::HashSet;

type NodeRange = (usize, usize);

/// Corpus investigation (FP=120, FN=161):
/// Root cause: nitrocop reported one offense per string at the string's start position,
/// while RuboCop reports one offense per format token at the token's exact position.
/// For heredocs and interpolated strings, this caused offenses at the wrong line (FP)
/// and missed per-token offenses on content lines (FN).
///
/// Additionally, format context for unannotated tokens was incorrectly propagated to
/// string parts inside interpolated strings. RuboCop's `format_string_in_typical_context?`
/// checks only the immediate parent node, so str parts inside a dstr (even when the dstr
/// is a format arg) are NOT in format context. This matches RuboCop's conservative treatment
/// of unannotated tokens in interpolated format strings.
///
/// Fix: per-token reporting at exact source positions + format context only for top-level
/// string nodes (not propagated to parts of interpolated strings).
///
/// Corpus investigation (FP=26, FN=3):
/// Three root causes:
/// 1. Multi-line format-context strings (heredocs, %[] literals): In Parser gem, these become
///    dstr nodes, so str parts lose format context. Prism keeps them as StringNode, so nitrocop
///    incorrectly flagged unannotated tokens. Fix: skip format context for strings with newlines.
/// 2. `%#{var}s` pattern in literal text (single-quoted heredocs): The `#` was treated as a
///    printf flag, making `{var}` parse as a template token. Fix: negative lookbehind for `#`
///    before `{` in template token detection, matching RuboCop's `(?<!#)` in TEMPLATE_NAME regex.
/// 3. AllowedMethods too broad: `collect_all_string_offsets` recursively traversed into nested
///    CallNodes, suppressing strings whose nearest send ancestor was NOT the allowed method.
///    Fix: stop traversal at CallNode boundaries (`collect_shallow_string_offsets`), matching
///    RuboCop's `each_ancestor(:send).first` check.
///
/// Remaining corpus FN (2026-03): single-line heredoc receivers used with `%`, e.g.
/// `<<-'SQL' % [cols, vals]`. The previous multiline skip treated every multiline `StringNode`
/// in format context as losing format context, which was correct for multiline percent literals
/// and multiline heredocs (Parser treats those as `dstr`) but too broad for single-line
/// heredocs. RuboCop keeps single-line heredoc receivers as `str` in this context and still
/// reports `%s` tokens. Fix: only keep format context for heredoc receivers whose content is
/// a single line; multiline heredocs and percent literals still lose format context.
///
/// Plain multiline quoted strings (2026-03): Prism keeps `"line1\nline2 %{tok}"` as one
/// `StringNode`, but Parser splits it into `dstr` parts and RuboCop still visits each
/// `str` part in aggressive mode. The bug was treating continuation lines as always safe,
/// which hid real template-token offenses from corpus strings like Strong Migrations'
/// embedded code examples. Fix: keep multiline handling only for format-context-sensitive
/// checks (conservative mode and unannotated tokens), not for named-token detection.
/// The one exception is continuation-line parts whose raw source contains escaped newlines
/// before the token (for example `"...\n  details: \\n\\n%{explanation}"`): RuboCop's
/// `str` node `value` drops the trailing `%{...}` in that shape, so we skip that narrow
/// case to avoid a new FP.
///
/// Adjacent string literals in `format(...)` calls (2026-03): Prism gives the top-level
/// concatenation node and its first child string the same start offset. Tracking context
/// by start offset leaked format context onto the first child and falsely flagged `%d/%s`
/// in the first segment of a concatenated format string. Fix: track exact node spans
/// instead of start offsets.
///
/// Variant style divergence (2026-04): For `EnforcedStyle: template`, nitrocop was flagging
/// ALL annotated tokens (regardless of format type), but RuboCop only flags annotated tokens
/// with type 's' (matching the documentation example `%<foo>s`). RuboCop's
/// `correctable_sequence?` returns true only for type 's' when style is template, because
/// only `%s` annotated tokens can be autocorrected to template style. Other annotated types
/// like `%<foo>d` cannot be directly converted. Fix: track format type in `FormatToken`
/// and only flag annotated tokens with `format_type == 's'` in template style.
///
/// Additionally, for `EnforcedStyle: template`, nitrocop was flagging ALL unannotated tokens
/// (via `MaxUnannotatedPlaceholdersAllowed` check), but RuboCop only flags unannotated tokens
/// with type 's' in template style. Tokens like `%02x`, `%d`, `%f` are not correctable to
/// template style and are not flagged. Fix: filter unannotated tokens to only those with
/// `format_type == 's'` before checking against `MaxUnannotatedPlaceholdersAllowed` in
/// template style.
pub struct FormatStringToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenStyle {
    Annotated,
    Template,
    Unannotated,
}

struct FormatToken {
    style: TokenStyle,
    /// Byte offset within the content string where this token starts
    offset: usize,
    /// The format type character (e.g., 's', 'd', 'f') for annotated and unannotated tokens.
    /// Template tokens don't have a type character (stored as None).
    format_type: Option<u8>,
}

impl FormatStringToken {
    /// Find all format tokens in a string and return their styles and positions.
    /// Handles: annotated (%<name>s), template (%{name}), unannotated (%s),
    /// positional (%1$s), and template-with-flags (%-20.5{name}).
    fn find_tokens(s: &[u8]) -> Vec<FormatToken> {
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < s.len() {
            if s[i] == b'%' && i + 1 < s.len() {
                if s[i + 1] == b'%' {
                    i += 2;
                    continue;
                }
                let start = i;
                let mut j = i + 1;

                // Skip positional specifier: N$ (digits followed by $)
                let pos_start = j;
                while j < s.len() && s[j].is_ascii_digit() {
                    j += 1;
                }
                if j > pos_start && j < s.len() && s[j] == b'$' {
                    j += 1; // skip $
                } else {
                    j = pos_start; // reset if no $
                }

                // Skip flags: -, +, space, 0, #
                while j < s.len() && matches!(s[j], b'-' | b'+' | b' ' | b'0' | b'#') {
                    j += 1;
                }

                // Skip width: digits or * (with optional N$)
                if j < s.len() && s[j] == b'*' {
                    j += 1;
                    // Width from positional arg: *N$
                    let w_start = j;
                    while j < s.len() && s[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j > w_start && j < s.len() && s[j] == b'$' {
                        j += 1;
                    } else {
                        j = w_start; // no N$ after *, that's fine
                    }
                } else {
                    while j < s.len() && s[j].is_ascii_digit() {
                        j += 1;
                    }
                }

                // Skip precision: .digits or .* (with optional N$)
                if j < s.len() && s[j] == b'.' {
                    j += 1;
                    if j < s.len() && s[j] == b'*' {
                        j += 1;
                        let p_start = j;
                        while j < s.len() && s[j].is_ascii_digit() {
                            j += 1;
                        }
                        if j > p_start && j < s.len() && s[j] == b'$' {
                            j += 1;
                        } else {
                            j = p_start; // no N$ after .*, fine
                        }
                    } else {
                        while j < s.len() && s[j].is_ascii_digit() {
                            j += 1;
                        }
                    }
                }

                // Now check what follows: type letter, {name}, or <name>
                if j < s.len() && s[j] == b'<' {
                    // Annotated: %[N$][flags][width][.prec]<name>type
                    let mut k = j + 1;
                    let mut has_word_char = false;
                    while k < s.len() && (s[k].is_ascii_alphanumeric() || s[k] == b'_') {
                        has_word_char = true;
                        k += 1;
                    }
                    if has_word_char && k < s.len() && s[k] == b'>' {
                        k += 1;
                        // Optional trailing type after >
                        let format_type = if k < s.len() && is_format_type(s[k]) {
                            let t = s[k];
                            k += 1;
                            Some(t)
                        } else {
                            None
                        };
                        tokens.push(FormatToken {
                            style: TokenStyle::Annotated,
                            offset: start,
                            format_type,
                        });
                        i = k;
                        continue;
                    }
                } else if j < s.len() && s[j] == b'{' {
                    // Template: %[flags][width][.prec]{name}
                    // But NOT if preceded by '#' — that's Ruby interpolation #{...}
                    // matching RuboCop's (?<!#) negative lookbehind in TEMPLATE_NAME regex
                    if j > 0 && s[j - 1] == b'#' {
                        // Skip: this is %#{ which is Ruby interpolation, not a format template
                        i += 1;
                        continue;
                    }
                    let mut k = j + 1;
                    let mut has_word_char = false;
                    while k < s.len() && (s[k].is_ascii_alphanumeric() || s[k] == b'_') {
                        has_word_char = true;
                        k += 1;
                    }
                    if has_word_char && k < s.len() && s[k] == b'}' {
                        tokens.push(FormatToken {
                            style: TokenStyle::Template,
                            offset: start,
                            format_type: None,
                        });
                        i = k + 1;
                        continue;
                    }
                } else if j < s.len() && is_format_type(s[j]) {
                    // Unannotated: %[N$][flags][width][.prec]type
                    let format_type = s[j];
                    tokens.push(FormatToken {
                        style: TokenStyle::Unannotated,
                        offset: start,
                        format_type: Some(format_type),
                    });
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
        tokens
    }
}

fn is_format_type(b: u8) -> bool {
    matches!(
        b,
        b's' | b'd'
            | b'f'
            | b'g'
            | b'e'
            | b'x'
            | b'X'
            | b'o'
            | b'b'
            | b'B'
            | b'i'
            | b'u'
            | b'c'
            | b'p'
            | b'a'
            | b'A'
            | b'E'
            | b'G'
    )
}

impl Cop for FormatStringToken {
    fn name(&self) -> &'static str {
        "Style/FormatStringToken"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "annotated");
        let max_unannotated = config.get_usize("MaxUnannotatedPlaceholdersAllowed", 1);
        let mode = config.get_str("Mode", "aggressive");
        let allowed_methods = config.get_string_array("AllowedMethods");
        let allowed_patterns = config.get_string_array("AllowedPatterns");

        let mut visitor = FormatStringTokenVisitor {
            cop: self,
            source,
            diagnostics: Vec::new(),
            style: style.to_string(),
            max_unannotated,
            conservative: mode == "conservative",
            allowed_methods,
            allowed_patterns,
            format_context_ranges: HashSet::new(),
            allowed_method_string_ranges: HashSet::new(),
            inside_xstr_or_regexp: 0,
        };

        // First pass: collect string spans in format contexts and allowed method contexts
        let mut collector = FormatContextCollector {
            format_context_ranges: &mut visitor.format_context_ranges,
            allowed_method_string_ranges: &mut visitor.allowed_method_string_ranges,
            allowed_methods: &visitor.allowed_methods,
            allowed_patterns: &visitor.allowed_patterns,
        };
        collector.visit(&parse_result.node());

        // Second pass: check strings
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

/// Collects exact spans of string nodes that are in a format context
/// (first arg to format/sprintf/printf, or LHS of %).
struct FormatContextCollector<'a> {
    format_context_ranges: &'a mut HashSet<NodeRange>,
    allowed_method_string_ranges: &'a mut HashSet<NodeRange>,
    allowed_methods: &'a Option<Vec<String>>,
    allowed_patterns: &'a Option<Vec<String>>,
}

impl FormatContextCollector<'_> {
    fn is_allowed_method(&self, method_name: &str) -> bool {
        if let Some(methods) = self.allowed_methods {
            if methods.iter().any(|m| m == method_name) {
                return true;
            }
        }
        if let Some(patterns) = self.allowed_patterns {
            for pat in patterns {
                if method_name.contains(pat.as_str()) {
                    return true;
                }
            }
        }
        false
    }

    /// Collect the exact span of the top-level string/interpolated-string node only.
    /// Does NOT propagate to parts inside interpolated strings, matching RuboCop's
    /// `format_string_in_typical_context?` which only checks the immediate parent.
    fn collect_top_level_string_range(
        node: &ruby_prism::Node<'_>,
        ranges: &mut HashSet<NodeRange>,
    ) {
        if node_type_groups::is_any_string_node(node) {
            let location = node.location();
            ranges.insert((location.start_offset(), location.end_offset()));
        }
    }

    /// Collect exact string spans in a subtree, stopping at nested CallNode boundaries.
    /// This matches RuboCop's `use_allowed_method?` which checks `each_ancestor(:send).first`,
    /// meaning only the NEAREST send ancestor matters. Strings inside nested method calls
    /// have a different nearest send ancestor and should NOT be suppressed.
    fn collect_shallow_string_ranges(node: &ruby_prism::Node<'_>, ranges: &mut HashSet<NodeRange>) {
        if node_type_groups::is_any_string_node(node) {
            let location = node.location();
            ranges.insert((location.start_offset(), location.end_offset()));
        }
        struct ShallowStringCollector<'a> {
            ranges: &'a mut HashSet<NodeRange>,
        }
        impl<'pr> Visit<'pr> for ShallowStringCollector<'_> {
            fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
                let location = node.location();
                self.ranges
                    .insert((location.start_offset(), location.end_offset()));
                ruby_prism::visit_string_node(self, node);
            }
            fn visit_interpolated_string_node(
                &mut self,
                node: &ruby_prism::InterpolatedStringNode<'pr>,
            ) {
                let location = node.location();
                self.ranges
                    .insert((location.start_offset(), location.end_offset()));
                ruby_prism::visit_interpolated_string_node(self, node);
            }
            fn visit_call_node(&mut self, _node: &ruby_prism::CallNode<'pr>) {
                // Stop recursion at nested call nodes: strings inside nested
                // method calls have that call as their nearest send ancestor,
                // so AllowedMethods should not suppress them.
            }
        }
        let mut sc = ShallowStringCollector { ranges };
        sc.visit(node);
    }
}

impl<'pr> Visit<'pr> for FormatContextCollector<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = node.name();
        let method_name = std::str::from_utf8(name.as_slice()).unwrap_or("");

        // Check if this is a format method: format, sprintf, printf
        if matches!(method_name, "format" | "sprintf" | "printf") {
            // The first argument is the format string - only mark the top-level node
            if let Some(args) = node.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                if !arg_list.is_empty() {
                    Self::collect_top_level_string_range(&arg_list[0], self.format_context_ranges);
                }
            }
        }

        // Check if this is the % operator: "string" % args
        if method_name == "%" {
            if let Some(receiver) = node.receiver() {
                Self::collect_top_level_string_range(&receiver, self.format_context_ranges);
            }
        }

        // Check if any ancestor method is in AllowedMethods
        if self.is_allowed_method(method_name) {
            // Suppress strings that are direct args (or in non-call subtrees like hashes/arrays),
            // but NOT strings nested inside other method calls (their nearest send differs).
            if let Some(args) = node.arguments() {
                for arg in args.arguments().iter() {
                    Self::collect_shallow_string_ranges(&arg, self.allowed_method_string_ranges);
                }
            }
        }

        ruby_prism::visit_call_node(self, node);
    }
}

struct FormatStringTokenVisitor<'a> {
    cop: &'a FormatStringToken,
    source: &'a SourceFile,
    diagnostics: Vec<Diagnostic>,
    style: String,
    max_unannotated: usize,
    conservative: bool,
    allowed_methods: Option<Vec<String>>,
    allowed_patterns: Option<Vec<String>>,
    /// Exact spans of top-level strings that are in a format context (format/sprintf/printf/%)
    format_context_ranges: HashSet<NodeRange>,
    /// Exact spans of strings that are args to allowed methods
    allowed_method_string_ranges: HashSet<NodeRange>,
    /// Depth counter for xstr/regexp contexts (skip strings inside these)
    inside_xstr_or_regexp: usize,
}

impl FormatStringTokenVisitor<'_> {
    fn loses_format_context_when_multiline(node: &ruby_prism::StringNode<'_>) -> bool {
        let content = node.content_loc().as_slice();
        if !content.contains(&b'\n') {
            return false;
        }

        let Some(opening) = node.opening_loc() else {
            // Bare StringNode parts only appear inside a larger dstr-like construct.
            return true;
        };
        let opening = opening.as_slice();

        if opening.starts_with(b"<<") {
            // Parser keeps single-line heredocs as `str` here, but multiline heredocs
            // become `dstr`, so their parts lose format context.
            let newline_count = content.iter().filter(|&&b| b == b'\n').count();
            return newline_count > 1;
        }

        if opening.starts_with(b"%") {
            return true;
        }

        true
    }

    fn is_plain_multiline_quoted_string(node: &ruby_prism::StringNode<'_>) -> bool {
        let content = node.content_loc().as_slice();
        if !content.contains(&b'\n') {
            return false;
        }

        let Some(opening) = node.opening_loc() else {
            return false;
        };
        let opening = opening.as_slice();
        !opening.starts_with(b"<<") && !opening.starts_with(b"%")
    }

    fn line_contains_escaped_newline_before_token(content: &[u8], token_offset: usize) -> bool {
        let line_start = content[..token_offset]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |idx| idx + 1);
        let line_prefix = &content[line_start..token_offset];

        line_prefix
            .windows(2)
            .any(|pair| matches!(pair, [b'\\', b'n'] | [b'\\', b'r']))
    }

    fn check_string_content(
        &mut self,
        content: &[u8],
        content_start_offset: usize,
        in_format_context: bool,
        plain_multiline_quoted: bool,
    ) {
        let content_str = match std::str::from_utf8(content) {
            Ok(s) => s,
            Err(_) => return,
        };

        if !content_str.contains('%') {
            return;
        }

        let tokens = FormatStringToken::find_tokens(content);
        if tokens.is_empty() {
            return;
        }

        // Separate tokens by style
        let unannotated: Vec<&FormatToken> = tokens
            .iter()
            .filter(|t| t.style == TokenStyle::Unannotated)
            .collect();
        let named: Vec<&FormatToken> = tokens
            .iter()
            .filter(|t| t.style != TokenStyle::Unannotated)
            .collect();

        // Per RuboCop: unannotated tokens are always treated conservatively.
        // Only flag when the string is directly in a format context (not parts of dstr).
        let check_unannotated = in_format_context;
        let check_named = if self.conservative {
            in_format_context
        } else {
            true
        };
        let (first_line, _) = self.source.offset_to_line_col(content_start_offset);

        match self.style.as_str() {
            "annotated" => {
                // Flag template tokens
                if check_named {
                    for tok in &named {
                        if tok.style == TokenStyle::Template {
                            let (line, column) = self
                                .source
                                .offset_to_line_col(content_start_offset + tok.offset);
                            if plain_multiline_quoted
                                && line > first_line
                                && Self::line_contains_escaped_newline_before_token(
                                    content, tok.offset,
                                )
                            {
                                continue;
                            }
                            self.diagnostics.push(self.cop.diagnostic(
                                self.source,
                                line,
                                column,
                                "Prefer annotated tokens (like `%<foo>s`) over template tokens (like `%{foo}`).".to_string(),
                            ));
                        }
                    }
                }
                // Flag unannotated tokens (only if count exceeds max AND in format context)
                if check_unannotated && unannotated.len() > self.max_unannotated {
                    // RuboCop reports one offense per unannotated token
                    for tok in &unannotated {
                        let (line, column) = self
                            .source
                            .offset_to_line_col(content_start_offset + tok.offset);
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            "Prefer annotated tokens (like `%<foo>s`) over unannotated tokens (like `%s`).".to_string(),
                        ));
                    }
                }
            }
            "template" => {
                if check_named {
                    for tok in &named {
                        // For template style, only flag annotated tokens with type 's'.
                        // RuboCop's correctable_sequence? returns true only for type 's'
                        // when style is template, matching the documentation example
                        // which only shows %<foo>s (not %<foo>d or other types) as bad.
                        if tok.style == TokenStyle::Annotated && tok.format_type == Some(b's') {
                            let (line, column) = self
                                .source
                                .offset_to_line_col(content_start_offset + tok.offset);
                            if plain_multiline_quoted
                                && line > first_line
                                && Self::line_contains_escaped_newline_before_token(
                                    content, tok.offset,
                                )
                            {
                                continue;
                            }
                            self.diagnostics.push(self.cop.diagnostic(
                                self.source,
                                line,
                                column,
                                "Prefer template tokens (like `%{foo}`) over annotated tokens (like `%<foo>s`).".to_string(),
                            ));
                        }
                    }
                }
                // In template style, only flag unannotated tokens with type 's'.
                // RuboCop's correctable_sequence? returns true only for type 's'
                // when style is template, because only %s can be autocorrected to %{foo}.
                let unannotated_s: Vec<&FormatToken> = unannotated
                    .iter()
                    .filter(|t| t.format_type == Some(b's'))
                    .copied()
                    .collect();
                if check_unannotated && unannotated_s.len() > self.max_unannotated {
                    for tok in unannotated_s {
                        let (line, column) = self
                            .source
                            .offset_to_line_col(content_start_offset + tok.offset);
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            "Prefer template tokens (like `%{foo}`) over unannotated tokens (like `%s`).".to_string(),
                        ));
                    }
                }
            }
            "unannotated" => {
                if check_named {
                    for tok in &named {
                        let (line, column) = self
                            .source
                            .offset_to_line_col(content_start_offset + tok.offset);
                        if plain_multiline_quoted
                            && line > first_line
                            && Self::line_contains_escaped_newline_before_token(content, tok.offset)
                        {
                            continue;
                        }
                        let msg = if tok.style == TokenStyle::Annotated {
                            "Prefer unannotated tokens (like `%s`) over annotated tokens (like `%<foo>s`)."
                        } else {
                            "Prefer unannotated tokens (like `%s`) over template tokens (like `%{foo}`)."
                        };
                        self.diagnostics.push(self.cop.diagnostic(
                            self.source,
                            line,
                            column,
                            msg.to_string(),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

impl<'pr> Visit<'pr> for FormatStringTokenVisitor<'_> {
    fn visit_string_node(&mut self, node: &ruby_prism::StringNode<'pr>) {
        // Skip strings inside xstr (backticks) or regexp, matching RuboCop's
        // format_string_token? which checks node.each_ancestor(:xstr, :regexp).any?
        if self.inside_xstr_or_regexp > 0 {
            return;
        }

        let location = node.location();
        let range = (location.start_offset(), location.end_offset());

        // Skip if this string is an argument to an allowed method
        if self.allowed_method_string_ranges.contains(&range) {
            return;
        }

        let raw_format_context = self.format_context_ranges.contains(&range);

        // Use content_loc for positional mapping (raw source bytes)
        let content_loc = node.content_loc();
        let content = content_loc.as_slice();

        // Some multiline Prism StringNodes correspond to Parser dstr nodes whose parts lose
        // format context. Keep single-line heredoc receivers in format context.
        let in_format_context =
            raw_format_context && !Self::loses_format_context_when_multiline(node);
        let plain_multiline_quoted = Self::is_plain_multiline_quoted_string(node);
        let content_start = content_loc.start_offset();

        self.check_string_content(
            content,
            content_start,
            in_format_context,
            plain_multiline_quoted,
        );
    }

    fn visit_interpolated_x_string_node(
        &mut self,
        node: &ruby_prism::InterpolatedXStringNode<'pr>,
    ) {
        self.inside_xstr_or_regexp += 1;
        ruby_prism::visit_interpolated_x_string_node(self, node);
        self.inside_xstr_or_regexp -= 1;
    }

    fn visit_x_string_node(&mut self, node: &ruby_prism::XStringNode<'pr>) {
        self.inside_xstr_or_regexp += 1;
        ruby_prism::visit_x_string_node(self, node);
        self.inside_xstr_or_regexp -= 1;
    }

    fn visit_interpolated_regular_expression_node(
        &mut self,
        node: &ruby_prism::InterpolatedRegularExpressionNode<'pr>,
    ) {
        self.inside_xstr_or_regexp += 1;
        ruby_prism::visit_interpolated_regular_expression_node(self, node);
        self.inside_xstr_or_regexp -= 1;
    }

    fn visit_regular_expression_node(&mut self, node: &ruby_prism::RegularExpressionNode<'pr>) {
        self.inside_xstr_or_regexp += 1;
        ruby_prism::visit_regular_expression_node(self, node);
        self.inside_xstr_or_regexp -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(FormatStringToken, "cops/style/format_string_token");
}
