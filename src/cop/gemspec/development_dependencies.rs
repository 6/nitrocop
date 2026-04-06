use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Gemspec/DevelopmentDependencies cop.
///
/// ## Variant: `EnforcedStyle: gemspec`
/// When style is `gemspec`, flags bare `gem 'name'` calls (single string-literal
/// argument, no version constraints) in `Gemfile` / `gems.rb` files. This matches
/// RuboCop's `(send _ :gem (str #forbidden_gem? ...))` NodePattern which only
/// captures sends with exactly one `(str)` argument — multi-arg calls like
/// `gem 'foo', '~> 1.0'` are not flagged.
///
/// ## Fixed (gemspec variant):
/// - **FN (48)**: Trailing `#` comments containing commas (e.g.,
///   `gem 'base64' # Ruby 3.3, removed...`) inflated `count_top_level_args`,
///   causing single-arg gem calls to appear multi-arg and be skipped.
///   Fix: strip trailing comments before counting arguments.
/// - **FP (26)**: `=begin`/`=end` block comments were not tracked, so gem calls
///   inside multi-line block comments were falsely flagged. Fix: skip lines
///   between `=begin` and `=end`.
pub struct DevelopmentDependencies;

impl Cop for DevelopmentDependencies {
    fn name(&self) -> &'static str {
        "Gemspec/DevelopmentDependencies"
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/*.gemspec", "**/Gemfile", "**/gems.rb"]
    }

    fn check_lines(
        &self,
        source: &SourceFile,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "Gemfile");
        let allowed_gems = config.get_string_array("AllowedGems").unwrap_or_default();

        // When style is "gemspec", flag `gem` calls in Gemfile/gems.rb
        if style == "gemspec" {
            let path = source.path_str();
            let filename = std::path::Path::new(path)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("");
            if filename != "Gemfile" && filename != "gems.rb" {
                return;
            }
            check_gem_calls(self, source, &allowed_gems, diagnostics);
            return;
        }

        // For "Gemfile" or "gems.rb" styles, flag add_development_dependency calls
        let lines: Vec<&[u8]> = source.lines().collect();
        let mut in_block_comment = false;
        for (line_idx, line) in lines.iter().enumerate() {
            let line_str = match std::str::from_utf8(line) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let trimmed = line_str.trim();
            // Track =begin/=end block comments
            if trimmed == "=begin" || trimmed.starts_with("=begin ") {
                in_block_comment = true;
                continue;
            }
            if in_block_comment {
                if trimmed == "=end" || trimmed.starts_with("=end ") {
                    in_block_comment = false;
                }
                continue;
            }
            if trimmed.starts_with('#') {
                continue;
            }
            if let Some(pos) = line_str.find(".add_development_dependency") {
                let after_method = &line_str[pos + ".add_development_dependency".len()..];
                // If the line has an unclosed paren, join continuation lines
                let joined;
                let effective_after = if has_unclosed_paren(after_method) {
                    joined = join_continuation_lines(after_method, &lines, line_idx);
                    joined.as_str()
                } else {
                    after_method
                };
                // Only flag when the first argument is a string literal (quoted).
                // Dynamic args like `dep.name` or bare variables should be skipped,
                // matching RuboCop's `(send _ :add_development_dependency (str ...) ...)`
                if !has_string_literal_arg(effective_after) {
                    continue;
                }
                // RuboCop's NodePattern is (send _ :add_development_dependency (str ...) _? _?)
                // which matches at most 3 total arguments (gem name + up to 2 version constraints).
                // Skip lines with more than 3 args to avoid false positives.
                if count_top_level_args(effective_after) > 3 {
                    continue;
                }
                if is_gem_allowed(after_method, &allowed_gems) {
                    continue;
                }
                diagnostics.push(self.diagnostic(
                    source,
                    line_idx + 1,
                    pos + 1, // skip the dot
                    format!("Specify development dependencies in `{style}` instead of gemspec."),
                ));
            }
        }
    }
}

