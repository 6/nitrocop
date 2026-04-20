use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use ruby_prism::Visit;

use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/UniqueValidationWithoutIndex
///
/// Checks that uniqueness validations have a corresponding unique index
/// on the database column(s). Requires schema analysis (db/schema.rb).
///
/// Corpus runs invoke nitrocop with overlay configs that can place
/// `config_dir()` outside the target repo. When that happens, the global schema
/// singleton is unset because `db/schema.rb` is looked up in the wrong directory.
/// This cop falls back to loading `db/schema.rb` relative to the current source
/// file's repo root when the global schema is unavailable.
///
/// Corpus mismatches also showed two model-resolution gaps:
/// nested model classes inside modules/classes were resolved as only the
/// innermost constant, and compound table names like `DnsAlias` /
/// `ServiceStatus` were inflected from the whole snake-cased string instead of
/// pluralizing only the final segment. This implementation now reconstructs the
/// full enclosing model name and pluralizes only the last table-name segment so
/// it matches RuboCop on namespaced and compound model names.
///
/// Later corpus FPs were not matcher bugs: `--only Rails/UniqueValidationWithoutIndex`
/// was force-enabling the Rails plugin department even when the target repo's
/// config never loaded `rubocop-rails`. That behavior now stays limited to
/// `--force-default-config`, so project-configured runs match RuboCop's
/// plugin-loading rules.
///
/// ## Synthetic corpus note
/// RuboCop's SchemaLoader crashes on `t.timestamps` (no arguments) in
/// db/schema.rb because `Column.new` calls `node.first_argument.str_content`
/// which raises NoMethodError on nil. When schema loading fails, both RuboCop
/// and nitrocop silently skip schema-dependent cops. The synthetic schema was
/// fixed to use explicit `t.datetime "created_at"` columns instead.
pub struct UniqueValidationWithoutIndex;

/// Fallback schema loader that finds db/schema.rb relative to the source file's
/// repo root when the global schema is unavailable.
fn schema_for_source(source: &SourceFile) -> Option<&'static crate::schema::Schema> {
    if let Some(schema) = crate::schema::get() {
        return Some(schema);
    }

    static FALLBACK_SCHEMAS: OnceLock<
        Mutex<HashMap<PathBuf, Option<&'static crate::schema::Schema>>>,
    > = OnceLock::new();

    let repo_root = source
        .path
        .ancestors()
        .find(|path| path.join("db/schema.rb").is_file())?
        .to_path_buf();

    let mut cache = FALLBACK_SCHEMAS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .ok()?;

    if let Some(schema) = cache.get(&repo_root).copied() {
        return schema;
    }

    let schema = std::fs::read(repo_root.join("db/schema.rb"))
        .ok()
        .and_then(|bytes| crate::schema::Schema::parse(&bytes))
        .map(|schema| Box::leak(Box::new(schema)) as &'static crate::schema::Schema);

    cache.insert(repo_root, schema);
    schema
}

const MSG: &str = "Uniqueness validation should have a unique index on the database column.";

impl Cop for UniqueValidationWithoutIndex {
    fn name(&self) -> &'static str {
        "Rails/UniqueValidationWithoutIndex"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let schema = match schema_for_source(source) {
            Some(s) => s,
            None => return,
        };

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method_name = call.name();
        let method_str = std::str::from_utf8(method_name.as_slice()).unwrap_or("");

        // Note: RuboCop only handles `validates`, not `validates_uniqueness_of`.
        // Skip to match RuboCop's behavior.
        if method_str == "validates" {
            self.check_validates(source, &call, parse_result, schema, diagnostics);
        }
    }
}

impl UniqueValidationWithoutIndex {
    fn check_validates(
        &self,
        source: &SourceFile,
        call: &ruby_prism::CallNode<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        schema: &crate::schema::Schema,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };
        let arg_list: Vec<_> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        // First arg is the attribute name (symbol)
        let attr_name = match extract_attribute_name(&arg_list[0]) {
            Some(n) => n,
            None => return,
        };

