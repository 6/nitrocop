use crate::cop::shared::node_type::{
    BLOCK_NODE, CLASS_VARIABLE_AND_WRITE_NODE, CLASS_VARIABLE_OPERATOR_WRITE_NODE,
    CLASS_VARIABLE_OR_WRITE_NODE, CLASS_VARIABLE_WRITE_NODE, DEF_NODE, FOR_NODE,
    GLOBAL_VARIABLE_AND_WRITE_NODE, GLOBAL_VARIABLE_OPERATOR_WRITE_NODE,
    GLOBAL_VARIABLE_OR_WRITE_NODE, GLOBAL_VARIABLE_WRITE_NODE, INSTANCE_VARIABLE_AND_WRITE_NODE,
    INSTANCE_VARIABLE_OPERATOR_WRITE_NODE, INSTANCE_VARIABLE_OR_WRITE_NODE,
    INSTANCE_VARIABLE_WRITE_NODE, LAMBDA_NODE, LOCAL_VARIABLE_AND_WRITE_NODE,
    LOCAL_VARIABLE_OPERATOR_WRITE_NODE, LOCAL_VARIABLE_OR_WRITE_NODE, LOCAL_VARIABLE_READ_NODE,
    LOCAL_VARIABLE_WRITE_NODE, MULTI_WRITE_NODE,
};
use crate::cop::shared::util::is_snake_case;
use crate::cop::{CodeMap, Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// ## Corpus investigation (2026-03-08)
///
/// Corpus oracle reported FP=0, FN=2,911.
///
/// FN=2,911: nitrocop only checked LocalVariableWriteNode but RuboCop
/// checks all variable types (ivar, cvar, gvar) and method parameters.
/// Fixed by adding InstanceVariableWriteNode, ClassVariableWriteNode,
/// GlobalVariableWriteNode, and DefNode (for parameters) handling.
/// Also fixed AllowedPatterns to use regex matching instead of substring.
///
/// ## Corpus investigation (2026-03-10)
///
/// Corpus oracle reported FP=823, FN=2301.
///
/// FP=823: RuboCop's `on_gvasgn` only checks forbidden names on global
/// variables, NOT naming style. Fixed by skipping style check for globals.
///
/// FN=~2000: RuboCop has `alias on_lvar on_lvasgn` — it flags local
/// variable READS as well as writes. Fixed by adding LocalVariableReadNode.
///
/// FN: RuboCop handles block args via `alias on_blockarg on_lvasgn`.
/// Block params (`{ |fooBar| }`) were not checked. Fixed by adding
/// BlockNode/LambdaNode parameter handling.
///
/// FN: RuboCop checks underscore-prefixed variables like `_myLocal` for
/// style violations. Only bare `_` is skipped. Fixed by removing the
/// underscore-prefix skip.
///
/// ## Corpus investigation (2026-03-10, round 2)
///
/// FN=90: Missing compound assignment nodes (||=, &&=, +=) and
/// multi-assignment target nodes for all variable types. RuboCop's
/// `on_lvasgn` alias covers these via Parser gem unification, but Prism
/// splits them into separate node types. Fixed by adding
/// LocalVariable{Or,And,Operator}WriteNode, LocalVariableTargetNode,
/// and equivalent nodes for instance, class, and global variables.
///
/// ## Corpus investigation (2026-03-10, round 3)
///
/// FP=2: Pattern matching destructuring (`=> { newName: }`) and regex
/// named captures (`/(?<channelClaim>\w+)/ =~ str`) produce
/// `LocalVariableTargetNode` in Prism, but in the Parser gem these are
/// `match_var` and `match_with_lvasgn` respectively — RuboCop has no
/// handler for either. Fixed by removing `*_TARGET_NODE` from
/// `interested_node_types` and instead handling targets via
/// `MULTI_WRITE_NODE` and `FOR_NODE` parent nodes only.
///
/// FN=6: Non-ASCII variable names (CJK characters, emoji) passed
/// `is_snake_case` because it allowed all non-ASCII bytes. RuboCop's
/// snake_case regex uses `[[:lower:]]` which only matches Unicode
/// lowercase letters. Fixed by updating `is_snake_case` to validate
/// non-ASCII characters via `char::is_lowercase()`.
///
/// ## Corpus investigation (2026-03-11, round 4)
///
/// FN=3: `rescue => badVar` produces `LocalVariableTargetNode` in Prism,
/// which we excluded from `interested_node_types` (round 3) because
/// pattern matching and regex captures also produce TargetNodes. However,
/// in the Parser gem, `rescue => e` produces an `lvasgn` node, so
/// RuboCop's `on_lvasgn`/`on_ivasgn`/`on_cvasgn` handlers check it.
/// Additionally, Prism's `Visit` trait calls `visit_rescue_node()` directly
/// without going through `visit_branch_node_enter()`, so `check_node`
/// never sees RescueNode. Fixed by adding a `check_source` implementation
/// with a custom `RescueRefVisitor` that overrides `visit_rescue_node`
/// to check rescue reference variables.
///
/// ## Variant divergence fix (2026-04-05)
///
/// camelCase variant: 30,271 FP, 356 FN (baseline).
///
/// FP=30,271: `is_lower_camel_case` operated on raw bytes and rejected
/// ALL underscores and ALL non-ASCII characters. RuboCop's camelCase regex
/// `/^@{0,2}(?:_|_?[[:lower:]][\d[[:lower:]][[:upper:]]]*)[!?=]?$/` accepts:
/// (1) bare `_`, (2) optional `_` prefix + lowercase start + alphanumeric body,
/// (3) Unicode lowercase letters via `[[:lower:]]`. Fixed by rewriting
/// `is_lower_camel_case` to use `char::is_lowercase()`/`is_uppercase()` for
/// Unicode support and handle the optional leading underscore per the regex.
/// Verified on 15 corpus repos: per-repo offense counts match RuboCop exactly.
pub struct VariableName;

impl Cop for VariableName {
    fn name(&self) -> &'static str {
        "Naming/VariableName"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            LOCAL_VARIABLE_WRITE_NODE,
            LOCAL_VARIABLE_READ_NODE,
            LOCAL_VARIABLE_OR_WRITE_NODE,
            LOCAL_VARIABLE_AND_WRITE_NODE,
            LOCAL_VARIABLE_OPERATOR_WRITE_NODE,
            INSTANCE_VARIABLE_WRITE_NODE,
            INSTANCE_VARIABLE_OR_WRITE_NODE,
            INSTANCE_VARIABLE_AND_WRITE_NODE,
            INSTANCE_VARIABLE_OPERATOR_WRITE_NODE,
            CLASS_VARIABLE_WRITE_NODE,
            CLASS_VARIABLE_OR_WRITE_NODE,
            CLASS_VARIABLE_AND_WRITE_NODE,
            CLASS_VARIABLE_OPERATOR_WRITE_NODE,
            GLOBAL_VARIABLE_WRITE_NODE,
            GLOBAL_VARIABLE_OR_WRITE_NODE,
            GLOBAL_VARIABLE_AND_WRITE_NODE,
            GLOBAL_VARIABLE_OPERATOR_WRITE_NODE,
            MULTI_WRITE_NODE,
            FOR_NODE,
            DEF_NODE,
            BLOCK_NODE,
            LAMBDA_NODE,
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
        // Handle DefNode for method parameters
        if let Some(def_node) = node.as_def_node() {
            self.check_parameters(source, def_node, config, diagnostics);
            return;
        }

        // Handle BlockNode/LambdaNode for block parameters
        if let Some(block_node) = node.as_block_node() {
            self.check_block_parameters(source, block_node, config, diagnostics);
            return;
        }
        if let Some(lambda_node) = node.as_lambda_node() {
            if let Some(params) = lambda_node.parameters() {
                if let Some(block_params) = params.as_block_parameters_node() {
                    if let Some(params_node) = block_params.parameters() {
                        self.check_params_node(source, params_node, config, diagnostics);
                    }
                }
            }
            return;
        }

        // Handle LocalVariableReadNode
        if let Some(n) = node.as_local_variable_read_node() {
            let name = n.name().as_slice();
            let name_str = std::str::from_utf8(name).unwrap_or("");
            let (line, column) = source.offset_to_line_col(n.location().start_offset());
            self.check_variable_name(source, name_str, line, column, config, diagnostics);
            return;
        }

        // Handle MultiWriteNode — iterate over target children.
        // We handle targets here (instead of via LOCAL_VARIABLE_TARGET_NODE etc.)
        // because LocalVariableTargetNode also appears in pattern matching and
        // regex captures, which RuboCop does NOT check (they use match_var and
        // match_with_lvasgn in the Parser gem, not lvasgn).
        if let Some(mw) = node.as_multi_write_node() {
            self.check_multi_write_targets(source, mw, config, diagnostics);
            return;
        }

        // Handle ForNode — check the index variable
        if let Some(for_node) = node.as_for_node() {
            self.check_for_index(source, for_node, config, diagnostics);
            return;
        }

        // Extract variable name and location based on node type
        let (raw_name, start_offset, is_global) =
            // Local variable writes and compound assignments
            if let Some(n) = node.as_local_variable_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_local_variable_or_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_local_variable_and_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_local_variable_operator_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            }
            // Instance variable writes and compound assignments
            else if let Some(n) = node.as_instance_variable_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_instance_variable_or_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_instance_variable_and_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_instance_variable_operator_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            }
            // Class variable writes and compound assignments
            else if let Some(n) = node.as_class_variable_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_class_variable_or_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_class_variable_and_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            } else if let Some(n) = node.as_class_variable_operator_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), false)
            }
            // Global variable writes and compound assignments
            else if let Some(n) = node.as_global_variable_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), true)
            } else if let Some(n) = node.as_global_variable_or_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), true)
            } else if let Some(n) = node.as_global_variable_and_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), true)
            } else if let Some(n) = node.as_global_variable_operator_write_node() {
                (n.name().as_slice(), n.name_loc().start_offset(), true)
            } else {
                return;
            };

        let raw_name_str = std::str::from_utf8(raw_name).unwrap_or("");

        // Strip prefixes to get the bare variable name
        let var_name_str = raw_name_str
            .strip_prefix("@@")
            .or_else(|| raw_name_str.strip_prefix('@'))
            .or_else(|| raw_name_str.strip_prefix('$'))
            .unwrap_or(raw_name_str);

        // Skip special globals ($_, $0, $1, $!, $@, etc.)
        if is_global
            && (var_name_str.is_empty()
                || var_name_str == "_"
                || var_name_str.starts_with(|c: char| c.is_ascii_digit())
                || (var_name_str.len() == 1 && !var_name_str.as_bytes()[0].is_ascii_alphabetic()))
        {
            return;
        }

        let (line, column) = source.offset_to_line_col(start_offset);

        if is_global {
            // RuboCop only checks forbidden names on global variables, not style
            self.check_forbidden_only(source, var_name_str, line, column, config, diagnostics);
        } else {
            self.check_variable_name(source, var_name_str, line, column, config, diagnostics);
        }
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Handle rescue => var references. In Prism, RescueNode is visited via
        // visit_rescue_node() which doesn't trigger visit_branch_node_enter(),
        // so check_node never sees it. We use a custom visitor to find rescue
        // references specifically.
        use ruby_prism::Visit;
        let mut visitor = RescueRefVisitor {
            cop: self,
            source,
            config,
            diagnostics,
        };
        visitor.visit(&parse_result.node());
    }
}

