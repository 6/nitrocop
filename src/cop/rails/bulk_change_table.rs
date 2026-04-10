use crate::cop::shared::method_dispatch_predicates;
use crate::cop::shared::node_type::{CALL_NODE, DEF_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Mirrors RuboCop's adapter resolution and migration-node coverage for
/// `Rails/BulkChangeTable`.
///
/// Adapter discovery first checks the current project root, then falls back to
/// the enclosing git checkout of the analyzed file to match RuboCop when corpus
/// runs execute from `/tmp`.
///
/// This update fixes false negatives from `config/database.yml` files that are
/// recoverable for RuboCop but fail our strict YAML parse because of duplicate
/// keys/sections or inline ERB values. The text fallback now resolves adapters
/// from merged `development: <<: *default` sections and from the first nested
/// development database entry, while still skipping files with standalone
/// top-level ERB control-flow lines like `<% ... %>` that RuboCop ignores after
/// a Psych syntax error.
pub struct BulkChangeTable;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DatabaseKind {
    Mysql,
    PostgreSQL,
}

/// Combinable alter methods for both MySQL and PostgreSQL.
const BASE_COMBINABLE_ALTER_METHODS: &[&[u8]] = &[
    b"add_column",
    b"remove_column",
    b"remove_columns",
    b"add_timestamps",
    b"remove_timestamps",
    b"change_column",
];

/// Combinable alter methods only supported by MySQL.
const MYSQL_COMBINABLE_ALTER_METHODS: &[&[u8]] = &[b"rename_column", b"add_index", b"remove_index"];

/// Combinable alter methods supported by PostgreSQL 5.2+.
const POSTGRESQL_COMBINABLE_ALTER_METHODS: &[&[u8]] = &[b"change_column_default"];

/// Combinable alter methods supported by PostgreSQL 6.1+.
const POSTGRESQL_61_COMBINABLE_ALTER_METHODS: &[&[u8]] = &[b"change_column_null"];

/// Combinable transformations inside `change_table` blocks for both MySQL and PostgreSQL.
const BASE_COMBINABLE_TABLE_METHODS: &[&[u8]] = &[
    b"primary_key",
    b"column",
    b"string",
    b"text",
    b"integer",
    b"bigint",
    b"float",
    b"decimal",
    b"numeric",
    b"datetime",
    b"timestamp",
    b"time",
    b"date",
    b"binary",
    b"boolean",
    b"json",
    b"virtual",
    b"remove",
    b"timestamps",
    b"remove_timestamps",
    b"change",
];

/// Combinable transformations only supported by MySQL.
const MYSQL_COMBINABLE_TABLE_METHODS: &[&[u8]] = &[b"rename", b"index", b"remove_index"];

/// Combinable transformations supported by PostgreSQL 5.2+.
const POSTGRESQL_COMBINABLE_TABLE_METHODS: &[&[u8]] = &[b"change_default"];

/// Combinable transformations supported by PostgreSQL 6.1+.
const POSTGRESQL_61_COMBINABLE_TABLE_METHODS: &[&[u8]] = &[b"change_null"];

/// Extract the table name from the first argument of an alter method call.
fn extract_table_name(call: &ruby_prism::CallNode<'_>) -> Option<Vec<u8>> {
    let args = call.arguments()?;
    let first = args.arguments().iter().next()?;

    if let Some(sym) = first.as_symbol_node() {
        return Some(sym.unescaped().to_vec());
    }
    if let Some(s) = first.as_string_node() {
        return Some(s.unescaped().to_vec());
    }
    None
}

fn database_kind(config: &CopConfig, source: &SourceFile) -> Option<DatabaseKind> {
    match config.get_str("Database", "") {
        "mysql" => Some(DatabaseKind::Mysql),
        "postgresql" => Some(DatabaseKind::PostgreSQL),
        "" => database_kind_from_yaml(source).or_else(database_kind_from_env),
        _ => None,
    }
}

fn database_kind_from_yaml(source: &SourceFile) -> Option<DatabaseKind> {
    if let Some(database) = std::env::current_dir()
        .ok()
        .map(|cwd| cwd.join("config/database.yml"))
        .filter(|path| path.is_file())
        .and_then(|path| parse_database_yml(&path))
    {
        return Some(database);
    }

    let repo_root = source
        .path
        .parent()?
        .ancestors()
        .find(|path| path.join(".git").exists())?;
    let database_yml = repo_root.join("config/database.yml");
    if !database_yml.is_file() {
        return None;
    }

    parse_database_yml(&database_yml)
}

fn parse_database_yml(path: &std::path::Path) -> Option<DatabaseKind> {
    let contents = std::fs::read_to_string(path).ok()?;

    if has_top_level_erb_control_flow(&contents) {
        return None;
    }

    if let Ok(mut yaml) = serde_yml::from_str::<serde_yml::Value>(&contents) {
        let _ = yaml.apply_merge();
        if let Some(database) = database_kind_from_parsed_yaml(&yaml) {
            return Some(database);
        }
    }

    database_kind_from_text(&contents)
}

fn has_top_level_erb_control_flow(contents: &str) -> bool {
    contents.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("%>")
            || trimmed.starts_with("-%>")
            || ((trimmed.starts_with("<%") || trimmed.starts_with("<%-"))
                && !trimmed.starts_with("<%=")
                && !trimmed.starts_with("<%-=")
                && !trimmed.starts_with("<%#")
                && !trimmed.starts_with("<%-#"))
    })
}

