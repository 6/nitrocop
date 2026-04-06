use std::cell::RefCell;
use std::collections::HashSet;

use ruby_prism::Visit;

use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::node_type::{
    CALL_AND_WRITE_NODE, CALL_NODE, CALL_OPERATOR_WRITE_NODE, CALL_OR_WRITE_NODE, CASE_MATCH_NODE,
    CASE_NODE, CLASS_VARIABLE_AND_WRITE_NODE, CLASS_VARIABLE_OPERATOR_WRITE_NODE,
    CLASS_VARIABLE_OR_WRITE_NODE, CLASS_VARIABLE_WRITE_NODE, CONSTANT_AND_WRITE_NODE,
    CONSTANT_OPERATOR_WRITE_NODE, CONSTANT_OR_WRITE_NODE, CONSTANT_PATH_AND_WRITE_NODE,
    CONSTANT_PATH_OPERATOR_WRITE_NODE, CONSTANT_PATH_OR_WRITE_NODE, CONSTANT_PATH_WRITE_NODE,
    CONSTANT_WRITE_NODE, GLOBAL_VARIABLE_AND_WRITE_NODE, GLOBAL_VARIABLE_OPERATOR_WRITE_NODE,
    GLOBAL_VARIABLE_OR_WRITE_NODE, GLOBAL_VARIABLE_WRITE_NODE, IF_NODE, INDEX_AND_WRITE_NODE,
    INDEX_OPERATOR_WRITE_NODE, INDEX_OR_WRITE_NODE, INSTANCE_VARIABLE_AND_WRITE_NODE,
    INSTANCE_VARIABLE_OPERATOR_WRITE_NODE, INSTANCE_VARIABLE_OR_WRITE_NODE,
    INSTANCE_VARIABLE_WRITE_NODE, LOCAL_VARIABLE_AND_WRITE_NODE,
    LOCAL_VARIABLE_OPERATOR_WRITE_NODE, LOCAL_VARIABLE_OR_WRITE_NODE, LOCAL_VARIABLE_WRITE_NODE,
    MULTI_WRITE_NODE, UNLESS_NODE,
};
use crate::cop::shared::util::unwrap_parentheses;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use regex::Regex;

const MSG: &str = "Use the return of the conditional for variable assignment and comparison.";
const ASSIGN_INSIDE_MSG: &str = "Assign variables inside of conditionals.";

/// Checks for `if`, `unless`, `case`, and `case/in` statements where each
/// branch assigns to the same variable. Suggests using the return value of
/// the conditional instead.
///
/// Supports local, instance, class, global variable writes, constant writes,
/// setter calls (`obj.x =`), index setters (`obj[k] =`), shovel sends
/// (`obj << value`), comparison/operator sends (`==`, `!=`, `<=`, `>=`, `=~`,
/// `!~`, `<`, `>`, `<=>`), and compound assignments (`+=`, `&&=`, `||=`).
///
/// Handles `if/elsif/else` chains (all branches must assign the same target),
/// `unless/else`, and ternary expressions with assignments.
///
/// Respects `SingleLineConditionsOnly` (default true): skips when any branch
/// has multiple statements. Skips offenses whose autocorrection would exceed
/// `Layout/LineLength`.
///
/// ## Corpus findings
///
/// FN reduction (2026-04-04): a large remaining corpus bucket was `if`/`else`
/// and `case` branches that both used `<<` on the same receiver, such as
/// `message << ...` and `this_sig_lines << ...`. Prism represents those as
/// `CallNode`s, not write nodes, so they needed the same target-key handling
/// as setter/index assignments.
///
/// FN reduction (2026-04-04): Prism also uses dedicated node types for
/// compound writes on calls and indexes, e.g. `foo.bar ||=`, `foo.bar +=`,
/// `foo[bar] ||=`, and `foo[bar] +=`. Those do not come through as `CallNode`
/// or variable write nodes, so this cop must derive stable target keys from
/// the receiver, method/index, and operator to match RuboCop.
///
/// FN reduction (2026-04-06): RuboCop's `assignment_type?` also treats
/// comparison/operator sends like `match.should == true` and `foo =~ /bar/`
/// as assignment-like for this cop. nitrocop was only handling setters and
/// `<<`, which missed whole comparison-based branch patterns.
///
/// FN reduction (2026-04-06): the line-length guard must match RuboCop's
/// `longest_line` behavior exactly. Counting bytes instead of characters and
/// re-adding the node's starting column made indented or unicode-containing
/// branches look too long, which suppressed legitimate offenses.
///
/// Variant fix (2026-04-06): `EnforcedStyle: assign_inside_condition` now
/// checks assignment-like nodes whose RHS is an `if`, `unless`, `case`, or
/// `case in`, instead of returning early for every non-default style. It also
/// mirrors RuboCop's nested-node ignore behavior, so assignments inside the
/// subtree of another assignment-like node do not double-report.
pub struct ConditionalAssignment;

