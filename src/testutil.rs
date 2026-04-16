use ruby_prism::Visit;

use crate::cop::walker::CopWalker;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;

/// An expected offense parsed from a fixture annotation.
#[derive(Debug, Clone)]
pub struct ExpectedOffense {
    pub line: usize,
    pub column: usize,
    pub cop_name: String,
    pub message: String,
}

/// Result of parsing a fixture file, including source, expected offenses,
/// and optional directives like `# nitrocop-filename:`.
pub struct ParsedFixture {
    pub source: Vec<u8>,
    pub expected: Vec<ExpectedOffense>,
    pub filename: Option<String>,
}

struct RawAnnotation {
    column: usize,
    cop_name: String,
    message: String,
}

/// Try to parse an annotation line.
///
/// Annotation format: optional leading whitespace, then one or more `^` characters,
/// then a space, then `Department/CopName: Message`.
///
/// The column of the offense is the byte position of the first `^` in the line.
///
/// This intentionally rejects lines that merely contain `^` in other contexts
/// (e.g., Ruby XOR `x ^ y`, caret in strings) because:
/// - The `^` must be the first non-whitespace character
/// - Must be followed by ` Department/CopName: message` (with `/` and `: `)
fn try_parse_annotation(line: &str) -> Option<RawAnnotation> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('^') {
        return None;
    }

    let caret_count = trimmed.bytes().take_while(|&b| b == b'^').count();
    let after_carets = &trimmed[caret_count..];
    if !after_carets.starts_with(' ') {
        return None;
    }

    let rest = after_carets[1..].trim_end();
    let colon_space = rest.find(": ")?;
    let cop_name = &rest[..colon_space];
    let message = &rest[colon_space + 2..];

    // Cop names must contain '/' (e.g., Layout/Foo, Style/Bar)
    if !cop_name.contains('/') {
        return None;
    }

    // Column = byte position of first '^' in the original line
    let column = line.len() - trimmed.len();

    Some(RawAnnotation {
        column,
        cop_name: cop_name.to_string(),
        message: message.to_string(),
    })
}

/// Try to parse a `# nitrocop-filename: <name>` directive.
///
/// This directive overrides the filename passed to `SourceFile` when running
/// the cop. Only valid on the very first line of a fixture file.
fn try_parse_filename_directive(line: &str) -> Option<String> {
    line.strip_prefix("# nitrocop-filename: ")
        .map(|s| s.trim_end().to_string())
}

/// Try to parse a `# nitrocop-expect: L:C Department/CopName: Message` annotation.
///
/// Unlike `^` annotations (which infer line from position), these specify the
/// offense location directly. Use for offenses that can't be annotated with `^`
/// (e.g., trailing blanks, missing newlines).
fn try_parse_expect_annotation(line: &str) -> Option<ExpectedOffense> {
    let rest = line.strip_prefix("# nitrocop-expect: ")?;

    // Parse L:C
    let space_idx = rest.find(' ')?;
    let loc_part = &rest[..space_idx];
    let colon_idx = loc_part.find(':')?;
    let line_num: usize = loc_part[..colon_idx].parse().ok()?;
    let column: usize = loc_part[colon_idx + 1..].parse().ok()?;

    // Parse Department/CopName: message
    let after_loc = rest[space_idx + 1..].trim_end();
    let colon_space = after_loc.find(": ")?;
    let cop_name = &after_loc[..colon_space];
    let message = &after_loc[colon_space + 2..];

    if !cop_name.contains('/') {
        return None;
    }

    Some(ExpectedOffense {
        line: line_num,
        column,
        cop_name: cop_name.to_string(),
        message: message.to_string(),
    })
}