/// Check if a string has an unclosed parenthesis (more opens than closes).
fn has_unclosed_paren(s: &str) -> bool {
    let mut depth: i32 = 0;
    let bytes = s.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        match bytes[pos] {
            b'\'' | b'"' => {
                let quote = bytes[pos];
                pos += 1;
                while pos < bytes.len() && bytes[pos] != quote {
                    pos += 1;
                }
                if pos < bytes.len() {
                    pos += 1;
                }
            }
            b'(' => {
                depth += 1;
                pos += 1;
            }
            b')' => {
                depth -= 1;
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    depth > 0
}

/// Join continuation lines until parens are balanced.
fn join_continuation_lines(after: &str, lines: &[&[u8]], current_idx: usize) -> String {
    let mut result = after.to_string();
    let mut depth: i32 = 0;
    for &b in after.as_bytes() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
    }
    if depth <= 0 {
        return result;
    }
    for line in lines.iter().skip(current_idx + 1) {
        if let Ok(s) = std::str::from_utf8(line) {
            result.push(' ');
            result.push_str(s.trim());
            for &b in s.as_bytes() {
                match b {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
            }
            if depth <= 0 {
                break;
            }
        }
    }
    result
}

/// Check if the first argument after the method call is a string literal.
/// Recognizes standard quotes ('...', "...") and percent string literals
/// (%q<...>, %Q(...), %[...], etc.) which parse to `(str ...)` in RuboCop's AST.
/// Excludes `.freeze` suffixed strings which are `(send (str ...) :freeze)` in AST,
/// not bare `(str ...)` nodes, so RuboCop's NodePattern doesn't match them.
fn has_string_literal_arg(after_method: &str) -> bool {
    let trimmed = after_method.trim_start();
    let trimmed = if let Some(stripped) = trimmed.strip_prefix('(') {
        stripped.trim_start()
    } else {
        trimmed
    };
    if trimmed.starts_with('\'') || trimmed.starts_with('"') {
        let quote = trimmed.as_bytes()[0];
        // Find end of string literal and check for .freeze
        if let Some(end) = trimmed[1..].find(|c: char| c as u8 == quote) {
            let after_string = &trimmed[end + 2..];
            if after_string.starts_with(".freeze") {
                return false;
            }
        }
        return true;
    }
    if is_percent_string(trimmed) {
        return !has_freeze_suffix(trimmed);
    }
    false
}

/// Check if the string starts with a Ruby percent string literal.
/// Matches: %q<...>, %Q<...>, %<...>, %(, %[, %{, etc.
fn is_percent_string(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'%') {
        return false;
    }
    if bytes.len() < 2 {
        return false;
    }
    let next = match bytes[1] {
        b'q' | b'Q' => {
            if bytes.len() < 3 {
                return false;
            }
            bytes[2]
        }
        other => other,
    };
    matches!(next, b'<' | b'(' | b'[' | b'{')
}

/// Check if a percent string literal has a `.freeze` suffix.
/// E.g., `%q<rails>.freeze` -> true, `%q<rails>` -> false.
fn has_freeze_suffix(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'%') || bytes.len() < 3 {
        return false;
    }
    let start = match bytes[1] {
        b'q' | b'Q' => 3,
        _ => 2,
    };
    if start > bytes.len() {
        return false;
    }
    let opener = bytes[start - 1];
    let closer = match opener {
        b'<' => b'>',
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return false,
    };
    // Find the closing delimiter
    if let Some(end) = s[start..].find(|c: char| c as u8 == closer) {
        let after = &s[start + end + 1..];
        after.starts_with(".freeze")
    } else {
        false
    }
}

/// Strip trailing Ruby `#` comment from a string, respecting quoted strings.
/// E.g., `'foo' # comment, here` -> `'foo' `, `"bar"#, path: ...` -> `"bar"`.
fn strip_trailing_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1; // skip escaped char
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // skip closing quote
                }
            }
            b'#' => return &s[..i],
            _ => i += 1,
        }
    }
    s
}

/// Count top-level arguments in a method call (commas not inside brackets/parens).
/// Returns the number of arguments (1 for a single arg, 2 for two, etc.).
fn count_top_level_args(after_method: &str) -> usize {
    let trimmed = after_method.trim_start();
    let content = if let Some(stripped) = trimmed.strip_prefix('(') {
        stripped
    } else {
        trimmed
    };
    let mut depth = 0usize;
    let mut count = 1;
    for ch in content.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            ',' if depth == 0 => count += 1,
            '\n' => break,
            _ => {}
        }
    }
    count
}

