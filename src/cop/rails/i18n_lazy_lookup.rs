use crate::cop::shared::access_modifier_predicates;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Rails/I18nLazyLookup — checks for places where I18n lazy lookup can be used.
///
/// In lazy mode (default), flags explicit keys like `t('books.create.success')` that could
/// be replaced with `t('.success')`. In explicit mode, flags lazy keys.
///
/// Key matching derives controller path from the file path and compares against the translation
/// key. Only public action methods in Controller classes are checked.
///
/// FP fix: In lazy mode, symbol keys (e.g., `t(:'spotlight.admin_users.index.page_title')`)
/// are now skipped. RuboCop's `handle_lazy_style` compares `key_node.value` (a Symbol) against
/// `scoped_key` (a String) — in Ruby, `Symbol == String` is always false, so symbol keys are
/// never flagged. Previously nitrocop converted both to byte slices and compared them, causing
/// 13 false positives across spotlight, hyrax, and avalon repos.
///
/// Location fix: Both lazy and explicit styles now report the offense at the key node's
/// location (the string/symbol argument), not the enclosing call node's location. This matches
/// RuboCop's behavior where the offense is reported on `key_node`, not on `node`. This is
/// particularly important for multi-line t() calls where the key may appear on a different
/// line than the method name.
pub struct I18nLazyLookup;

impl Cop for I18nLazyLookup {
    fn name(&self) -> &'static str {
        "Rails/I18nLazyLookup"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/controllers/**/*.rb"]
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
        let path = source.path_str();
        if !is_controller_file(path) {
            return;
        }

        let style = config.get_str("EnforcedStyle", "lazy");
        let controller_prefix = controller_prefix_from_path(path);

        let mut visitor = I18nLazyLookupVisitor {
            cop: self,
            source,
            style,
            controller_prefix,
            current_method: None,
            in_controller_class: false,
            method_visibility: Visibility::Public,
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Public,
    Private,
    Protected,
}

struct I18nLazyLookupVisitor<'a> {
    cop: &'a I18nLazyLookup,
    source: &'a SourceFile,
    style: &'a str,
    controller_prefix: Option<String>,
    current_method: Option<Vec<u8>>,
    in_controller_class: bool,
    method_visibility: Visibility,
    diagnostics: Vec<Diagnostic>,
}

impl<'pr> Visit<'pr> for I18nLazyLookupVisitor<'_> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        let prev_in_controller = self.in_controller_class;
        let prev_visibility = self.method_visibility;

        // Check if the class name ends with "Controller"
        let class_name = node.constant_path();
        let name_ends_with_controller = class_name_ends_with_controller(&class_name);
        self.in_controller_class = name_ends_with_controller;
        self.method_visibility = Visibility::Public;

        ruby_prism::visit_class_node(self, node);

        self.in_controller_class = prev_in_controller;
        self.method_visibility = prev_visibility;
    }

    fn visit_def_node(&mut self, node: &ruby_prism::DefNode<'pr>) {
        let prev_method = self.current_method.take();
        self.current_method = Some(node.name().as_slice().to_vec());
        ruby_prism::visit_def_node(self, node);
        self.current_method = prev_method;
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let method = node.name().as_slice();

        // Track visibility modifiers: bare `private`, `protected`, `public` calls
        if access_modifier_predicates::is_bare_access_modifier(node) {
            match method {
                b"private" => self.method_visibility = Visibility::Private,
                b"protected" => self.method_visibility = Visibility::Protected,
                b"public" => self.method_visibility = Visibility::Public,
                _ => {}
            }
        }

        // Only match bare t/translate calls (no receiver)
        let is_bare_t = (method == b"t" || method == b"translate") && node.receiver().is_none();

        if is_bare_t {
            if let Some(args) = node.arguments() {
                let arg_list: Vec<_> = args.arguments().iter().collect();
                if !arg_list.is_empty() {
                    let key_loc = arg_list[0].location();
                    let (key, is_symbol) = if let Some(s) = arg_list[0].as_string_node() {
                        (Some(s.unescaped().to_vec()), false)
                    } else if let Some(sym) = arg_list[0].as_symbol_node() {
                        (Some(sym.unescaped().to_vec()), true)
                    } else {
                        (None, false)
                    };

                    if let Some(key) = key {
                        self.check_key(node, &key, is_symbol, key_loc);
                    }
                }
            }
        }

        ruby_prism::visit_call_node(self, node);
    }
}

impl I18nLazyLookupVisitor<'_> {
    fn check_key(
        &mut self,
        node: &ruby_prism::CallNode<'_>,
        key: &[u8],
        is_symbol: bool,
        key_loc: ruby_prism::Location,
    ) {
        // Only flag inside Controller classes with public methods
        if !self.in_controller_class {
            return;
        }
        if self.method_visibility != Visibility::Public {
            return;
        }

        match self.style {
            "explicit" => {
                // Flag lazy lookups (keys starting with '.')
                if !key.starts_with(b".") {
                    return;
                }
                let (line, column) = self.source.offset_to_line_col(key_loc.start_offset());
                self.diagnostics.push(self.cop.diagnostic(
                    self.source,
                    line,
                    column,
                    "Use explicit lookup for i18n keys.".to_string(),
                ));
            }
            _ => {
                // "lazy" (default): flag explicit lookups that could use lazy lookup
                if key.starts_with(b".") {
                    return;
                }
                // RuboCop only flags string keys in lazy mode — symbol keys are
                // skipped because its internal comparison (Symbol == String) is false.
                if is_symbol {
                    return;
                }
                // Must have at least 3 segments (controller.action.key)
                let dot_count = key.iter().filter(|&&b| b == b'.').count();
                if dot_count < 2 {
                    return;
                }
                // Must be inside a method def
                let method_name = match &self.current_method {
                    Some(m) => m,
                    None => return,
                };

                // Match RuboCop's behavior: construct a scoped key from
                // controller_path + action_name + last segment of the given key.
                // Only flag if the full key matches this scoped key exactly.
                // This means keys with extra intermediate segments (e.g.,
                // "books.create.flash.success" vs scoped "books.create.success")
                // are NOT flagged.
                if let Some(ref prefix) = self.controller_prefix {
                    let method_str = std::str::from_utf8(method_name).unwrap_or("");
                    let key_str = std::str::from_utf8(key).unwrap_or("");
                    // Extract last segment of the key
                    let last_segment = match key_str.rsplit('.').next() {
                        Some(s) => s,
                        None => return,
                    };
                    let scoped_key = format!("{}.{}.{}", prefix, method_str, last_segment);
                    if key_str != scoped_key {
                        return;
                    }
                }

                let (line, column) = self.source.offset_to_line_col(key_loc.start_offset());
                self.diagnostics.push(self.cop.diagnostic(
                    self.source,
                    line,
                    column,
                    "Use lazy lookup for i18n keys.".to_string(),
                ));
            }
        }
    }
}

