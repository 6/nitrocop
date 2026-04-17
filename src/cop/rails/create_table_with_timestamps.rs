use ruby_prism::Visit;

use crate::cop::shared::node_type::{
    ASSOC_NODE, BLOCK_NODE, CALL_NODE, FALSE_NODE, KEYWORD_HASH_NODE, STRING_NODE, SYMBOL_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Flags `create_table` calls that omit timestamp columns.
///
/// Corpus investigation (2026-04-17) found three gaps relative to RuboCop:
/// - `create_table` without a block was skipped entirely.
/// - non-`timestamps` block-pass forms like `create_table :users, &:columns`
///   were also skipped.
/// - the built-in scope only covered `db/migrate/**/*.rb`, but RuboCop's
///   default config runs this cop on all `db/**/*.rb` files, including
///   `db/schema.rb`.
pub struct CreateTableWithTimestamps;

/// Walk a node tree looking for `timestamps` or `datetime :created_at/:updated_at`.
struct TimestampFinder {
    found: bool,
}

impl<'pr> Visit<'pr> for TimestampFinder {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        let name = node.name().as_slice();
        if name == b"timestamps" {
            self.found = true;
            return;
        }
        // Check for `t.datetime :created_at` or `t.datetime :updated_at`
        if name == b"datetime" {
            if let Some(args) = node.arguments() {
                if let Some(first) = args.arguments().iter().next() {
                    if let Some(sym) = first.as_symbol_node() {
                        let val = sym.unescaped();
                        if val == b"created_at" || val == b"updated_at" {
                            self.found = true;
                            return;
                        }
                    }
                    if let Some(s) = first.as_string_node() {
                        let val = s.unescaped();
                        if val == b"created_at" || val == b"updated_at" {
                            self.found = true;
                            return;
                        }
                    }
                }
            }
        }
        if !self.found {
            ruby_prism::visit_call_node(self, node);
        }
    }
}

/// Check if `create_table` has `id: false` option.
///
/// Prism can represent the option hash as either `KeywordHashNode` (`id: false`)
/// or `HashNode` (`{:id => false}`), so we need to handle both forms.
fn has_id_false(call: &ruby_prism::CallNode<'_>) -> bool {
    let args = match call.arguments() {
        Some(a) => a,
        None => return false,
    };
    for arg in args.arguments().iter() {
        let elements = if let Some(kw) = arg.as_keyword_hash_node() {
            kw.elements()
        } else if let Some(hash) = arg.as_hash_node() {
            hash.elements()
        } else {
            continue;
        };
        for elem in elements.iter() {
            let assoc = match elem.as_assoc_node() {
                Some(a) => a,
                None => continue,
            };
            let key = match assoc.key().as_symbol_node() {
                Some(s) => s,
                None => continue,
            };
            if key.unescaped() == b"id" && assoc.value().as_false_node().is_some() {
                return true;
            }
        }
    }
    false
}

fn has_timestamps_block_pass(call: &ruby_prism::CallNode<'_>) -> bool {
    let Some(block) = call.block() else {
        return false;
    };
    let Some(block_arg) = block.as_block_argument_node() else {
        return false;
    };
    let Some(expr) = block_arg.expression() else {
        return false;
    };
    expr.as_symbol_node()
        .is_some_and(|sym| sym.unescaped() == b"timestamps")
}

impl Cop for CreateTableWithTimestamps {
    fn name(&self) -> &'static str {
        "Rails/CreateTableWithTimestamps"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/**/*.rb"]
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &[
            "db/**/*_create_active_storage_tables.active_storage.rb",
            "db/**/*_create_active_storage_variant_records.active_storage.rb",
        ]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            ASSOC_NODE,
            BLOCK_NODE,
            CALL_NODE,
            FALSE_NODE,
            KEYWORD_HASH_NODE,
            STRING_NODE,
            SYMBOL_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Start from CallNode `create_table`, then access its block
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        if call.name().as_slice() != b"create_table" {
            return;
        }

        // Skip `create_table :x, id: false` — join tables don't need timestamps
        if has_id_false(&call) {
            return;
        }

