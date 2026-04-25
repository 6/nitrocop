//! Schema analysis for `db/schema.rb`.
//!
//! Provides a global `OnceLock<Option<Schema>>` that is initialized once
//! before the parallel lint loop. Schema-aware cops call `schema::get()`
//! to access parsed table/column/index metadata.

use std::path::Path;
use std::sync::OnceLock;

use ruby_prism::Visit;

/// Global schema singleton. Initialized once in `run_linter()`.
static SCHEMA: OnceLock<Option<Schema>> = OnceLock::new();

/// Thread-local schema override for tests. Checked before the global singleton.
#[cfg(test)]
thread_local! {
    static TEST_SCHEMA: std::cell::RefCell<Option<Schema>> = const { std::cell::RefCell::new(None) };
}

/// Initialize the global schema by walking up from `cwd` looking for
/// `db/schema.rb`, mirroring RuboCop's `RuboCop::Rails::SchemaLoader#db_schema_path`
/// which starts at `Pathname.pwd` and walks toward the filesystem root.
///
/// Safe to call multiple times — only the first call takes effect. The
/// `_legacy_root` argument is preserved for call-site compatibility but
/// ignored; schema discovery is anchored to the current working directory
/// to match RuboCop. Pinning to `cwd` is important for corpus runs where
/// the user invokes nitrocop from outside the repo (e.g., `cwd=/tmp`):
/// RuboCop returns no schema in that case and the cop skips, so nitrocop
/// must skip too rather than walking up from each source file.
pub fn init(_legacy_root: Option<&Path>) {
    SCHEMA.get_or_init(|| {
        let cwd = std::env::current_dir().ok()?;
        let mut path = cwd.as_path();
        loop {
            let candidate = path.join("db/schema.rb");
            if candidate.is_file() {
                let bytes = std::fs::read(&candidate).ok()?;
                return Schema::parse(&bytes);
            }
            path = path.parent()?;
        }
    });
}

/// Set a thread-local schema for testing. Call with `None` to clear.
#[cfg(test)]
pub fn set_test_schema(schema: Option<Schema>) {
    TEST_SCHEMA.with(|s| {
        *s.borrow_mut() = schema;
    });
}

/// Get a reference to the parsed schema, if available.
///
/// In test mode, checks thread-local override first.
#[cfg(not(test))]
pub fn get() -> Option<&'static Schema> {
    SCHEMA.get().and_then(|o| o.as_ref())
}

/// Get a reference to the parsed schema, if available.
///
/// In test mode, checks thread-local override first, then falls back to global.
/// Returns a leaked &'static reference when using thread-local (acceptable in tests).
#[cfg(test)]
pub fn get() -> Option<&'static Schema> {
    TEST_SCHEMA.with(|s| {
        let borrow = s.borrow();
        if let Some(ref schema) = *borrow {
            // Leak the clone for 'static lifetime (test-only, acceptable)
            let boxed = Box::new(schema.clone());
            return Some(&*Box::leak(boxed));
        }
        drop(borrow);
        SCHEMA.get().and_then(|o| o.as_ref())
    })
}

#[derive(Debug, Clone)]
pub struct Schema {
    pub tables: Vec<Table>,
    pub add_indices: Vec<AddIndex>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub indices: Vec<Index>,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Index {
    pub columns: Vec<String>,
    pub expression: Option<String>,
    pub unique: bool,
}

/// Represents a top-level `add_index` call (outside `create_table` blocks).
#[derive(Debug, Clone)]
pub struct AddIndex {
    pub table_name: String,
    pub columns: Vec<String>,
    pub expression: Option<String>,
    pub unique: bool,
}

impl Schema {
    /// Parse `db/schema.rb` content into a Schema.
    pub fn parse(source: &[u8]) -> Option<Schema> {
        let parse_result = ruby_prism::parse(source);
        let mut visitor = SchemaVisitor::new();
        visitor.visit(&parse_result.node());
        Some(Schema {
            tables: visitor.tables,
            add_indices: visitor.add_indices,
        })
    }

