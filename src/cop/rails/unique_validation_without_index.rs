use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

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
        let attr_name = match extract_symbol_name(&arg_list[0]) {
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
        let class_name = match crate::schema::find_enclosing_class_name(
            source.as_bytes(),
            call.location().start_offset(),
            parse_result,
        ) {
            Some(n) => n,
            None => return,
        };
        let table_name = crate::schema::table_name_from_source(source.as_bytes(), &class_name);

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

/// Extract a symbol name from a symbol node.
fn extract_symbol_name(node: &ruby_prism::Node<'_>) -> Option<String> {
    node.as_symbol_node()
        .map(|s| String::from_utf8_lossy(s.unescaped()).to_string())
}

/// Find a specific key's value in keyword hash arguments.
fn find_hash_value<'a>(args: &[ruby_prism::Node<'a>], key: &str) -> Option<ruby_prism::Node<'a>> {
    for arg in args {
        let elements = if let Some(kh) = arg.as_keyword_hash_node() {
            kh.elements().iter().collect::<Vec<_>>()
        } else if let Some(h) = arg.as_hash_node() {
            h.elements().iter().collect::<Vec<_>>()
        } else {
            continue;
        };

        for elem in elements {
            if let Some(assoc) = elem.as_assoc_node() {
                if assoc_key_matches(&assoc.key(), key) {
                    return Some(assoc.value());
                }
            }
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
        let elements = if let Some(kh) = arg.as_keyword_hash_node() {
            kh.elements().iter().collect::<Vec<_>>()
        } else if let Some(h) = arg.as_hash_node() {
            h.elements().iter().collect::<Vec<_>>()
        } else {
            continue;
        };

        for elem in elements {
            if let Some(assoc) = elem.as_assoc_node() {
                let key = assoc.key();
                if assoc_key_matches(&key, "if")
                    || assoc_key_matches(&key, "unless")
                    || assoc_key_matches(&key, "conditions")
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Check if a node is a hash containing conditional keys (if:, unless:, conditions:).
fn is_hash_with_conditional(node: &ruby_prism::Node<'_>) -> bool {
    let elements = if let Some(h) = node.as_hash_node() {
        h.elements().iter().collect::<Vec<_>>()
    } else if let Some(kh) = node.as_keyword_hash_node() {
        kh.elements().iter().collect::<Vec<_>>()
    } else {
        return false;
    };

    for elem in elements {
        if let Some(assoc) = elem.as_assoc_node() {
            let key = assoc.key();
            if assoc_key_matches(&key, "if")
                || assoc_key_matches(&key, "unless")
                || assoc_key_matches(&key, "conditions")
            {
                return true;
            }
        }
    }
    false
}

/// Extract scope columns from the uniqueness value.
/// The value can be: `true`, `{ scope: :col }`, or `{ scope: [:col1, :col2] }`.
fn extract_scope_columns(uniqueness_value: &ruby_prism::Node<'_>) -> Option<Vec<String>> {
    let elements = if let Some(h) = uniqueness_value.as_hash_node() {
        h.elements().iter().collect::<Vec<_>>()
    } else if let Some(kh) = uniqueness_value.as_keyword_hash_node() {
        kh.elements().iter().collect::<Vec<_>>()
    } else {
        return None;
    };

    for elem in elements {
        if let Some(assoc) = elem.as_assoc_node() {
            if assoc_key_matches(&assoc.key(), "scope") {
                return extract_scope_from_node(&assoc.value());
            }
        }
    }
    None
}

/// Extract column names from a scope value (symbol, string, or array of them).
fn extract_scope_from_node(node: &ruby_prism::Node<'_>) -> Option<Vec<String>> {
    if let Some(sym) = node.as_symbol_node() {
        return Some(vec![String::from_utf8_lossy(sym.unescaped()).to_string()]);
    }
    if let Some(s) = node.as_string_node() {
        return Some(vec![String::from_utf8_lossy(s.unescaped()).to_string()]);
    }
    if let Some(arr) = node.as_array_node() {
        let cols: Vec<String> = arr
            .elements()
            .iter()
            .filter_map(|e| {
                if let Some(sym) = e.as_symbol_node() {
                    Some(String::from_utf8_lossy(sym.unescaped()).to_string())
                } else {
                    e.as_string_node()
                        .map(|s| String::from_utf8_lossy(s.unescaped()).to_string())
                }
            })
            .collect();
        if !cols.is_empty() {
            return Some(cols);
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
