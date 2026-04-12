pub mod class_definition_in_task;
pub mod desc;
pub mod duplicate_namespace;
pub mod duplicate_task;
pub mod method_definition_in_task;

use super::registry::CopRegistry;

/// Default Include patterns for Rake cops.
///
/// Matches rubocop-rake's `config/default.yml` exactly: root-only `Rakefile`
/// (NOT `**/Rakefile`) plus `**/*.rake` at any depth. The `Rakefile` pattern
/// intentionally does not have a `**/` prefix so nested `demo/Rakefile`,
/// `cocoapods/Rakefile`, etc. are not inspected — RuboCop only lints the
/// root-level Rakefile unless a repo's own config opts in.
pub const RAKE_DEFAULT_INCLUDE: &[&str] = &["Rakefile", "**/*.rake"];

/// Extract the task or namespace name from the first argument of a call node.
/// Handles `:name` (symbol), `"name"` (string), and `name: [deps]` (hash) patterns.
pub fn extract_task_name(call: &ruby_prism::CallNode<'_>) -> Option<String> {
    let args = call.arguments()?;
    let first_arg = args.arguments().iter().next()?;

    // Symbol argument: task :foo
    if let Some(sym) = first_arg.as_symbol_node() {
        return Some(String::from_utf8_lossy(sym.unescaped()).to_string());
    }

    // String argument: task "foo"
    if let Some(s) = first_arg.as_string_node() {
        return Some(String::from_utf8_lossy(s.unescaped()).to_string());
    }

    // Hash argument: task foo: [:dep] or task :foo => [:dep]
    if let Some(hash) = first_arg.as_keyword_hash_node() {
        for elem in hash.elements().iter() {
            if let Some(assoc) = elem.as_assoc_node() {
                let key = assoc.key();
                if let Some(sym) = key.as_symbol_node() {
                    return Some(String::from_utf8_lossy(sym.unescaped()).to_string());
                }
                if let Some(s) = key.as_string_node() {
                    return Some(String::from_utf8_lossy(s.unescaped()).to_string());
                }
            }
        }
    }

    None
}

pub fn register_all(registry: &mut CopRegistry) {
    registry.register(Box::new(class_definition_in_task::ClassDefinitionInTask));
    registry.register(Box::new(desc::Desc));
    registry.register(Box::new(duplicate_namespace::DuplicateNamespace));
    registry.register(Box::new(duplicate_task::DuplicateTask));
    registry.register(Box::new(method_definition_in_task::MethodDefinitionInTask));
}