/// Visitor that finds rescue node references (rescue => var) and checks their names.
/// This is needed because Prism's Visit trait calls visit_rescue_node() directly,
/// bypassing visit_branch_node_enter(), so check_node never sees RescueNode.
struct RescueRefVisitor<'a> {
    cop: &'a VariableName,
    source: &'a SourceFile,
    config: &'a CopConfig,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<'pr> ruby_prism::Visit<'pr> for RescueRefVisitor<'_> {
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        if let Some(ref_node) = node.reference() {
            self.cop
                .check_target_node(self.source, &ref_node, self.config, self.diagnostics);
        }
        // Continue walking into children (exceptions, statements, subsequent)
        ruby_prism::visit_rescue_node(self, node);
    }
}

impl VariableName {
    fn check_multi_write_targets(
        &self,
        source: &SourceFile,
        mw: ruby_prism::MultiWriteNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for target in mw.lefts().iter() {
            self.check_target_node(source, &target, config, diagnostics);
        }
        if let Some(rest) = mw.rest() {
            self.check_target_node(source, &rest, config, diagnostics);
        }
        for target in mw.rights().iter() {
            self.check_target_node(source, &target, config, diagnostics);
        }
    }

    fn check_for_index(
        &self,
        source: &SourceFile,
        for_node: ruby_prism::ForNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let index = for_node.index();
        self.check_target_node(source, &index, config, diagnostics);
    }

