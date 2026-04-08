use crate::cop::shared::node_type::{CALL_NODE, INTEGER_NODE, STRING_NODE, SYMBOL_NODE};
use crate::cop::shared::util::keyword_arg_value;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/HttpStatus — enforces symbolic or numeric HTTP status codes.
///
/// ## Investigation findings (2026-03-08)
///
/// **FP root cause (12 FP):** RuboCop's `http_status` NodePattern requires `(send nil? ...)` —
/// the method call must be receiverless. Nitrocop was not checking for nil receiver, so
/// `foo.render(status: 200)` or `response.head(200)` were incorrectly flagged.
/// Fix: return early when `call.receiver().is_some()`.
///
/// **FN root cause (9 FN, 2026-03-08):** RuboCop's `status_code` pattern matches `${int sym str}` — it
/// handles string status codes like `'200'` in addition to integer and symbol nodes.
/// Nitrocop only handled IntegerNode and SymbolNode, missing StringNode.
/// Fix: add StringNode handling — parse string content as integer, then look up in status maps.
///
/// **FN root cause (5 FN, 2026-03-16):** Rack-style status strings like `"404 Not Found"` and
/// `"401 Unauthorized"` were not parsed correctly. The previous code did a plain integer parse
/// of the entire string, which fails for strings with a trailing description. RuboCop uses
/// `Rack::Utils` which accepts these "status reason phrase" strings and extracts the numeric
/// prefix. Fix: split on whitespace and parse only the leading token, matching Rack's behavior.
/// The diagnostic message uses the full original string content as the "current" value (e.g.
/// `over '404 Not Found'`), matching RuboCop's output exactly.
///
/// **FP root cause (2 FP, 2026-03-18):** Strings like `"404 AWOL"` and `"500 Sorry"` have
/// custom non-standard reason phrases. The cop was parsing the numeric prefix and flagging them,
/// but RuboCop uses `Rack::Utils::SYMBOL_TO_STATUS_CODE` which only recognizes known reason
/// phrases. A string like `"404 AWOL"` doesn't match any Rack status mapping, so RuboCop skips
/// it. Fix: when a string status contains whitespace, validate that it exactly matches a known
/// Rack "code reason-phrase" pair. Strings with unknown/custom reason phrases are not flagged.
///
/// ## Corpus investigation (2026-03-19, updated 2026-03-23)
///
/// Corpus oracle reported FP=21, FN=0.
///
/// **FP root cause (21 FP, 2026-03-23):** RuboCop's `http_status` NodePattern for
/// `head` and `assert_response` is `(send nil? {:head :assert_response} ${int sym} ...)`
/// which only matches direct positional args (`head 200`, `head :ok`), NOT keyword hash
/// args like `head status: 204`. The `$hash`/`status_code` pattern is only used for
/// `render`, `redirect_to`, and `assert_redirected_to`. Nitrocop was incorrectly checking
/// keyword `status:` for all methods. Fix: skip keyword arg lookup for `head` and
/// `assert_response` (the `DIRECT_STATUS_METHODS`), matching RuboCop's NodePattern exactly.
/// The previous investigation incorrectly attributed these FPs to file-exclusion issues.
///
/// **Variant divergence fix (numeric style, 2026-04-08):** RuboCop's `NumericStyleChecker`
/// uses `Rack::Utils::SYMBOL_TO_STATUS_CODE` to determine if a symbol maps to a valid
/// HTTP status code. The FP root cause was that Nitrocop's `symbol_to_status_code`
/// included `:unprocessable_entity` (422) which is NOT in `Rack::Utils::SYMBOL_TO_STATUS_CODE`
/// (Rails 7.1 renamed it to `:unprocessable_content`). The FN root cause was that
/// `:unprocessable_content` was NOT in Nitrocop's map but IS in `Rack::Utils`.
/// Fix: updated `symbol_to_status_code` to use `:unprocessable_content` (matching Rack::Utils)
/// and `status_code_to_symbol` to return `:unprocessable_content` for code 422.
/// This ensures nitrocop matches RuboCop's behavior for the `numeric` EnforcedStyle.
pub struct HttpStatus;

