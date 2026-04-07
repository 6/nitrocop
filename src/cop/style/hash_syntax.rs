use crate::cop::shared::node_type::{
    ASSOC_NODE, HASH_NODE, IMPLICIT_NODE, KEYWORD_HASH_NODE, LOCAL_VARIABLE_READ_NODE, SYMBOL_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Style/HashSyntax: checks hash literal syntax (rocket vs ruby19).
///
/// Fixed: quoted symbol keys like `:"chef version"` and interpolated symbol keys
/// like `:"#{field}_string"` were incorrectly treated as unconvertible. Prism
/// parses the latter as `InterpolatedSymbolNode`, but RuboCop's `any_sym_type?`
/// treats both forms as symbol keys. The cop now accepts both plain and
/// interpolated quoted symbols when deciding whether `=>` can become Ruby 1.9
/// label syntax on Ruby >= 2.2.
///
/// Fixed: quoted symbol keys already in 1.9 syntax (e.g. `"font-variant":`)
/// have Prism opening `"` or `'` (without `:` prefix), unlike rocket-syntax
/// `:"key" =>` which has opening `:"`. The `is_acceptable_19_symbol` check
/// now recognizes both forms, so hashes mixing 1.9-style and rocket-style
/// quoted symbol keys correctly flag only the rocket entries.
pub struct HashSyntax;

impl Cop for HashSyntax {
    fn name(&self) -> &'static str {
        "Style/HashSyntax"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            HASH_NODE,
            IMPLICIT_NODE,
            KEYWORD_HASH_NODE,
            LOCAL_VARIABLE_READ_NODE,
            SYMBOL_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Handle both explicit hashes `{ k: v }` and implicit keyword hashes `foo(k: v)`
        let elements: Vec<ruby_prism::Node<'_>> = if let Some(hash_node) = node.as_hash_node() {
            hash_node.elements().iter().collect()
        } else if let Some(kw_hash) = node.as_keyword_hash_node() {
            kw_hash.elements().iter().collect()
        } else {
            return;
        };

        let enforced_style = config.get_str("EnforcedStyle", "ruby19");
        let enforced_shorthand = config.get_str("EnforcedShorthandSyntax", "either");
        let use_rockets_symbol_vals = config.get_bool("UseHashRocketsWithSymbolValues", false);
        let prefer_rockets_nonalnum =
            config.get_bool("PreferHashRocketsForNonAlnumEndingSymbols", false);
        let target_ruby_version = target_ruby_version(config);

        // EnforcedShorthandSyntax: check Ruby 3.1 hash value omission syntax
        // This is checked separately from the main EnforcedStyle
        if enforced_shorthand != "either" {
            let mut shorthand_diags = Vec::new();
            check_shorthand_syntax(
                self,
                source,
                &elements,
                enforced_shorthand,
                &mut shorthand_diags,
            );
            if !shorthand_diags.is_empty() {
                diagnostics.extend(shorthand_diags);
                return;
            }
        }

        match enforced_style {
            "ruby19" => {
                // UseHashRocketsWithSymbolValues: if any value is a symbol, don't flag rockets
                if use_rockets_symbol_vals {
                    let has_symbol_value = elements.iter().any(|elem| {
                        if let Some(assoc) = elem.as_assoc_node() {
                            assoc.value().as_symbol_node().is_some()
                        } else {
                            false
                        }
                    });
                    if has_symbol_value {
                        return;
                    }
                }

                let has_unconvertible = elements.iter().any(|elem| {
                    let assoc = match elem.as_assoc_node() {
                        Some(a) => a,
                        None => return false,
                    };
                    let key = assoc.key();
                    !is_symbol_like_key(&key)
                        || !is_acceptable_19_key(&key, prefer_rockets_nonalnum, target_ruby_version)
                });

                if has_unconvertible {
                    return;
                }

                let mut diags = Vec::new();
                for elem in &elements {
                    let assoc = match elem.as_assoc_node() {
                        Some(a) => a,
                        None => continue,
                    };
                    let key = assoc.key();
                    if is_symbol_like_key(&key) {
                        if let Some(op_loc) = assoc.operator_loc() {
                            if op_loc.as_slice() == b"=>" {
                                let (line, column) =
                                    source.offset_to_line_col(key.location().start_offset());
                                diags.push(self.diagnostic(
                                    source,
                                    line,
                                    column,
                                    "Use the new Ruby 1.9 hash syntax.".to_string(),
                                ));
                            }
                        }
                    }
                }
                diagnostics.extend(diags);
            }
            "ruby19_no_mixed_keys" => {
                // UseHashRocketsWithSymbolValues: if any value is a symbol, don't flag rockets
                if use_rockets_symbol_vals {
                    let has_symbol_value = elements.iter().any(|elem| {
                        if let Some(assoc) = elem.as_assoc_node() {
                            assoc.value().as_symbol_node().is_some()
                        } else {
                            false
                        }
                    });
                    if has_symbol_value {
                        return;
                    }
                }

                // Check if any key is NOT a symbol (non-symbol keys can't use 1.9 at all)
                let has_non_symbol = elements.iter().any(|elem| {
                    let assoc = match elem.as_assoc_node() {
                        Some(a) => a,
                        None => return false,
                    };
                    let key = assoc.key();
                    !is_symbol_like_key(&key)
                });

                if has_non_symbol {
                    // Non-symbol keys make the hash unconvertible to 1.9, return early
                    return;
                }

                // Check if all keys are acceptable 1.9 symbols (sym_indices? equivalent)
                let all_acceptable = elements.iter().all(|elem| {
                    let assoc = match elem.as_assoc_node() {
                        Some(a) => a,
                        None => return false,
                    };
                    let key = assoc.key();
                    is_symbol_like_key(&key)
                        && is_acceptable_19_key(&key, prefer_rockets_nonalnum, target_ruby_version)
                });

                if all_acceptable {
                    // All keys are acceptable 1.9 symbols - flag rockets
                    let mut diags = Vec::new();
                    for elem in &elements {
                        let assoc = match elem.as_assoc_node() {
                            Some(a) => a,
                            None => continue,
                        };
                        let key = assoc.key();
                        if is_symbol_like_key(&key) {
                            if let Some(op_loc) = assoc.operator_loc() {
                                if op_loc.as_slice() == b"=>" {
                                    let (line, column) =
                                        source.offset_to_line_col(key.location().start_offset());
                                    diags.push(self.diagnostic(
                                        source,
                                        line,
                                        column,
                                        "Use the new Ruby 1.9 hash syntax.".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    diagnostics.extend(diags);
                } else {
                    // Some keys can't use 1.9 - check for mixed styles (colons)
                    let mut diags = Vec::new();
                    for elem in &elements {
                        let assoc = match elem.as_assoc_node() {
                            Some(a) => a,
                            None => continue,
                        };
                        let key = assoc.key();
                        if is_symbol_like_key(&key) {
                            if let Some(op_loc) = assoc.operator_loc() {
                                if op_loc.as_slice() != b"=>" {
                                    // Uses colon, which is mixing styles when some keys can't use 1.9
                                    let (line, column) =
                                        source.offset_to_line_col(key.location().start_offset());
                                    diags.push(self.diagnostic(
                                        source,
                                        line,
                                        column,
                                        "Don't mix styles in the same hash.".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    diagnostics.extend(diags);
                }
            }
            "hash_rockets" => {
                let mut diags = Vec::new();
                for elem in &elements {
                    let assoc = match elem.as_assoc_node() {
                        Some(a) => a,
                        None => continue,
                    };
                    let key = assoc.key();
                    if is_symbol_like_key(&key) {
                        let uses_rocket = assoc
                            .operator_loc()
                            .is_some_and(|op| op.as_slice() == b"=>");
                        if !uses_rocket {
                            let (line, column) =
                                source.offset_to_line_col(key.location().start_offset());
                            diags.push(self.diagnostic(
                                source,
                                line,
                                column,
                                "Use hash rockets syntax.".to_string(),
                            ));
                        }
                    }
                }
                diagnostics.extend(diags);
            }
            "no_mixed_keys" => {
                // All keys must use the same syntax
                let mut has_ruby19 = false;
                let mut has_rockets = false;
                for elem in &elements {
                    let assoc = match elem.as_assoc_node() {
                        Some(a) => a,
                        None => continue,
                    };
                    if let Some(op_loc) = assoc.operator_loc() {
                        if op_loc.as_slice() == b"=>" {
                            has_rockets = true;
                        } else {
                            has_ruby19 = true;
                        }
                    } else {
                        has_ruby19 = true;
                    }
                }
                if has_ruby19 && has_rockets {
                    let (line, column) = source.offset_to_line_col(node.location().start_offset());
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        "Don't mix styles in the same hash.".to_string(),
                    ));
                }
            }
            _ => {}
        }
    }
}

/// Check EnforcedShorthandSyntax for Ruby 3.1 hash value omission.
fn check_shorthand_syntax(
    cop: &HashSyntax,
    source: &SourceFile,
    elements: &[ruby_prism::Node<'_>],
    enforced_shorthand: &str,
    diags: &mut Vec<Diagnostic>,
) {
    let mut has_shorthand = false;
    let mut has_explicit = false;

    for elem in elements {
        let assoc = match elem.as_assoc_node() {
            Some(a) => a,
            None => continue,
        };
        let key = assoc.key();
        // Only applies to symbol keys in ruby19 style (key: value)
        if key.as_symbol_node().is_none() {
            continue;
        }
        // Check if value uses implicit node (shorthand `{x:}`)
        if assoc.value().as_implicit_node().is_some() {
            has_shorthand = true;
        } else {
            has_explicit = true;
        }
    }

    match enforced_shorthand {
        "always" => {
            // Flag explicit pairs that could use shorthand: value is a local variable
            // read whose name matches the key symbol
            for elem in elements {
                let assoc = match elem.as_assoc_node() {
                    Some(a) => a,
                    None => continue,
                };
                let key = assoc.key();
                let sym = match key.as_symbol_node() {
                    Some(s) => s,
                    None => continue,
                };
                let value = assoc.value();
                if value.as_implicit_node().is_some() {
                    continue; // Already using shorthand
                }
                // Check if value is a local variable read matching the key name
                if let Some(lvar) = value.as_local_variable_read_node() {
                    if lvar.name().as_slice() == sym.unescaped() {
                        let (line, column) =
                            source.offset_to_line_col(key.location().start_offset());
                        diags.push(cop.diagnostic(
                            source,
                            line,
                            column,
                            "Omit the hash value.".to_string(),
                        ));
                    }
                }
            }
        }
        "never" => {
            // Flag shorthand pairs (implicit node values)
            for elem in elements {
                let assoc = match elem.as_assoc_node() {
                    Some(a) => a,
                    None => continue,
                };
                if assoc.value().as_implicit_node().is_some() {
                    let (line, column) =
                        source.offset_to_line_col(assoc.key().location().start_offset());
                    diags.push(cop.diagnostic(
                        source,
                        line,
                        column,
                        "Include the hash value.".to_string(),
                    ));
                }
            }
        }
        "consistent" => {
            // All pairs must use the same style
            if has_shorthand && has_explicit {
                // Flag at the hash level
                let first_elem = elements.first().unwrap();
                let (line, column) =
                    source.offset_to_line_col(first_elem.location().start_offset());
                diags.push(cop.diagnostic(
                    source,
                    line,
                    column,
                    "Don't mix explicit and shorthand hash values.".to_string(),
                ));
            }
        }
        _ => {}
    }
}

fn is_symbol_like_key(key: &ruby_prism::Node<'_>) -> bool {
    key.as_symbol_node().is_some() || key.as_interpolated_symbol_node().is_some()
}

fn is_acceptable_19_key(
    key: &ruby_prism::Node<'_>,
    prefer_rockets_nonalnum: bool,
    target_ruby_version: f64,
) -> bool {
    if let Some(sym) = key.as_symbol_node() {
        return is_acceptable_19_symbol(&sym, prefer_rockets_nonalnum, target_ruby_version);
    }

    // Interpolated symbol keys are always quoted (e.g. `:"#{field}_string"`),
    // so they follow RuboCop's quoted-symbol path and are convertible on Ruby >= 2.2.
    key.as_interpolated_symbol_node().is_some() && target_ruby_version > 2.1
}

/// Check if a symbol node represents an acceptable Ruby 1.9 syntax key.
/// This includes simple identifiers (`:foo` → `foo:`) and quoted symbols
/// (`:"chef version"` → `"chef version":`, available since Ruby 2.2).
fn is_acceptable_19_symbol(
    sym: &ruby_prism::SymbolNode,
    prefer_rockets_nonalnum: bool,
    target_ruby_version: f64,
) -> bool {
    let name = sym.unescaped();
    // Quoted symbol keys can have different openings depending on syntax:
    //   - Rocket syntax: `:"key" =>` or `:'key' =>` → opening is `:"` or `:'`
    //   - Ruby 1.9 syntax: `"key":` or `'key':` → opening is `"` or `'`
    // Both forms are convertible to 1.9 label syntax on Ruby >= 2.2.
    let is_quoted_symbol = sym
        .opening_loc()
        .is_some_and(|opening| matches!(opening.as_slice(), b":\"" | b":'" | b"\"" | b"'"));

    if is_quoted_symbol {
        return target_ruby_version > 2.1;
    }

    // Simple identifier: `:foo`, `:foo_bar`, `:foo?`, `:foo!`
    if is_simple_symbol_identifier(name) {
        if prefer_rockets_nonalnum && !name.is_empty() {
            let last = name[name.len() - 1];
            if !last.is_ascii_alphanumeric() {
                return false;
            }
        }
        return true;
    }

    false
}

fn target_ruby_version(config: &CopConfig) -> f64 {
    config
        .options
        .get("TargetRubyVersion")
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_u64().map(|value| value as f64))
        })
        .unwrap_or(2.7)
}

/// Check if a symbol's unescaped name is a simple Ruby identifier.
/// Valid: `foo`, `foo_bar`, `foo?`, `foo!`
/// Invalid: `foo bar`, `123`, `foo=`, empty
fn is_simple_symbol_identifier(name: &[u8]) -> bool {
    if name.is_empty() {
        return false;
    }
    // Must start with a letter or underscore
    let first = name[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return false;
    }
    // Rest must be word characters, optionally ending with ? or !
    // Note: `=` ending symbols (setter methods like `:foo=`) cannot use
    // Ruby 1.9 hash syntax, so they are NOT convertible.
    let (body, _suffix) = if name.len() > 1 {
        let last = name[name.len() - 1];
        if last == b'?' || last == b'!' {
            (&name[1..name.len() - 1], Some(last))
        } else {
            (&name[1..], None)
        }
    } else {
        (&[] as &[u8], None)
    };
    body.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::testutil::run_cop_full_with_config;

    crate::cop_fixture_tests!(HashSyntax, "cops/style/hash_syntax");

    #[test]
    fn config_hash_rockets() {
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("hash_rockets".into()),
            )]),
            ..CopConfig::default()
        };
        let source = b"{ a: 1 }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("hash rockets"));
    }

    #[test]
    fn mixed_key_types_skipped_in_ruby19() {
        use crate::testutil::run_cop_full;
        // Hash with string key and symbol key — should not be flagged
        let source = b"{ \"@type\" => \"Person\", :name => \"foo\" }\n";
        let diags = run_cop_full(&HashSyntax, source);
        assert!(diags.is_empty(), "Mixed key hash should not be flagged");
    }

    #[test]
    fn use_hash_rockets_with_symbol_values() {
        let config = CopConfig {
            options: HashMap::from([(
                "UseHashRocketsWithSymbolValues".into(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };
        // Hash with symbol value should not be flagged when UseHashRocketsWithSymbolValues is true
        let source = b"{ :foo => :bar }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Should allow rockets when value is a symbol"
        );
    }

    #[test]
    fn shorthand_never_flags_omission() {
        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedShorthandSyntax".into(),
                serde_yml::Value::String("never".into()),
            )]),
            ..CopConfig::default()
        };
        // Ruby 3.1 hash value omission: `{x:}` (shorthand)
        let source = b"x = 1; {x:}\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("Include the hash value")),
            "Should flag shorthand with EnforcedShorthandSyntax: never"
        );
    }

    #[test]
    fn quoted_symbol_keys_require_ruby_22() {
        let config = CopConfig {
            options: HashMap::from([(
                "TargetRubyVersion".into(),
                serde_yml::Value::Number(serde_yml::value::Number::from(2.1)),
            )]),
            ..CopConfig::default()
        };
        let source = b"{ :\"string\" => 0 }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Quoted symbol keys should stay on hash rockets before Ruby 2.2"
        );
    }

    #[test]
    fn interpolated_symbol_keys_require_ruby_22() {
        let config = CopConfig {
            options: HashMap::from([(
                "TargetRubyVersion".into(),
                serde_yml::Value::Number(serde_yml::value::Number::from(2.1)),
            )]),
            ..CopConfig::default()
        };
        let source = br##"{ :"#{field}_string" => nil }"##;
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Interpolated symbol keys should stay on hash rockets before Ruby 2.2"
        );
    }

    #[test]
    fn interpolated_symbol_keys_register_offense() {
        let source =
            br##"task :"setup:#{provider}" => File.join(ARTIFACT_DIR, "#{provider}.box")"##;
        let diags = crate::testutil::run_cop_full(&HashSyntax, source);
        assert_eq!(diags.len(), 1);
        assert!(
            diags[0]
                .message
                .contains("Use the new Ruby 1.9 hash syntax")
        );
    }

    #[test]
    fn shorthand_either_allows_all() {
        // Default "either" should not flag anything shorthand-related
        let source = b"x = 1; {x:}\n";
        use crate::testutil::run_cop_full;
        let diags = run_cop_full(&HashSyntax, source);
        assert!(
            !diags.iter().any(|d| d.message.contains("hash value")),
            "Default 'either' should not flag shorthand"
        );
    }

    #[test]
    fn prefer_rockets_for_nonalnum_ending_symbols() {
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "PreferHashRocketsForNonAlnumEndingSymbols".into(),
                serde_yml::Value::Bool(true),
            )]),
            ..CopConfig::default()
        };
        // Hash with symbol key ending in `?` should not be flagged (non-alnum ending)
        let source = b"{ :production? => false }\n";
        let diags = run_cop_full_with_config(&HashSyntax, source, config);
        assert!(
            diags.is_empty(),
            "Should allow rockets for non-alnum ending symbols"
        );
    }
}