impl Cop for ConditionalAssignment {
    fn name(&self) -> &'static str {
        "Style/ConditionalAssignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CASE_MATCH_NODE,
            CASE_NODE,
            IF_NODE,
            UNLESS_NODE,
            CALL_NODE,
            CALL_AND_WRITE_NODE,
            CALL_OPERATOR_WRITE_NODE,
            CALL_OR_WRITE_NODE,
            CLASS_VARIABLE_AND_WRITE_NODE,
            CLASS_VARIABLE_OPERATOR_WRITE_NODE,
            CLASS_VARIABLE_OR_WRITE_NODE,
            CLASS_VARIABLE_WRITE_NODE,
            CONSTANT_AND_WRITE_NODE,
            CONSTANT_OPERATOR_WRITE_NODE,
            CONSTANT_OR_WRITE_NODE,
            CONSTANT_PATH_AND_WRITE_NODE,
            CONSTANT_PATH_OPERATOR_WRITE_NODE,
            CONSTANT_PATH_OR_WRITE_NODE,
            CONSTANT_PATH_WRITE_NODE,
            CONSTANT_WRITE_NODE,
            GLOBAL_VARIABLE_AND_WRITE_NODE,
            GLOBAL_VARIABLE_OPERATOR_WRITE_NODE,
            GLOBAL_VARIABLE_OR_WRITE_NODE,
            GLOBAL_VARIABLE_WRITE_NODE,
            INDEX_AND_WRITE_NODE,
            INDEX_OPERATOR_WRITE_NODE,
            INDEX_OR_WRITE_NODE,
            INSTANCE_VARIABLE_AND_WRITE_NODE,
            INSTANCE_VARIABLE_OPERATOR_WRITE_NODE,
            INSTANCE_VARIABLE_OR_WRITE_NODE,
            INSTANCE_VARIABLE_WRITE_NODE,
            LOCAL_VARIABLE_AND_WRITE_NODE,
            LOCAL_VARIABLE_OPERATOR_WRITE_NODE,
            LOCAL_VARIABLE_OR_WRITE_NODE,
            LOCAL_VARIABLE_WRITE_NODE,
            MULTI_WRITE_NODE,
        ]
    }

    fn check_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "assign_to_condition");
        let single_line_only = config.get_bool("SingleLineConditionsOnly", true);
        let include_ternary = config.get_bool("IncludeTernaryExpressions", true);
        let max_line_length = config.get_usize("MaxLineLength", 120);
        let line_length_enabled = config.get_bool("LineLengthEnabled", max_line_length > 0);

        if style == "assign_inside_condition" {
            self.check_assignment_to_condition(
                source,
                node,
                parse_result,
                single_line_only,
                include_ternary,
                diagnostics,
            );
            return;
        }

        if style != "assign_to_condition" {
            return;
        }

        if let Some(if_node) = node.as_if_node() {
            // Ternary: Prism represents ternary as IfNode with no if_keyword_loc
            if if_node.if_keyword_loc().is_none() {
                if include_ternary {
                    self.check_ternary(
                        source,
                        &if_node,
                        max_line_length,
                        line_length_enabled,
                        diagnostics,
                    );
                }
                return;
            }
            // Must be top-level if, not elsif
            if let Some(kw) = if_node.if_keyword_loc() {
                if kw.as_slice() == b"elsif" {
                    return;
                }
            }
            self.check_if(
                source,
                &if_node,
                single_line_only,
                max_line_length,
                line_length_enabled,
                diagnostics,
            );
        } else if let Some(case_node) = node.as_case_node() {
            self.check_case(
                source,
                &case_node,
                single_line_only,
                max_line_length,
                line_length_enabled,
                diagnostics,
            );
        } else if let Some(cm) = node.as_case_match_node() {
            self.check_case_match(
                source,
                &cm,
                single_line_only,
                max_line_length,
                line_length_enabled,
                diagnostics,
            );
        } else if let Some(unless_node) = node.as_unless_node() {
            self.check_unless(
                source,
                &unless_node,
                single_line_only,
                max_line_length,
                line_length_enabled,
                diagnostics,
            );
        }
    }
}