/// Parse fixture content into clean source bytes, expected offenses, and directives.
///
/// Supports three kinds of special lines (all stripped from the clean source):
///
/// - `# nitrocop-filename: <name>` (first line only) — overrides the test filename
/// - `# nitrocop-expect: L:C Department/CopName: Message` — explicit offense location
/// - `^ Department/CopName: Message` — positional annotation (after the source line)
///
/// # Convention
///
/// Positional (`^`) annotations must appear *after* the source line they reference.
/// The annotated line number is the count of source lines seen so far.
///
/// # Panics
///
/// Panics if a `^` annotation appears before any source line.
pub fn parse_fixture(raw: &[u8]) -> ParsedFixture {
    let text = std::str::from_utf8(raw).expect("fixture must be valid UTF-8");
    let elements: Vec<&str> = text.split('\n').collect();

    let mut source_lines: Vec<&str> = Vec::new();
    let mut expected: Vec<ExpectedOffense> = Vec::new();
    let mut filename: Option<String> = None;

    let mut start_idx = 0;
    if !elements.is_empty() {
        if let Some(name) = try_parse_filename_directive(elements[0]) {
            filename = Some(name);
            start_idx = 1;
        }
    }

    for (raw_idx, element) in elements.iter().enumerate().skip(start_idx) {
        // Check for # nitrocop-expect: annotations
        if let Some(expect) = try_parse_expect_annotation(element) {
            expected.push(expect);
            continue;
        }

        if let Some(annotation) = try_parse_annotation(element) {
            assert!(
                !source_lines.is_empty(),
                "Annotation on raw line {} appears before any source line. \
                 Annotations must follow the source line they reference.\n\
                 Line: {:?}",
                raw_idx + 1,
                element,
            );
            // Annotation refers to the last source line added
            let source_line_number = source_lines.len(); // 1-indexed
            expected.push(ExpectedOffense {
                line: source_line_number,
                column: annotation.column,
                cop_name: annotation.cop_name,
                message: annotation.message,
            });
        } else {
            source_lines.push(element);
        }
    }

    let clean = source_lines.join("\n");
    ParsedFixture {
        source: clean.into_bytes(),
        expected,
        filename,
    }
}

/// Run a cop on raw source bytes and return the diagnostics.
///
/// Use this for custom assertions where the standard `assert_cop_offenses`
/// helpers don't fit (e.g., checking severity, partial matching, or
/// testing cops that depend on raw byte layout like TrailingEmptyLines).
pub fn run_cop(cop: &dyn Cop, source_bytes: &[u8]) -> Vec<Diagnostic> {
    run_cop_with_config(cop, source_bytes, CopConfig::default())
}

/// Run a cop on raw source bytes with a specific config and return diagnostics.
pub fn run_cop_with_config(
    cop: &dyn Cop,
    source_bytes: &[u8],
    config: CopConfig,
) -> Vec<Diagnostic> {
    let source = SourceFile::from_bytes("test.rb", source_bytes.to_vec());
    let parse_result = crate::parse::parse_source(source.as_bytes());
    let mut diagnostics = Vec::new();
    if !parse_result.errors().next().is_some()
        || cop.should_run_line_checks_on_invalid_syntax(&config, &parse_result)
    {
        cop.check_lines(&source, &config, &mut diagnostics, None);
    }
    diagnostics
}

/// Run a cop on fixture bytes (with annotations) and assert offenses match.
pub fn assert_cop_offenses(cop: &dyn Cop, fixture_bytes: &[u8]) {
    assert_cop_offenses_with_config(cop, fixture_bytes, CopConfig::default());
}

/// Run a cop on fixture bytes with a specific config and assert offenses match.
///
/// Both expected and actual diagnostics are sorted by (line, column) before
/// comparison, so annotation order in the fixture doesn't need to match the
/// cop's emission order.
pub fn assert_cop_offenses_with_config(cop: &dyn Cop, fixture_bytes: &[u8], config: CopConfig) {
    let parsed = parse_fixture(fixture_bytes);
    let filename = parsed.filename.as_deref().unwrap_or("test.rb");
    let mut expected = parsed.expected;
    let source = SourceFile::from_bytes(filename, parsed.source);
    let parse_result = crate::parse::parse_source(source.as_bytes());
    let mut diagnostics = Vec::new();
    if !parse_result.errors().next().is_some()
        || cop.should_run_line_checks_on_invalid_syntax(&config, &parse_result)
    {
        cop.check_lines(&source, &config, &mut diagnostics, None);
    }

    // Sort both for order-independent comparison
    expected.sort_by_key(|e| (e.line, e.column));
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    assert_eq!(
        diagnostics.len(),
        expected.len(),
        "Expected {} offense(s) but got {}.\nExpected:\n{}\nActual:\n{}",
        expected.len(),
        diagnostics.len(),
        format_expected(&expected),
        format_diagnostics(&diagnostics),
    );

    for (i, (diag, exp)) in diagnostics.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            diag.location.line,
            exp.line,
            "Offense #{}: line mismatch (expected {} got {})\n  expected: {}:{} {}: {}\n  actual:   {d}",
            i + 1,
            exp.line,
            diag.location.line,
            exp.line,
            exp.column,
            exp.cop_name,
            exp.message,
            d = diag,
        );
        assert_eq!(
            diag.location.column,
            exp.column,
            "Offense #{}: column mismatch (expected {} got {})\n  expected: {}:{} {}: {}\n  actual:   {d}",
            i + 1,
            exp.column,
            diag.location.column,
            exp.line,
            exp.column,
            exp.cop_name,
            exp.message,
            d = diag,
        );
        assert_eq!(
            diag.cop_name,
            exp.cop_name,
            "Offense #{}: cop name mismatch\n  expected: {}\n  actual:   {}",
            i + 1,
            exp.cop_name,
            diag.cop_name,
        );
        assert_eq!(
            diag.message,
            exp.message,
            "Offense #{}: message mismatch for {}\n  expected: {:?}\n  actual:   {:?}",
            i + 1,
            exp.cop_name,
            exp.message,
            diag.message,
        );
    }
}

