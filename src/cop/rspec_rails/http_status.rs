use crate::cop::rspec_rails::RSPEC_RAILS_DEFAULT_INCLUDE;
use crate::cop::shared::node_type::{CALL_NODE, INTEGER_NODE, STRING_NODE, SYMBOL_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// RSpecRails/HttpStatus: enforces use of symbolic, numeric, or be_status style
/// for `have_http_status` calls.
///
/// Variant fixes applied:
/// - `numeric` style: unknown symbols (not in PERMITTED_SYMBOLS and not a known
///   status code) and ALL string arguments (including numeric strings like "200")
///   are now flagged as "Unknown status code.", matching RuboCop's NumericStyleChecker
///   which flags anything that is `!int_type? && !allowed_symbol?`.
/// - `be_status` style: unknown symbols (e.g. `:oki_doki`) are now flagged with
///   "Prefer `be_<sym>`..." rather than silently skipped, matching RuboCop's
///   BeStatusStyleChecker which suggests `be_<sym>` for any non-allowed symbol.
pub struct HttpStatus;

fn status_code_to_symbol(code: i64) -> Option<&'static str> {
    match code {
        100 => Some("continue"),
        101 => Some("switching_protocols"),
        102 => Some("processing"),
        103 => Some("early_hints"),
        200 => Some("ok"),
        201 => Some("created"),
        202 => Some("accepted"),
        203 => Some("non_authoritative_information"),
        204 => Some("no_content"),
        205 => Some("reset_content"),
        206 => Some("partial_content"),
        207 => Some("multi_status"),
        208 => Some("already_reported"),
        226 => Some("im_used"),
        300 => Some("multiple_choices"),
        301 => Some("moved_permanently"),
        302 => Some("found"),
        303 => Some("see_other"),
        304 => Some("not_modified"),
        305 => Some("use_proxy"),
        307 => Some("temporary_redirect"),
        308 => Some("permanent_redirect"),
        400 => Some("bad_request"),
        401 => Some("unauthorized"),
        402 => Some("payment_required"),
        403 => Some("forbidden"),
        404 => Some("not_found"),
        405 => Some("method_not_allowed"),
        406 => Some("not_acceptable"),
        407 => Some("proxy_authentication_required"),
        408 => Some("request_timeout"),
        409 => Some("conflict"),
        410 => Some("gone"),
        411 => Some("length_required"),
        412 => Some("precondition_failed"),
        413 => Some("payload_too_large"),
        414 => Some("uri_too_long"),
        415 => Some("unsupported_media_type"),
        416 => Some("range_not_satisfiable"),
        417 => Some("expectation_failed"),
        421 => Some("misdirected_request"),
        422 => Some("unprocessable_entity"),
        423 => Some("locked"),
        424 => Some("failed_dependency"),
        425 => Some("too_early"),
        426 => Some("upgrade_required"),
        428 => Some("precondition_required"),
        429 => Some("too_many_requests"),
        431 => Some("request_header_fields_too_large"),
        451 => Some("unavailable_for_legal_reasons"),
        500 => Some("internal_server_error"),
        501 => Some("not_implemented"),
        502 => Some("bad_gateway"),
        503 => Some("service_unavailable"),
        504 => Some("gateway_timeout"),
        505 => Some("http_version_not_supported"),
        506 => Some("variant_also_negotiates"),
        507 => Some("insufficient_storage"),
        508 => Some("loop_detected"),
        510 => Some("not_extended"),
        511 => Some("network_authentication_required"),
        _ => None,
    }
}

fn symbol_to_status_code(sym: &[u8]) -> Option<i64> {
    match sym {
        b"continue" => Some(100),
        b"switching_protocols" => Some(101),
        b"processing" => Some(102),
        b"early_hints" => Some(103),
        b"ok" => Some(200),
        b"created" => Some(201),
        b"accepted" => Some(202),
        b"non_authoritative_information" => Some(203),
        b"no_content" => Some(204),
        b"reset_content" => Some(205),
        b"partial_content" => Some(206),
        b"multi_status" => Some(207),
        b"already_reported" => Some(208),
        b"im_used" => Some(226),
        b"multiple_choices" => Some(300),
        b"moved_permanently" => Some(301),
        b"found" => Some(302),
        b"see_other" => Some(303),
        b"not_modified" => Some(304),
        b"use_proxy" => Some(305),
        b"temporary_redirect" => Some(307),
        b"permanent_redirect" => Some(308),
        b"bad_request" => Some(400),
        b"unauthorized" => Some(401),
        b"payment_required" => Some(402),
        b"forbidden" => Some(403),
        b"not_found" => Some(404),
        b"method_not_allowed" => Some(405),
        b"not_acceptable" => Some(406),
        b"proxy_authentication_required" => Some(407),
        b"request_timeout" => Some(408),
        b"conflict" => Some(409),
        b"gone" => Some(410),
        b"length_required" => Some(411),
        b"precondition_failed" => Some(412),
        b"payload_too_large" => Some(413),
        b"uri_too_long" => Some(414),
        b"unsupported_media_type" => Some(415),
        b"range_not_satisfiable" => Some(416),
        b"expectation_failed" => Some(417),
        b"misdirected_request" => Some(421),
        b"unprocessable_entity" => Some(422),
        b"locked" => Some(423),
        b"failed_dependency" => Some(424),
        b"too_early" => Some(425),
        b"upgrade_required" => Some(426),
        b"precondition_required" => Some(428),
        b"too_many_requests" => Some(429),
        b"request_header_fields_too_large" => Some(431),
        b"unavailable_for_legal_reasons" => Some(451),
        b"internal_server_error" => Some(500),
        b"not_implemented" => Some(501),
        b"bad_gateway" => Some(502),
        b"service_unavailable" => Some(503),
        b"gateway_timeout" => Some(504),
        b"http_version_not_supported" => Some(505),
        b"variant_also_negotiates" => Some(506),
        b"insufficient_storage" => Some(507),
        b"loop_detected" => Some(508),
        b"not_extended" => Some(510),
        b"network_authentication_required" => Some(511),
        _ => None,
    }
}

