use crate::cop::shared::node_type::GLOBAL_VARIABLE_READ_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/SpecialGlobalVars: Flags Perl-style global variables and suggests English equivalents.
///
/// RuboCop treats secondary English aliases like `$PID`, `$FS`, `$OFS`, `$RS`,
/// `$ORS`, and `$NR` as managed names for the non-default styles, but it does
/// not manage `$MATCH`, `$PREMATCH`, `$POSTMATCH`, or `$LAST_PAREN_MATCH` in
/// any style. The previous one-to-one reverse map missed the aliases and
/// over-reported the match globals, which only showed up under
/// `use_perl_names` / `use_builtin_english_names`.
pub struct SpecialGlobalVars;

fn english_style_preferred(name: &[u8]) -> Option<&'static str> {
    match name {
        b"$:" => Some("$LOAD_PATH"),
        b"$\"" => Some("$LOADED_FEATURES"),
        b"$!" => Some("$ERROR_INFO"),
        b"$@" => Some("$ERROR_POSITION"),
        b"$;" => Some("$FIELD_SEPARATOR"),
        b"$," => Some("$OUTPUT_FIELD_SEPARATOR"),
        b"$/" => Some("$INPUT_RECORD_SEPARATOR"),
        b"$\\" => Some("$OUTPUT_RECORD_SEPARATOR"),
        b"$." => Some("$INPUT_LINE_NUMBER"),
        b"$0" => Some("$PROGRAM_NAME"),
        b"$$" => Some("$PROCESS_ID"),
        b"$?" => Some("$CHILD_STATUS"),
        b"$~" => Some("$LAST_MATCH_INFO"),
        b"$_" => Some("$LAST_READ_LINE"),
        b"$>" => Some("$DEFAULT_OUTPUT"),
        b"$<" => Some("$DEFAULT_INPUT"),
        b"$=" => Some("$IGNORECASE"),
        b"$*" => Some("$ARGV"),
        _ => None,
    }
}

/// Returns true if the English name is a Ruby builtin global that does not
/// require `require 'English'` to be available.
fn is_builtin_english(english: &str) -> bool {
    matches!(english, "$LOAD_PATH" | "$LOADED_FEATURES" | "$PROGRAM_NAME")
}

fn perl_style_preferred(name: &[u8]) -> Option<&'static str> {
    match name {
        b"$:" | b"$LOAD_PATH" => Some("$:"),
        b"$\"" | b"$LOADED_FEATURES" => Some("$\""),
        b"$0" | b"$PROGRAM_NAME" => Some("$0"),
        b"$!" | b"$ERROR_INFO" => Some("$!"),
        b"$@" | b"$ERROR_POSITION" => Some("$@"),
        b"$;" | b"$FIELD_SEPARATOR" | b"$FS" => Some("$;"),
        b"$," | b"$OUTPUT_FIELD_SEPARATOR" | b"$OFS" => Some("$,"),
        b"$/" | b"$INPUT_RECORD_SEPARATOR" | b"$RS" => Some("$/"),
        b"$\\" | b"$OUTPUT_RECORD_SEPARATOR" | b"$ORS" => Some("$\\"),
        b"$." | b"$INPUT_LINE_NUMBER" | b"$NR" => Some("$."),
        b"$_" | b"$LAST_READ_LINE" => Some("$_"),
        b"$>" | b"$DEFAULT_OUTPUT" => Some("$>"),
        b"$<" | b"$DEFAULT_INPUT" => Some("$<"),
        b"$$" | b"$PROCESS_ID" | b"$PID" => Some("$$"),
        b"$?" | b"$CHILD_STATUS" => Some("$?"),
        b"$~" | b"$LAST_MATCH_INFO" => Some("$~"),
        b"$=" | b"$IGNORECASE" => Some("$="),
        b"$*" | b"$ARGV" => Some("$*"),
        _ => None,
    }
}

fn builtin_english_preferred(name: &[u8]) -> Option<&'static str> {
    match name {
        b"$:" | b"$LOAD_PATH" => Some("$LOAD_PATH"),
        b"$\"" | b"$LOADED_FEATURES" => Some("$LOADED_FEATURES"),
        b"$0" | b"$PROGRAM_NAME" => Some("$PROGRAM_NAME"),
        _ => None,
    }
}