/// Assert a cop produces no offenses on the given source bytes.
pub fn assert_cop_no_offenses(cop: &dyn Cop, source_bytes: &[u8]) {
    assert_cop_no_offenses_with_config(cop, source_bytes, CopConfig::default());
}

/// Assert a cop produces no offenses on the given source bytes with a specific config.
pub fn assert_cop_no_offenses_with_config(cop: &dyn Cop, source_bytes: &[u8], config: CopConfig) {
    let source = SourceFile::from_bytes("test.rb", source_bytes.to_vec());
    let parse_result = crate::parse::parse_source(source.as_bytes());
    let mut diagnostics = Vec::new();
    if !parse_result.errors().next().is_some()
        || cop.should_run_line_checks_on_invalid_syntax(&config, &parse_result)
    {
        cop.check_lines(&source, &config, &mut diagnostics, None);
    }

    assert!(
        diagnostics.is_empty(),
        "Expected no offenses but got {}:\n{}",
        diagnostics.len(),
        format_diagnostics(&diagnostics),
    );
}

// ---- Full-pipeline helpers (check_lines + check_source + check_node walk) ----

/// Run all three cop methods on raw source bytes and return diagnostics.
pub fn run_cop_full(cop: &dyn Cop, source_bytes: &[u8]) -> Vec<Diagnostic> {
    run_cop_full_with_config(cop, source_bytes, CopConfig::default())
}

/// Run all three cop methods with a specific config and return diagnostics.
pub fn run_cop_full_with_config(
    cop: &dyn Cop,
    source_bytes: &[u8],
    config: CopConfig,
) -> Vec<Diagnostic> {
    run_cop_full_internal(cop, source_bytes, config, "test.rb")
}

/// Internal helper that runs all three cop methods with a configurable filename.
pub fn run_cop_full_internal(
    cop: &dyn Cop,
    source_bytes: &[u8],
    config: CopConfig,
    filename: &str,
) -> Vec<Diagnostic> {
    let source = SourceFile::from_bytes(filename, source_bytes.to_vec());
    let parse_result = crate::parse::parse_source(source.as_bytes());
    let code_map = CodeMap::from_parse_result(source.as_bytes(), &parse_result);
    let should_run_lines = !parse_result.errors().next().is_some()
        || cop.should_run_line_checks_on_invalid_syntax(&config, &parse_result);

    let mut diagnostics = Vec::new();

    // Line-based checks
    if should_run_lines {
        cop.check_lines(&source, &config, &mut diagnostics, None);
    }

    // Source-based checks
    cop.check_source(
        &source,
        &parse_result,
        &code_map,
        &config,
        &mut diagnostics,
        None,
    );

    // AST-based checks
    let mut walker = CopWalker {
        cop,
        source: &source,
        parse_result: &parse_result,
        cop_config: &config,
        diagnostics: Vec::new(),
        corrections: None,
    };
    walker.visit(&parse_result.node());
    diagnostics.extend(walker.diagnostics);

    // VariableForce engine
    if let Some(consumer) = cop.as_variable_force_consumer() {
        let registered = crate::cop::variable_force::engine::RegisteredConsumer {
            consumer,
            config: &config,
        };
        let consumers = [registered];
        let mut engine = crate::cop::variable_force::engine::Engine::new(&source, &consumers);
        engine.run(&parse_result);
        diagnostics.extend(engine.into_diagnostics());
    }

    diagnostics
}