    /// Look up a table by name.
    pub fn table_by(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    /// Get all top-level add_index entries for a given table name.
    pub fn add_indices_for(&self, table_name: &str) -> Vec<&AddIndex> {
        self.add_indices
            .iter()
            .filter(|i| i.table_name == table_name)
            .collect()
    }

    /// Check if a unique index exists covering the given columns for a table.
    ///
    /// Checks both inline indices (inside create_table) and top-level add_index calls.
    /// Column order doesn't matter — we check set equality.
    pub fn has_unique_index(&self, table_name: &str, columns: &[String]) -> bool {
        let mut sorted_cols: Vec<&str> = columns.iter().map(|s| s.as_str()).collect();
        sorted_cols.sort();

        // Check inline indices
        if let Some(table) = self.table_by(table_name) {
            for idx in &table.indices {
                if !idx.unique {
                    continue;
                }
                let mut idx_cols: Vec<&str> = idx.columns.iter().map(|s| s.as_str()).collect();
                idx_cols.sort();
                if idx_cols == sorted_cols {
                    return true;
                }
                // Also check expression indices
                if let Some(ref expr) = idx.expression {
                    if columns.len() == 1 && expr.contains(&columns[0]) {
                        return true;
                    }
                }
            }
        }

        // Check top-level add_index
        for idx in self.add_indices_for(table_name) {
            if !idx.unique {
                continue;
            }
            let mut idx_cols: Vec<&str> = idx.columns.iter().map(|s| s.as_str()).collect();
            idx_cols.sort();
            if idx_cols == sorted_cols {
                return true;
            }
            if let Some(ref expr) = idx.expression {
                if columns.len() == 1 && expr.contains(&columns[0]) {
                    return true;
                }
            }
        }

        false
    }
}

impl Table {
    /// Check if this table has a column with the given name.
    pub fn has_column(&self, name: &str) -> bool {
        self.columns.iter().any(|c| c.name == name)
    }
}

// ---- Schema parser (Prism AST visitor) ----

struct SchemaVisitor {
    tables: Vec<Table>,
    add_indices: Vec<AddIndex>,
    /// When inside a `create_table` block, accumulates columns and indices.
    current_table: Option<(String, Vec<Column>, Vec<Index>)>,
}

impl SchemaVisitor {
    fn new() -> Self {
        Self {
            tables: Vec::new(),
            add_indices: Vec::new(),
            current_table: None,
        }
    }
}

/// Extract a string value from a string or symbol node.
fn extract_string_value(node: &ruby_prism::Node<'_>) -> Option<String> {
    if let Some(s) = node.as_string_node() {
        Some(String::from_utf8_lossy(s.unescaped()).to_string())
    } else {
        node.as_symbol_node()
            .map(|s| String::from_utf8_lossy(s.unescaped()).to_string())
    }
}

impl<'a> Visit<'a> for SchemaVisitor {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'a>) {
        let name = node.name();
        let name_str = std::str::from_utf8(name.as_slice()).unwrap_or("");

        match name_str {
            "create_table" => {
                // Extract table name from first argument
                if let Some(args) = node.arguments() {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if let Some(first) = arg_list.first() {
                        if let Some(table_name) = extract_string_value(first) {
                            self.current_table = Some((table_name, Vec::new(), Vec::new()));
                        }
                    }
                }
                // Visit the block body to find column definitions
                if let Some(block) = node.block() {
                    if let Some(block_node) = block.as_block_node() {
                        if let Some(body) = block_node.body() {
                            self.visit(&body);
                        }
                    }
                }
                // Finalize the table
                if let Some((name, columns, indices)) = self.current_table.take() {
                    self.tables.push(Table {
                        name,
                        columns,
                        indices,
                    });
                }
                return; // Don't recurse further
            }
            "add_index" => {
                // Top-level add_index "table_name", ["col1", "col2"], unique: true
                if let Some(args) = node.arguments() {
                    let arg_list: Vec<_> = args.arguments().iter().collect();
                    if arg_list.len() >= 2 {
                        let table_name = extract_string_value(&arg_list[0]);
                        let (columns, expression) = extract_index_columns(&arg_list[1]);
                        let unique = has_unique_option(&arg_list[2..]);

                        if let Some(tn) = table_name {
                            self.add_indices.push(AddIndex {
                                table_name: tn,
                                columns,
                                expression,
                                unique,
                            });
                        }
                    }
                }
                return;
            }
            _ => {}
        }