#[derive(Default)]
struct TextDatabaseSection {
    adapter: Option<String>,
    merge_anchor: Option<String>,
    first_nested_name: Option<String>,
    nested: std::collections::HashMap<String, TextDatabaseSection>,
}

fn database_kind_from_parsed_yaml(yaml: &serde_yml::Value) -> Option<DatabaseKind> {
    let development = yaml
        .as_mapping()?
        .get(serde_yml::Value::String("development".to_string()))?
        .as_mapping()?;

    adapter_from_mapping(development).and_then(database_kind_from_adapter)
}

fn database_kind_from_text(contents: &str) -> Option<DatabaseKind> {
    let mut sections = std::collections::HashMap::<String, TextDatabaseSection>::new();
    let mut anchors = std::collections::HashMap::<String, String>::new();
    let mut current_top: Option<String> = None;
    let mut current_nested: Option<String> = None;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("<%")
            || trimmed.starts_with("%>")
        {
            continue;
        }

        let indent = line.chars().take_while(|ch| ch.is_whitespace()).count();
        if indent == 0 {
            current_nested = None;
            let Some((section_name, section_value)) = parse_yaml_mapping_entry(trimmed) else {
                current_top = None;
                continue;
            };

            current_top = Some(section_name.to_string());

            if let Some(anchor) = extract_anchor_name(section_value) {
                anchors.insert(anchor.to_string(), section_name.to_string());
            }

            sections.entry(section_name.to_string()).or_default();
            continue;
        }

        let Some(top_name) = current_top.as_deref() else {
            continue;
        };

        if indent == 2 {
            current_nested = None;

            let Some((key, value)) = parse_yaml_mapping_entry(trimmed) else {
                continue;
            };

            let section = sections.entry(top_name.to_string()).or_default();

            match key {
                "adapter" => section.adapter = Some(clean_yaml_scalar(value).to_string()),
                "<<" => {
                    section.merge_anchor = extract_anchor_reference(value).map(str::to_string);
                }
                _ if is_nested_mapping(value) => {
                    let nested_name = key.to_string();
                    if section.first_nested_name.is_none() {
                        section.first_nested_name = Some(nested_name.clone());
                    }
                    section.nested.entry(nested_name.clone()).or_default();
                    current_nested = Some(nested_name);
                }
                _ => {}
            }
            continue;
        }

        if indent < 4 {
            continue;
        }

        let Some(nested_name) = current_nested.as_deref() else {
            continue;
        };
        let Some((key, value)) = parse_yaml_mapping_entry(trimmed) else {
            continue;
        };
        let Some(nested_section) = sections
            .get_mut(top_name)
            .and_then(|section| section.nested.get_mut(nested_name))
        else {
            continue;
        };

        match key {
            "adapter" => nested_section.adapter = Some(clean_yaml_scalar(value).to_string()),
            "<<" => {
                nested_section.merge_anchor = extract_anchor_reference(value).map(str::to_string);
            }
            _ => {}
        }
    }

    sections
        .get("development")
        .and_then(|section| resolve_text_section_database_kind(section, &sections, &anchors, 0))
}