impl ConditionalAssignment {
    fn check_if(
        &self,
        source: &SourceFile,
        if_node: &ruby_prism::IfNode<'_>,
        single_line_only: bool,
        max_line_length: usize,
        line_length_enabled: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let bodies: Vec<Vec<ruby_prism::Node<'_>>> = match collect_if_bodies(if_node) {
            Some(bodies) => bodies,
            None => return,
        };
        let branches: Vec<&[ruby_prism::Node<'_>]> = bodies.iter().map(|v| v.as_slice()).collect();
        self.check_branches(
            source,
            &if_node.location(),
            &branches,
            single_line_only,
            max_line_length,
            line_length_enabled,
            diagnostics,
        );
    }

    fn check_case(
        &self,
        source: &SourceFile,
        case_node: &ruby_prism::CaseNode<'_>,
        single_line_only: bool,
        max_line_length: usize,
        line_length_enabled: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let bodies: Vec<Vec<ruby_prism::Node<'_>>> = match collect_case_bodies(case_node) {
            Some(bodies) => bodies,
            None => return,
        };
        let branches: Vec<&[ruby_prism::Node<'_>]> = bodies.iter().map(|v| v.as_slice()).collect();
        self.check_branches(
            source,
            &case_node.location(),
            &branches,
            single_line_only,
            max_line_length,
            line_length_enabled,
            diagnostics,
        );
    }

    fn check_case_match(
        &self,
        source: &SourceFile,
        case_match: &ruby_prism::CaseMatchNode<'_>,
        single_line_only: bool,
        max_line_length: usize,
        line_length_enabled: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let bodies: Vec<Vec<ruby_prism::Node<'_>>> = match collect_case_match_bodies(case_match) {
            Some(bodies) => bodies,
            None => return,
        };
        let branches: Vec<&[ruby_prism::Node<'_>]> = bodies.iter().map(|v| v.as_slice()).collect();
        self.check_branches(
            source,
            &case_match.location(),
            &branches,
            single_line_only,
            max_line_length,
            line_length_enabled,
            diagnostics,
        );
    }

    fn check_unless(
        &self,
        source: &SourceFile,
        unless_node: &ruby_prism::UnlessNode<'_>,
        single_line_only: bool,
        max_line_length: usize,
        line_length_enabled: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let bodies: Vec<Vec<ruby_prism::Node<'_>>> = match collect_unless_bodies(unless_node) {
            Some(bodies) => bodies,
            None => return,
        };
        let unless_body = &bodies[0];
        let else_body = &bodies[1];
        let branches: [&[ruby_prism::Node<'_>]; 2] = [&unless_body, &else_body];
        self.check_branches(
            source,
            &unless_node.location(),
            &branches,
            single_line_only,
            max_line_length,
            line_length_enabled,
            diagnostics,
        );
    }

    fn check_ternary(
        &self,
        source: &SourceFile,
        if_node: &ruby_prism::IfNode<'_>,
        max_line_length: usize,
        line_length_enabled: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let if_stmts = match if_node.statements() {
            Some(s) => s,
            None => return,
        };
        let if_body: Vec<_> = if_stmts.body().iter().collect();
        if if_body.len() != 1 {
            return;
        }

        let subsequent = match if_node.subsequent() {
            Some(s) => s,
            None => return,
        };
        let else_node = match subsequent.as_else_node() {
            Some(e) => e,
            None => return,
        };
        let else_stmts = match else_node.statements() {
            Some(s) => s,
            None => return,
        };
        let else_body: Vec<_> = else_stmts.body().iter().collect();
        if else_body.len() != 1 {
            return;
        }

        let if_info = match get_assignment_info(&if_body[0]) {
            Some(i) => i,
            None => return,
        };
        let else_info = match get_assignment_info(&else_body[0]) {
            Some(i) => i,
            None => return,
        };

        if if_info.key != else_info.key {
            return;
        }

        if line_length_enabled && max_line_length > 0 {
            if exceeds_line_limit(&if_node.location(), &if_info.lhs_text, max_line_length) {
                return;
            }
        }

        let loc = if_node.location();
        let (line, col) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(source, line, col, MSG.to_string()));
    }