    fn check_target_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let (raw_name, start_offset, is_global) =
            if let Some(n) = node.as_local_variable_target_node() {
                (n.name().as_slice(), n.location().start_offset(), false)
            } else if let Some(n) = node.as_instance_variable_target_node() {
                (n.name().as_slice(), n.location().start_offset(), false)
            } else if let Some(n) = node.as_class_variable_target_node() {
                (n.name().as_slice(), n.location().start_offset(), false)
            } else if let Some(n) = node.as_global_variable_target_node() {
                (n.name().as_slice(), n.location().start_offset(), true)
            } else if let Some(n) = node.as_splat_node() {
                // *rest in multi-assignment: check inner expression
                if let Some(expr) = n.expression() {
                    self.check_target_node(source, &expr, config, diagnostics);
                }
                return;
            } else if let Some(mw) = node.as_multi_target_node() {
                // Nested destructuring: (a, b), c = ...
                for target in mw.lefts().iter() {
                    self.check_target_node(source, &target, config, diagnostics);
                }
                if let Some(rest) = mw.rest() {
                    self.check_target_node(source, &rest, config, diagnostics);
                }
                for target in mw.rights().iter() {
                    self.check_target_node(source, &target, config, diagnostics);
                }
                return;
            } else {
                return;
            };

        let raw_name_str = std::str::from_utf8(raw_name).unwrap_or("");
        let var_name_str = raw_name_str
            .strip_prefix("@@")
            .or_else(|| raw_name_str.strip_prefix('@'))
            .or_else(|| raw_name_str.strip_prefix('$'))
            .unwrap_or(raw_name_str);