fn parse_yaml_mapping_entry(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    Some((key.trim(), value.trim()))
}

fn clean_yaml_scalar(value: &str) -> &str {
    value
        .split('#')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .trim_matches(['"', '\''])
}

fn extract_anchor_name(value: &str) -> Option<&str> {
    clean_yaml_scalar(value)
        .split_whitespace()
        .next()
        .and_then(|token| token.strip_prefix('&'))
}

fn extract_anchor_reference(value: &str) -> Option<&str> {
    clean_yaml_scalar(value)
        .split_whitespace()
        .next()
        .and_then(|token| token.strip_prefix('*'))
}

fn is_nested_mapping(value: &str) -> bool {
    value.is_empty() || value.trim_start().starts_with('&')
}

fn resolve_text_section_database_kind(
    section: &TextDatabaseSection,
    sections: &std::collections::HashMap<String, TextDatabaseSection>,
    anchors: &std::collections::HashMap<String, String>,
    depth: usize,
) -> Option<DatabaseKind> {
    if depth > 8 {
        return None;
    }

    if let Some(adapter) = section.adapter.as_deref() {
        if let Some(database) = database_kind_from_adapter(adapter) {
            return Some(database);
        }
    }

    if let Some(anchor) = section.merge_anchor.as_deref() {
        if let Some(section_name) = anchors.get(anchor) {
            if let Some(merged_section) = sections.get(section_name) {
                if let Some(database) =
                    resolve_text_section_database_kind(merged_section, sections, anchors, depth + 1)
                {
                    return Some(database);
                }
            }
        }
    }

    if let Some(nested_name) = section.first_nested_name.as_deref() {
        if let Some(nested_section) = section.nested.get(nested_name) {
            return resolve_text_section_database_kind(
                nested_section,
                sections,
                anchors,
                depth + 1,
            );
        }
    }

    None
}

fn adapter_from_mapping(mapping: &serde_yml::Mapping) -> Option<&str> {
    if let Some(adapter) = mapping
        .get(serde_yml::Value::String("adapter".to_string()))
        .and_then(|value| value.as_str())
    {
        return Some(adapter);
    }

    mapping
        .values()
        .filter_map(|value| value.as_mapping())
        .find_map(|nested| {
            nested
                .get(serde_yml::Value::String("adapter".to_string()))
                .and_then(|value| value.as_str())
        })
}

fn database_kind_from_adapter(adapter: &str) -> Option<DatabaseKind> {
    match adapter {
        "mysql2" | "trilogy" => Some(DatabaseKind::Mysql),
        "postgresql" | "postgis" => Some(DatabaseKind::PostgreSQL),
        _ => None,
    }
}

fn database_kind_from_env() -> Option<DatabaseKind> {
    let database_url = std::env::var("DATABASE_URL").ok()?;

    if database_url.starts_with("mysql2://") || database_url.starts_with("trilogy://") {
        return Some(DatabaseKind::Mysql);
    }

    if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
        return Some(DatabaseKind::PostgreSQL);
    }

    None
}

fn supports_bulk_alter(database: DatabaseKind, config: &CopConfig) -> bool {
    match database {
        DatabaseKind::Mysql => true,
        DatabaseKind::PostgreSQL => config
            .target_rails_version()
            .is_some_and(|version| version >= 5.2),
    }
}

