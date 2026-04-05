use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/RakeEnvironment cop - checks that rake tasks depend on :environment.
///
/// ## Investigation findings (2026-03-08)
///
/// **30 FP:** The `:default` task was not excluded from the check. RuboCop skips
/// `task :default` / `task default: [...]` because the default task is just a
/// dispatcher and doesn't need `:environment`.
///
/// **63 FN:** The cop only handled `task :name` (SymbolNode/StringNode as first arg)
/// but not the hash-first-arg form `task name: :dep do ... end` where the first arg
/// is a KeywordHashNode. In this form, the key is the task name and the value is the
/// dependency list. Tasks like `task foo: :environment` were not recognized at all,
/// and tasks like `task foo: []` (hash form, no deps) were not flagged.
///
/// **Fix:** Extract task name from both symbol/string first-arg and hash-first-arg
/// forms. Skip if task name is "default". For hash-first-arg, check the value for
/// dependencies (symbol = has dep, non-empty array = has dep, empty array = no dep).
///
/// ## Investigation findings (2026-03-10)
///
/// **45 FP:** `has_dependencies()` was too restrictive — only recognized SymbolNode,
/// StringNode, and non-empty ArrayNode as dependencies. Method calls (`task foo: dep`),
/// constants, and variables were not recognized, causing false positives. RuboCop's
/// `with_hash_style_dependencies?` treats ANY non-array, non-nil value as having
/// dependencies (the `else true` branch).
///
/// **Fix:** Simplified `has_dependencies()` to return `true` for anything except an
/// empty array, matching RuboCop's logic exactly.
///
/// ## Investigation findings (2026-03-15)
///
/// **62 FN:** The cop's `else` branch returned early (skipping the offense) when the
/// first argument was not a SymbolNode, StringNode, or KeywordHashNode. This missed
/// task definitions where the task name is a local variable (`task name do`), method
/// call (`task(a.to_sym) {}`), or other expression. RuboCop's `task_name` returns
/// `nil` for these cases, which is != `:default`, so it proceeds to check
/// dependencies and flags the offense.
///
/// **Fix:** Changed the `else` branch to set `task_name_is_default = false` and
/// `hash_first_arg = false`, allowing the cop to continue checking for dependencies
/// and flag the offense when appropriate.
///
/// ## Investigation findings (2026-03-24)
///
/// **3 FN:** Old Rake 0.8 style `task :name, :arg1, :arg2, :needs => [deps]` was
/// not flagged. The cop iterated ALL remaining args (`arg_list[1..]`) for dependency
/// hashes and found the `:needs` hash, treating it as having dependencies. RuboCop
/// only checks `arguments[1]` (the second argument) — since `:arg1` is a symbol,
/// not a hash, it doesn't find dependencies and flags the offense.
///
/// **1 FP:** `task({name => deps}, &block)` with an explicit `HashNode` (curly braces)
/// was not handled as first argument. Only `KeywordHashNode` (implicit hash) was
/// recognized, so the explicit hash fell to the `else` branch and was flagged.
///
/// **Fix:** Changed dependency check to only inspect `arg_list[1]` (matching RuboCop).
/// Added `HashNode` handling alongside `KeywordHashNode` for first-argument hashes.
pub struct RakeEnvironment;

impl Cop for RakeEnvironment {
    fn name(&self) -> &'static str {
        "Rails/RakeEnvironment"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/Rakefile", "**/*.rake"]
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/lib/capistrano/tasks/**/*.rake"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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
        // Start from CallNode `task`, then check if it has a block
        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // Must be receiverless `task` call
        if call.receiver().is_some() {
            return;
        }

        if call.name().as_slice() != b"task" {
            return;
        }

        // Must have a block
        if call.block().is_none() {
            return;
        }

        let args = match call.arguments() {
            Some(a) => a,
            None => return,
        };