        // Inside create_table block: handle column definitions and index
        if self.current_table.is_some() {
            match name_str {
                "index" => {
                    // t.index ["col1", "col2"], unique: true
                    if let Some(args) = node.arguments() {
                        let arg_list: Vec<_> = args.arguments().iter().collect();
                        if !arg_list.is_empty() {
                            let (columns, expression) = extract_index_columns(&arg_list[0]);
                            let unique = has_unique_option(&arg_list[1..]);
                            if let Some((_, _, ref mut indices)) = self.current_table {
                                indices.push(Index {
                                    columns,
                                    expression,
                                    unique,
                                });
                            }
                        }
                    }
                    return;
                }
                // Column type methods: t.string, t.integer, t.text, etc.
                "string" | "text" | "integer" | "bigint" | "float" | "decimal" | "numeric"
                | "datetime" | "timestamp" | "time" | "date" | "binary" | "blob" | "boolean"
                | "json" | "jsonb" | "uuid" | "inet" | "cidr" | "macaddr" | "hstore" | "ltree"
                | "tsvector" | "tsquery" | "point" | "line" | "lseg" | "box" | "path"
                | "polygon" | "circle" | "bit" | "bit_varying" | "money" | "interval"
                | "int4range" | "int8range" | "numrange" | "tsrange" | "tstzrange"
                | "daterange" | "enum" | "serial" | "bigserial" | "virtual" | "column"
                | "references" => {
                    if let Some(args) = node.arguments() {
                        let arg_list: Vec<_> = args.arguments().iter().collect();
                        if let Some(first) = arg_list.first() {
                            if let Some(col_name) = extract_string_value(first) {
                                if let Some((_, ref mut columns, _)) = self.current_table {
                                    columns.push(Column { name: col_name });
                                }
                            }
                        }
                    }
                    return;
                }
                // t.timestamps adds created_at and updated_at
                "timestamps" => {
                    if let Some((_, ref mut columns, _)) = self.current_table {
                        columns.push(Column {
                            name: "created_at".to_string(),
                        });
                        columns.push(Column {
                            name: "updated_at".to_string(),
                        });
                    }
                    return;
                }
                _ => {}
            }
        }

