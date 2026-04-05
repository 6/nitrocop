use crate::cop::shared::node_type::{INTERPOLATED_STRING_NODE, STRING_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Corpus investigation (2026-03-18):
/// - FP (4): Double-quoted heredocs (`<<-"CODE"`) whose body contains backslash
///   sequences (e.g. `\n`, `\\u{...}`). RuboCop's regex `/\\/` exempts ANY heredoc
///   body with a backslash regardless of quote style, but nitrocop only checked for
///   backslashes in single-quoted heredocs. Fixed by applying the backslash check to
///   all quoted heredocs (both `'` and `"`).
/// - FN (15): Heredocs used as method arguments with additional args on the same line
///   (e.g. `process(<<~'END', option: true)`). When Prism wraps these as
///   `InterpolatedStringNode`, the byte range between `opening_loc` end and
///   `closing_loc` start includes the rest of the opening line (method args), not
///   just the heredoc body. This caused false backslash/interpolation matches from
///   the argument text. Fixed by iterating over the node's `parts` to get only the
///   actual heredoc body content.
pub struct RedundantHeredocDelimiterQuotes;

impl Cop for RedundantHeredocDelimiterQuotes {
    fn name(&self) -> &'static str {
        "Style/RedundantHeredocDelimiterQuotes"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[INTERPOLATED_STRING_NODE, STRING_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Check both StringNode (non-interpolated heredoc) and InterpolatedStringNode (heredoc with interp)
        let opening_loc = if let Some(s) = node.as_string_node() {
            s.opening_loc()
        } else if let Some(s) = node.as_interpolated_string_node() {
            s.opening_loc()
        } else {
            return;
        };

        let opening = match opening_loc {
            Some(loc) => loc,
            None => return,
        };

        let open_bytes = opening.as_slice();
        // Must be a heredoc: starts with <<
        if !open_bytes.starts_with(b"<<") {
            return;
        }

        // Check for quoted delimiter: <<~'EOS', <<-"EOS", <<"EOS", <<'EOS'
        // Skip backquote heredocs: <<~`EOS`
        let rest = &open_bytes[2..];
        // Strip optional ~ or -
        let rest = if rest.starts_with(b"~") || rest.starts_with(b"-") {
            &rest[1..]
        } else {
            rest
        };

        if rest.len() < 2 {
            return;
        }

        let quote_char = rest[0];
        if quote_char != b'\'' && quote_char != b'"' {
            return;
        }

        // Extract the delimiter name (between quotes).
        // Malformed heredocs (e.g. unterminated `<<"\rA`) may lack a closing quote,
        // so bail if the closing byte doesn't match the opening quote.
        if rest[rest.len() - 1] != quote_char {
            return;
        }
        let delim = &rest[1..rest.len() - 1];

        // If the delimiter contains any non-word character, quotes are required.
        // Unquoted heredoc identifiers must be valid Ruby identifiers (alphanumeric + underscore).
        // This matches RuboCop's /\W/ check on the delimiter.
        if delim.is_empty()
            || delim
                .iter()
                .any(|&b| !b.is_ascii_alphanumeric() && b != b'_')
        {
            return;
        }

        // RuboCop exempts heredocs whose body contains interpolation patterns
        // (#{, #@, #$) or backslash escapes. The check applies to both single-
        // and double-quoted delimiters.
        //
        // For InterpolatedStringNode heredocs, we must use the node's parts to
        // get the actual body content, not the byte range between opening_loc and
        // closing_loc, because the latter includes the rest of the opening line
        // (method args etc.) which may contain false-positive backslash/interpolation
        // matches.
        if let Some(s) = node.as_string_node() {
            let body = s.content_loc().as_slice();
            if body_has_interpolation_or_escape(body) {
                return;
            }
        } else if let Some(s) = node.as_interpolated_string_node() {
            // Check each part's raw source for interpolation/escape patterns.
            // Parts contain only the actual heredoc body, not trailing args.
            for part in s.parts().iter() {
                let part_bytes = if let Some(str_node) = part.as_string_node() {
                    str_node.content_loc().as_slice()
                } else if let Some(emb) = part.as_embedded_statements_node() {
                    // Embedded statements node means #{...} interpolation exists
                    let _ = emb;
                    return;
                } else if let Some(ev) = part.as_embedded_variable_node() {
                    // Embedded variable node means #@var or #$var interpolation
                    let _ = ev;
                    return;
                } else {
                    // Unknown part type — conservatively skip
                    return;
                };
                if body_has_interpolation_or_escape(part_bytes) {
                    return;
                }
            }
        }

        // Build the suggested replacement
        let prefix = &open_bytes[..open_bytes.len() - rest.len()];
        let prefix_str = String::from_utf8_lossy(prefix);
        let delim_str = String::from_utf8_lossy(delim);

        let (line, column) = source.offset_to_line_col(opening.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!(
                "Remove the redundant heredoc delimiter quotes, use `{}{}` instead.",
                prefix_str, delim_str
            ),
        ));
    }
}

/// Check if heredoc body bytes contain interpolation patterns or backslash escapes.
/// Matches RuboCop's `STRING_INTERPOLATION_OR_ESCAPED_CHARACTER_PATTERN = /#(\{|@|\$)|\\/.freeze`
fn body_has_interpolation_or_escape(body: &[u8]) -> bool {
    // Check for interpolation patterns: #{, #@, #$
    if body
        .windows(2)
        .any(|w| w == b"#{" || w == b"#@" || w == b"#$")
    {
        return true;
    }
    // Check for backslash — RuboCop exempts ALL quoted heredocs (single or double)
    // whose body contains a backslash, not just single-quoted ones.
    body.contains(&b'\\')
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        RedundantHeredocDelimiterQuotes,
        "cops/style/redundant_heredoc_delimiter_quotes"
    );
}