fn status_code_to_symbol(code: i64) -> Option<&'static str> {
    match code {
        100 => Some(":continue"),
        101 => Some(":switching_protocols"),
        102 => Some(":processing"),
        103 => Some(":early_hints"),
        200 => Some(":ok"),
        201 => Some(":created"),
        202 => Some(":accepted"),
        203 => Some(":non_authoritative_information"),
        204 => Some(":no_content"),
        205 => Some(":reset_content"),
        206 => Some(":partial_content"),
        207 => Some(":multi_status"),
        208 => Some(":already_reported"),
        226 => Some(":im_used"),
        300 => Some(":multiple_choices"),
        301 => Some(":moved_permanently"),
        302 => Some(":found"),
        303 => Some(":see_other"),
        304 => Some(":not_modified"),
        305 => Some(":use_proxy"),
        307 => Some(":temporary_redirect"),
        308 => Some(":permanent_redirect"),
        400 => Some(":bad_request"),
        401 => Some(":unauthorized"),
        402 => Some(":payment_required"),
        403 => Some(":forbidden"),
        404 => Some(":not_found"),
        405 => Some(":method_not_allowed"),
        406 => Some(":not_acceptable"),
        407 => Some(":proxy_authentication_required"),
        408 => Some(":request_timeout"),
        409 => Some(":conflict"),
        410 => Some(":gone"),
        411 => Some(":length_required"),
        412 => Some(":precondition_failed"),
        413 => Some(":payload_too_large"),
        414 => Some(":uri_too_long"),
        415 => Some(":unsupported_media_type"),
        416 => Some(":range_not_satisfiable"),
        417 => Some(":expectation_failed"),
        421 => Some(":misdirected_request"),
        422 => Some(":unprocessable_content"),
        423 => Some(":locked"),
        424 => Some(":failed_dependency"),
        425 => Some(":too_early"),
        426 => Some(":upgrade_required"),
        428 => Some(":precondition_required"),
        429 => Some(":too_many_requests"),
        431 => Some(":request_header_fields_too_large"),
        451 => Some(":unavailable_for_legal_reasons"),
        500 => Some(":internal_server_error"),
        501 => Some(":not_implemented"),
        502 => Some(":bad_gateway"),
        503 => Some(":service_unavailable"),
        504 => Some(":gateway_timeout"),
        505 => Some(":http_version_not_supported"),
        506 => Some(":variant_also_negotiates"),
        507 => Some(":insufficient_storage"),
        508 => Some(":loop_detected"),
        510 => Some(":not_extended"),
        511 => Some(":network_authentication_required"),
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
        b"unprocessable_content" => Some(422),
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

/// Returns the canonical Rack reason phrase for a given status code (e.g. 404 → "Not Found").
/// Used to validate Rack-style string status codes like "404 Not Found".
/// RuboCop only flags strings that exactly match a known Rack status format, so
/// strings like "404 AWOL" or "500 Sorry" with custom reason phrases must be skipped.
fn status_code_to_rack_reason(code: i64) -> Option<&'static str> {
    match code {
        100 => Some("Continue"),
        101 => Some("Switching Protocols"),
        102 => Some("Processing"),
        103 => Some("Early Hints"),
        200 => Some("OK"),
        201 => Some("Created"),
        202 => Some("Accepted"),
        203 => Some("Non-Authoritative Information"),
        204 => Some("No Content"),
        205 => Some("Reset Content"),
        206 => Some("Partial Content"),
        207 => Some("Multi-Status"),
        208 => Some("Already Reported"),
        226 => Some("IM Used"),
        300 => Some("Multiple Choices"),
        301 => Some("Moved Permanently"),
        302 => Some("Found"),
        303 => Some("See Other"),
        304 => Some("Not Modified"),
        305 => Some("Use Proxy"),
        307 => Some("Temporary Redirect"),
        308 => Some("Permanent Redirect"),
        400 => Some("Bad Request"),
        401 => Some("Unauthorized"),
        402 => Some("Payment Required"),
        403 => Some("Forbidden"),
        404 => Some("Not Found"),
        405 => Some("Method Not Allowed"),
        406 => Some("Not Acceptable"),
        407 => Some("Proxy Authentication Required"),
        408 => Some("Request Timeout"),
        409 => Some("Conflict"),
        410 => Some("Gone"),
        411 => Some("Length Required"),
        412 => Some("Precondition Failed"),
        413 => Some("Payload Too Large"),
        414 => Some("URI Too Long"),
        415 => Some("Unsupported Media Type"),
        416 => Some("Range Not Satisfiable"),
        417 => Some("Expectation Failed"),
        421 => Some("Misdirected Request"),
        422 => Some("Unprocessable Entity"),
        423 => Some("Locked"),
        424 => Some("Failed Dependency"),
        425 => Some("Too Early"),
        426 => Some("Upgrade Required"),
        428 => Some("Precondition Required"),
        429 => Some("Too Many Requests"),
        431 => Some("Request Header Fields Too Large"),
        451 => Some("Unavailable For Legal Reasons"),
        500 => Some("Internal Server Error"),
        501 => Some("Not Implemented"),
        502 => Some("Bad Gateway"),
        503 => Some("Service Unavailable"),
        504 => Some("Gateway Timeout"),
        505 => Some("HTTP Version Not Supported"),
        506 => Some("Variant Also Negotiates"),
        507 => Some("Insufficient Storage"),
        508 => Some("Loop Detected"),
        510 => Some("Not Extended"),
        511 => Some("Network Authentication Required"),
        _ => None,
    }
}