/// Run all three cop methods on fixture bytes and assert offenses match.
pub fn assert_cop_offenses_full(cop: &dyn Cop, fixture_bytes: &[u8]) {
    assert_cop_offenses_full_with_config(cop, fixture_bytes, CopConfig::default());
}

/// Run all three cop methods with config on fixture bytes and assert offenses match.
pub fn assert_cop_offenses_full_with_config(
    cop: &dyn Cop,
    fixture_bytes: &[u8],
    config: CopConfig,
) {
    let parsed = parse_fixture(fixture_bytes);
    let filename = parsed.filename.as_deref().unwrap_or("test.rb");
    let mut expected = parsed.expected;
    let mut diagnostics = run_cop_full_internal(cop, &parsed.source, config, filename);

    expected.sort_by_key(|e| (e.line, e.column));
    diagnostics.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));

    assert_eq!(
        diagnostics.len(),
        expected.len(),
        "Expected {} offense(s) but got {}.\nExpected:\n{}\nActual:\n{}",
        expected.len(),
        diagnostics.len(),
        format_expected(&expected),
        format_diagnostics(&diagnostics),
    );

    for (i, (diag, exp)) in diagnostics.iter().zip(expected.iter()).enumerate() {
        assert_eq!(
            diag.location.line,
            exp.line,
            "Offense #{}: line mismatch (expected {} got {})\n  expected: {}:{} {}: {}\n  actual:   {d}",
            i + 1,
            exp.line,
            diag.location.line,
            exp.line,
            exp.column,
            exp.cop_name,
            exp.message,
            d = diag,
        );
        assert_eq!(
            diag.location.column,
            exp.column,
            "Offense #{}: column mismatch (expected {} got {})\n  expected: {}:{} {}: {}\n  actual:   {d}",
            i + 1,
            exp.column,
            diag.location.column,
            exp.line,
            exp.column,
            exp.cop_name,
            exp.message,
            d = diag,
        );
        assert_eq!(
            diag.cop_name,
            exp.cop_name,
            "Offense #{}: cop name mismatch\n  expected: {}\n  actual:   {}",
            i + 1,
            exp.cop_name,
            diag.cop_name,
        );
        assert_eq!(
            diag.message,
            exp.message,
            "Offense #{}: message mismatch for {}\n  expected: {:?}\n  actual:   {:?}",
            i + 1,
            exp.cop_name,
            exp.message,
            diag.message,
        );
    }
}

/// Assert a cop produces no offenses using the full pipeline.
pub fn assert_cop_no_offenses_full(cop: &dyn Cop, source_bytes: &[u8]) {
    assert_cop_no_offenses_full_with_config(cop, source_bytes, CopConfig::default());
}

/// Assert a cop produces no offenses using the full pipeline with config.
pub fn assert_cop_no_offenses_full_with_config(
    cop: &dyn Cop,
    source_bytes: &[u8],
    config: CopConfig,
) {
    let parsed = parse_fixture(source_bytes);
    let filename = parsed.filename.as_deref().unwrap_or("test.rb");
    let diagnostics = run_cop_full_internal(cop, &parsed.source, config, filename);

    assert!(
        diagnostics.is_empty(),
        "Expected no offenses but got {}:\n{}",
        diagnostics.len(),
        format_diagnostics(&diagnostics),
    );
}

// ---- Autocorrect testing helpers ----

/// Run all three cop methods with corrections enabled. Returns (diagnostics, corrections).
pub fn run_cop_autocorrect(
    cop: &dyn Cop,
    source_bytes: &[u8],
) -> (Vec<Diagnostic>, Vec<crate::correction::Correction>) {
    run_cop_autocorrect_with_config(cop, source_bytes, CopConfig::default())
}

/// Run all three cop methods with corrections enabled and a specific config.
pub fn run_cop_autocorrect_with_config(
    cop: &dyn Cop,
    source_bytes: &[u8],
    config: CopConfig,
) -> (Vec<Diagnostic>, Vec<crate::correction::Correction>) {
    run_cop_autocorrect_internal(cop, source_bytes, config, "test.rb")
}

