use crate::cop::shared::node_type::GLOBAL_VARIABLE_READ_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/SpecialGlobalVars: Flags Perl-style global variables and suggests English equivalents.
///
/// FN fix: Added `$:` → `$LOAD_PATH`, `$"` → `$LOADED_FEATURES`, and `$=` → `$IGNORECASE`
/// to the perl_to_english/english_to_perl maps (they were missing entirely).
/// Also fixed message generation: builtin globals (`$LOAD_PATH`, `$LOADED_FEATURES`,
/// `$PROGRAM_NAME`) do not need `require 'English'` — they are always available in Ruby.
/// The "require 'English'" hint is now only appended for non-builtin English names.
pub struct SpecialGlobalVars;

fn perl_to_english(name: &[u8]) -> Option<&'static str> {
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
        b"$&" => Some("$MATCH"),
        b"$'" => Some("$POSTMATCH"),
        b"$`" => Some("$PREMATCH"),
        b"$+" => Some("$LAST_PAREN_MATCH"),
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

fn english_to_perl(name: &[u8]) -> Option<&'static str> {
    match name {
        b"$LOAD_PATH" => Some("$:"),
        b"$LOADED_FEATURES" => Some("$\""),
        b"$ERROR_INFO" => Some("$!"),
        b"$ERROR_POSITION" => Some("$@"),
        b"$FIELD_SEPARATOR" => Some("$;"),
        b"$OUTPUT_FIELD_SEPARATOR" => Some("$,"),
        b"$INPUT_RECORD_SEPARATOR" => Some("$/"),
        b"$OUTPUT_RECORD_SEPARATOR" => Some("$\\"),
        b"$INPUT_LINE_NUMBER" => Some("$."),
        b"$PROGRAM_NAME" => Some("$0"),
        b"$PROCESS_ID" => Some("$$"),
        b"$CHILD_STATUS" => Some("$?"),
        b"$LAST_MATCH_INFO" => Some("$~"),
        b"$MATCH" => Some("$&"),
        b"$POSTMATCH" => Some("$'"),
        b"$PREMATCH" => Some("$`"),
        b"$LAST_PAREN_MATCH" => Some("$+"),
        b"$LAST_READ_LINE" => Some("$_"),
        b"$DEFAULT_OUTPUT" => Some("$>"),
        b"$DEFAULT_INPUT" => Some("$<"),
        b"$IGNORECASE" => Some("$="),
        b"$ARGV" => Some("$*"),
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
                // Flag all English-style names, suggest perl equivalents
                if let Some(perl) = english_to_perl(var_name) {
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
                // Like use_perl_names but allows builtin English globals
                // ($LOAD_PATH, $LOADED_FEATURES, $PROGRAM_NAME).
                // Also flags perl names that have builtin equivalents ($:, $", $0)
                // and suggests the English builtin instead.
                if let Some(perl) = english_to_perl(var_name) {
                    let english_name = std::str::from_utf8(var_name).unwrap_or("$?");
                    if is_builtin_english(english_name) {
                        // Builtin English name — allowed
                        return;
                    }
                    // Non-builtin English name → suggest perl
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
                } else if let Some(english) = perl_to_english(var_name) {
                    if is_builtin_english(english) {
                        // Perl name for a builtin → suggest English builtin
                        let perl_name = std::str::from_utf8(var_name).unwrap_or("$?");
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        let mut diag = self.diagnostic(
                            source,
                            line,
                            column,
                            format!("Prefer `{}` over `{}`.", english, perl_name),
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
                    }
                    // Perl name for non-builtin → allowed
                }
            }
            _ => {
                // "use_english_names" (default): flag Perl-style names
                if let Some(english) = perl_to_english(var_name) {
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
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("use_perl_names".into()),
            )]),
            ..CopConfig::default()
        };
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
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("use_builtin_english_names".into()),
            )]),
            ..CopConfig::default()
        };
        assert_cop_offenses_full_with_config(
            &SpecialGlobalVars,
            include_bytes!(
                "../../../tests/fixtures/cops/style/special_global_vars/use_builtin_english_names_offense.rb"
            ),
            config,
        );
    }

    #[test]
    fn use_builtin_english_names_no_offense_fixture() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("use_builtin_english_names".into()),
            )]),
            ..CopConfig::default()
        };
        assert_cop_no_offenses_full_with_config(
            &SpecialGlobalVars,
            include_bytes!(
                "../../../tests/fixtures/cops/style/special_global_vars/use_builtin_english_names_no_offense.rb"
            ),
            config,
        );
    }

    #[test]
    fn use_perl_names_allows_perl() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("use_perl_names".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"puts $!\n";
        let diags = run_cop_full_with_config(&SpecialGlobalVars, source, config);
        assert!(
            diags.is_empty(),
            "Should allow perl-style var with use_perl_names"
        );
    }
}
