use crate::cop::shared::constant_predicates;
use crate::cop::shared::node_type::{CALL_NODE, CONSTANT_PATH_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/ResponseParsedBody
///
/// Detection note: the matcher already handles the corpus-shaped nested RSpec
/// form `expect(JSON.parse(response.body)).to eq({ ... })`; the fixture locks
/// in that multiline/nested case so it does not regress.
///
/// FN fix: RuboCop's ResponseParsedBody cop does NOT use `requires_gem
/// 'railties'` — it only checks `minimum_target_rails_version 5.0` via the
/// `TargetRailsVersion` config setting. We use `target_rails_version()`
/// directly (not `rails_version_at_least()`) because the latter also
/// requires `railties` in `Gemfile.lock`, which the corpus lockfile does
/// not have (it only bundles linter gems, not Rails itself).
///
/// Corpus note: the remaining large FN bucket is a config/runtime artifact,
/// not a matcher bug. This cop is Include-gated (`spec/controllers/**/*.rb`,
/// `spec/requests/**/*.rb`, `test/controllers/**/*.rb`,
/// `test/integration/**/*.rb`), and those patterns resolve relative to
/// `base_dir` (CWD for non-dotfile configs). Reproduced on
/// `Arie/serveme@7f28d1b`: running from `/tmp` reports 0 offenses, while
/// running with CWD at the repo root reports the expected 42 offenses.
/// Keep the narrow default includes; widening or removing them would trade
/// these corpus FNs for real false positives outside controller/request tests.
pub struct ResponseParsedBody;

impl Cop for ResponseParsedBody {
    fn name(&self) -> &'static str {
        "Rails/ResponseParsedBody"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &[
            "spec/controllers/**/*.rb",
            "spec/requests/**/*.rb",
            "test/controllers/**/*.rb",
            "test/integration/**/*.rb",
        ]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, CONSTANT_PATH_NODE]
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
        // minimum_target_rails_version 5.0
        // Use target_rails_version() directly — RuboCop's cop does NOT use
        // `requires_gem 'railties'`, only `minimum_target_rails_version 5.0`.
        if !config.target_rails_version().is_some_and(|v| v >= 5.0) {
            return;
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.name().as_slice() != b"parse" {
            return;
        }

        // Must have exactly 1 argument (response.body) — no keyword args or extra args.
        // RuboCop's node pattern requires exactly one argument:
        //   (send (const {nil? cbase} :JSON) :parse (send (send nil? :response) :body))
        // If there are additional arguments (e.g., symbolize_names: true), it does NOT match.
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.len() != 1 {
            return;
        }

        let arg_call = match arg_list[0].as_call_node() {
            Some(c) => c,
            None => return,
        };
        if arg_call.name().as_slice() != b"body" {
            return;
        }

        // The receiver of .body should be `response`
        let body_recv = match arg_call.receiver() {
            Some(r) => r,
            None => return,
        };
        let body_recv_call = match body_recv.as_call_node() {
            Some(c) => c,
            None => return,
        };
        if body_recv_call.name().as_slice() != b"response" {
            return;
        }

        // Receiver must be constant `JSON` or `Nokogiri::HTML`/`Nokogiri::HTML5`
        let recv = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        // Check for JSON.parse(response.body)
        if constant_predicates::constant_short_name(&recv) == Some(b"JSON") {
            let loc = node.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                "Prefer `response.parsed_body` to `JSON.parse(response.body)`.".to_string(),
            ));
        }

        // Check for Nokogiri::HTML.parse(response.body) / Nokogiri::HTML5.parse(response.body)
        // RuboCop only checks Nokogiri patterns when target_rails_version >= 7.1
        if config.target_rails_version().is_some_and(|v| v >= 7.1) {
            if let Some(cp) = recv.as_constant_path_node() {
                if let Some(name) = cp.name() {
                    let name_bytes = name.as_slice();
                    if name_bytes == b"HTML" || name_bytes == b"HTML5" {
                        if let Some(parent) = cp.parent() {
                            if constant_predicates::constant_short_name(&parent)
                                == Some(b"Nokogiri")
                            {
                                let const_name = std::str::from_utf8(name_bytes).unwrap_or("HTML");
                                let loc = node.location();
                                let (line, column) = source.offset_to_line_col(loc.start_offset());
                                diagnostics.push(self.diagnostic(
                                    source,
                                    line,
                                    column,
                                    format!("Prefer `response.parsed_body` to `Nokogiri::{const_name}.parse(response.body)`."),
                                ));
                            }
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

    crate::cop_rails_fixture_tests!(ResponseParsedBody, "cops/rails/response_parsed_body", 5.0);

    #[test]
    fn nokogiri_requires_rails_7_1() {
        // Nokogiri::HTML.parse(response.body) is only flagged at Rails >= 7.1
        let source = b"Nokogiri::HTML.parse(response.body)\nNokogiri::HTML5.parse(response.body)\n";

        // At 5.0 — no offense
        let parsed = crate::testutil::parse_fixture(source);
        let mut options = std::collections::HashMap::new();
        options.insert(
            "TargetRailsVersion".to_string(),
            serde_yml::Value::Number(serde_yml::value::Number::from(5.0)),
        );
        let config = crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        };
        let diags =
            crate::testutil::run_cop_full_with_config(&ResponseParsedBody, &parsed.source, config);
        assert!(diags.is_empty(), "Nokogiri should not fire at Rails 5.0");

        // At 7.1 — offense
        let mut options = std::collections::HashMap::new();
        options.insert(
            "TargetRailsVersion".to_string(),
            serde_yml::Value::Number(serde_yml::value::Number::from(7.1)),
        );
        let config = crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        };
        let diags =
            crate::testutil::run_cop_full_with_config(&ResponseParsedBody, &parsed.source, config);
        assert_eq!(diags.len(), 2, "Nokogiri should fire at Rails 7.1");
    }
}