        // Look for uniqueness: key in keyword args
        let uniqueness_value = match find_hash_value(&arg_list[1..], "uniqueness") {
            Some(v) => v,
            None => return,
        };

        // Skip if uniqueness: false or uniqueness: nil
        if uniqueness_value.as_false_node().is_some() || uniqueness_value.as_nil_node().is_some() {
            return;
        }

        // Skip if conditional (if:, unless:, conditions: present in outer hash)
        if has_conditional_keys(&arg_list[1..]) {
            return;
        }
        // Also check inside the uniqueness hash for conditionals
        if is_hash_with_conditional(&uniqueness_value) {
            return;
        }

        // Resolve table name
        let class_name = match find_enclosing_model_name(
            source.as_bytes(),
            call.location().start_offset(),
            parse_result,
        ) {
            Some(n) => n,
            None => return,
        };
        let table_name = table_name_from_source(source.as_bytes(), &class_name);

        // Check table exists in schema
        if schema.table_by(&table_name).is_none() {
            return;
        }

        // Collect columns: the validated attribute + scope columns
        let mut columns = vec![attr_name];
        if let Some(scope_cols) = extract_scope_columns(&uniqueness_value) {
            columns.extend(scope_cols);
        }

        // Resolve association names to foreign key columns (e.g., :user → user_id)
        if let Some(table) = schema.table_by(&table_name) {
            columns = columns
                .into_iter()
                .map(|c| resolve_column(table, &c))
                .collect();
        }

        // Check for unique index
        if !schema.has_unique_index(&table_name, &columns) {
            let loc = call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(source, line, column, MSG.to_string()));
        }
    }
}

/// Resolve a column name: if the table has a `{name}_id` column but not
/// `{name}`, use `{name}_id`. This handles the standard Rails convention
/// where `belongs_to :user` creates a `user_id` foreign key column.
fn resolve_column(table: &crate::schema::Table, name: &str) -> String {
    if table.has_column(name) {
        return name.to_string();
    }
    let id_name = format!("{name}_id");
    if table.has_column(&id_name) {
        return id_name;
    }
    name.to_string()
}

/// Extract an attribute name from a symbol or string node.
fn extract_attribute_name(node: &ruby_prism::Node<'_>) -> Option<String> {
    if let Some(sym) = node.as_symbol_node() {
        Some(String::from_utf8_lossy(sym.unescaped()).to_string())
    } else {
        node.as_string_node()
            .map(|s| String::from_utf8_lossy(s.unescaped()).to_string())
    }
}

/// Find a specific key's value in keyword hash arguments.
fn find_hash_value<'a>(args: &[ruby_prism::Node<'a>], key: &str) -> Option<ruby_prism::Node<'a>> {
    for arg in args {
        let mut found = None;
        visit_assocs(arg, |assoc| {
            if assoc_key_matches(&assoc.key(), key) {
                found = Some(assoc.value());
                true
            } else {
                false
            }
        });
        if let Some(value) = found {
            return Some(value);
        }
    }
    None
}

/// Check if an assoc key (symbol or string) matches the given name.
fn assoc_key_matches(key: &ruby_prism::Node<'_>, name: &str) -> bool {
    if let Some(sym) = key.as_symbol_node() {
        sym.unescaped() == name.as_bytes()
    } else if let Some(s) = key.as_string_node() {
        s.unescaped() == name.as_bytes()
    } else {
        false
    }
}