/// Check if a class constant path ends with "Controller".
/// Handles both simple names (`BooksController`) and qualified paths (`Admin::BooksController`).
fn class_name_ends_with_controller(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(const_read) = node.as_constant_read_node() {
        return const_read.name().as_slice().ends_with(b"Controller");
    }
    if let Some(const_path) = node.as_constant_path_node() {
        if let Some(name) = const_path.name() {
            return name.as_slice().ends_with(b"Controller");
        }
    }
    false
}

/// Check if the file path looks like a Rails controller file.
fn is_controller_file(path: &str) -> bool {
    if path.contains("controllers/") || path.contains("controllers\\") {
        return true;
    }
    if path.ends_with("_controller.rb") {
        return true;
    }
    false
}

/// Derive a controller prefix from the file path for key matching.
/// E.g., "app/controllers/admin/accounts_controller.rb" => "admin.accounts"
fn controller_prefix_from_path(path: &str) -> Option<String> {
    let idx = path.find("controllers/")?;
    let rest = &path[idx + "controllers/".len()..];
    let rest = rest.strip_suffix(".rb")?;
    let rest = rest.strip_suffix("_controller").unwrap_or(rest);
    Some(rest.replace('/', "."))
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(I18nLazyLookup, "cops/rails/i18n_lazy_lookup");

    #[test]
    fn explicit_style_flags_lazy_key() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("explicit".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"# nitrocop-filename: app/controllers/books_controller.rb\nclass BooksController < ApplicationController\n  def create\n    t('.success')\n    ^^^^^^^^^^^^^ Rails/I18nLazyLookup: Use explicit lookup for i18n keys.\n  end\nend\n";
        assert_cop_offenses_full_with_config(&I18nLazyLookup, source, config);
    }

    #[test]
    fn explicit_style_allows_explicit_key() {
        use crate::cop::CopConfig;
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("explicit".to_string()),
            )]),
            ..CopConfig::default()
        };
        let source = b"# nitrocop-filename: app/controllers/books_controller.rb\nt('books.create.success')\n";
        assert_cop_no_offenses_full_with_config(&I18nLazyLookup, source, config);
    }

    #[test]
    fn does_not_flag_outside_controller() {
        use crate::testutil::assert_cop_no_offenses;
        assert_cop_no_offenses(&I18nLazyLookup, b"t('admin.reports.processed_msg')\n");
    }

    #[test]
    fn does_not_flag_i18n_t() {
        use crate::testutil::assert_cop_no_offenses;
        assert_cop_no_offenses(&I18nLazyLookup, b"I18n.t('admin.reports.processed_msg')\n");
    }

    #[test]
    fn does_not_flag_mismatched_action() {
        use crate::testutil::assert_cop_no_offenses_full;
        // Key has action 'update' but we're in method 'validate_confirmation_token'
        let source = b"# nitrocop-filename: app/controllers/email_confirmations_controller.rb\nclass EmailConfirmationsController < ApplicationController\n  def validate_confirmation_token\n    t(\"email_confirmations.update.token_failure\")\n  end\nend\n";
        assert_cop_no_offenses_full(&I18nLazyLookup, source);
    }

    #[test]
    fn does_not_flag_concern_module() {
        use crate::testutil::assert_cop_no_offenses_full;
        // Concern module in controllers/ path — not a Controller class
        let source = b"# nitrocop-filename: app/controllers/concerns/linkable_controller.rb\nmodule LinkableController\n  def sync\n    t(\"concerns.linkable.sync.success\")\n  end\nend\n";
        assert_cop_no_offenses_full(&I18nLazyLookup, source);
    }

    #[test]
    fn does_not_flag_private_method_in_controller() {
        use crate::testutil::assert_cop_no_offenses_full;
        let source = b"# nitrocop-filename: app/controllers/application_controller.rb\nclass ApplicationController < ActionController::Base\n  def index\n    render plain: \"ok\"\n  end\n\n  private\n\n  def require_cookies\n    t(\"application.require_cookies.cookies_needed\")\n  end\nend\n";
        assert_cop_no_offenses_full(&I18nLazyLookup, source);
    }

    #[test]
    fn controller_prefix_extraction() {
        assert_eq!(
            controller_prefix_from_path("app/controllers/admin/accounts_controller.rb"),
            Some("admin.accounts".to_string())
        );
        assert_eq!(
            controller_prefix_from_path("app/controllers/books_controller.rb"),
            Some("books".to_string())
        );
        assert_eq!(controller_prefix_from_path("app/models/user.rb"), None);
    }
}