impl Cop for SpecialGlobalVars {
    fn name(&self) -> &'static str {
        "Style/SpecialGlobalVars"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[GLOBAL_VARIABLE_READ_NODE]
    }

    fn supports_autocorrect(&self) -> bool {
        true
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
        let require_english = config.get_bool("RequireEnglish", true);
        let enforced_style = config.get_str("EnforcedStyle", "use_english_names");
        let gvar = match node.as_global_variable_read_node() {
            Some(g) => g,
            None => return,
        };

        let loc = gvar.location();
        let var_name = loc.as_slice();

        match enforced_style {
            "use_perl_names" => {
                if let Some(perl) = perl_style_preferred(var_name) {
                    if perl.as_bytes() == var_name {
                        return;
                    }
                    let english_name = std::str::from_utf8(var_name).unwrap_or("$?");
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        format!("Prefer `{}` over `{}`.", perl, english_name),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: loc.start_offset(),
                            end: loc.end_offset(),
                            replacement: perl.to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            "use_builtin_english_names" => {
                if let Some(english) = builtin_english_preferred(var_name) {
                    if english.as_bytes() == var_name {
                        return;
                    }
                    let global_name = std::str::from_utf8(var_name).unwrap_or("$?");
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        format!("Prefer `{}` over `{}`.", english, global_name),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: loc.start_offset(),
                            end: loc.end_offset(),
                            replacement: english.to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                } else if let Some(perl) = perl_style_preferred(var_name) {
                    if perl.as_bytes() == var_name {
                        return;
                    }
                    let english_name = std::str::from_utf8(var_name).unwrap_or("$?");
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    let mut diag = self.diagnostic(
                        source,
                        line,
                        column,
                        format!("Prefer `{}` over `{}`.", perl, english_name),
                    );
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: loc.start_offset(),
                            end: loc.end_offset(),
                            replacement: perl.to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
            _ => {
                // "use_english_names" (default): flag Perl-style names
                if let Some(english) = english_style_preferred(var_name) {
                    let perl_name = std::str::from_utf8(var_name).unwrap_or("$?");
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    let msg = if require_english && !is_builtin_english(english) {
                        format!(
                            "Prefer `{}` over `{}`. Use `require 'English'` to access it.",
                            english, perl_name
                        )
                    } else {
                        format!("Prefer `{}` over `{}`.", english, perl_name)
                    };
                    let mut diag = self.diagnostic(source, line, column, msg);
                    if let Some(ref mut corr) = corrections {
                        corr.push(crate::correction::Correction {
                            start: loc.start_offset(),
                            end: loc.end_offset(),
                            replacement: english.to_string(),
                            cop_name: self.name(),
                            cop_index: 0,
                        });
                        diag.corrected = true;
                    }
                    diagnostics.push(diag);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(SpecialGlobalVars, "cops/style/special_global_vars");
    crate::cop_autocorrect_fixture_tests!(SpecialGlobalVars, "cops/style/special_global_vars");

    fn enforced_style_config(style: &str) -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String(style.into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn regular_global_is_ignored() {
        let source = b"x = $foo\n";
        let diags = run_cop_full(&SpecialGlobalVars, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn multiple_perl_vars_all_flagged() {
        let source = b"puts $!\nputs $$\n";
        let diags = run_cop_full(&SpecialGlobalVars, source);
        assert_eq!(diags.len(), 2);
    }

    #[test]
    fn use_perl_names_flags_english() {
        use crate::testutil::run_cop_full_with_config;
        let config = enforced_style_config("use_perl_names");
        let source = b"puts $ERROR_INFO\n";
        let diags = run_cop_full_with_config(&SpecialGlobalVars, source, config);
        assert_eq!(
            diags.len(),
            1,
            "Should flag English-style var with use_perl_names"
        );
        assert!(
            diags[0].message.contains("$!"),
            "Should suggest perl equivalent"
        );
    }

    #[test]
    fn require_english_includes_require_hint() {
        // Default RequireEnglish is true, so message should include the require hint
        let source = b"puts $!\n";
        let diags = run_cop_full(&SpecialGlobalVars, source);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0].message.contains("require 'English'"),
            "Default (RequireEnglish: true) should include require hint"
        );
    }

    #[test]
    fn require_english_false_omits_hint() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([("RequireEnglish".into(), serde_yml::Value::Bool(false))]),
            ..CopConfig::default()
        };
        let source = b"puts $!\n";
        let diags = run_cop_full_with_config(&SpecialGlobalVars, source, config);
        assert_eq!(diags.len(), 1);
        assert!(
            !diags[0].message.contains("require 'English'"),
            "RequireEnglish: false should not include require hint"
        );
    }

    #[test]
    fn use_builtin_english_names_offense_fixture() {
        use crate::testutil::assert_cop_offenses_full_with_config;
        assert_cop_offenses_full_with_config(
            &SpecialGlobalVars,
            include_bytes!(
                "../../../tests/fixtures/cops/style/special_global_vars/use_builtin_english_names_offense.rb"
            ),
            enforced_style_config("use_builtin_english_names"),
        );
    }

    #[test]
    fn use_builtin_english_names_no_offense_fixture() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        assert_cop_no_offenses_full_with_config(
            &SpecialGlobalVars,
            include_bytes!(
                "../../../tests/fixtures/cops/style/special_global_vars/use_builtin_english_names_no_offense.rb"
            ),
            enforced_style_config("use_builtin_english_names"),
        );
    }

    #[test]
    fn use_perl_names_offense_fixture() {
        use crate::testutil::assert_cop_offenses_full_with_config;

        assert_cop_offenses_full_with_config(
            &SpecialGlobalVars,
            include_bytes!(
                "../../../tests/fixtures/cops/style/special_global_vars/use_perl_names_offense.rb"
            ),
            enforced_style_config("use_perl_names"),
        );
    }

    #[test]
    fn use_perl_names_no_offense_fixture() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;

        assert_cop_no_offenses_full_with_config(
            &SpecialGlobalVars,
            include_bytes!(
                "../../../tests/fixtures/cops/style/special_global_vars/use_perl_names_no_offense.rb"
            ),
            enforced_style_config("use_perl_names"),
        );
    }

    #[test]
    fn use_perl_names_allows_perl() {
        use crate::testutil::run_cop_full_with_config;
        let config = enforced_style_config("use_perl_names");
        let source = b"puts $!\n";
        let diags = run_cop_full_with_config(&SpecialGlobalVars, source, config);
        assert!(
            diags.is_empty(),
            "Should allow perl-style var with use_perl_names"
        );
    }
}