        // Default: recurse into children
        ruby_prism::visit_call_node(self, node);
    }
}

/// Extract column names from an index column argument (array or string).
fn extract_index_columns(node: &ruby_prism::Node<'_>) -> (Vec<String>, Option<String>) {
    if let Some(arr) = node.as_array_node() {
        let cols: Vec<String> = arr
            .elements()
            .iter()
            .filter_map(|e| {
                if let Some(s) = e.as_string_node() {
                    Some(String::from_utf8_lossy(s.unescaped()).to_string())
                } else {
                    e.as_symbol_node()
                        .map(|s| String::from_utf8_lossy(s.unescaped()).to_string())
                }
            })
            .collect();
        (cols, None)
    } else if let Some(s) = node.as_string_node() {
        let value = String::from_utf8_lossy(s.unescaped()).to_string();
        if value.contains('(') || value.contains(' ') {
            (Vec::new(), Some(value))
        } else {
            (vec![value], None)
        }
    } else if let Some(s) = node.as_symbol_node() {
        (
            vec![String::from_utf8_lossy(s.unescaped()).to_string()],
            None,
        )
    } else {
        (Vec::new(), None)
    }
}

/// Check if any of the remaining arguments contain `unique: true`.
fn has_unique_option(args: &[ruby_prism::Node<'_>]) -> bool {
    for arg in args {
        let elements: Vec<_> = if let Some(kh) = arg.as_keyword_hash_node() {
            kh.elements().iter().collect()
        } else if let Some(h) = arg.as_hash_node() {
            h.elements().iter().collect()
        } else {
            continue;
        };

        for elem in elements {
            if let Some(assoc) = elem.as_assoc_node() {
                let key = assoc.key();
                let key_name = if let Some(sym) = key.as_symbol_node() {
                    String::from_utf8_lossy(sym.unescaped()).to_string()
                } else if let Some(s) = key.as_string_node() {
                    String::from_utf8_lossy(s.unescaped()).to_string()
                } else {
                    continue;
                };
                if key_name == "unique" && assoc.value().as_true_node().is_some() {
                    return true;
                }
            }
        }
    }
    false
}

// ---- Table name resolution helpers ----

/// Convert CamelCase to snake_case (ActiveSupport's `underscore` method).
///
/// Examples:
/// - `User` → `user`
/// - `UserProfile` → `user_profile`
/// - `HTMLParser` → `html_parser`
/// - `URLValidator` → `url_validator`
pub fn camel_to_snake(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            if prev.is_lowercase()
                || prev.is_ascii_digit()
                || (prev.is_uppercase() && i + 1 < chars.len() && chars[i + 1].is_lowercase())
            {
                result.push('_');
            }
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

/// Basic English pluralization following Rails conventions.
pub fn pluralize(s: &str) -> String {
    if s.is_empty() {
        return s.to_string();
    }

    let uncountable = [
        "equipment",
        "information",
        "rice",
        "money",
        "species",
        "series",
        "fish",
        "sheep",
        "jeans",
        "police",
        "news",
        "data",
        "media",
    ];
    let lower = s.to_lowercase();
    if uncountable.contains(&lower.as_str()) {
        return s.to_string();
    }

    let irregulars: &[(&str, &str)] = &[
        ("person", "people"),
        ("man", "men"),
        ("woman", "women"),
        ("child", "children"),
        ("sex", "sexes"),
        ("move", "moves"),
        ("zombie", "zombies"),
        ("goose", "geese"),
        ("ox", "oxen"),
        ("mouse", "mice"),
        ("tooth", "teeth"),
        ("foot", "feet"),
        ("datum", "data"),
        ("medium", "media"),
        ("criterion", "criteria"),
        ("index", "indices"),
        ("matrix", "matrices"),
        ("vertex", "vertices"),
        ("status", "statuses"),
        ("alias", "aliases"),
        ("bus", "buses"),
        ("octopus", "octopi"),
        ("virus", "viruses"),
        ("campus", "campuses"),
        ("quiz", "quizzes"),
    ];
    for &(singular, plural) in irregulars {
        if lower == singular {
            return plural.to_string();
        }
    }

    if lower.ends_with("ies")
        || lower.ends_with("ses")
        || lower.ends_with("xes")
        || lower.ends_with("zes")
        || lower.ends_with("ches")
        || lower.ends_with("shes")
    {
        return s.to_string();
    }

    if lower.ends_with("is") {
        return format!("{}es", &s[..s.len() - 2]);
    }

    if lower.ends_with("us") {
        return format!("{}i", &s[..s.len() - 2]);
    }

    if lower.ends_with("ss")
        || lower.ends_with("sh")
        || lower.ends_with("ch")
        || lower.ends_with('x')
        || lower.ends_with('z')
    {
        return format!("{s}es");
    }

    if lower.ends_with('y') {
        let before_y = lower.as_bytes().get(lower.len() - 2).copied().unwrap_or(0);
        if matches!(before_y, b'a' | b'e' | b'i' | b'o' | b'u') {
            return format!("{s}s");
        } else {
            return format!("{}ies", &s[..s.len() - 1]);
        }
    }

    if lower.ends_with('f') {
        return format!("{}ves", &s[..s.len() - 1]);
    }

    if lower.ends_with("fe") {
        return format!("{}ves", &s[..s.len() - 2]);
    }

    format!("{s}s")
}

/// Derive the table name for a class from source.
///
/// Strategy:
/// 1. Look for `self.table_name = '...'` in the source
/// 2. If not found, derive from class name: CamelCase → snake_case → pluralize
pub fn table_name_from_source(source: &[u8], class_name: &str) -> String {
    if let Ok(text) = std::str::from_utf8(source) {
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("self.table_name") {
                let rest = rest.trim();
                if let Some(rest) = rest.strip_prefix('=') {
                    let rest = rest.trim();
                    if let Some(val) = extract_quoted_string(rest) {
                        return val.to_string();
                    }
                }
            }
        }
    }
    // Handle namespaced classes: `Web::Setting` → `web_settings`
    // Replace `::` with `_` to match Rails table name convention.
    let snake = camel_to_snake(class_name).replace("::", "_");
    pluralize(&snake)
}

/// Extract a single/double quoted string value.
fn extract_quoted_string(s: &str) -> Option<&str> {
    if (s.starts_with('\'') || s.starts_with('"')) && s.len() >= 2 {
        let quote = s.as_bytes()[0];
        if let Some(end) = s[1..].find(quote as char) {
            return Some(&s[1..1 + end]);
        }
    }
    None
}

/// Extract the class name from the enclosing class node.
///
/// Walks the AST to find the innermost `class` statement that contains
/// the given byte offset, then extracts the constant name.
pub fn find_enclosing_class_name(
    source: &[u8],
    node_offset: usize,
    parse_result: &ruby_prism::ParseResult<'_>,
) -> Option<String> {
    let mut finder = ClassFinder {
        source,
        target_offset: node_offset,
        class_name: None,
    };
    finder.visit(&parse_result.node());
    finder.class_name
}

struct ClassFinder<'a> {
    source: &'a [u8],
    target_offset: usize,
    class_name: Option<String>,
}

impl<'a> Visit<'a> for ClassFinder<'a> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'a>) {
        let loc = node.location();
        if self.target_offset >= loc.start_offset() && self.target_offset < loc.end_offset() {
            let name_node = node.constant_path();
            if let Some(n) = extract_constant_name(self.source, &name_node) {
                self.class_name = Some(n);
            }
            ruby_prism::visit_class_node(self, node);
        }
    }
}