    fn check_assignment_to_condition(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        parse_result: &ruby_prism::ParseResult<'_>,
        single_line_only: bool,
        include_ternary: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if has_assignment_like_ancestor(parse_result, source, node) {
            return;
        }

        let bodies: Vec<Vec<ruby_prism::Node<'_>>> =
            match assignment_branches(node, include_ternary) {
                Some(bodies) => bodies,
                None => return,
            };

        if single_line_only && bodies.iter().any(|branch| branch.len() > 1) {
            return;
        }

        let loc = node.location();
        let (line, col) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(source, line, col, ASSIGN_INSIDE_MSG.to_string()));
    }

    #[allow(clippy::too_many_arguments)]
    fn check_branches(
        &self,
        source: &SourceFile,
        node_loc: &ruby_prism::Location<'_>,
        branches: &[&[ruby_prism::Node<'_>]],
        single_line_only: bool,
        max_line_length: usize,
        line_length_enabled: bool,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        if branches.is_empty() {
            return;
        }

        for branch in branches {
            if branch.is_empty() {
                return;
            }
            if single_line_only && branch.len() > 1 {
                return;
            }
        }

        // Check last statement of each branch is an assignment to the same target
        let mut first_key: Option<String> = None;
        let mut lhs_text = String::new();

        for branch in branches {
            let last = &branch[branch.len() - 1];
            let info = match get_assignment_info(last) {
                Some(i) => i,
                None => return,
            };
            match &first_key {
                None => {
                    first_key = Some(info.key);
                    lhs_text = info.lhs_text;
                }
                Some(k) => {
                    if info.key != *k {
                        return;
                    }
                }
            }
        }

        // Line length guard
        if line_length_enabled && max_line_length > 0 && !lhs_text.is_empty() {
            if exceeds_line_limit(node_loc, &lhs_text, max_line_length) {
                return;
            }
        }

        let (line, col) = source.offset_to_line_col(node_loc.start_offset());
        diagnostics.push(self.diagnostic(source, line, col, MSG.to_string()));
    }
}

struct AssignInfo {
    key: String,
    lhs_text: String, // e.g. "x = ", "@foo = ", "obj.method = "
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct NodeKey {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct CacheKey {
    parse_result_ptr: usize,
    source_ptr: usize,
    source_len: usize,
}

thread_local! {
    static NESTED_ASSIGNMENT_CACHE: RefCell<Option<(CacheKey, HashSet<NodeKey>)>> =
        const { RefCell::new(None) };
}

fn node_source(node: ruby_prism::Node<'_>) -> String {
    String::from_utf8_lossy(node.location().as_slice()).to_string()
}

fn node_source_ref(node: &ruby_prism::Node<'_>) -> String {
    String::from_utf8_lossy(node.location().as_slice()).to_string()
}

fn receiver_source(receiver: Option<ruby_prism::Node<'_>>) -> String {
    receiver.map_or(String::new(), node_source)
}

fn call_target_source(receiver: Option<ruby_prism::Node<'_>>, method_name: &[u8]) -> String {
    let receiver = receiver_source(receiver);
    let method_name = String::from_utf8_lossy(method_name);
    if receiver.is_empty() {
        method_name.to_string()
    } else {
        format!("{}.{}", receiver, method_name)
    }
}

fn index_target_source(
    receiver: Option<ruby_prism::Node<'_>>,
    args: Option<ruby_prism::ArgumentsNode<'_>>,
    drop_last_argument: bool,
) -> Option<String> {
    let args = args?;
    let arg_list: Vec<_> = args.arguments().iter().collect();
    let end = if drop_last_argument {
        arg_list.len().checked_sub(1)?
    } else {
        arg_list.len()
    };
    if end == 0 {
        return None;
    }

    let receiver = receiver_source(receiver);
    let indices = arg_list[..end]
        .iter()
        .map(node_source_ref)
        .collect::<Vec<_>>()
        .join(", ");

    if receiver.is_empty() {
        Some(format!("[{}]", indices))
    } else {
        Some(format!("{}[{}]", receiver, indices))
    }
}

fn call_matches_assignment_like_method(method: &[u8]) -> bool {
    matches!(
        method,
        b"[]=" | b"<<" | b"=~" | b"!~" | b"<=>" | b"<" | b">"
    ) || method.ends_with(b"=")
}

fn operator_call_assignment_info(
    receiver: Option<ruby_prism::Node<'_>>,
    method: &[u8],
) -> AssignInfo {
    let recv_src = receiver_source(receiver);
    let method_str = String::from_utf8_lossy(method);
    let lhs_text = if recv_src.is_empty() {
        format!("{} ", method_str)
    } else {
        format!("{} {} ", recv_src, method_str)
    };

    AssignInfo {
        key: format!("call:{}:{}", recv_src, method_str),
        lhs_text,
    }
}

fn unwrap_assignment_rhs<'a>(mut node: ruby_prism::Node<'a>) -> ruby_prism::Node<'a> {
    loop {
        let before = (node.location().start_offset(), node.location().end_offset());
        node = unwrap_parentheses(node);
        let after = (node.location().start_offset(), node.location().end_offset());
        if before != after {
            continue;
        }

        let Some(begin_node) = node.as_begin_node() else {
            break;
        };
        if begin_node.rescue_clause().is_some()
            || begin_node.else_clause().is_some()
            || begin_node.ensure_clause().is_some()
        {
            break;
        }

        let Some(statements) = begin_node.statements() else {
            break;
        };
        if statements.body().len() != 1 {
            break;
        }

        node = statements.body().iter().next().unwrap();
    }

    node
}

fn assignment_value<'a>(node: &ruby_prism::Node<'a>) -> Option<ruby_prism::Node<'a>> {
    if let Some(n) = node.as_local_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_multi_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_local_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_call_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_index_operator_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_local_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_local_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_instance_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_class_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_global_variable_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_constant_path_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_call_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_call_or_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_index_and_write_node() {
        return Some(n.value());
    }
    if let Some(n) = node.as_index_or_write_node() {
        return Some(n.value());
    }
    if let Some(call) = node.as_call_node() {
        if !call_matches_assignment_like_method(call.name().as_slice()) {
            return None;
        }
        let args = call.arguments()?;
        return args.arguments().iter().last();
    }
    None
}