        let (line, column) = source.offset_to_line_col(start_offset);

        if is_global {
            self.check_forbidden_only(source, var_name_str, line, column, config, diagnostics);
        } else {
            self.check_variable_name(source, var_name_str, line, column, config, diagnostics);
        }
    }

    fn check_forbidden_only(
        &self,
        source: &SourceFile,
        var_name_str: &str,
        line: usize,
        column: usize,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let forbidden_identifiers = config.get_string_array("ForbiddenIdentifiers");
        let forbidden_patterns = config.get_string_array("ForbiddenPatterns");

        if let Some(forbidden) = &forbidden_identifiers {
            if forbidden.iter().any(|f| f == var_name_str) {
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("`{var_name_str}` is forbidden, use another variable name instead."),
                ));
            }
        }

        if let Some(patterns) = &forbidden_patterns {
            for pattern in patterns {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if re.is_match(var_name_str) {
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            format!(
                                "`{var_name_str}` is forbidden, use another variable name instead."
                            ),
                        ));
                    }
                }
            }
        }
    }

    fn check_variable_name(
        &self,
        source: &SourceFile,
        var_name_str: &str,
        line: usize,
        column: usize,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let enforced_style = config.get_str("EnforcedStyle", "snake_case");
        let allowed_identifiers = config.get_string_array("AllowedIdentifiers");
        let allowed_patterns = config.get_string_array("AllowedPatterns");
        let forbidden_identifiers = config.get_string_array("ForbiddenIdentifiers");
        let forbidden_patterns = config.get_string_array("ForbiddenPatterns");

        // ForbiddenIdentifiers: flag if var name is in the forbidden list
        if let Some(forbidden) = &forbidden_identifiers {
            if forbidden.iter().any(|f| f == var_name_str) {
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("`{var_name_str}` is forbidden, use another variable name instead."),
                ));
            }
        }

        // ForbiddenPatterns: flag if var name matches any forbidden regex
        if let Some(patterns) = &forbidden_patterns {
            for pattern in patterns {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if re.is_match(var_name_str) {
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            format!(
                                "`{var_name_str}` is forbidden, use another variable name instead."
                            ),
                        ));
                    }
                }
            }
        }

        // AllowedIdentifiers: skip if var name is explicitly allowed
        if let Some(allowed) = &allowed_identifiers {
            if allowed.iter().any(|a| a == var_name_str) {
                return;
            }
        }

        // AllowedPatterns: skip if var name matches any regex pattern
        if let Some(patterns) = &allowed_patterns {
            for p in patterns {
                if let Ok(re) = regex::Regex::new(p) {
                    if re.is_match(var_name_str) {
                        return;
                    }
                }
            }
        }

        // Check naming style
        let var_name = var_name_str.as_bytes();
        let style_ok = match enforced_style {
            "camelCase" => is_lower_camel_case(var_name),
            _ => is_snake_case(var_name), // snake_case is default
        };

        if style_ok {
            return;
        }

        let style_msg = match enforced_style {
            "camelCase" => "camelCase",
            _ => "snake_case",
        };

        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!("Use {style_msg} for variable names."),
        ));
    }

    fn check_block_parameters(
        &self,
        source: &SourceFile,
        block_node: ruby_prism::BlockNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let params = match block_node.parameters() {
            Some(p) => p,
            None => return,
        };

        let block_params = match params.as_block_parameters_node() {
            Some(bp) => bp,
            None => return,
        };

        let params_node = match block_params.parameters() {
            Some(p) => p,
            None => return,
        };

        self.check_params_node(source, params_node, config, diagnostics);
    }

    fn check_params_node(
        &self,
        source: &SourceFile,
        params: ruby_prism::ParametersNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for param in params.requireds().iter() {
            if let Some(req) = param.as_required_parameter_node() {
                let name = req.name().as_slice();
                let name_str = std::str::from_utf8(name).unwrap_or("");
                let (line, column) = source.offset_to_line_col(req.location().start_offset());
                self.check_variable_name(source, name_str, line, column, config, diagnostics);
            }
        }

        for param in params.optionals().iter() {
            if let Some(opt) = param.as_optional_parameter_node() {
                let name = opt.name().as_slice();
                let name_str = std::str::from_utf8(name).unwrap_or("");
                let (line, column) = source.offset_to_line_col(opt.name_loc().start_offset());
                self.check_variable_name(source, name_str, line, column, config, diagnostics);
            }
        }

        for param in params.keywords().iter() {
            if let Some(kw) = param.as_required_keyword_parameter_node() {
                let name = kw.name().as_slice();
                let name_str = std::str::from_utf8(name).unwrap_or("");
                let clean_name = name_str.strip_suffix(':').unwrap_or(name_str);
                let (line, column) = source.offset_to_line_col(kw.name_loc().start_offset());
                self.check_variable_name(source, clean_name, line, column, config, diagnostics);
            }
            if let Some(kw) = param.as_optional_keyword_parameter_node() {
                let name = kw.name().as_slice();
                let name_str = std::str::from_utf8(name).unwrap_or("");
                let clean_name = name_str.strip_suffix(':').unwrap_or(name_str);
                let (line, column) = source.offset_to_line_col(kw.name_loc().start_offset());
                self.check_variable_name(source, clean_name, line, column, config, diagnostics);
            }
        }

        if let Some(rest) = params.rest() {
            if let Some(rest_param) = rest.as_rest_parameter_node() {
                if let Some(name) = rest_param.name() {
                    let name_str = std::str::from_utf8(name.as_slice()).unwrap_or("");
                    if let Some(name_loc) = rest_param.name_loc() {
                        let (line, column) = source.offset_to_line_col(name_loc.start_offset());
                        self.check_variable_name(
                            source,
                            name_str,
                            line,
                            column,
                            config,
                            diagnostics,
                        );
                    }
                }
            }
        }

        if let Some(kw_rest) = params.keyword_rest() {
            if let Some(kw_rest_param) = kw_rest.as_keyword_rest_parameter_node() {
                if let Some(name) = kw_rest_param.name() {
                    let name_str = std::str::from_utf8(name.as_slice()).unwrap_or("");
                    if let Some(name_loc) = kw_rest_param.name_loc() {
                        let (line, column) = source.offset_to_line_col(name_loc.start_offset());
                        self.check_variable_name(
                            source,
                            name_str,
                            line,
                            column,
                            config,
                            diagnostics,
                        );
                    }
                }
            }
        }

        if let Some(block) = params.block() {
            if let Some(name) = block.name() {
                let name_str = std::str::from_utf8(name.as_slice()).unwrap_or("");
                if let Some(name_loc) = block.name_loc() {
                    let (line, column) = source.offset_to_line_col(name_loc.start_offset());
                    self.check_variable_name(source, name_str, line, column, config, diagnostics);
                }
            }
        }
    }

    fn check_parameters(
        &self,
        source: &SourceFile,
        def_node: ruby_prism::DefNode<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let params = match def_node.parameters() {
            Some(p) => p,
            None => return,
        };

        self.check_params_node(source, params, config, diagnostics);
    }
}

