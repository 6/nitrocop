use crate::cop::shared::node_type::{
    CALL_NODE, CLASS_NODE, MODULE_NODE, SINGLETON_CLASS_NODE, STATEMENTS_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Enforces grouping of accessor declarations (`attr_reader`, `attr_writer`,
/// `attr_accessor`, `attr`) in class and module bodies.
///
/// ## Investigation findings (2026-03-15)
///
/// The original nitrocop implementation used a contiguity-based approach: it tracked
/// consecutive accessor declarations and grouped them by adjacency. This diverged
/// significantly from RuboCop's algorithm, which uses a sibling-based approach:
///
/// **RuboCop's algorithm:**
/// 1. Iterates ALL `send` nodes in the class/module body that are `attribute_accessor?`
/// 2. For each accessor, checks `previous_line_comment?` — if the source line immediately
///    before the accessor is a comment, the accessor is excluded from grouping
/// 3. Checks `groupable_accessor?` — examines the previous sibling (left sibling in the
///    statement list). An accessor is NOT groupable if:
///    - The previous sibling is a non-accessor send that is not an access modifier
///      (e.g., `sig { ... }`, `annotation_method :foo`) AND there's no blank line gap
///    - The previous sibling is a block node wrapping a send (Sorbet `sig { ... }`)
///      AND there's no blank line gap
/// 4. Finds all same-type, same-visibility siblings that are also groupable and not
///    preceded by a comment — reports offense if >1 such siblings exist
///
/// **Root causes of FPs (294):**
/// - Accessors preceded by a comment on the previous line were flagged (should be excluded)
/// - Accessors preceded by annotation method calls (Sorbet sig, etc.) were flagged
///
/// **Root causes of FNs (582):**
/// - Non-contiguous same-type accessors in the same visibility scope were missed because
///   the old code only checked adjacent sequences. RuboCop considers ALL siblings in the
///   class body, not just consecutive ones.
/// - Accessors separated by `def` blocks or other code were not grouped.
///
/// Fix: rewrote to match RuboCop's sibling-based `groupable_sibling_accessors` approach.
///
/// ## Investigation findings (2026-03-15, inline RBS annotations)
///
/// 67 FPs from accessors with inline RBS::Inline `#:` type comments (e.g.,
/// `attr_accessor :label #: String`). RuboCop's `groupable_accessor?` checks if
/// the previous sibling expression has an inline `#:` comment on the same line.
/// If it does, the current accessor is NOT groupable, because grouping would
/// lose per-attribute type annotations.
///
/// Fix: added `has_inline_rbs_comment()` check in `is_groupable_accessor()` to
/// detect `#:` on the previous sibling's source line and return false (not groupable).
///
/// ## Investigation findings (2026-03-27, block-form DSL calls)
///
/// 3 FNs remained in the corpus when an accessor group followed a block-form DSL call
/// such as `mattr_accessor ... do` or `config_section ... do`. RuboCop unwraps a
/// preceding block expression to its inner send and compares the accessor against that
/// send node's `last_line`, which is the call line rather than the `end` line.
///
/// Prism exposes these constructs as a `CallNode` whose `location()` spans through the
/// block terminator. The previous nitrocop port used that full span, so it treated the
/// first accessor as immediately adjacent to the block and marked it ungroupable. That
/// dropped the first accessor in longer groups and suppressed the entire offense when the
/// group only had two accessors.
///
/// Fix: when the previous sibling is a call with a real `BlockNode`, measure blank-line
/// spacing from the block start line instead of the call's full end line. This matches
/// RuboCop's unwrapped-send behavior without broadening grouping after ordinary calls.
///
/// ## Investigation findings (2026-03-31, bare accessor calls)
///
/// 2 FPs from bare `attr` calls with no arguments (used as annotation/decorator methods,
/// e.g., in Oj::Serializer). RuboCop's `attribute_accessor?` node matcher uses an
/// intersection pattern that requires at least one argument: `[(send nil? ${:attr ...} $...)
/// (_ _ _ _ ...)]`. The second sub-pattern `(_ _ _ _ ...)` requires at least 3 children
/// (receiver, method_name, one argument), so bare `attr` without arguments does not match.
///
/// Fix: added `call.arguments().is_some()` check when identifying accessor calls and when
/// checking if the previous sibling is an accessor in `is_groupable_accessor`.
///
/// ## Investigation findings (2026-04-05, EnforcedStyle: separated)
///
/// 8,358 FNs from the `separated` style variant — the implementation only handled
/// `EnforcedStyle: grouped` (the default) and had no code path for `separated`.
///
/// RuboCop's `separated` style flags any accessor call (`attr_reader`, `attr_writer`,
/// `attr_accessor`, `attr`) that has more than one argument. The same `previous_line_comment?`
/// and `groupable_accessor?` filters apply: accessors preceded by a comment or a non-accessor
/// send without a blank line gap are excluded.
///
/// Fix: added `check_separated()` that iterates accessor calls in the class/module body and
/// reports an offense when `arguments.len() > 1`, applying the same comment/groupability
/// filters. Message: "Use one attribute per `<accessor>`."
pub struct AccessorGrouping;

const ACCESSOR_METHODS: &[&str] = &["attr_reader", "attr_writer", "attr_accessor", "attr"];

impl Cop for AccessorGrouping {
    fn name(&self) -> &'static str {
        "Style/AccessorGrouping"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            CLASS_NODE,
            MODULE_NODE,
            SINGLETON_CLASS_NODE,
            STATEMENTS_NODE,
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
        let enforced_style = config.get_str("EnforcedStyle", "grouped");

        // Only check class and module bodies
        let body = if let Some(class_node) = node.as_class_node() {
            class_node.body()
        } else if let Some(module_node) = node.as_module_node() {
            module_node.body()
        } else if let Some(sclass) = node.as_singleton_class_node() {
            sclass.body()
        } else {
            return;
        };

        let body = match body {
            Some(b) => b,
            None => return,
        };

        let stmts = match body.as_statements_node() {
            Some(s) => s,
            None => return,
        };

        if enforced_style == "grouped" {
            check_grouped(self, source, &stmts, diagnostics);
        } else if enforced_style == "separated" {
            check_separated(self, source, &stmts, diagnostics);
        }
    }
}

/// Info about each statement in the class/module body.
struct StmtInfo {
    /// Index in the statement list
    idx: usize,
    /// Whether this statement is an accessor call (attr_reader, etc.)
    is_accessor: bool,
    /// The accessor method name (e.g., "attr_reader"), empty if not accessor
    accessor_name: String,
    /// Visibility scope of this statement (public/protected/private)
    visibility: &'static str,
    /// Whether this accessor is "groupable" per RuboCop's logic
    groupable: bool,
    /// Whether the line before this accessor is a comment
    has_previous_line_comment: bool,
}

fn check_separated(
    cop: &AccessorGrouping,
    source: &SourceFile,
    stmts: &ruby_prism::StatementsNode<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let stmt_list: Vec<_> = stmts.body().iter().collect();

    for (idx, stmt) in stmt_list.iter().enumerate() {
        let call = match stmt.as_call_node() {
            Some(c) => c,
            None => continue,
        };

        let name = std::str::from_utf8(call.name().as_slice()).unwrap_or("");
        if !ACCESSOR_METHODS.contains(&name) || call.receiver().is_some() {
            continue;
        }

        let arg_count = call.arguments().map_or(0, |args| args.arguments().len());
        if arg_count <= 1 {
            continue;
        }

        // Same filters as grouped style: skip if previous line is a comment or not groupable
        if previous_line_is_comment(source, stmt.location().start_offset()) {
            continue;
        }
        if !is_groupable_accessor(source, &stmt_list, idx) {
            continue;
        }

        let loc = stmt.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(cop.diagnostic(
            source,
            line,
            column,
            format!("Use one attribute per `{}`.", name),
        ));
    }
}

fn check_grouped(
    cop: &AccessorGrouping,
    source: &SourceFile,
    stmts: &ruby_prism::StatementsNode<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let stmt_list: Vec<_> = stmts.body().iter().collect();
    if stmt_list.is_empty() {
        return;
    }

    // Build info for each statement
    let mut infos: Vec<StmtInfo> = Vec::with_capacity(stmt_list.len());
    let mut current_visibility: &'static str = "public";

    for (idx, stmt) in stmt_list.iter().enumerate() {
        let mut info = StmtInfo {
            idx,
            is_accessor: false,
            accessor_name: String::new(),
            visibility: current_visibility,
            groupable: true,
            has_previous_line_comment: false,
        };

        if let Some(call) = stmt.as_call_node() {
            let name = std::str::from_utf8(call.name().as_slice()).unwrap_or("");

            // Track bare visibility modifiers
            if matches!(name, "private" | "protected" | "public")
                && call.arguments().is_none()
                && call.block().is_none()
            {
                current_visibility = match name {
                    "private" => "private",
                    "protected" => "protected",
                    _ => "public",
                };
                info.visibility = current_visibility;
                infos.push(info);
                continue;
            }

            if ACCESSOR_METHODS.contains(&name)
                && call.receiver().is_none()
                && call.arguments().is_some()
            {
                info.is_accessor = true;
                info.accessor_name = name.to_string();

                // Check previous_line_comment: is the source line before this accessor a comment?
                info.has_previous_line_comment =
                    previous_line_is_comment(source, stmt.location().start_offset());

                // Check groupable_accessor: examine the previous sibling
                info.groupable = is_groupable_accessor(source, &stmt_list, idx);
            }
        }

        infos.push(info);
    }

    // For each accessor, find groupable sibling accessors (same type, same visibility,
    // both groupable and not preceded by a comment)
    // Use a set to avoid reporting the same accessor twice
    let mut reported = vec![false; stmt_list.len()];

    for i in 0..infos.len() {
        if !infos[i].is_accessor {
            continue;
        }
        if reported[i] {
            continue;
        }
        // Skip accessors that have a previous line comment or are not groupable
        if infos[i].has_previous_line_comment || !infos[i].groupable {
            continue;
        }

        // Find all groupable siblings with the same accessor type and visibility
        let mut group: Vec<usize> = Vec::new();
        for j in 0..infos.len() {
            if !infos[j].is_accessor {
                continue;
            }
            if infos[j].accessor_name != infos[i].accessor_name {
                continue;
            }
            if infos[j].visibility != infos[i].visibility {
                continue;
            }
            if !infos[j].groupable || infos[j].has_previous_line_comment {
                continue;
            }
            group.push(j);
        }

        if group.len() > 1 {
            for &g in &group {
                if !reported[g] {
                    reported[g] = true;
                    let stmt = &stmt_list[infos[g].idx];
                    let loc = stmt.location();
                    let (line, column) = source.offset_to_line_col(loc.start_offset());
                    diagnostics.push(cop.diagnostic(
                        source,
                        line,
                        column,
                        format!(
                            "Group together all `{}` attributes.",
                            infos[g].accessor_name
                        ),
                    ));
                }
            }
        }
    }
}

/// Check if the source line immediately before the given offset is a comment line.
/// Matches RuboCop's `previous_line_comment?` which checks `processed_source[node.first_line - 2]`.
fn previous_line_is_comment(source: &SourceFile, start_offset: usize) -> bool {
    let (line, _) = source.offset_to_line_col(start_offset);
    if line <= 1 {
        return false;
    }
    // Get the previous line (line is 1-based, so line-2 is the 0-based index of previous line)
    let prev_line_idx = line - 2;
    for (i, source_line) in source.lines().enumerate() {
        if i == prev_line_idx {
            let trimmed = source_line
                .iter()
                .copied()
                .skip_while(|&b| b == b' ' || b == b'\t')
                .collect::<Vec<_>>();
            return trimmed.first() == Some(&b'#');
        }
    }
    false
}

/// Check if an accessor at index `idx` in `stmt_list` is "groupable" per RuboCop's logic.
///
/// RuboCop's `groupable_accessor?` examines the previous sibling (left sibling):
/// 1. No previous sibling -> groupable
/// 2. Previous is a block type (e.g., `sig { ... }`) -> unwrap to send child; if unwrapped
///    is not a send, groupable. Otherwise treat as send case below.
/// 3. Previous is NOT a send type (def, class, constant, etc.) -> groupable
/// 4. Previous IS a send: groupable only if it's an accessor, access modifier, OR there's
///    a blank line gap (> 1 line between them)
/// 5. Previous expression has an inline RBS `#:` annotation comment -> NOT groupable
fn is_groupable_accessor(
    source: &SourceFile,
    stmt_list: &[ruby_prism::Node<'_>],
    idx: usize,
) -> bool {
    if idx == 0 {
        return true;
    }

    let prev = &stmt_list[idx - 1];
    let curr = &stmt_list[idx];

    // Check if previous is a call node (send type in RuboCop terms).
    // In Prism, a call with a block (like `sig { ... }`) is still a CallNode.
    if let Some(prev_call) = prev.as_call_node() {
        let prev_name = std::str::from_utf8(prev_call.name().as_slice()).unwrap_or("");
        let prev_end_line = previous_expression_last_line(source, &prev_call);
        let curr_start_line = source.offset_to_line_col(curr.location().start_offset()).0;

        // RuboCop: accessors with RBS::Inline `#:` annotations on the previous expression
        // are not groupable. Check if the previous sibling's source line contains `#:`.
        if has_inline_rbs_comment(source, prev.location().start_offset()) {
            return false;
        }

        // Previous is an accessor — groupable (must have arguments; bare `attr` etc. are not accessors)
        if ACCESSOR_METHODS.contains(&prev_name)
            && prev_call.receiver().is_none()
            && prev_call.arguments().is_some()
        {
            return true;
        }

        // Previous is a bare access modifier — groupable
        if matches!(prev_name, "private" | "protected" | "public")
            && prev_call.arguments().is_none()
            && prev_call.block().is_none()
        {
            return true;
        }

        // Previous is some other send (annotation, macro, etc.) — NOT groupable
        // unless there's a blank line gap (> 1 line between them)
        return curr_start_line - prev_end_line > 1;
    }

    // Previous is not a send type (def, class, constant assignment, begin, etc.)
    // Per RuboCop: `return true unless previous_expression.send_type?` -> groupable
    true
}

/// RuboCop unwraps a previous block expression to its inner send before comparing
/// line spacing. Prism keeps block-form sends as a single `CallNode` whose location
/// extends through `end`, so use the block start line to recover the inner send span.
fn previous_expression_last_line(source: &SourceFile, call: &ruby_prism::CallNode<'_>) -> usize {
    if let Some(block) = call.block().and_then(|b| b.as_block_node()) {
        return source.offset_to_line_col(block.location().start_offset()).0;
    }

    source.offset_to_line_col(call.location().end_offset()).0
}

/// Check if the source line containing the node at `start_offset` has an inline
/// RBS::Inline annotation comment (`#:` syntax). RuboCop checks
/// `processed_source.comments.any? { |c| same_line?(c, prev) && c.text.start_with?('#:') }`.
fn has_inline_rbs_comment(source: &SourceFile, start_offset: usize) -> bool {
    let (line, _) = source.offset_to_line_col(start_offset);
    // line is 1-based; get the 0-based index
    let line_idx = line - 1;
    for (i, source_line) in source.lines().enumerate() {
        if i == line_idx {
            // Look for `#:` in the line (not at the start — it's an inline comment)
            // We need to find a `#` that's followed by `:` and is a comment, not inside a string.
            // Simple heuristic: find `#:` after the code portion. Since these are accessor
            // declarations, the pattern is `attr_reader :foo #: Type`.
            if let Some(pos) = source_line.windows(2).position(|w| w == b"#:") {
                // Make sure it's not at the start (that would be a regular comment, not inline)
                // and that it's preceded by whitespace (i.e., it's a trailing comment)
                if pos > 0 {
                    return true;
                }
            }
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(AccessorGrouping, "cops/style/accessor_grouping");

    fn separated_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("separated".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn offense_separated() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &AccessorGrouping,
            include_bytes!(
                "../../../tests/fixtures/cops/style/accessor_grouping/offense.separated.rb"
            ),
            separated_config(),
        );
    }

    #[test]
    fn no_offense_separated() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &AccessorGrouping,
            include_bytes!(
                "../../../tests/fixtures/cops/style/accessor_grouping/no_offense.separated.rb"
            ),
            separated_config(),
        );
    }
}