fn collect_if_bodies<'a>(
    if_node: &ruby_prism::IfNode<'a>,
) -> Option<Vec<Vec<ruby_prism::Node<'a>>>> {
    let if_statements = if_node.statements()?;
    let mut bodies = vec![if_statements.body().iter().collect()];

    let mut current_subsequent = if_node.subsequent();
    loop {
        let subsequent = current_subsequent?;
        if let Some(elsif_node) = subsequent.as_if_node() {
            let statements = elsif_node.statements()?;
            bodies.push(statements.body().iter().collect());
            current_subsequent = elsif_node.subsequent();
            continue;
        }

        let else_node = subsequent.as_else_node()?;
        let else_statements = else_node.statements()?;
        bodies.push(else_statements.body().iter().collect());
        break;
    }

    Some(bodies)
}

fn collect_case_bodies<'a>(
    case_node: &ruby_prism::CaseNode<'a>,
) -> Option<Vec<Vec<ruby_prism::Node<'a>>>> {
    let else_clause = case_node.else_clause()?;
    let mut bodies = Vec::new();

    for condition in case_node.conditions().iter() {
        let when_node = condition.as_when_node()?;
        let statements = when_node.statements()?;
        bodies.push(statements.body().iter().collect());
    }

    let else_statements = else_clause.statements()?;
    bodies.push(else_statements.body().iter().collect());
    Some(bodies)
}

fn collect_case_match_bodies<'a>(
    case_match: &ruby_prism::CaseMatchNode<'a>,
) -> Option<Vec<Vec<ruby_prism::Node<'a>>>> {
    let else_clause = case_match.else_clause()?;
    let mut bodies = Vec::new();

    for condition in case_match.conditions().iter() {
        let in_node = condition.as_in_node()?;
        let statements = in_node.statements()?;
        bodies.push(statements.body().iter().collect());
    }

    let else_statements = else_clause.statements()?;
    bodies.push(else_statements.body().iter().collect());
    Some(bodies)
}

fn collect_unless_bodies<'a>(
    unless_node: &ruby_prism::UnlessNode<'a>,
) -> Option<Vec<Vec<ruby_prism::Node<'a>>>> {
    let else_clause = unless_node.else_clause()?;
    let unless_statements = unless_node.statements()?;
    let else_statements = else_clause.statements()?;

    Some(vec![
        unless_statements.body().iter().collect(),
        else_statements.body().iter().collect(),
    ])
}

fn assignment_branches<'a>(
    node: &'a ruby_prism::Node<'a>,
    include_ternary: bool,
) -> Option<Vec<Vec<ruby_prism::Node<'a>>>> {
    let rhs = unwrap_assignment_rhs(assignment_value(node)?);

    if let Some(if_node) = rhs.as_if_node() {
        if if_node.if_keyword_loc().is_none() && !include_ternary {
            return None;
        }
        return collect_if_bodies(&if_node);
    }
    if let Some(unless_node) = rhs.as_unless_node() {
        return collect_unless_bodies(&unless_node);
    }
    if let Some(case_node) = rhs.as_case_node() {
        return collect_case_bodies(&case_node);
    }
    if let Some(case_match) = rhs.as_case_match_node() {
        return collect_case_match_bodies(&case_match);
    }
    None
}