/// Returns true if the name is lowerCamelCase per RuboCop's regex:
/// `/^@{0,2}(?:_|_?[[:lower:]][\d[[:lower:]][[:upper:]]]*)[!?=]?$/`
///
/// After stripping sigil prefixes (@/@@/$), the bare name must be:
/// - `_` alone, OR
/// - optional `_` prefix, then a Unicode lowercase letter, then zero or more
///   of: Unicode lowercase, Unicode uppercase, or ASCII digit.
fn is_lower_camel_case(name: &[u8]) -> bool {
    if name.is_empty() {
        return true;
    }

    let s = match std::str::from_utf8(name) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let mut chars = s.chars();
    let first = chars.next().unwrap(); // name is not empty

    // Handle optional leading underscore
    let start_char = if first == '_' {
        match chars.next() {
            None => return true, // bare `_` is valid
            Some(c) => c,
        }
    } else {
        first
    };

    // First non-underscore char must be Unicode lowercase
    if !start_char.is_lowercase() {
        return false;
    }

    // Remaining chars must be lowercase, uppercase, or ASCII digit
    for ch in chars {
        if !(ch.is_lowercase() || ch.is_uppercase() || ch.is_ascii_digit()) {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(VariableName, "cops/naming/variable_name");

    fn camel_case_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("camelCase".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn config_enforced_style_camel_case() {
        use crate::testutil::run_cop_full_with_config;
        let source = b"myVar = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            diags.is_empty(),
            "camelCase variable should not be flagged in camelCase mode"
        );
    }

    #[test]
    fn config_enforced_style_camel_case_flags_snake() {
        use crate::testutil::run_cop_full_with_config;
        let source = b"my_var = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            !diags.is_empty(),
            "snake_case variable should be flagged in camelCase mode"
        );
    }

    #[test]
    fn camel_case_accepts_underscore_prefix() {
        use crate::testutil::run_cop_full_with_config;
        // RuboCop accepts _myLocal in camelCase mode (unused var convention)
        let source = b"_myLocal = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            diags.is_empty(),
            "_myLocal should not be flagged in camelCase mode: got {:?}",
            diags
        );
    }

    #[test]
    fn camel_case_accepts_bare_underscore() {
        use crate::testutil::run_cop_full_with_config;
        // RuboCop always accepts `_ = 1`
        let source = b"_ = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            diags.is_empty(),
            "bare _ should not be flagged in camelCase mode: got {:?}",
            diags
        );
    }

    #[test]
    fn camel_case_accepts_non_ascii_lowercase() {
        use crate::testutil::run_cop_full_with_config;
        // RuboCop: `léo = 1` is accepted in camelCase mode
        let source = "léo = 1\n".as_bytes();
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            diags.is_empty(),
            "léo should not be flagged in camelCase mode: got {:?}",
            diags
        );
    }

    #[test]
    fn camel_case_flags_underscore_snake_case() {
        use crate::testutil::run_cop_full_with_config;
        // _my_var has underscore in body → not valid camelCase
        let source = b"_my_var = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            !diags.is_empty(),
            "_my_var should be flagged in camelCase mode"
        );
    }

    #[test]
    fn camel_case_flags_uppercase_after_underscore() {
        use crate::testutil::run_cop_full_with_config;
        // _MyVar starts uppercase after underscore → not valid camelCase
        let source = b"_MyVar = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            !diags.is_empty(),
            "_MyVar should be flagged in camelCase mode"
        );
    }

    #[test]
    fn camel_case_flags_double_underscore() {
        use crate::testutil::run_cop_full_with_config;
        // __x has double underscore → not valid camelCase
        let source = b"__x = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(!diags.is_empty(), "__x should be flagged in camelCase mode");
    }

    #[test]
    fn camel_case_accepts_single_lowercase() {
        use crate::testutil::run_cop_full_with_config;
        let source = b"x = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            diags.is_empty(),
            "single lowercase letter should not be flagged in camelCase mode"
        );
    }

    #[test]
    fn camel_case_accepts_underscore_prefix_lowercase() {
        use crate::testutil::run_cop_full_with_config;
        // _unused is valid camelCase (underscore + all lowercase is fine)
        let source = b"_unused = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, camel_case_config());
        assert!(
            diags.is_empty(),
            "_unused should not be flagged in camelCase mode: got {:?}",
            diags
        );
    }

    #[test]
    fn config_forbidden_identifiers() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "ForbiddenIdentifiers".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("data".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"data = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, config);
        assert!(
            !diags.is_empty(),
            "Forbidden variable name should be flagged"
        );
        assert!(diags[0].message.contains("forbidden"));
    }

    #[test]
    fn rescue_variable_flagged() {
        use crate::testutil::run_cop_full;
        let source = b"begin\n  something\nrescue => badError\n  nil\nend\n";
        let diags = run_cop_full(&VariableName, source);
        assert!(
            !diags.is_empty(),
            "rescue => badError should be flagged: got {:?}",
            diags
        );
    }

    #[test]
    fn config_forbidden_patterns() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "ForbiddenPatterns".into(),
                serde_yml::Value::Sequence(vec![serde_yml::Value::String("_tmp\\z".into())]),
            )]),
            ..CopConfig::default()
        };
        let source = b"data_tmp = 1\n";
        let diags = run_cop_full_with_config(&VariableName, source, config);
        assert!(
            !diags.is_empty(),
            "Variable matching forbidden pattern should be flagged"
        );
    }
}