/// Find the parent module name for a `has_*` association, replicating RuboCop's
/// `parent_module_name` behavior.
///
/// Unlike `find_enclosing_class_name`, this returns `None` if there is any
/// non-`class_eval` block node in the ancestor chain between the target and
/// the root. For `ClassName.class_eval do...end` blocks, it returns the
/// constant receiver name. This matches RuboCop's behavior where
/// `parent_module_name` returns `nil` when any regular block ancestor is found.
pub fn find_parent_module_name(
    source: &[u8],
    node_offset: usize,
    parse_result: &ruby_prism::ParseResult<'_>,
) -> Option<String> {
    let mut finder = ParentModuleFinder {
        source,
        target_offset: node_offset,
        result: None,
        found: false,
    };
    finder.visit(&parse_result.node());
    if finder.found { finder.result } else { None }
}

struct ParentModuleFinder<'a> {
    source: &'a [u8],
    target_offset: usize,
    /// The resolved class name (None means "blocked by a non-class_eval block").
    result: Option<String>,
    /// Whether we found the target at all.
    found: bool,
}

impl<'a> ParentModuleFinder<'a> {
    fn contains(&self, node: &ruby_prism::Node<'_>) -> bool {
        let loc = node.location();
        self.target_offset >= loc.start_offset() && self.target_offset < loc.end_offset()
    }
}

impl<'a> Visit<'a> for ParentModuleFinder<'a> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'a>) {
        let node_ref = node.as_node();
        if self.contains(&node_ref) {
            let name_node = node.constant_path();
            if let Some(n) = extract_constant_name(self.source, &name_node) {
                self.result = Some(n);
            }
            ruby_prism::visit_class_node(self, node);
        }
    }

    fn visit_module_node(&mut self, node: &ruby_prism::ModuleNode<'a>) {
        let node_ref = node.as_node();
        if self.contains(&node_ref) {
            ruby_prism::visit_module_node(self, node);
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'a>) {
        let node_ref = node.as_node();
        if !self.contains(&node_ref) {
            return;
        }

        // Check if this is the target call (at the exact offset)
        if node.location().start_offset() == self.target_offset {
            self.found = true;
            return;
        }

        // Check if this call has a block containing the target
        if let Some(block) = node.block() {
            let block_loc = block.location();
            if self.target_offset >= block_loc.start_offset()
                && self.target_offset < block_loc.end_offset()
            {
                // This is a block ancestor. Check if it's class_eval on a constant.
                let method_name = node.name();
                if method_name.as_slice() == b"class_eval" {
                    if let Some(recv) = node.receiver() {
                        if let Some(name) = extract_constant_name(self.source, &recv) {
                            // class_eval on a constant — use the constant name
                            self.result = Some(name);
                            // Continue visiting inside the block
                            ruby_prism::visit_call_node(self, node);
                            return;
                        }
                    }
                }
                // Non-class_eval block — RuboCop returns nil
                self.result = None;
                // Still need to recurse to find the target, but mark as blocked
                let saved = self.result.take();
                ruby_prism::visit_call_node(self, node);
                // Force result to None since we're blocked by a non-class_eval block
                let _ = saved;
                self.result = None;
                return;
            }
        }

        ruby_prism::visit_call_node(self, node);
    }

    // Handle standalone block nodes (e.g., `begin...end` blocks, though rare)
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'a>) {
        let node_ref = node.as_node();
        if self.contains(&node_ref) {
            ruby_prism::visit_block_node(self, node);
        }
    }
}