/// Extract the content of a percent string literal (e.g., `%q<erubis>` -> `erubis`).
fn extract_percent_string_content(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'%') || bytes.len() < 3 {
        return None;
    }
    let start = match bytes[1] {
        b'q' | b'Q' => 3,
        _ => 2,
    };
    if start > bytes.len() {
        return None;
    }
    let opener = bytes[start - 1];
    let closer = match opener {
        b'<' => b'>',
        b'(' => b')',
        b'[' => b']',
        b'{' => b'}',
        _ => return None,
    };
    let content = &s[start..];
    content
        .find(|c: char| c as u8 == closer)
        .map(|end| &content[..end])
}

/// Check `gem` calls in Gemfile/gems.rb for the `gemspec` enforced style.
/// RuboCop's pattern `(send _ :gem (str #forbidden_gem? ...))` only matches
/// calls with exactly one string-literal argument (no version constraints).
fn check_gem_calls(
    cop: &DevelopmentDependencies,
    source: &SourceFile,
    allowed_gems: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let lines: Vec<&[u8]> = source.lines().collect();
    let mut in_block_comment = false;
    for (line_idx, line) in lines.iter().enumerate() {
        let line_str = match std::str::from_utf8(line) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let trimmed = line_str.trim();
        // Track =begin/=end block comments
        if trimmed == "=begin" || trimmed.starts_with("=begin ") {
            in_block_comment = true;
            continue;
        }
        if in_block_comment {
            if trimmed == "=end" || trimmed.starts_with("=end ") {
                in_block_comment = false;
            }
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some((gem_pos, after)) = find_gem_call(line_str) {
            if !has_string_literal_arg(after) {
                continue;
            }
            // Strip trailing comment before counting args so commas in comments
            // don't inflate the argument count.
            let after_no_comment = strip_trailing_comment(after);
            // RuboCop's NodePattern only matches single-arg gem calls (no version constraints)
            if count_top_level_args(after_no_comment) > 1 {
                continue;
            }
            if is_gem_allowed(after, allowed_gems) {
                continue;
            }
            diagnostics.push(cop.diagnostic(
                source,
                line_idx + 1,
                gem_pos,
                "Specify development dependencies in `gemspec`.".to_string(),
            ));
        }
    }
}

/// Find a `gem` method call in a line. Returns (position_of_gem, text_after_gem).
/// Ensures `gem` is a whole word (not part of `gemspec`, `gems`, etc.) and
/// appears in method-call position — not inside a string, regex, or other literal.
/// RuboCop uses AST matching (`(send _ :gem ...)`), so only real send nodes count.
fn find_gem_call(line: &str) -> Option<(usize, &str)> {
    let bytes = line.as_bytes();
    let mut search_from = 0;
    loop {
        let idx = line[search_from..].find("gem")?;
        let pos = search_from + idx;

        // Word boundary check before
        if pos > 0 {
            let prev = bytes[pos - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                search_from = pos + 3;
                continue;
            }
            // Skip `gem` inside literals: if preceded by a non-whitespace,
            // non-statement character like /, ', ", it's inside a regex or string,
            // not a real method call. Real gem calls are at line start (after
            // whitespace) or after statement separators like ; or keywords.
            if !prev.is_ascii_whitespace() && prev != b';' {
                search_from = pos + 3;
                continue;
            }
        }
        // Word boundary check after
        let end = pos + 3;
        if end < bytes.len() {
            let next_byte = bytes[end];
            if next_byte.is_ascii_alphanumeric() || next_byte == b'_' {
                search_from = end;
                continue;
            }
        }

        let after = if end <= line.len() { &line[end..] } else { "" };
        return Some((pos, after));
    }
}

/// Check if the gem name following the method call is in the allowed list.
fn is_gem_allowed(after_method: &str, allowed_gems: &[String]) -> bool {
    if allowed_gems.is_empty() {
        return false;
    }
    // Try to extract gem name from patterns like:
    //   ('gem_name', ...) or  'gem_name' or "gem_name"
    let trimmed = after_method.trim_start();
    let trimmed = if let Some(stripped) = trimmed.strip_prefix('(') {
        stripped.trim_start()
    } else {
        trimmed
    };
    let gem_name = if trimmed.starts_with('\'') || trimmed.starts_with('"') {
        let quote = trimmed.as_bytes()[0];
        let rest = &trimmed[1..];
        rest.find(|c: char| c as u8 == quote)
            .map(|end| &rest[..end])
    } else if is_percent_string(trimmed) {
        extract_percent_string_content(trimmed)
    } else {
        None
    };
    if let Some(name) = gem_name {
        allowed_gems.iter().any(|g| g == name)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        DevelopmentDependencies,
        "cops/gemspec/development_dependencies"
    );

    fn gemspec_style_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("gemspec".to_string()),
        );
        options.insert(
            "AllowedGems".to_string(),
            serde_yml::Value::Sequence(vec![serde_yml::Value::String("allowed".to_string())]),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn gemspec_style_offense() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &DevelopmentDependencies,
            br#"# nitrocop-filename: Gemfile
gem 'example'
^^^ Gemspec/DevelopmentDependencies: Specify development dependencies in `gemspec`.
gem 'foo'
^^^ Gemspec/DevelopmentDependencies: Specify development dependencies in `gemspec`.
gem('bar')
^^^ Gemspec/DevelopmentDependencies: Specify development dependencies in `gemspec`.
"#,
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_no_offense_gemspec_file() {
        // In gemspec style, add_development_dependency in gemspec files is OK
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DevelopmentDependencies,
            b"# nitrocop-filename: example.gemspec\nGem::Specification.new do |s|\n  s.add_development_dependency 'foo'\nend\n",
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_no_offense_version_args() {
        // gem calls with version constraints are not flagged (RuboCop single-arg pattern)
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DevelopmentDependencies,
            b"# nitrocop-filename: Gemfile\ngem 'rails', '~> 7.0'\ngem 'puma', '>= 5.0'\n",
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_no_offense_allowed_gem() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DevelopmentDependencies,
            b"# nitrocop-filename: Gemfile\ngem 'allowed'\n",
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_no_offense_freeze() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DevelopmentDependencies,
            b"# nitrocop-filename: Gemfile\ngem 'rails'.freeze\n",
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_no_offense_dynamic_arg() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DevelopmentDependencies,
            b"# nitrocop-filename: Gemfile\ngem foo\n",
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_gems_rb() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &DevelopmentDependencies,
            br#"# nitrocop-filename: gems.rb
gem 'example'
^^^ Gemspec/DevelopmentDependencies: Specify development dependencies in `gemspec`.
"#,
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_no_offense_gem_in_string_or_regex() {
        // `gem` inside gsub_file regex/string args is not a real gem call
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DevelopmentDependencies,
            b"# nitrocop-filename: Gemfile\ngsub_file 'Gemfile', /gem 'sdoc',\\s+'~> 0.4.0'/, ''\n",
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_offense_trailing_comment_with_commas() {
        // gem calls with trailing comments containing commas should still be flagged
        // (RuboCop's AST ignores comments; commas in comments are not arguments)
        crate::testutil::assert_cop_offenses_full_with_config(
            &DevelopmentDependencies,
            br##"# nitrocop-filename: Gemfile
gem 'base64'     # Deprecation notice in Ruby 3.3, removed from default gems
^^^ Gemspec/DevelopmentDependencies: Specify development dependencies in `gemspec`.
gem "deploy"#, path: "../../deploy"
^^^ Gemspec/DevelopmentDependencies: Specify development dependencies in `gemspec`.
gem "ostruct" # for Ruby 4, probably safe to delete
^^^ Gemspec/DevelopmentDependencies: Specify development dependencies in `gemspec`.
"##,
            gemspec_style_config(),
        );
    }

    #[test]
    fn gemspec_style_no_offense_begin_end_block() {
        // gem calls inside =begin/=end block comments should not be flagged
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &DevelopmentDependencies,
            br##"# nitrocop-filename: Gemfile
=begin
gem 'commented_out'
gem 'also_commented'
=end
"##,
            gemspec_style_config(),
        );
    }
}