/// Internal helper that runs all three cop methods with corrections enabled.
pub fn run_cop_autocorrect_internal(
    cop: &dyn Cop,
    source_bytes: &[u8],
    config: CopConfig,
    filename: &str,
) -> (Vec<Diagnostic>, Vec<crate::correction::Correction>) {
    let source = SourceFile::from_bytes(filename, source_bytes.to_vec());
    let parse_result = crate::parse::parse_source(source.as_bytes());
    let code_map = CodeMap::from_parse_result(source.as_bytes(), &parse_result);
    let should_run_lines = !parse_result.errors().next().is_some()
        || cop.should_run_line_checks_on_invalid_syntax(&config, &parse_result);

    let mut diagnostics = Vec::new();
    let mut corrections = Vec::new();

    // Line-based checks
    if should_run_lines {
        cop.check_lines(&source, &config, &mut diagnostics, Some(&mut corrections));
    }

    // Source-based checks
    cop.check_source(
        &source,
        &parse_result,
        &code_map,
        &config,
        &mut diagnostics,
        Some(&mut corrections),
    );

    // AST-based checks
    let mut walker = CopWalker {
        cop,
        source: &source,
        parse_result: &parse_result,
        cop_config: &config,
        diagnostics: Vec::new(),
        corrections: Some(Vec::new()),
    };
    walker.visit(&parse_result.node());
    diagnostics.extend(walker.diagnostics);
    if let Some(walker_corrections) = walker.corrections {
        corrections.extend(walker_corrections);
    }

    (diagnostics, corrections)
}

/// Assert that a cop's autocorrect produces the expected corrected source.
///
/// Takes the offense fixture (with ^ annotations), strips annotations to get
/// the input source, runs the cop with corrections, applies corrections, and
/// compares byte-for-byte against `expected_bytes`.
pub fn assert_cop_autocorrect(cop: &dyn Cop, fixture_bytes: &[u8], expected_bytes: &[u8]) {
    assert_cop_autocorrect_with_config(cop, fixture_bytes, expected_bytes, CopConfig::default());
}

/// Assert autocorrect with a specific config.
pub fn assert_cop_autocorrect_with_config(
    cop: &dyn Cop,
    fixture_bytes: &[u8],
    expected_bytes: &[u8],
    config: CopConfig,
) {
    let parsed = parse_fixture(fixture_bytes);
    let filename = parsed.filename.as_deref().unwrap_or("test.rb");
    let (_diagnostics, corrections) =
        run_cop_autocorrect_internal(cop, &parsed.source, config, filename);

    assert!(
        !corrections.is_empty(),
        "Cop {} produced no corrections — does it implement autocorrect?",
        cop.name(),
    );

    let correction_set = crate::correction::CorrectionSet::from_vec(corrections);
    let corrected = correction_set.apply(&parsed.source);

    if corrected != expected_bytes {
        let corrected_str = String::from_utf8_lossy(&corrected);
        let expected_str = String::from_utf8_lossy(expected_bytes);
        panic!(
            "Autocorrect output does not match expected.\n\
             === Expected ===\n{expected_str}\n\
             === Got ===\n{corrected_str}\n\
             === Diff ===\n{}",
            simple_diff(&expected_str, &corrected_str),
        );
    }
}

/// Simple line-by-line diff for test failure output.
fn simple_diff(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    let max_lines = exp_lines.len().max(act_lines.len());
    for i in 0..max_lines {
        let exp = exp_lines.get(i).unwrap_or(&"<missing>");
        let act = act_lines.get(i).unwrap_or(&"<missing>");
        if exp != act {
            out.push_str(&format!("  line {}: expected: {exp:?}\n", i + 1));
            out.push_str(&format!("  line {}:      got: {act:?}\n", i + 1));
        }
    }
    if out.is_empty() {
        if expected.len() != actual.len() {
            out.push_str(&format!(
                "  byte length differs: expected {} vs got {}",
                expected.len(),
                actual.len()
            ));
        }
    }
    out
}

