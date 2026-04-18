use crate::cop::shared::node_type::CLASS_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Checks that migration class names match the camelized form of the file name.
///
/// RuboCop equivalent: `Rails/MigrationClassName`
///
/// The cop extracts the expected class name by stripping the timestamp prefix
/// and `.rb` suffix from the filename, removing any gem suffix (e.g.,
/// `.active_storage`), then camelizing. For example,
/// `db/migrate/20220101_create_users.rb` expects `CreateUsers`.
///
/// RuboCop only checks classes whose superclass is a versioned
/// `ActiveRecord::Migration[...]` send. Unversioned legacy migrations like
/// `class CreateNewsArticle < ActiveRecord::Migration` are ignored, including
/// nested legacy sub-migrations inside wrapper migrations such as `add_rpush.rb`.
/// nitrocop mirrors that quirk to avoid false positives while still checking
/// nested versioned migration classes.
///
/// Comparison is case-insensitive (matching RuboCop's `casecmp`) to tolerate
/// ActiveSupport inflection differences like `OAuth` vs `Oauth`.
pub struct MigrationClassName;

impl Cop for MigrationClassName {
    fn name(&self) -> &'static str {
        "Rails/MigrationClassName"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["db/migrate/**/*.rb"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CLASS_NODE]
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
        let class_node = match node.as_class_node() {
            Some(c) => c,
            None => return,
        };

        // Check if class inherits from ActiveRecord::Migration
        let superclass = match class_node.superclass() {
            Some(s) => s,
            None => return,
        };

        if !is_versioned_migration_superclass(superclass) {
            return;
        }

        // Extract expected class name from filename
        let expected = expected_class_name(source.path_str());

        // Get actual class name
        let class_name_str = match std::str::from_utf8(class_node.name().as_slice()) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Case-insensitive comparison (matches RuboCop's casecmp)
        if class_name_str.eq_ignore_ascii_case(&expected) {
            return;
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!("Replace with `{}` that matches the file name.", expected),
        ));
    }
}

fn is_versioned_migration_superclass(node: ruby_prism::Node<'_>) -> bool {
    let Some(call) = node.as_call_node() else {
        return false;
    };

    if call.name().as_slice() != b"[]" {
        return false;
    }

    let Some(receiver) = call.receiver() else {
        return false;
    };

    if !is_active_record_migration_const(receiver) {
        return false;
    }

    let Some(arguments) = call.arguments() else {
        return false;
    };

    let args = arguments.arguments();
    args.len() == 1
        && args
            .iter()
            .next()
            .is_some_and(|argument| argument.as_float_node().is_some())
}

fn is_active_record_migration_const(node: ruby_prism::Node<'_>) -> bool {
    let Some(migration) = node.as_constant_path_node() else {
        return false;
    };

    if !migration
        .name()
        .is_some_and(|name| name.as_slice() == b"Migration")
    {
        return false;
    }

    let Some(parent) = migration.parent() else {
        return false;
    };

    if let Some(active_record) = parent.as_constant_read_node() {
        return active_record.name().as_slice() == b"ActiveRecord";
    }

    parent.as_constant_path_node().is_some_and(|active_record| {
        active_record
            .name()
            .is_some_and(|name| name.as_slice() == b"ActiveRecord")
            && active_record.parent().is_none()
    })
}

/// Extract the expected CamelCase class name from a migration file path.
///
/// Steps:
/// 1. Take the file stem (basename without final extension)
/// 2. Remove gem suffix (everything from the first remaining `.` onward)
/// 3. Remove leading timestamp digits followed by `_`
/// 4. Camelize: split on `_`, capitalize each part, join
fn expected_class_name(path: &str) -> String {
    let p = std::path::Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(path);

    // Remove gem suffix (e.g., add_blobs.active_storage -> add_blobs)
    let without_gem = match stem.find('.') {
        Some(idx) => &stem[..idx],
        None => stem,
    };

    // Remove timestamp prefix (\A\d+_)
    let without_timestamp = strip_timestamp_prefix(without_gem);

    camelize(without_timestamp)
}

/// Strip a leading run of digits followed by `_` (the migration timestamp).
fn strip_timestamp_prefix(s: &str) -> &str {
    let digit_count = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digit_count > 0 && s.as_bytes().get(digit_count) == Some(&b'_') {
        &s[digit_count + 1..]
    } else {
        s
    }
}

/// Camelize a snake_case string: split on `_`, capitalize each part, join.
///
/// Matches Ruby's `word.split('_').map(&:capitalize).join`.
fn camelize(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    let rest: String = chars.flat_map(|c| c.to_lowercase()).collect();
                    format!("{}{}", upper, rest)
                }
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(MigrationClassName, "cops/rails/migration_class_name");

    #[test]
    fn test_expected_class_name() {
        assert_eq!(
            expected_class_name("db/migrate/20220101_create_users.rb"),
            "CreateUsers"
        );
        assert_eq!(
            expected_class_name("db/migrate/20180808151237_add_rpush.rb"),
            "AddRpush"
        );
        assert_eq!(
            expected_class_name("db/migrate/20131002215400_clean_openvas_settings.rb"),
            "CleanOpenvasSettings"
        );
        assert_eq!(
            expected_class_name("db/migrate/20220101050505_add_blobs.active_storage.rb"),
            "AddBlobs"
        );
    }

    #[test]
    fn test_camelize() {
        assert_eq!(camelize("add_users"), "AddUsers");
        assert_eq!(camelize("create_posts"), "CreatePosts");
        assert_eq!(camelize("clean_openvas_settings"), "CleanOpenvasSettings");
        assert_eq!(camelize("add_rpush"), "AddRpush");
    }

    #[test]
    fn test_strip_timestamp_prefix() {
        assert_eq!(strip_timestamp_prefix("20220101_add_users"), "add_users");
        assert_eq!(strip_timestamp_prefix("add_users"), "add_users");
        assert_eq!(strip_timestamp_prefix("123_foo"), "foo");
    }

    #[test]
    fn ignores_unversioned_top_level_migrations() {
        let source = b"# nitrocop-filename: db/migrate/007_news_article.rb\nclass CreateNewsArticle < ActiveRecord::Migration\nend\n";

        crate::testutil::assert_cop_no_offenses_full(&MigrationClassName, source);
    }
}
