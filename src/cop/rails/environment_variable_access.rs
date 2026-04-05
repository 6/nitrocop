use crate::cop::shared::node_type::{
    CALL_NODE, INDEX_AND_WRITE_NODE, INDEX_OPERATOR_WRITE_NODE, INDEX_OR_WRITE_NODE,
    MULTI_WRITE_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/EnvironmentVariableAccess — flags direct ENV reads and writes post initialization.
///
/// ## Root cause of 2,587 FNs (2026-03-18)
///
/// The previous implementation had an early-return guard `if method != b"[]" { return; }`
/// that silently skipped ALL non-read accesses. This meant:
/// - `ENV['FOO'] = 'bar'` (CallNode with method `[]=`) → was skipped → FN (WRITE_MSG)
/// - `ENV.fetch('FOO')` (CallNode with method `fetch`) → was skipped → FN (READ_MSG)
/// - `ENV.store('KEY', 'v')`, `ENV.delete('KEY')` → were skipped → FN (READ_MSG)
/// - `::ENV['FOO'] = 'bar'` → was skipped → FN (WRITE_MSG)
///
/// ## Fix
///
/// Rewrote the check to match RuboCop's behavior:
/// - Any CallNode with receiver ENV and method NOT `[]=` → READ_MSG (unless AllowReads)
/// - Any CallNode with receiver ENV and method `[]=` → WRITE_MSG (unless AllowWrites)
///
/// RuboCop flags the `const` (ENV) node, not the call node. We flag the call node
/// to get the full expression span, matching RuboCop's location behavior.
///
/// Note: `env_read?` in RuboCop's NodePattern matches `!:[]=` — any method except `[]=`.
/// This includes `[]`, `fetch`, `store`, `delete`, `merge`, etc. We replicate this.
///
/// Note: Prism parses `ENV['FOO'] = 'bar'` as a `CallNode` with method `[]=`, not as
/// an `IndexAndWriteNode`. The `IndexAndWriteNode` is only used for `ENV['FOO'] += val`
/// style compound assignments.
///
/// ## Root cause of 105 FNs (2026-03-18)
///
/// `ENV['KEY'] ||= value` is parsed as `IndexOrWriteNode`, not `CallNode`.
/// Similarly `&&=` → `IndexAndWriteNode`, `+=` → `IndexOperatorWriteNode`.
/// These are all writes (RuboCop's `env_write?` matches `indexasgn`).
/// Fixed by adding these three node types to `interested_node_types`.
///
/// ## Root cause of 1 FN (2026-03-18)
///
/// Multi-assignment `ENV['A'], ENV['B'] = a, b` is parsed as `MultiWriteNode`
/// with `IndexTargetNode` children (not `CallNode`). Fixed by adding
/// `MULTI_WRITE_NODE` to `interested_node_types` and iterating `lefts()`
/// to check each target for ENV receiver.
///
/// ## Remaining 34 corpus FN (2026-03-26) — config bug, not cop logic
///
/// The cop detection logic is correct. All 34 FN are caused by a config
/// resolution bug in `src/config/mod.rs` (`is_cop_match` / `build_glob_set`):
/// when `AllCops.Exclude` contains absolute paths (e.g. `/tmp/foo.rb`), the
/// pattern-cop Include matching silently breaks, causing the cop to be skipped
/// for files that should match Include patterns like `**/lib/**/*.rb`.
///
/// The corpus runner (`bench/corpus/run_nitrocop.py`) generates per-repo configs
/// via `gen_repo_config.py`, which adds absolute-path Exclude entries for files
/// with parse errors. These absolute paths trigger the bug. Affected repos:
/// `cjstewart88__Tubalr` (14 FN), `pitluga__supply_drop` (10 FN),
/// `liaoziyang__stackneveroverflow` (7 FN), `databasically__lowdown` (3 FN).
///
/// The fix belongs in `src/config/mod.rs`, not in this cop. This bug likely
/// affects all pattern cops (those with Include/Exclude in their config), not
/// just `Rails/EnvironmentVariableAccess`.
pub struct EnvironmentVariableAccess;

const READ_MSG: &str = "Do not read from `ENV` directly post initialization.";
const WRITE_MSG: &str = "Do not write to `ENV` directly post initialization.";

impl Cop for EnvironmentVariableAccess {
    fn name(&self) -> &'static str {
        "Rails/EnvironmentVariableAccess"
    }

    fn default_enabled(&self) -> bool {
        false
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/lib/**/*.rake"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            INDEX_OR_WRITE_NODE,
            INDEX_AND_WRITE_NODE,
            INDEX_OPERATOR_WRITE_NODE,
            MULTI_WRITE_NODE,
        ]
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
        let allow_reads = config.get_bool("AllowReads", false);
        let allow_writes = config.get_bool("AllowWrites", false);

        // Handle multi-write: ENV['A'], ENV['B'] = a, b
        // Prism parses these targets as IndexTargetNode inside MultiWriteNode.
        if let Some(mw) = node.as_multi_write_node() {
            if !allow_writes {
                for target in mw.lefts().iter() {
                    if let Some(idx) = target.as_index_target_node() {
                        let recv = idx.receiver();
                        if is_env_receiver(&recv) {
                            let loc = recv.location();
                            let (line, column) = source.offset_to_line_col(loc.start_offset());
                            diagnostics.push(self.diagnostic(
                                source,
                                line,
                                column,
                                WRITE_MSG.to_string(),
                            ));
                        }
                    }
                }
                // Also check rest target if present
                if let Some(rest) = mw.rest() {
                    if let Some(idx) = rest.as_index_target_node() {
                        let recv = idx.receiver();
                        if is_env_receiver(&recv) {
                            let loc = recv.location();
                            let (line, column) = source.offset_to_line_col(loc.start_offset());
                            diagnostics.push(self.diagnostic(
                                source,
                                line,
                                column,
                                WRITE_MSG.to_string(),
                            ));
                        }
                    }
                }
            }
            return;
        }

        // Handle ENV['KEY'] ||= val, &&= val, += val (IndexOrWriteNode, IndexAndWriteNode,
        // IndexOperatorWriteNode). These are all writes — RuboCop's env_write? matches `indexasgn`.
        if let Some(recv) = node
            .as_index_or_write_node()
            .and_then(|n| n.receiver())
            .or_else(|| node.as_index_and_write_node().and_then(|n| n.receiver()))
            .or_else(|| {
                node.as_index_operator_write_node()
                    .and_then(|n| n.receiver())
            })
        {
            if !allow_writes && is_env_receiver(&recv) {
                let loc = recv.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(source, line, column, WRITE_MSG.to_string()));
            }
            return;
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let recv = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        if !is_env_receiver(&recv) {
            return;
        }

        let method = call.name();
        let method_bytes = method.as_slice();

        // `[]=` is a write; everything else is a read
        if method_bytes == b"[]=" {
            if !allow_writes {
                let loc = recv.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(source, line, column, WRITE_MSG.to_string()));
                // RuboCop's on_const fires for every const child of the send node,
                // including const arguments. Report write offenses for those too.
                report_const_args(self, source, &call, WRITE_MSG, diagnostics);
            }
        } else if !allow_reads {
            let loc = recv.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(source, line, column, READ_MSG.to_string()));
            // RuboCop's on_const fires for every const child of the send node,
            // including const arguments. Report read offenses for those too.
            report_const_args(self, source, &call, READ_MSG, diagnostics);
        }
    }
}