/// Permitted symbols that are not specific status codes (allowed in numeric style).
const PERMITTED_SYMBOLS: &[&[u8]] = &[b"error", b"success", b"missing", b"redirect"];

impl Cop for HttpStatus {
    fn name(&self) -> &'static str {
        "RSpecRails/HttpStatus"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        RSPEC_RAILS_DEFAULT_INCLUDE
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, INTEGER_NODE, STRING_NODE, SYMBOL_NODE]
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
        let style = config.get_str("EnforcedStyle", "symbolic");

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.name().as_slice() != b"have_http_status" {
            return;
        }

        // No receiver (bare `have_http_status`)
        if call.receiver().is_some() {
            return;
        }

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.len() != 1 {
            return;
        }

        let arg = &arg_list[0];

        let result = match style {
            "symbolic" => self.check_symbolic_style(source, &call, arg),
            "numeric" => self.check_numeric_style(source, &call, arg),
            "be_status" => self.check_be_status_style(source, &call, arg),
            _ => Vec::new(),
        };
        diagnostics.extend(result);
    }
}

impl HttpStatus {
    /// Symbolic style: flag numeric args and string args, prefer symbols.
    fn check_symbolic_style(
        &self,
        source: &SourceFile,
        _call: &ruby_prism::CallNode<'_>,
        arg: &ruby_prism::Node<'_>,
    ) -> Vec<Diagnostic> {
        if arg.as_integer_node().is_some() {
            let loc = arg.location();
            let code_text = std::str::from_utf8(loc.as_slice()).unwrap_or("");
            if let Ok(code_num) = code_text.parse::<i64>() {
                if let Some(sym) = status_code_to_symbol(code_num) {
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    return vec![self.diagnostic(
                        source,
                        line,
                        column,
                        format!("Prefer `:{sym}` over `{code_num}` to describe HTTP status code."),
                    )];
                }
            }
            // Custom code with no symbol mapping - no offense
            return Vec::new();
        }

        if let Some(str_node) = arg.as_string_node() {
            let content = str_node.unescaped();
            let s = std::str::from_utf8(content).unwrap_or("");
            let loc = arg.location();
            let source_text = std::str::from_utf8(loc.as_slice()).unwrap_or("");

            // Try as numeric string
            if let Ok(code_num) = s.parse::<i64>() {
                if let Some(sym) = status_code_to_symbol(code_num) {
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    return vec![self.diagnostic(
                        source,
                        line,
                        column,
                        format!(
                            "Prefer `:{sym}` over `{source_text}` to describe HTTP status code."
                        ),
                    )];
                }
                return Vec::new();
            }

            // Non-numeric string -> unknown status code
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            return vec![self.diagnostic(source, line, column, "Unknown status code.".to_string())];
        }

        Vec::new()
    }

    /// Numeric style: flag symbolic args, prefer numeric codes.
    /// RuboCop's NumericStyleChecker flags anything that is not an integer
    /// and not an allowed symbol (:error, :success, :missing, :redirect).
    /// This includes unknown symbols and ALL strings (even numeric ones like "200").
    fn check_numeric_style(
        &self,
        source: &SourceFile,
        _call: &ruby_prism::CallNode<'_>,
        arg: &ruby_prism::Node<'_>,
    ) -> Vec<Diagnostic> {
        if let Some(sym) = arg.as_symbol_node() {
            let sym_name = sym.unescaped();
            // Allow generic symbols
            if PERMITTED_SYMBOLS.contains(&sym_name) {
                return Vec::new();
            }
            if let Some(code) = symbol_to_status_code(sym_name) {
                let sym_str = std::str::from_utf8(sym_name).unwrap_or("?");
                let loc = arg.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                return vec![self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Prefer `{code}` over `:{sym_str}` to describe HTTP status code."),
                )];
            }
            // Unknown symbol (not in allowed list and not a known status code)
            let loc = arg.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            return vec![self.diagnostic(source, line, column, "Unknown status code.".to_string())];
        }