struct NestedAssignmentVisitor {
    assignment_like_depth: usize,
    assignment_like_stack: Vec<bool>,
    nested_nodes: HashSet<NodeKey>,
}

impl NestedAssignmentVisitor {
    fn record_node(&mut self, node: ruby_prism::Node<'_>) {
        let is_assignment_like = assignment_value(&node).is_some();
        if is_assignment_like && self.assignment_like_depth > 0 {
            self.nested_nodes.insert(node_key(&node));
        }
        if is_assignment_like {
            self.assignment_like_depth += 1;
        }
        self.assignment_like_stack.push(is_assignment_like);
    }

    fn leave_node(&mut self) {
        if self.assignment_like_stack.pop().unwrap_or(false) {
            self.assignment_like_depth = self.assignment_like_depth.saturating_sub(1);
        }
    }
}

impl<'pr> Visit<'pr> for NestedAssignmentVisitor {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.record_node(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.leave_node();
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.record_node(node);
    }

    fn visit_leaf_node_leave(&mut self) {
        self.leave_node();
    }
}

fn node_key(node: &ruby_prism::Node<'_>) -> NodeKey {
    let loc = node.location();
    NodeKey {
        start: loc.start_offset(),
        end: loc.end_offset(),
    }
}

fn has_assignment_like_ancestor(
    parse_result: &ruby_prism::ParseResult<'_>,
    source: &SourceFile,
    node: &ruby_prism::Node<'_>,
) -> bool {
    let cache_key = CacheKey {
        parse_result_ptr: parse_result as *const _ as usize,
        source_ptr: source.as_bytes().as_ptr() as usize,
        source_len: source.as_bytes().len(),
    };

    NESTED_ASSIGNMENT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let needs_rebuild = !matches!(cache.as_ref(), Some((key, _)) if *key == cache_key);

        if needs_rebuild {
            let mut visitor = NestedAssignmentVisitor {
                assignment_like_depth: 0,
                assignment_like_stack: Vec::new(),
                nested_nodes: HashSet::new(),
            };
            visitor.visit(&parse_result.node());
            *cache = Some((cache_key, visitor.nested_nodes));
        }

        cache
            .as_ref()
            .is_some_and(|(_, nested_nodes)| nested_nodes.contains(&node_key(node)))
    })
}