/// Permitted symbols that are not specific status codes (used by numeric style).
const PERMITTED_SYMBOLS: &[&[u8]] = &[b"error", b"success", b"missing", b"redirect"];

const STATUS_METHODS: &[&[u8]] = &[
    b"render",
    b"head",
    b"redirect_to",
    b"assert_response",
    b"assert_redirected_to",
];

/// Methods where status is passed as a direct first argument (not keyword)
const DIRECT_STATUS_METHODS: &[&[u8]] = &[b"head", b"assert_response"];

impl Cop for HttpStatus {
    fn name(&self) -> &'static str {
        "Rails/HttpStatus"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
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
        if !STATUS_METHODS.contains(&call.name().as_slice()) {
            return;
        }

        // RuboCop requires nil receiver (receiverless call): (send nil? ...)
        // e.g., `render status: 200` matches but `response.head 200` does not.
        if call.receiver().is_some() {
            return;
        }

        let method_name = call.name().as_slice();

        // RuboCop's NodePattern for head/assert_response only matches direct positional
        // args: `(send nil? {:head :assert_response} ${int sym} ...)`. It does NOT match
        // keyword hash args like `head status: 204`. Only render, redirect_to, and
        // assert_redirected_to use the `$hash` pattern that checks keyword `status:`.
        let keyword_status = if DIRECT_STATUS_METHODS.contains(&method_name) {
            None
        } else {
            keyword_arg_value(&call, b"status")
        };