/// Check if any keyword args contain if:, unless:, or conditions: keys.
fn has_conditional_keys(args: &[ruby_prism::Node<'_>]) -> bool {
    for arg in args {
        let mut has_conditionals = false;
        visit_assocs(arg, |assoc| {
            let key = assoc.key();
            has_conditionals = assoc_key_matches(&key, "if")
                || assoc_key_matches(&key, "unless")
                || assoc_key_matches(&key, "conditions");
            has_conditionals
        });
        if has_conditionals {
            return true;
        }
    }
    false
}

/// Visit assoc pairs represented either directly or inside a hash-like node.
fn visit_assocs<'a, F>(node: &ruby_prism::Node<'a>, mut visitor: F) -> bool
where
    F: FnMut(ruby_prism::AssocNode<'a>) -> bool,
{
    if let Some(assoc) = node.as_assoc_node() {
        return visitor(assoc);
    }

    let elements = if let Some(hash) = node.as_hash_node() {
        Some(hash.elements())
    } else {
        node.as_keyword_hash_node().map(|hash| hash.elements())
    };

    let Some(elements) = elements else {
        return false;
    };

    for elem in elements.iter() {
        if let Some(assoc) = elem.as_assoc_node() {
            if visitor(assoc) {
                return true;
            }
        }
    }
    false
}

/// Check if a node is a hash containing conditional keys (if:, unless:, conditions:).
fn is_hash_with_conditional(node: &ruby_prism::Node<'_>) -> bool {
    let mut has_conditionals = false;
    visit_assocs(node, |assoc| {
        let key = assoc.key();
        has_conditionals = assoc_key_matches(&key, "if")
            || assoc_key_matches(&key, "unless")
            || assoc_key_matches(&key, "conditions");
        has_conditionals
    });
    has_conditionals
}

/// Extract scope columns from the uniqueness value.
/// The value can be: `true`, `{ scope: :col }`, or `{ scope: [:col1, :col2] }`.
fn extract_scope_columns(uniqueness_value: &ruby_prism::Node<'_>) -> Option<Vec<String>> {
    let mut scope = None;
    visit_assocs(uniqueness_value, |assoc| {
        if assoc_key_matches(&assoc.key(), "scope") {
            scope = extract_scope_from_node(&assoc.value());
            true
        } else {
            false
        }
    });
    scope
}

/// Extract column names from a scope value (symbol, string, or array of them).
fn extract_scope_from_node(node: &ruby_prism::Node<'_>) -> Option<Vec<String>> {
    if let Some(call) = node.as_call_node() {
        let no_args = call
            .arguments()
            .is_none_or(|args| args.arguments().is_empty());
        if call.name().as_slice() == b"freeze" && no_args && call.block().is_none() {
            if let Some(receiver) = call.receiver() {
                return extract_scope_from_node(&receiver);
            }
        }
    }

    if let Some(sym) = node.as_symbol_node() {
        return Some(vec![String::from_utf8_lossy(sym.unescaped()).to_string()]);
    }
    if let Some(s) = node.as_string_node() {
        return Some(vec![String::from_utf8_lossy(s.unescaped()).to_string()]);
    }
    if let Some(arr) = node.as_array_node() {
        let mut cols = Vec::new();
        for element in arr.elements().iter() {
            if let Some(sym) = element.as_symbol_node() {
                cols.push(String::from_utf8_lossy(sym.unescaped()).to_string());
            } else if let Some(string) = element.as_string_node() {
                cols.push(String::from_utf8_lossy(string.unescaped()).to_string());
            } else {
                return None;
            }
        }
        if !cols.is_empty() {
            return Some(cols);
        }
    }
    None
}

fn find_enclosing_model_name(
    source: &[u8],
    node_offset: usize,
    parse_result: &ruby_prism::ParseResult<'_>,
) -> Option<String> {
    let mut finder = EnclosingModelFinder {
        source,
        target_offset: node_offset,
        scope: Vec::new(),
        class_name: None,
    };
    finder.visit(&parse_result.node());
    finder.class_name.map(|segments| segments.join("::"))
}

struct EnclosingModelFinder<'a> {
    source: &'a [u8],
    target_offset: usize,
    scope: Vec<String>,
    class_name: Option<Vec<String>>,
}