fn format_expected(expected: &[ExpectedOffense]) -> String {
    expected
        .iter()
        .map(|e| format!("  {}:{} {}: {}", e.line, e.column, e.cop_name, e.message))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_diagnostics(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| format!("  {d}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Annotation parser unit tests ----

    #[test]
    fn parse_annotation_with_carets() {
        let ann = try_parse_annotation("     ^^^ Layout/Foo: some message").unwrap();
        assert_eq!(ann.column, 5);
        assert_eq!(ann.cop_name, "Layout/Foo");
        assert_eq!(ann.message, "some message");
    }

    #[test]
    fn parse_annotation_at_column_zero() {
        let ann = try_parse_annotation("^^^ Style/Bar: msg").unwrap();
        assert_eq!(ann.column, 0);
        assert_eq!(ann.cop_name, "Style/Bar");
        assert_eq!(ann.message, "msg");
    }

    #[test]
    fn parse_annotation_single_caret() {
        let ann = try_parse_annotation("^ Layout/X: m").unwrap();
        assert_eq!(ann.column, 0);
        assert_eq!(ann.cop_name, "Layout/X");
        assert_eq!(ann.message, "m");
    }

    #[test]
    fn parse_annotation_many_carets() {
        let ann = try_parse_annotation("^^^^^^^^^^ Layout/LineLength: Line is too long. [130/120]")
            .unwrap();
        assert_eq!(ann.column, 0);
        assert_eq!(ann.message, "Line is too long. [130/120]");
    }

    #[test]
    fn parse_annotation_message_with_special_chars() {
        let ann = try_parse_annotation("^^^ Style/Foo: Use `bar` instead of 'baz'.").unwrap();
        assert_eq!(ann.message, "Use `bar` instead of 'baz'.");
    }

    // ---- False-positive rejection tests ----

    #[test]
    fn rejects_non_annotation_lines() {
        assert!(try_parse_annotation("x = 1").is_none());
        assert!(try_parse_annotation("# just a comment").is_none());
        assert!(try_parse_annotation("").is_none());
        assert!(try_parse_annotation("   ").is_none());
    }

    #[test]
    fn rejects_ruby_xor_operator() {
        // Ruby XOR: x ^ y — caret is NOT the first non-whitespace char
        assert!(try_parse_annotation("x ^ y").is_none());
        assert!(try_parse_annotation("result = a ^ b").is_none());
    }

    #[test]
    fn rejects_carets_without_cop_name() {
        // Must have Department/Name format
        assert!(try_parse_annotation("^^^ no slash here").is_none());
        assert!(try_parse_annotation("^^^ justtext").is_none());
    }

    #[test]
    fn rejects_carets_without_space_after() {
        assert!(try_parse_annotation("^^^Layout/Foo: msg").is_none());
    }

    #[test]
    fn rejects_carets_without_colon_space() {
        assert!(try_parse_annotation("^^^ Layout/Foo msg").is_none());
        assert!(try_parse_annotation("^^^ Layout/Foo:msg").is_none());
    }

    #[test]
    fn rejects_ruby_regex_with_carets() {
        // /^foo/ — not an annotation because it starts with /
        assert!(try_parse_annotation("/^foo/").is_none());
    }

    #[test]
    fn rejects_caret_in_string() {
        assert!(try_parse_annotation("  puts \"^hello\"").is_none());
    }

    // ---- parse_fixture tests ----

    #[test]
    fn parse_fixture_strips_annotations() {
        let raw = b"x = 1\n     ^^^ Layout/Foo: msg\ny = 2\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"x = 1\ny = 2\n");
        assert_eq!(parsed.expected.len(), 1);
        assert_eq!(parsed.expected[0].line, 1);
        assert_eq!(parsed.expected[0].column, 5);
        assert_eq!(parsed.expected[0].cop_name, "Layout/Foo");
        assert_eq!(parsed.expected[0].message, "msg");
        assert!(parsed.filename.is_none());
    }

    #[test]
    fn parse_fixture_multiple_annotations_same_line() {
        let raw = b"line1\n^^^ A/B: m1\n  ^^^ C/D: m2\nline2\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"line1\nline2\n");
        assert_eq!(parsed.expected.len(), 2);
        // Both reference source line 1
        assert_eq!(parsed.expected[0].line, 1);
        assert_eq!(parsed.expected[0].column, 0);
        assert_eq!(parsed.expected[1].line, 1);
        assert_eq!(parsed.expected[1].column, 2);
    }

    #[test]
    fn parse_fixture_annotations_on_different_lines() {
        let raw = b"line1\n     ^^^ A/B: m1\nline2\n  ^^^ C/D: m2\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"line1\nline2\n");
        assert_eq!(parsed.expected.len(), 2);
        assert_eq!(parsed.expected[0].line, 1);
        assert_eq!(parsed.expected[1].line, 2);
    }

    #[test]
    fn parse_fixture_no_annotations() {
        let raw = b"x = 1\ny = 2\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"x = 1\ny = 2\n");
        assert!(parsed.expected.is_empty());
    }

    #[test]
    fn parse_fixture_no_trailing_newline() {
        let raw = b"x = 1\n     ^^^ A/B: m";
        let parsed = parse_fixture(raw);
        // Annotation is last, no trailing source line → no trailing newline
        assert_eq!(parsed.source, b"x = 1");
        assert_eq!(parsed.expected.len(), 1);
        assert_eq!(parsed.expected[0].line, 1);
    }

    #[test]
    fn parse_fixture_preserves_trailing_whitespace_in_source() {
        // Trailing spaces on source line must be preserved in clean output
        let raw = b"x = 1   \n        ^^^ Layout/Foo: msg\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"x = 1   \n");
        assert_eq!(parsed.expected.len(), 1);
        assert_eq!(parsed.expected[0].column, 8);
    }

    #[test]
    fn parse_fixture_empty_source_lines_preserved() {
        // Empty lines in source (e.g., blank lines) must be kept
        let raw = b"\n^^^ A/B: m\nx = 1\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"\nx = 1\n");
        assert_eq!(parsed.expected.len(), 1);
        assert_eq!(parsed.expected[0].line, 1); // the empty line
    }

    // ---- nitrocop-filename directive tests ----

    #[test]
    fn parse_fixture_filename_directive() {
        let raw = b"# nitrocop-filename: MyClass.rb\nx = 1\n^ A/B: msg\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.filename.as_deref(), Some("MyClass.rb"));
        assert_eq!(parsed.source, b"x = 1\n");
        assert_eq!(parsed.expected.len(), 1);
        assert_eq!(parsed.expected[0].line, 1);
    }

    #[test]
    fn parse_fixture_filename_not_on_first_line() {
        // # nitrocop-filename: on a non-first line is treated as a source line
        let raw = b"x = 1\n# nitrocop-filename: Foo.rb\n";
        let parsed = parse_fixture(raw);
        assert!(parsed.filename.is_none());
        assert_eq!(parsed.source, b"x = 1\n# nitrocop-filename: Foo.rb\n");
    }

    // ---- nitrocop-expect annotation tests ----

    #[test]
    fn parse_fixture_expect_annotation() {
        let raw = b"# nitrocop-expect: 1:0 A/B: msg\nx = 1\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"x = 1\n");
        assert_eq!(parsed.expected.len(), 1);
        assert_eq!(parsed.expected[0].line, 1);
        assert_eq!(parsed.expected[0].column, 0);
        assert_eq!(parsed.expected[0].cop_name, "A/B");
        assert_eq!(parsed.expected[0].message, "msg");
    }

    #[test]
    fn parse_fixture_expect_and_caret_mixed() {
        let raw = b"# nitrocop-expect: 2:0 A/B: m1\nx = 1\n^ C/D: m2\ny = 2\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.source, b"x = 1\ny = 2\n");
        assert_eq!(parsed.expected.len(), 2);
        // nitrocop-expect comes first
        assert_eq!(parsed.expected[0].line, 2);
        assert_eq!(parsed.expected[0].cop_name, "A/B");
        // caret annotation
        assert_eq!(parsed.expected[1].line, 1);
        assert_eq!(parsed.expected[1].cop_name, "C/D");
    }

    #[test]
    fn parse_fixture_filename_and_expect() {
        let raw = b"# nitrocop-filename: Bad.rb\n# nitrocop-expect: 1:0 A/B: msg\nx = 1\n";
        let parsed = parse_fixture(raw);
        assert_eq!(parsed.filename.as_deref(), Some("Bad.rb"));
        assert_eq!(parsed.source, b"x = 1\n");
        assert_eq!(parsed.expected.len(), 1);
        assert_eq!(parsed.expected[0].line, 1);
    }

    #[test]
    #[should_panic(expected = "Annotation on raw line 1 appears before any source line")]
    fn parse_fixture_annotation_before_source_panics() {
        let raw = b"^^^ A/B: should panic\nx = 1\n";
        parse_fixture(raw);
    }

    // ---- run_cop helper tests ----

    #[test]
    fn run_cop_returns_diagnostics() {
        use crate::cop::layout::trailing_whitespace::TrailingWhitespace;
        let diags = run_cop(&TrailingWhitespace, b"x = 1  \n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 1);
        assert_eq!(diags[0].location.column, 5);
        assert_eq!(diags[0].cop_name, "Layout/TrailingWhitespace");
    }

    #[test]
    fn run_cop_with_config_applies_config() {
        use crate::cop::layout::line_length::LineLength;
        use std::collections::HashMap;
        let mut options = HashMap::new();
        options.insert("Max".to_string(), serde_yml::Value::Number(10.into()));
        let config = CopConfig {
            options,
            ..CopConfig::default()
        };
        let diags = run_cop_full_with_config(&LineLength, b"short\nthis is longer\n", config);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].location.line, 2);
    }

    #[test]
    fn run_cop_no_offenses_returns_empty() {
        use crate::cop::layout::trailing_whitespace::TrailingWhitespace;
        let diags = run_cop(&TrailingWhitespace, b"x = 1\ny = 2\n");
        assert!(diags.is_empty());
    }

    // ---- assert helper tests ----

    #[test]
    fn assert_cop_offenses_with_config_works() {
        use crate::cop::layout::line_length::LineLength;
        use std::collections::HashMap;
        let mut options = HashMap::new();
        options.insert("Max".to_string(), serde_yml::Value::Number(10.into()));
        let config = CopConfig {
            options,
            ..CopConfig::default()
        };
        // "longer than ten" = 15 chars, exceeds Max:10, offense at column 10
        let fixture = b"short\nlonger than ten\n          ^^^^^ Layout/LineLength: Line is too long. [15/10]\n";
        assert_cop_offenses_full_with_config(&LineLength, fixture, config);
    }

    #[test]
    fn assert_cop_no_offenses_with_config_works() {
        use crate::cop::layout::line_length::LineLength;
        use std::collections::HashMap;
        let mut options = HashMap::new();
        options.insert("Max".to_string(), serde_yml::Value::Number(200.into()));
        let config = CopConfig {
            options,
            ..CopConfig::default()
        };
        assert_cop_no_offenses_full_with_config(&LineLength, b"short line\n", config);
    }
}

/// Assert that every `.rb` file in a fixture directory is referenced by a test.
///
/// Files named `offense.rb`, `no_offense.rb`, and `corrected.rb` are auto-discovered
/// by the `cop_fixture_tests!` / `cop_autocorrect_fixture_tests!` macros. Files in
/// an `offense/` subdirectory are discovered by `cop_scenario_fixture_tests!`. Any
/// other `.rb` file must be explicitly loaded via `include_bytes!` in a test — if
/// not, it is dead code that will never be tested.
///
/// Call this from the macro-generated test with the fixture directory path and the
/// path to the cop's Rust source file.
pub fn assert_no_orphaned_fixtures(fixture_dir: &str, cop_source_path: &str) {
    use std::path::Path;

    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = manifest.join("tests/fixtures").join(fixture_dir);
    if !dir.is_dir() {
        return;
    }

    // Read the cop's Rust source to check for include_bytes! references
    let cop_source = std::fs::read_to_string(manifest.join(cop_source_path)).unwrap_or_default();

    // Standard filenames auto-discovered by macros
    let macro_discovered: &[&str] = &["offense.rb", "no_offense.rb", "corrected.rb"];

    let mut orphans: Vec<String> = Vec::new();

    fn walk_rb(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk_rb(&p, out);
                } else if p.extension().is_some_and(|e| e == "rb") {
                    out.push(p);
                }
            }
        }
    }

    let mut rb_files = Vec::new();
    walk_rb(&dir, &mut rb_files);

    for rb_path in &rb_files {
        let filename = rb_path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("");
        let parent_name = rb_path
            .parent()
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_str()
            .unwrap_or("");

        // Standard names are auto-discovered by the macro
        if macro_discovered.contains(&filename) && parent_name != "offense" {
            continue;
        }

        // Files in offense/ subdirectory are loaded by cop_scenario_fixture_tests!
        // which passes the filename explicitly — check for it in source
        if parent_name == "offense" {
            if cop_source.contains(filename) {
                continue;
            }
        } else {
            // Non-standard filename — must appear in the cop source via include_bytes!
            if cop_source.contains(filename) {
                continue;
            }
        }

        orphans.push(rb_path.display().to_string());
    }

    if !orphans.is_empty() {
        orphans.sort();
        panic!(
            "Found {} orphaned fixture file(s) in tests/fixtures/{} \
             not referenced by any test in {}.\n\
             Each non-standard fixture must be loaded via include_bytes!() \
             in a #[test] function.\n\n{}\n",
            orphans.len(),
            fixture_dir,
            cop_source_path,
            orphans.join("\n"),
        );
    }
}