/// Extract the full name from a constant node (ConstantReadNode or ConstantPathNode).
/// For `Web::Setting`, returns `"Web::Setting"` (not just `"Setting"`).
fn extract_constant_name(source: &[u8], node: &ruby_prism::Node<'_>) -> Option<String> {
    if let Some(cr) = node.as_constant_read_node() {
        Some(String::from_utf8_lossy(cr.name().as_slice()).to_string())
    } else if let Some(cp) = node.as_constant_path_node() {
        // Extract the full text of the constant path (e.g., "Web::Setting")
        let loc = cp.location();
        let text = std::str::from_utf8(&source[loc.start_offset()..loc.end_offset()]).ok()?;
        // Strip leading :: (top-level constant)
        let text = text.strip_prefix("::").unwrap_or(text);
        Some(text.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camel_to_snake() {
        assert_eq!(camel_to_snake("User"), "user");
        assert_eq!(camel_to_snake("UserProfile"), "user_profile");
        assert_eq!(camel_to_snake("HTMLParser"), "html_parser");
        assert_eq!(camel_to_snake("URLValidator"), "url_validator");
        assert_eq!(camel_to_snake("A"), "a");
        assert_eq!(camel_to_snake("ABC"), "abc");
        assert_eq!(camel_to_snake("FooBar"), "foo_bar");
    }

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize("user"), "users");
        assert_eq!(pluralize("user_profile"), "user_profiles");
        assert_eq!(pluralize("category"), "categories");
        assert_eq!(pluralize("bus"), "buses");
        assert_eq!(pluralize("box"), "boxes");
        assert_eq!(pluralize("church"), "churches");
        assert_eq!(pluralize("fish"), "fish");
        assert_eq!(pluralize("person"), "people");
        assert_eq!(pluralize("status"), "statuses");
        assert_eq!(pluralize("company"), "companies");
        assert_eq!(pluralize("day"), "days");
        assert_eq!(pluralize("analysis"), "analyses");
    }

    #[test]
    fn test_table_name_from_class() {
        assert_eq!(
            table_name_from_source(b"class User < ApplicationRecord\nend\n", "User"),
            "users"
        );
        assert_eq!(
            table_name_from_source(
                b"class UserProfile < ApplicationRecord\nend\n",
                "UserProfile"
            ),
            "user_profiles"
        );
    }

    #[test]
    fn test_table_name_explicit() {
        let source = b"class User < ApplicationRecord\n  self.table_name = 'accounts'\nend\n";
        assert_eq!(table_name_from_source(source, "User"), "accounts");
    }

    #[test]
    fn test_parse_schema_basic() {
        let schema = br#"
ActiveRecord::Schema[7.0].define(version: 2024_01_01) do
  create_table "users", force: :cascade do |t|
    t.string "name"
    t.string "email"
    t.integer "age"
    t.index ["email"], unique: true
  end

  create_table "posts", force: :cascade do |t|
    t.string "title"
    t.text "body"
    t.bigint "user_id"
    t.index ["user_id"]
  end

  add_index "users", ["name", "email"], unique: true
end
"#;
        let parsed = Schema::parse(schema).unwrap();
        assert_eq!(parsed.tables.len(), 2);

        let users = parsed.table_by("users").unwrap();
        assert_eq!(users.columns.len(), 3);
        assert!(users.has_column("name"));
        assert!(users.has_column("email"));
        assert!(users.has_column("age"));
        assert!(!users.has_column("missing"));
        assert_eq!(users.indices.len(), 1);
        assert!(users.indices[0].unique);
        assert_eq!(users.indices[0].columns, vec!["email"]);

        let posts = parsed.table_by("posts").unwrap();
        assert_eq!(posts.columns.len(), 3);
        assert!(posts.has_column("title"));

        assert_eq!(parsed.add_indices.len(), 1);
        assert_eq!(parsed.add_indices[0].table_name, "users");
        assert!(parsed.add_indices[0].unique);
        assert_eq!(parsed.add_indices[0].columns, vec!["name", "email"]);
    }

    #[test]
    fn test_parse_schema_empty() {
        let schema = b"ActiveRecord::Schema[7.0].define(version: 2024_01_01) do\nend\n";
        let parsed = Schema::parse(schema).unwrap();
        assert!(parsed.tables.is_empty());
        assert!(parsed.add_indices.is_empty());
    }

    #[test]
    fn test_has_unique_index() {
        let schema = br#"
ActiveRecord::Schema[7.0].define(version: 2024_01_01) do
  create_table "users", force: :cascade do |t|
    t.string "email"
    t.string "name"
    t.index ["email"], unique: true
  end
  add_index "users", ["name", "email"], unique: true
end
"#;
        let parsed = Schema::parse(schema).unwrap();
        assert!(parsed.has_unique_index("users", &["email".to_string()]));
        assert!(parsed.has_unique_index("users", &["name".to_string(), "email".to_string()]));
        assert!(parsed.has_unique_index("users", &["email".to_string(), "name".to_string()]));
        assert!(!parsed.has_unique_index("users", &["name".to_string()]));
        assert!(!parsed.has_unique_index("posts", &["email".to_string()]));
    }
}