impl<'a> EnclosingModelFinder<'a> {
    fn contains(&self, location: ruby_prism::Location) -> bool {
        self.target_offset >= location.start_offset() && self.target_offset < location.end_offset()
    }

    fn scoped_name(&self, name: &str) -> Vec<String> {
        if name.contains("::") {
            name.split("::").map(ToOwned::to_owned).collect()
        } else {
            let mut scoped = self.scope.clone();
            scoped.push(name.to_string());
            scoped
        }
    }
}

impl<'a> Visit<'a> for EnclosingModelFinder<'a> {
    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'a>) {
        if !self.contains(node.location()) {
            return;
        }

        let previous_scope = self.scope.clone();
        if let Some(name) = extract_constant_name(self.source, &node.constant_path()) {
            self.scope = self.scoped_name(&name);
        }

        ruby_prism::visit_module_node(self, node);
        self.scope = previous_scope;
    }

    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'a>) {
        if !self.contains(node.location()) {
            return;
        }

        let previous_scope = self.scope.clone();
        if let Some(name) = extract_constant_name(self.source, &node.constant_path()) {
            let scoped_name = self.scoped_name(&name);
            self.scope = scoped_name.clone();
            self.class_name = Some(scoped_name);
        }

        ruby_prism::visit_class_node(self, node);
        self.scope = previous_scope;
    }
}

fn extract_constant_name(source: &[u8], node: &ruby_prism::Node<'_>) -> Option<String> {
    if let Some(constant) = node.as_constant_read_node() {
        Some(String::from_utf8_lossy(constant.name().as_slice()).to_string())
    } else if let Some(path) = node.as_constant_path_node() {
        let location = path.location();
        let text =
            std::str::from_utf8(&source[location.start_offset()..location.end_offset()]).ok()?;
        Some(text.strip_prefix("::").unwrap_or(text).to_string())
    } else {
        None
    }
}

fn table_name_from_source(source: &[u8], class_name: &str) -> String {
    if let Ok(text) = std::str::from_utf8(source) {
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("self.table_name") {
                let rest = rest.trim();
                if let Some(rest) = rest.strip_prefix('=') {
                    let rest = rest.trim();
                    if let Some(value) = extract_quoted_string(rest) {
                        return value.to_string();
                    }
                }
            }
        }
    }

    let snake = crate::schema::camel_to_snake(class_name).replace("::", "_");
    pluralize_table_name(&snake)
}

fn pluralize_table_name(name: &str) -> String {
    if let Some((prefix, last_segment)) = name.rsplit_once('_') {
        format!("{prefix}_{}", crate::schema::pluralize(last_segment))
    } else {
        crate::schema::pluralize(name)
    }
}

fn extract_quoted_string(s: &str) -> Option<&str> {
    if (s.starts_with('\'') || s.starts_with('"')) && s.len() >= 2 {
        let quote = s.as_bytes()[0];
        if let Some(end) = s[1..].find(quote as char) {
            return Some(&s[1..1 + end]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Schema;

    fn setup_schema() {
        let schema_bytes = include_bytes!(
            "../../../tests/fixtures/cops/rails/unique_validation_without_index/schema.rb"
        );
        let schema = Schema::parse(schema_bytes).unwrap();
        crate::schema::set_test_schema(Some(schema));
    }

    #[test]
    fn offense_fixture() {
        setup_schema();
        crate::testutil::assert_cop_offenses_full(
            &UniqueValidationWithoutIndex,
            include_bytes!(
                "../../../tests/fixtures/cops/rails/unique_validation_without_index/offense.rb"
            ),
        );
        crate::schema::set_test_schema(None);
    }

    #[test]
    fn no_offense_fixture() {
        setup_schema();
        crate::testutil::assert_cop_no_offenses_full(
            &UniqueValidationWithoutIndex,
            include_bytes!(
                "../../../tests/fixtures/cops/rails/unique_validation_without_index/no_offense.rb"
            ),
        );
        crate::schema::set_test_schema(None);
    }
}