        let block = match call.block() {
            Some(b) => b,
            None => {
                let loc = node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Add `t.timestamps` to `create_table` block.".to_string(),
                ));
                return;
            }
        };

        let block_node = match block.as_block_node() {
            Some(b) => b,
            None => {
                if has_timestamps_block_pass(&call) {
                    return;
                }

                let loc = node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Add `t.timestamps` to `create_table` block.".to_string(),
                ));
                return;
            }
        };

        // Walk block body looking for timestamps call
        let body = match block_node.body() {
            Some(b) => b,
            None => {
                // Empty block -- flag it
                let loc = node.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    "Add `t.timestamps` to `create_table` block.".to_string(),
                ));
                return;
            }
        };

        let mut finder = TimestampFinder { found: false };
        finder.visit(&body);

        if finder.found {
            return;
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Add `t.timestamps` to `create_table` block.".to_string(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    use clap::Parser;

    use crate::cli::Args;
    use crate::cop::autocorrect_allowlist::AutocorrectAllowlist;
    use crate::cop::registry::CopRegistry;
    use crate::cop::tiers::TierMap;

    crate::cop_fixture_tests!(
        CreateTableWithTimestamps,
        "cops/rails/create_table_with_timestamps"
    );

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    fn write_file(root: &Path, rel: &str, contents: &[u8]) -> PathBuf {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, contents).unwrap();
        path
    }

    fn args_for_repo(repo: &Path) -> Args {
        Args::parse_from([
            "nitrocop",
            repo.to_str().unwrap(),
            "--preview",
            "--no-cache",
            "--only",
            "Rails/CreateTableWithTimestamps",
        ])
    }

    fn repo_filter_matches(
        repo: &Path,
        config_path: &Path,
        file_path: &Path,
    ) -> (
        bool,
        crate::linter::LintResult,
        crate::linter::LintResult,
        Vec<PathBuf>,
    ) {
        let config = crate::config::load_config(Some(config_path), None, None).unwrap();
        let registry = CopRegistry::default_registry();
        let tier_map = TierMap::load();
        let mut filters = config.build_cop_filters(&registry, &tier_map, true);
        filters.set_scan_root(repo.to_path_buf());
        let idx = registry
            .cops()
            .iter()
            .position(|cop| cop.name() == "Rails/CreateTableWithTimestamps")
            .unwrap();
        let matches = filters.is_cop_match(idx, file_path);
        let args = args_for_repo(repo);
        let allowlist = AutocorrectAllowlist::load();
        let roots = vec![repo.to_path_buf()];
        let discovered = crate::fs::discover_files(&roots, &config).unwrap();
        let source = SourceFile::from_path(file_path).unwrap();
        let single_file_result =
            crate::linter::lint_source(&source, &config, &registry, &args, &tier_map, &allowlist);

        let result = crate::linter::run_linter(
            &discovered,
            &config,
            &registry,
            &args,
            &tier_map,
            &allowlist,
        );

        (matches, single_file_result, result, discovered.files)
    }

    fn relative_diagnostic_paths(result: &crate::linter::LintResult, repo: &Path) -> Vec<String> {
        result
            .diagnostics
            .iter()
            .map(|d| {
                Path::new(&d.path)
                    .strip_prefix(repo)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn enabled_cop_uses_db_default_scope_for_schema_files() {
        let dir = temp_dir("nitrocop_test_ctwt_default_scope");
        let repo = dir.join("repo");
        let config_dir = dir.join("config");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&config_dir).unwrap();

        let config_path = write_file(
            &config_dir,
            "baseline.yml",
            b"Rails/CreateTableWithTimestamps:\n  Enabled: true\n",
        );
        write_file(
            &repo,
            "db/schema.rb",
            b"ActiveRecord::Schema[7.0].define(version: 1) do\n  create_table \"widgets\", force: :cascade do |t|\n    t.string \"name\"\n  end\nend\n",
        );
        write_file(
            &repo,
            "db/migrate/001_create_users.rb",
            b"class CreateUsers < ActiveRecord::Migration[7.0]\n  def change\n    create_table :users do |t|\n      t.string :name\n    end\n  end\nend\n",
        );

        let schema_path = repo.join("db/schema.rb");
        let (schema_matches, single_file_result, result, discovered_files) =
            repo_filter_matches(&repo, &config_path, &schema_path);
        let single_file_paths = relative_diagnostic_paths(&single_file_result, &repo);
        let paths = relative_diagnostic_paths(&result, &repo);

        assert!(
            schema_matches,
            "db/schema.rb should match the cop filter, got {:?}",
            discovered_files
        );
        assert!(
            single_file_paths.iter().any(|p| p == "db/schema.rb"),
            "expected single-file schema offense, got {:?} (discovered: {:?})",
            single_file_paths,
            discovered_files
        );
        assert!(
            paths.iter().any(|p| p == "db/schema.rb"),
            "expected db/schema.rb offense, got {:?} (single-file: {:?}, discovered: {:?})",
            paths,
            single_file_paths,
            discovered_files
        );
        assert!(
            paths.iter().any(|p| p == "db/migrate/001_create_users.rb"),
            "expected db/migrate offense, got {:?}",
            paths
        );
        assert_eq!(
            paths.len(),
            2,
            "expected exactly 2 offenses, got {:?}",
            paths
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn external_config_excludes_active_storage_files_via_repo_root() {
        let dir = temp_dir("nitrocop_test_ctwt_active_storage_exclude");
        let repo = dir.join("repo");
        let config_dir = dir.join("config");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&config_dir).unwrap();

        let config_path = write_file(
            &config_dir,
            "baseline.yml",
            b"Rails/CreateTableWithTimestamps:\n  Enabled: true\n  Include:\n    - db/**/*.rb\n  Exclude:\n    - db/**/*_create_active_storage_tables.active_storage.rb\n    - db/**/*_create_active_storage_variant_records.active_storage.rb\n",
        );
        write_file(
            &repo,
            "db/schema.rb",
            b"ActiveRecord::Schema[7.0].define(version: 1) do\n  create_table \"widgets\", force: :cascade do |t|\n    t.string \"name\"\n  end\nend\n",
        );
        write_file(
            &repo,
            "db/migrate/001_create_users.rb",
            b"class CreateUsers < ActiveRecord::Migration[7.0]\n  def change\n    create_table :users do |t|\n      t.string :name\n    end\n  end\nend\n",
        );
        write_file(
            &repo,
            "db/migrate/002_create_active_storage_tables.active_storage.rb",
            b"class CreateActiveStorageTables < ActiveRecord::Migration[7.0]\n  def change\n    create_table :active_storage_variant_records do |t|\n      t.string :variation_digest, null: false\n    end\n  end\nend\n",
        );

        let active_storage_path =
            repo.join("db/migrate/002_create_active_storage_tables.active_storage.rb");
        let (active_storage_matches, single_file_result, result, discovered_files) =
            repo_filter_matches(&repo, &config_path, &active_storage_path);
        let single_file_paths = relative_diagnostic_paths(&single_file_result, &repo);
        let paths = relative_diagnostic_paths(&result, &repo);

        assert!(
            !active_storage_matches,
            "active storage migration should be excluded by the cop filter, got {:?}",
            paths
        );
        assert!(
            paths.iter().any(|p| p == "db/schema.rb"),
            "expected db/schema.rb offense, got {:?} (single-file: {:?}, discovered: {:?})",
            paths,
            single_file_paths,
            discovered_files
        );
        assert!(
            paths.iter().any(|p| p == "db/migrate/001_create_users.rb"),
            "expected db/migrate offense, got {:?}",
            paths
        );
        assert!(
            !paths
                .iter()
                .any(|p| p == "db/migrate/002_create_active_storage_tables.active_storage.rb"),
            "active storage migration should be excluded, got {:?} (single-file: {:?}, discovered: {:?})",
            paths,
            single_file_paths,
            discovered_files
        );
        assert_eq!(
            paths.len(),
            2,
            "expected exactly 2 offenses, got {:?}",
            paths
        );

        fs::remove_dir_all(&dir).ok();
    }
}