        let check_status = |status_value: &ruby_prism::Node<'_>| -> Option<Diagnostic> {
            match style {
                "numeric" => {
                    if let Some(sym) = status_value.as_symbol_node() {
                        let sym_name = sym.unescaped();
                        if PERMITTED_SYMBOLS.contains(&sym_name) {
                            return None;
                        }
                        if let Some(code) = symbol_to_status_code(sym_name) {
                            let sym_str = std::str::from_utf8(sym_name).unwrap_or("?");
                            let val_loc = status_value.location();
                            let (line, column) = source.offset_to_line_col(val_loc.start_offset());
                            return Some(self.diagnostic(
                                source,
                                line,
                                column,
                                format!(
                                    "Prefer `{code}` over `:{sym_str}` to define HTTP status code."
                                ),
                            ));
                        }
                    }
                    None
                }
                _ => {
                    // symbolic style: flag integer and string status codes
                    // `display_val` is the "current" value shown in the message (matches RuboCop).
                    // For integers it's the number itself; for strings it's the original string
                    // content (e.g. "404 Not Found" stays as-is, not truncated to "404").
                    let (code_num_opt, display_val, val_loc) =
                        if let Some(_int) = status_value.as_integer_node() {
                            let loc = status_value.location();
                            let code_text = std::str::from_utf8(loc.as_slice()).unwrap_or("");
                            let num = code_text.parse::<i64>().ok();
                            let disp = code_text.to_string();
                            (num, disp, loc)
                        } else if let Some(str_node) = status_value.as_string_node() {
                            let content = str_node.unescaped();
                            let code_text = std::str::from_utf8(content).unwrap_or("");
                            // Support strings like "404" as well as "404 Not Found" (Rack-style).
                            // RuboCop uses Rack::Utils which only recognizes known reason phrases.
                            // Strings with custom/unknown reason phrases like "404 AWOL" must NOT
                            // be flagged. Validate that strings with whitespace exactly match
                            // a known Rack "code reason-phrase" pair before flagging.
                            let numeric_prefix =
                                code_text.split_ascii_whitespace().next().unwrap_or("");
                            let num = numeric_prefix.parse::<i64>().ok();
                            // If the string contains more than just the number, validate the
                            // reason phrase matches the canonical Rack reason phrase exactly.
                            let validated_num = if code_text.contains(' ') {
                                num.filter(|&n| {
                                    status_code_to_rack_reason(n)
                                        .map(|reason| {
                                            let expected = format!("{n} {reason}");
                                            code_text == expected
                                        })
                                        .unwrap_or(false)
                                })
                            } else {
                                num
                            };
                            (
                                validated_num,
                                code_text.to_string(),
                                status_value.location(),
                            )
                        } else {
                            (None, String::new(), status_value.location())
                        };
                    if let Some(code_num) = code_num_opt {
                        if let Some(sym) = status_code_to_symbol(code_num) {
                            let (line, column) = source.offset_to_line_col(val_loc.start_offset());
                            return Some(self.diagnostic(
                                source,
                                line,
                                column,
                                format!(
                                    "Prefer `{sym}` over `{display_val}` to define HTTP status code."
                                ),
                            ));
                        }
                    }
                    None
                }
            }
        };

        if let Some(ref kw) = keyword_status {
            if let Some(diag) = check_status(kw) {
                diagnostics.push(diag);
            }
        }

        // For head and assert_response, also check first direct argument
        if keyword_status.is_none() && DIRECT_STATUS_METHODS.contains(&method_name) {
            if let Some(args) = call.arguments() {
                for first in args.arguments().iter().take(1) {
                    if first.as_integer_node().is_some()
                        || first.as_symbol_node().is_some()
                        || first.as_string_node().is_some()
                    {
                        if let Some(diag) = check_status(&first) {
                            diagnostics.push(diag);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(HttpStatus, "cops/rails/http_status");

    #[test]
    fn numeric_style_flags_symbolic_status() {
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
        let source = b"render :foo, status: :ok\n";
        let diags = run_cop_full_with_config(&HttpStatus, source, config);
        assert!(!diags.is_empty(), "numeric style should flag symbolic :ok");
        assert!(
            diags[0].message.contains("200"),
            "message should mention 200: {}",
            diags[0].message
        );
    }

    #[test]
    fn numeric_style_allows_numeric_status() {
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
        let source = b"render :foo, status: 200\n";
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
        let source = b"render :foo, status: :error\n";
        assert_cop_no_offenses_full_with_config(&HttpStatus, source, config);
    }
}