/// Report offenses for any constant arguments inside a call to ENV.
/// RuboCop's `on_const` fires on every const descendant of the send node,
/// including constant arguments like `ENV.fetch("KEY", SOME_CONST)`.
/// The `env_read?` / `env_write?` pattern matches those const args because
/// their parent is the send node whose receiver is ENV.
fn report_const_args(
    cop: &EnvironmentVariableAccess,
    source: &SourceFile,
    call: &ruby_prism::CallNode<'_>,
    msg: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(args) = call.arguments() {
        for arg in args.arguments().iter() {
            if arg.as_constant_read_node().is_some() || arg.as_constant_path_node().is_some() {
                let loc = arg.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(cop.diagnostic(source, line, column, msg.to_string()));
            }
        }
    }
}

/// Check if a receiver node is `ENV` or `::ENV` (not `Foo::ENV`).
fn is_env_receiver(recv: &ruby_prism::Node<'_>) -> bool {
    if let Some(cr) = recv.as_constant_read_node() {
        cr.name().as_slice() == b"ENV"
    } else if let Some(cp) = recv.as_constant_path_node() {
        cp.parent().is_none() && cp.name().is_some_and(|n| n.as_slice() == b"ENV")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        EnvironmentVariableAccess,
        "cops/rails/environment_variable_access"
    );
}