fn is_postgresql_61_or_later(config: &CopConfig) -> bool {
    config
        .target_rails_version()
        .is_some_and(|version| version >= 6.1)
}

fn is_combinable_alter_method(name: &[u8], database: DatabaseKind, config: &CopConfig) -> bool {
    if BASE_COMBINABLE_ALTER_METHODS.contains(&name) {
        return true;
    }

    match database {
        DatabaseKind::Mysql => MYSQL_COMBINABLE_ALTER_METHODS.contains(&name),
        DatabaseKind::PostgreSQL => {
            POSTGRESQL_COMBINABLE_ALTER_METHODS.contains(&name)
                || (is_postgresql_61_or_later(config)
                    && POSTGRESQL_61_COMBINABLE_ALTER_METHODS.contains(&name))
        }
    }
}

fn is_combinable_table_method(name: &[u8], database: DatabaseKind, config: &CopConfig) -> bool {
    if BASE_COMBINABLE_TABLE_METHODS.contains(&name) {
        return true;
    }

    match database {
        DatabaseKind::Mysql => MYSQL_COMBINABLE_TABLE_METHODS.contains(&name),
        DatabaseKind::PostgreSQL => {
            POSTGRESQL_COMBINABLE_TABLE_METHODS.contains(&name)
                || (is_postgresql_61_or_later(config)
                    && POSTGRESQL_61_COMBINABLE_TABLE_METHODS.contains(&name))
        }
    }
}

/// Check if a change_table call has `bulk: true` or `bulk: false`.
fn has_bulk_option(call: &ruby_prism::CallNode<'_>) -> bool {
    if let Some(args) = call.arguments() {
        for arg in args.arguments().iter() {
            // Check KeywordHashNode (common in call args)
            if let Some(kw) = arg.as_keyword_hash_node() {
                for elem in kw.elements().iter() {
                    if let Some(assoc) = elem.as_assoc_node() {
                        if let Some(sym) = assoc.key().as_symbol_node() {
                            if sym.unescaped() == b"bulk" {
                                return true;
                            }
                        }
                    }
                }
            }
            // Check HashNode (explicit hash literal)
            if let Some(hash) = arg.as_hash_node() {
                for elem in hash.elements().iter() {
                    if let Some(assoc) = elem.as_assoc_node() {
                        if let Some(sym) = assoc.key().as_symbol_node() {
                            if sym.unescaped() == b"bulk" {
                                return true;
                            }
                        }
                    }
                }
            }
        }
    }
    false
}

fn count_remove_arguments(call: &ruby_prism::CallNode<'_>) -> usize {
    call.arguments()
        .map(|args| {
            args.arguments()
                .iter()
                .filter(|arg| arg.as_hash_node().is_none() && arg.as_keyword_hash_node().is_none())
                .count()
        })
        .unwrap_or(0)
}

fn count_combinable_table_call(
    call: &ruby_prism::CallNode<'_>,
    database: DatabaseKind,
    config: &CopConfig,
) -> usize {
    let name = call.name().as_slice();
    if call.receiver().is_none() || !is_combinable_table_method(name, database, config) {
        return 0;
    }

    if name == b"remove" {
        return count_remove_arguments(call);
    }

    1
}

/// Count combinable top-level transformations inside a change_table block body.
fn count_combinable_in_block(
    block_body: &ruby_prism::Node<'_>,
    database: DatabaseKind,
    config: &CopConfig,
) -> usize {
    if let Some(stmts) = block_body.as_statements_node() {
        return stmts
            .body()
            .iter()
            .filter_map(|stmt| stmt.as_call_node())
            .map(|call| count_combinable_table_call(&call, database, config))
            .sum();
    }

    block_body
        .as_call_node()
        .map(|call| count_combinable_table_call(&call, database, config))
        .unwrap_or(0)
}