fn get_assignment_info(node: &ruby_prism::Node<'_>) -> Option<AssignInfo> {
    if let Some(w) = node.as_local_variable_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("lvar:{}", name),
            lhs_text: format!("{} = ", name),
        });
    }
    if let Some(w) = node.as_instance_variable_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("ivar:{}", name),
            lhs_text: format!("{} = ", name),
        });
    }
    if let Some(w) = node.as_class_variable_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("cvar:{}", name),
            lhs_text: format!("{} = ", name),
        });
    }
    if let Some(w) = node.as_global_variable_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("gvar:{}", name),
            lhs_text: format!("{} = ", name),
        });
    }
    if let Some(w) = node.as_constant_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("const:{}", name),
            lhs_text: format!("{} = ", name),
        });
    }
    if let Some(w) = node.as_constant_path_write_node() {
        let target = String::from_utf8_lossy(w.target().location().as_slice()).to_string();
        return Some(AssignInfo {
            key: format!("constpath:{}", target),
            lhs_text: format!("{} = ", target),
        });
    }
    // Setter call: obj.method= value or obj[key]= value.
    // RuboCop also treats shovel and comparison/operator sends as
    // assignment-like here.
    if let Some(call) = node.as_call_node() {
        let method = call.name().as_slice();
        if !call_matches_assignment_like_method(method) {
            return None;
        }
        // Check []= BEFORE is_setter_method — is_setter_method matches any
        // name ending with `=`, which includes `[]=`.  The generic setter path
        // ignores the index arguments, so `flash[:success]=` and
        // `flash[:error]=` would incorrectly share the same assignment key.
        if method == b"[]=" {
            if let Some(target) = index_target_source(call.receiver(), call.arguments(), true) {
                return Some(AssignInfo {
                    key: format!("send:{}=", target),
                    lhs_text: format!("{} = ", target),
                });
            }
            return None;
        }
        if method_identifier_predicates::is_setter_method(method) {
            let method_str = String::from_utf8_lossy(method);
            let method_base = &method_str[..method_str.len().saturating_sub(1)];
            let target = call_target_source(call.receiver(), method_base.as_bytes());
            return Some(AssignInfo {
                key: format!("send:{}=", target),
                lhs_text: format!("{} = ", target),
            });
        }
        if method == b"<<" {
            let recv_src = receiver_source(call.receiver());
            return Some(AssignInfo {
                key: format!("send:{}<<", recv_src),
                lhs_text: format!("{} << ", recv_src),
            });
        }
        return Some(operator_call_assignment_info(call.receiver(), method));
    }
    // Operator assignments: x += 1
    if let Some(w) = node.as_local_variable_operator_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:lvar:{} {}", name, op),
            lhs_text: format!("{} {}= ", name, op),
        });
    }
    if let Some(w) = node.as_instance_variable_operator_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:ivar:{} {}", name, op),
            lhs_text: format!("{} {}= ", name, op),
        });
    }
    if let Some(w) = node.as_class_variable_operator_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:cvar:{} {}", name, op),
            lhs_text: format!("{} {}= ", name, op),
        });
    }
    if let Some(w) = node.as_global_variable_operator_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:gvar:{} {}", name, op),
            lhs_text: format!("{} {}= ", name, op),
        });
    }
    if let Some(w) = node.as_constant_operator_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:const:{} {}", name, op),
            lhs_text: format!("{} {}= ", name, op),
        });
    }
    if let Some(w) = node.as_constant_path_operator_write_node() {
        let target = String::from_utf8_lossy(w.target().location().as_slice()).to_string();
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:constpath:{} {}", target, op),
            lhs_text: format!("{} {}= ", target, op),
        });
    }
    if let Some(w) = node.as_call_operator_write_node() {
        let target = call_target_source(w.receiver(), w.read_name().as_slice());
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:call:{} {}", target, op),
            lhs_text: format!("{} {}= ", target, op),
        });
    }
    if let Some(w) = node.as_index_operator_write_node() {
        let target = index_target_source(w.receiver(), w.arguments(), false)?;
        let op = String::from_utf8_lossy(w.binary_operator().as_slice());
        return Some(AssignInfo {
            key: format!("op:index:{} {}", target, op),
            lhs_text: format!("{} {}= ", target, op),
        });
    }
    // And/Or assignments: x &&= 1, x ||= 1
    if let Some(w) = node.as_local_variable_and_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("and:lvar:{}", name),
            lhs_text: format!("{} &&= ", name),
        });
    }
    if let Some(w) = node.as_local_variable_or_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("or:lvar:{}", name),
            lhs_text: format!("{} ||= ", name),
        });
    }
    if let Some(w) = node.as_instance_variable_and_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("and:ivar:{}", name),
            lhs_text: format!("{} &&= ", name),
        });
    }
    if let Some(w) = node.as_instance_variable_or_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("or:ivar:{}", name),
            lhs_text: format!("{} ||= ", name),
        });
    }
    if let Some(w) = node.as_class_variable_and_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("and:cvar:{}", name),
            lhs_text: format!("{} &&= ", name),
        });
    }
    if let Some(w) = node.as_class_variable_or_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("or:cvar:{}", name),
            lhs_text: format!("{} ||= ", name),
        });
    }
    if let Some(w) = node.as_global_variable_and_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("and:gvar:{}", name),
            lhs_text: format!("{} &&= ", name),
        });
    }
    if let Some(w) = node.as_global_variable_or_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("or:gvar:{}", name),
            lhs_text: format!("{} ||= ", name),
        });
    }
    if let Some(w) = node.as_constant_and_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("and:const:{}", name),
            lhs_text: format!("{} &&= ", name),
        });
    }
    if let Some(w) = node.as_constant_or_write_node() {
        let name = String::from_utf8_lossy(w.name().as_slice());
        return Some(AssignInfo {
            key: format!("or:const:{}", name),
            lhs_text: format!("{} ||= ", name),
        });
    }
    if let Some(w) = node.as_constant_path_and_write_node() {
        let target = String::from_utf8_lossy(w.target().location().as_slice()).to_string();
        return Some(AssignInfo {
            key: format!("and:constpath:{}", target),
            lhs_text: format!("{} &&= ", target),
        });
    }
    if let Some(w) = node.as_constant_path_or_write_node() {
        let target = String::from_utf8_lossy(w.target().location().as_slice()).to_string();
        return Some(AssignInfo {
            key: format!("or:constpath:{}", target),
            lhs_text: format!("{} ||= ", target),
        });
    }
    if let Some(w) = node.as_call_and_write_node() {
        let target = call_target_source(w.receiver(), w.read_name().as_slice());
        return Some(AssignInfo {
            key: format!("and:call:{}", target),
            lhs_text: format!("{} &&= ", target),
        });
    }
    if let Some(w) = node.as_call_or_write_node() {
        let target = call_target_source(w.receiver(), w.read_name().as_slice());
        return Some(AssignInfo {
            key: format!("or:call:{}", target),
            lhs_text: format!("{} ||= ", target),
        });
    }
    if let Some(w) = node.as_index_and_write_node() {
        let target = index_target_source(w.receiver(), w.arguments(), false)?;
        return Some(AssignInfo {
            key: format!("and:index:{}", target),
            lhs_text: format!("{} &&= ", target),
        });
    }
    if let Some(w) = node.as_index_or_write_node() {
        let target = index_target_source(w.receiver(), w.arguments(), false)?;
        return Some(AssignInfo {
            key: format!("or:index:{}", target),
            lhs_text: format!("{} ||= ", target),
        });
    }
    None
}