        let arg_list: Vec<ruby_prism::Node<'_>> = args.arguments().iter().collect();
        if arg_list.is_empty() {
            return;
        }

        // Determine if this is a simple task definition (first arg is symbol/string)
        // or a hash-first-arg form (first arg is KeywordHashNode like `task foo: :dep`).
        let first = &arg_list[0];
        let task_name_is_default;
        let hash_first_arg;

        if let Some(sym) = first.as_symbol_node() {
            // task :foo do ... end
            task_name_is_default = sym.unescaped() == b"default";
            hash_first_arg = false;
        } else if let Some(s) = first.as_string_node() {
            // task 'foo' do ... end
            task_name_is_default = s.unescaped() == b"default";
            hash_first_arg = false;
        } else if first.as_keyword_hash_node().is_some() || first.as_hash_node().is_some() {
            // task foo: :dep do ... end  (KeywordHashNode, implicit hash)
            // task({foo => :dep}) { ... }  (HashNode, explicit hash with curlies)
            // Extract the key as the task name and check value for dependencies.
            let elements: Vec<ruby_prism::Node<'_>> = if let Some(kw) = first.as_keyword_hash_node()
            {
                kw.elements().iter().collect()
            } else if let Some(h) = first.as_hash_node() {
                h.elements().iter().collect()
            } else {
                unreachable!()
            };
            if let Some(first_elem) = elements.first() {
                if let Some(assoc) = first_elem.as_assoc_node() {
                    // Check task name from the key
                    let key = assoc.key();
                    task_name_is_default = is_name_default(&key);

                    // Check if value represents dependencies
                    let value = assoc.value();
                    if has_dependencies(&value) {
                        if task_name_is_default {
                            return;
                        }
                        // Has dependencies — not an offense (unless we also need to
                        // check remaining hash entries, but RuboCop doesn't).
                        return;
                    }
                    hash_first_arg = true;
                } else {
                    return;
                }
            } else {
                return;
            }
        } else {
            // Any other expression as task name (local variable, method call, constant, etc.)
            // — can't be :default, treat as non-default and check for dependencies.
            task_name_is_default = false;
            hash_first_arg = false;
        }

        // Skip :default task
        if task_name_is_default {
            return;
        }

        // For non-hash-first-arg form, check only arg_list[1] for dependency hash.
        // Matches RuboCop's `task_args = node.arguments[1]` — only the second
        // argument is checked, not all remaining args. This correctly handles
        // `task :foo, [:arg] => :dep` (second arg is a hash) while ignoring
        // `task :foo, :arg1, :arg2, :needs => [...]` (second arg is a symbol).
        if !hash_first_arg {
            if let Some(second) = arg_list.get(1) {
                if let Some(kw) = second.as_keyword_hash_node() {
                    for elem in kw.elements().iter() {
                        if let Some(assoc) = elem.as_assoc_node() {
                            let value = assoc.value();
                            if has_dependencies(&value) {
                                return;
                            }
                        }
                    }
                }
            }
        }

        let loc = node.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            "Add `:environment` dependency to the rake task.".to_string(),
        ));
    }
}

/// Check if a node represents the name "default" (as symbol or string).
fn is_name_default(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(sym) = node.as_symbol_node() {
        return sym.unescaped() == b"default";
    }
    if let Some(s) = node.as_string_node() {
        return s.unescaped() == b"default";
    }
    false
}

/// Check if a node represents non-empty dependencies.
/// Matches RuboCop's logic: anything except an empty array counts as a dependency.
/// This includes symbols, strings, method calls, constants, variables, etc.
fn has_dependencies(node: &ruby_prism::Node<'_>) -> bool {
    if let Some(arr) = node.as_array_node() {
        // Empty array means no dependencies
        return arr.elements().iter().next().is_some();
    }
    // Any non-array value is treated as a dependency (symbol, string, method call, etc.)
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(RakeEnvironment, "cops/rails/rake_environment");
}