impl Cop for BulkChangeTable {
    fn name(&self) -> &'static str {
        "Rails/BulkChangeTable"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/db/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, DEF_NODE]
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
        let database = match database_kind(config, source) {
            Some(database) if supports_bulk_alter(database, config) => database,
            _ => return,
        };

        if let Some(call) = node.as_call_node() {
            if method_dispatch_predicates::is_command(&call, b"change_table")
                && !has_bulk_option(&call)
            {
                if let Some(block_node) = call.block().and_then(|block| block.as_block_node()) {
                    if let Some(block_body) = block_node.body() {
                        if count_combinable_in_block(&block_body, database, config) > 1 {
                            let loc = call.location();
                            let (line, column) = source.offset_to_line_col(loc.start_offset());
                            diagnostics.push(
                                self.diagnostic(
                                    source,
                                    line,
                                    column,
                                    "You can combine alter queries using `bulk: true` options."
                                        .to_string(),
                                ),
                            );
                        }
                    }
                }
            }
            return;
        }

        let def_node = match node.as_def_node() {
            Some(d) => d,
            None => return,
        };

        let def_name = def_node.name().as_slice();
        if def_name != b"change" && def_name != b"up" && def_name != b"down" {
            return;
        }

        if def_node.receiver().is_some() {
            return;
        }

        let body = match def_node.body() {
            Some(b) => b,
            None => return,
        };

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        // Check for consecutive combinable alter methods targeting the same table.
        let mut current_table: Option<Vec<u8>> = None;
        let mut current_offset = 0;
        let mut current_count = 0usize;

        let mut flush_run = |table: &mut Option<Vec<u8>>, offset: usize, count: &mut usize| {
            if *count > 1 {
                if let Some(table_name) = table.as_deref() {
                    let table_str = std::str::from_utf8(table_name).unwrap_or("table");
                    let (line, column) = source.offset_to_line_col(offset);
                    diagnostics.push(self.diagnostic(
                        source,
                        line,
                        column,
                        format!(
                            "You can use `change_table :{table_str}, bulk: true` to combine alter queries."
                        ),
                    ));
                }
            }
            *table = None;
            *count = 0;
        };

        for stmt in stmts.body().iter() {
            if let Some(call) = stmt.as_call_node() {
                let name = call.name().as_slice();
                if call.receiver().is_none() && is_combinable_alter_method(name, database, config) {
                    if let Some(table) = extract_table_name(&call) {
                        if current_table.as_deref() == Some(table.as_slice()) {
                            current_count += 1;
                        } else {
                            flush_run(&mut current_table, current_offset, &mut current_count);
                            current_offset = call.location().start_offset();
                            current_table = Some(table);
                            current_count = 1;
                        }
                        continue;
                    }
                }
            }

            flush_run(&mut current_table, current_offset, &mut current_count);
        }

        flush_run(&mut current_table, current_offset, &mut current_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cop::CopConfig;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    struct CurrentDirGuard {
        previous: PathBuf,
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
        }
    }

    fn with_current_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        static CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _lock = CWD_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("cwd lock");
        let previous = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir).expect("set current dir");
        let _guard = CurrentDirGuard { previous };
        f()
    }

    fn mysql_config() -> CopConfig {
        let mut options = HashMap::new();
        options.insert(
            "Database".to_string(),
            serde_yml::Value::String("mysql".to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    fn rails_config(version: f64) -> CopConfig {
        let mut options = HashMap::new();
        options.insert(
            "TargetRailsVersion".to_string(),
            serde_yml::Value::Number(serde_yml::Number::from(version)),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    fn run_in_temp_project(
        source: &[u8],
        config: CopConfig,
        database_yml: Option<&str>,
    ) -> Vec<Diagnostic> {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config_dir = tempdir.path().join("config");
        let migrate_dir = tempdir.path().join("db/migrate");

        fs::create_dir_all(&config_dir).expect("config dir");
        fs::create_dir_all(&migrate_dir).expect("migrate dir");

        if let Some(database_yml) = database_yml {
            fs::write(config_dir.join("database.yml"), database_yml).expect("database.yml");
        }

        let path = migrate_dir.join("001_test.rb");
        with_current_dir(tempdir.path(), || {
            crate::testutil::run_cop_full_internal(
                &BulkChangeTable,
                source,
                config,
                path.to_str().unwrap(),
            )
        })
    }

    fn mark_as_git_repo(path: &Path) {
        fs::create_dir(path.join(".git")).expect("git dir");
    }

    #[test]
    fn offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &BulkChangeTable,
            include_bytes!("../../../tests/fixtures/cops/rails/bulk_change_table/offense.rb"),
            mysql_config(),
        );
    }

    #[test]
    fn no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &BulkChangeTable,
            include_bytes!("../../../tests/fixtures/cops/rails/bulk_change_table/no_offense.rb"),
            mysql_config(),
        );
    }

    #[test]
    fn detects_mysql_from_database_yml() {
        let source = b"def change\n  add_column :users, :twitter_token, :string\n  add_column :users, :twitter_secret, :string\nend\n";
        let diagnostics = run_in_temp_project(
            source,
            CopConfig::default(),
            Some("development:\n  adapter: mysql2\n"),
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "mysql2 database.yml should enable the cop"
        );
    }

    #[test]
    fn detects_nested_postgresql_from_database_yml() {
        let source = b"def up\n  change_column_default :events, :latitude, 0.0\n  change_column_default :events, :longitude, 0.0\nend\n";
        let diagnostics = run_in_temp_project(
            source,
            rails_config(5.2),
            Some("development:\n  primary:\n    adapter: postgresql\n"),
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "postgresql database.yml should enable PostgreSQL-specific methods on Rails 5.2+"
        );
    }

    #[test]
    fn detects_duplicate_default_keys_after_yaml_parse_failure() {
        let source = b"class AddLatitudeAndLongitudeToUser < ActiveRecord::Migration[5.0]\n  def change\n    add_column :users, :latitude, :float\n    add_column :users, :longitude, :float\n  end\nend\n";
        let diagnostics = run_in_temp_project(
            source,
            rails_config(7.0),
            Some(
                "default: &default\n  adapter: postgresql\n  encoding: unicode\n  pool: 5\n  pool: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n\ndevelopment:\n  <<: *default\n  database: registration_development\n",
            ),
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "Duplicate keys that break strict YAML parsing should still resolve the development adapter from the merged default"
        );
    }

    #[test]
    fn detects_duplicate_development_sections_after_yaml_parse_failure() {
        let source = b"class AddImapSettingsToMailSetting < ActiveRecord::Migration[5.0]\n  def change\n    add_column :mail_settings, :imap_address,  :string\n    add_column :mail_settings, :imap_port,     :string\n    add_column :mail_settings, :imap_password, :string\n    add_column :mail_settings, :imap_username, :string\n  end\nend\n";
        let diagnostics = run_in_temp_project(
            source,
            rails_config(7.0),
            Some(
                "default: &default\n  adapter: postgresql\n  encoding: unicode\n  pool: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n  host: localhost\n\ndevelopment:\n  <<: *default\n  database: marketing_development\n\ndevelopment:\n  <<: *default\n  database: marketing_test\n",
            ),
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "Duplicate development sections should still resolve the merged adapter when strict YAML parsing fails"
        );
    }

    #[test]
    fn detects_merged_defaults_with_inline_erb_control_flow() {
        let source = b"class RemoveFeaturesFromCategories < ActiveRecord::Migration\n  def change\n    remove_column :categories, :parent_id\n    remove_column :categories, :organization_id\n    remove_column :categories, :name_translations\n    remove_column :categories, :fqn_translations\n    remove_column :categories, :children_count\n    add_column :categories, :name, :string\n  end\nend\n";
        let diagnostics = run_in_temp_project(
            source,
            rails_config(7.0),
            Some(
                "defaults: &defaults\n  adapter: postgresql\n  username: <%= ENV['DATABASE_USER'] || ENV[\"POSTGRES_USER\"] || ENV[\"DATABASE_USERNAME\"] %>\n  password: <%= ENV['DATABASE_PASSWORD'] || ENV[\"POSTGRES_PASSWORD\"] %>\n  pool: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n  host: <%= ENV.fetch(\"DATABASE_HOST\") { \"localhost\" } %>\n  port: <%= ENV.fetch(\"DATABASE_PORT\") { \"5432\" } %>\n  template: 'template0'\n  encoding: unicode\n\ndevelopment:\n  <<: *defaults\n  database: <%= ENV.fetch('DATABASE_NAME', 'timeoverflow_development') %>\n\ntest:\n  <<: *defaults\n  database: timeoverflow_test\n\nproduction:\n  <<: *defaults\n  <%= \"url: #{ENV['DATABASE_URL']}\" if ENV['DATABASE_URL'].present? %>\n  <%= \"database: #{ENV.fetch('DATABASE_NAME', 'timeoverflow_production')}\" unless ENV['DATABASE_URL'].present? %>\n",
            ),
        );
        assert_eq!(
            diagnostics.len(),
            1,
            "Inline ERB control flow outside development should not prevent resolving an adapter inherited from merged defaults"
        );
    }

    #[test]
    fn skips_database_yml_with_top_level_erb_control_flow() {
        let source =
            b"def change\n  add_column :users, :name, :string\n  add_column :users, :age, :integer\nend\n";
        let discourse_like_yml = "development:\n  prepared_statements: false\n  adapter: postgresql\n<%\n  test_db = ENV['RAILS_DB']\n%>\n";
        let diagnostics = run_in_temp_project(source, rails_config(7.0), Some(discourse_like_yml));
        assert!(
            diagnostics.is_empty(),
            "Top-level ERB control flow in config/database.yml should disable database detection to match RuboCop"
        );
    }

    #[test]
    fn skips_postgresql_before_rails_5_2() {
        let source = b"def up\n  change_column_default :events, :latitude, 0.0\n  change_column_default :events, :longitude, 0.0\nend\n";
        let diagnostics = run_in_temp_project(
            source,
            rails_config(5.1),
            Some("development:\n  adapter: postgresql\n"),
        );
        assert!(
            diagnostics.is_empty(),
            "PostgreSQL bulk alter should stay disabled before Rails 5.2"
        );
    }

    #[test]
    fn skips_singleton_migration_methods() {
        let source = b"class AddFieldsToUsers < ActiveRecord::Migration\n  def self.up\n    add_column :users, :name, :string\n    add_column :users, :email, :string\n  end\nend\n";
        let diagnostics =
            crate::testutil::run_cop_full_with_config(&BulkChangeTable, source, mysql_config());
        assert!(
            diagnostics.is_empty(),
            "def self.up should stay ignored to match RuboCop"
        );
    }

    #[test]
    fn detects_erb_database_yml() {
        let source = b"def change\n  add_column :users, :name, :string\n  add_column :users, :age, :integer\nend\n";
        let erb_yml = "default: &default\n  adapter: postgresql\n  encoding: unicode\n  pool: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n\ndevelopment:\n  <<: *default\n  database: <%= ENV.fetch('DB_NAME') { 'dev' } %>\n";
        let diagnostics = run_in_temp_project(source, rails_config(5.2), Some(erb_yml));
        assert_eq!(
            diagnostics.len(),
            1,
            "ERB database.yml with anchors should still resolve adapter"
        );
    }

    #[test]
    fn detects_duplicate_development_section_merged_from_default() {
        let source = b"def change\n  add_column :log_entries, :type, :string\n  add_column :log_entries, :user_id, :integer\nend\n";
        let database_yml = "development: &default\n  adapter: postgresql\n  pool: 5\n\ndevelopment:\n  <<: *default\n  database: adventurers_league_log_development\n";
        let diagnostics = run_in_temp_project(source, rails_config(5.2), Some(database_yml));
        assert_eq!(
            diagnostics.len(),
            1,
            "Merged duplicate development sections should still resolve adapter"
        );
    }

    #[test]
    fn detects_nested_database_yml_with_merged_default() {
        let source = b"def change\n  add_column :users, :name, :string\n  add_column :users, :age, :integer\nend\n";
        let database_yml = "default: &default\n  adapter: postgresql\n  encoding: unicode\n  pool: <%= ENV.fetch(\"RAILS_MAX_THREADS\") { 5 } %>\n\ndevelopment:\n  primary:\n    <<: *default\n    database: enju_leaf_development\n  cache:\n    <<: *default\n    database: enju_leaf_development_cache\n    migrations_paths: db/cache_migrate\n";
        let diagnostics = run_in_temp_project(source, rails_config(5.2), Some(database_yml));
        assert_eq!(
            diagnostics.len(),
            1,
            "Nested development databases that inherit the adapter via << should resolve"
        );
    }

    #[test]
    fn detects_database_yml_from_source_repo_root_when_cwd_is_elsewhere() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let other_dir = tempfile::tempdir().expect("other dir");
        let config_dir = tempdir.path().join("config");
        let migrate_dir = tempdir.path().join("db/migrate");

        fs::create_dir_all(&config_dir).expect("config dir");
        fs::create_dir_all(&migrate_dir).expect("migrate dir");
        mark_as_git_repo(tempdir.path());
        fs::write(
            config_dir.join("database.yml"),
            "development:\n  adapter: mysql2\n",
        )
        .expect("database.yml");

        let path = migrate_dir.join("001_test.rb");
        let source = b"class AddNameAndAgeToUsers < ActiveRecord::Migration[6.0]\n  def change\n    add_column :users, :name, :string\n    add_column :users, :age, :integer\n  end\nend\n";
        let diagnostics = with_current_dir(other_dir.path(), || {
            crate::testutil::run_cop_full_internal(
                &BulkChangeTable,
                source,
                CopConfig::default(),
                path.to_str().unwrap(),
            )
        });

        assert_eq!(
            diagnostics.len(),
            1,
            "Should fall back to the source repo root when the process cwd is outside the repo"
        );
    }

    #[test]
    fn ignores_nested_app_database_yml_outside_project_root() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let other_dir = tempfile::tempdir().expect("other dir");
        let nested_config_dir = tempdir.path().join("spec/dummy/config");
        let nested_migrate_dir = tempdir.path().join("spec/dummy/db/migrate");

        fs::create_dir_all(&nested_config_dir).expect("nested config dir");
        fs::create_dir_all(&nested_migrate_dir).expect("nested migrate dir");
        mark_as_git_repo(tempdir.path());
        fs::write(
            nested_config_dir.join("database.yml"),
            "development:\n  adapter: mysql2\n",
        )
        .expect("nested database.yml");

        let path = nested_migrate_dir.join("001_test.rb");
        let source = b"class AddCountryAndCityToCourses < ActiveRecord::Migration[6.0]\n  def change\n    add_column :courses, :country, :string\n    add_column :courses, :city, :string\n  end\nend\n";
        let diagnostics = with_current_dir(other_dir.path(), || {
            crate::testutil::run_cop_full_internal(
                &BulkChangeTable,
                source,
                CopConfig::default(),
                path.to_str().unwrap(),
            )
        });

        assert!(
            diagnostics.is_empty(),
            "Nested app config/database.yml should not be used when the project root has no config/database.yml"
        );
    }

    #[test]
    fn skipped_when_database_cannot_be_resolved() {
        let source = b"# nitrocop-filename: db/migrate/001_test.rb\ndef change\n  add_column :users, :name, :string\n  add_column :users, :age, :integer\nend\n";
        let diagnostics = run_in_temp_project(source, CopConfig::default(), None);
        assert!(
            diagnostics.is_empty(),
            "Should not fire when the adapter cannot be resolved"
        );
    }
}