/// Check if the corrected form would exceed the configured line length.
/// Mirrors RuboCop's `correction_exceeds_line_limit?`: for each source line,
/// remove the assignment LHS (if present), find the longest remaining line,
/// and check if `lhs.len() + longest > max_line_length`.
fn exceeds_line_limit(
    node_loc: &ruby_prism::Location<'_>,
    lhs_text: &str,
    max_line_length: usize,
) -> bool {
    if lhs_text.is_empty() {
        return false;
    }

    let node_bytes = node_loc.as_slice();
    let src = match std::str::from_utf8(node_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let mut assignment_pattern = String::from(r"\s*");
    for ch in lhs_text.chars() {
        if ch.is_whitespace() {
            assignment_pattern.push_str(r"\s*");
        } else {
            assignment_pattern.push_str(&regex::escape(&ch.to_string()));
        }
    }
    let assignment_re = Regex::new(&assignment_pattern).ok();

    let lhs_len = lhs_text.chars().count();
    let max_remaining = src
        .lines()
        .map(|line| {
            let line = line.trim_end_matches('\r');
            let stripped = assignment_re
                .as_ref()
                .map_or_else(|| line.to_string(), |re| re.replace(line, "").into_owned());
            stripped.chars().count()
        })
        .max()
        .unwrap_or(0);

    lhs_len + max_remaining > max_line_length
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    crate::cop_fixture_tests!(ConditionalAssignment, "cops/style/conditional_assignment");

    fn assign_inside_condition_config() -> CopConfig {
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("assign_inside_condition".to_string()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn assign_inside_condition_flags_if_rhs_assignment() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ConditionalAssignment,
            b"x = if condition\n^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Assign variables inside of conditionals.\n  1\nelse\n  2\nend\n",
            assign_inside_condition_config(),
        );
    }

    #[test]
    fn assign_inside_condition_flags_call_rhs_assignment() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ConditionalAssignment,
            b"items << if condition\n^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Assign variables inside of conditionals.\n  1\nelse\n  2\nend\n",
            assign_inside_condition_config(),
        );
    }

    #[test]
    fn assign_inside_condition_flags_ternary_rhs_assignment() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ConditionalAssignment,
            b"value = condition ? 1 : 2\n^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Assign variables inside of conditionals.\n",
            assign_inside_condition_config(),
        );
    }

    #[test]
    fn assign_inside_condition_does_not_flag_assignment_inside_branches() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ConditionalAssignment,
            b"if condition\n  x = 1\nelse\n  x = 2\nend\n",
            assign_inside_condition_config(),
        );
    }

    #[test]
    fn assign_inside_condition_ignores_nested_assignment_inside_outer_assignment() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ConditionalAssignment,
            b"result = if max <= 1\n  me = is_senpai ? 'senpai' : 'kohai'\n  me\nelse\n  0\nend\n",
            assign_inside_condition_config(),
        );
    }

    #[test]
    fn assign_inside_condition_ignores_nested_assignment_inside_nonconditional_outer_assignment() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ConditionalAssignment,
            b"result = some_call(me = is_senpai ? 'senpai' : 'kohai')\n",
            assign_inside_condition_config(),
        );
    }
}