        if let Some(str_node) = arg.as_string_node() {
            let content = str_node.unescaped();
            let s = std::str::from_utf8(content).unwrap_or("");
            let loc = arg.location();
            let source_text = std::str::from_utf8(loc.as_slice()).unwrap_or("");

            // Try as symbolic string
            if let Some(code) = symbol_to_status_code(s.as_bytes()) {
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                return vec![self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Prefer `{code}` over `{source_text}` to describe HTTP status code."),
                )];
            }

            // Any other string (numeric or not) is flagged as unknown in numeric style,
            // since RuboCop requires integer literals, not string representations.
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            return vec![self.diagnostic(source, line, column, "Unknown status code.".to_string())];
        }

        Vec::new()
    }

    /// be_status style: flag have_http_status with any numeric/symbol/string,
    /// prefer be_ok, be_not_found, etc.
    fn check_be_status_style(
        &self,
        source: &SourceFile,
        call: &ruby_prism::CallNode<'_>,
        arg: &ruby_prism::Node<'_>,
    ) -> Vec<Diagnostic> {
        // Get the symbol name to construct be_<sym>
        let sym_name_opt = if arg.as_integer_node().is_some() {
            let loc = arg.location();
            let text = std::str::from_utf8(loc.as_slice()).unwrap_or("");
            text.parse::<i64>()
                .ok()
                .and_then(status_code_to_symbol)
                .map(|s| s.to_string())
        } else if let Some(sym) = arg.as_symbol_node() {
            let name = sym.unescaped();
            let name_str = std::str::from_utf8(name).unwrap_or("");
            // Allow generic symbols
            if PERMITTED_SYMBOLS.contains(&name) {
                return Vec::new();
            }
            // RuboCop's BeStatusStyleChecker suggests be_<sym> for ANY symbol
            // (known or unknown), as long as it's not in the allowed list.
            Some(name_str.to_string())
        } else if let Some(str_node) = arg.as_string_node() {
            let content = str_node.unescaped();
            let s = std::str::from_utf8(content).unwrap_or("");

            // Try as numeric string
            if let Ok(code_num) = s.parse::<i64>() {
                status_code_to_symbol(code_num).map(|sym| sym.to_string())
            } else if symbol_to_status_code(s.as_bytes()).is_some() {
                // Symbolic string like "ok"
                Some(s.to_string())
            } else {
                // Unknown string
                let loc = call.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                return vec![self.diagnostic(
                    source,
                    line,
                    column,
                    "Unknown status code.".to_string(),
                )];
            }
        } else {
            None
        };

        if let Some(sym_name) = sym_name_opt {
            let loc = call.location();
            let source_text = std::str::from_utf8(loc.as_slice()).unwrap_or("");
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            return vec![self.diagnostic(
                source,
                line,
                column,
                format!(
                    "Prefer `be_{sym_name}` over `{source_text}` to describe HTTP status code."
                ),
            )];
        }

        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(HttpStatus, "cops/rspecrails/http_status");

    #[test]
    fn numeric_style_flags_symbol() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("numeric".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to have_http_status :ok }\n";
        let diags = run_cop_full_with_config(&HttpStatus, source, config);
        assert!(!diags.is_empty(), "numeric style should flag symbolic :ok");
        assert!(diags[0].message.contains("200"));
    }

    #[test]
    fn numeric_style_allows_numeric() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("numeric".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to have_http_status 200 }\n";
        assert_cop_no_offenses_full_with_config(&HttpStatus, source, config);
    }

    #[test]
    fn numeric_style_permits_generic_symbols() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("numeric".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to have_http_status :error }\n";
        assert_cop_no_offenses_full_with_config(&HttpStatus, source, config);
    }

    #[test]
    fn be_status_style_flags_numeric() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("be_status".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to have_http_status 200 }\n";
        let diags = run_cop_full_with_config(&HttpStatus, source, config);
        assert!(!diags.is_empty());
        assert!(diags[0].message.contains("be_ok"));
    }

    #[test]
    fn numeric_style_flags_unknown_symbol() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("numeric".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to have_http_status :oki_doki }\n";
        let diags = run_cop_full_with_config(&HttpStatus, source, config);
        assert!(
            !diags.is_empty(),
            "numeric style should flag unknown symbol :oki_doki"
        );
        assert!(diags[0].message.contains("Unknown status code"));
    }

    #[test]
    fn numeric_style_flags_numeric_string() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("numeric".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to have_http_status \"200\" }\n";
        let diags = run_cop_full_with_config(&HttpStatus, source, config);
        assert!(
            !diags.is_empty(),
            "numeric style should flag numeric string \"200\""
        );
        assert!(diags[0].message.contains("Unknown status code"));
    }

    #[test]
    fn be_status_style_flags_unknown_symbol() {
        use crate::cop::CopConfig;
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("be_status".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"it { is_expected.to have_http_status :oki_doki }\n";
        let diags = run_cop_full_with_config(&HttpStatus, source, config);
        assert!(
            !diags.is_empty(),
            "be_status style should flag unknown symbol :oki_doki"
        );
        assert!(diags[0].message.contains("be_oki_doki"));
    }
}
