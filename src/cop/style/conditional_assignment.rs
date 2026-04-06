use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::node_type::{CASE_MATCH_NODE, CASE_NODE, IF_NODE, UNLESS_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

const MSG: &str = "Use the return of the conditional for variable assignment and comparison.";

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
/// The line-length guard uses a full body-line analysis (all lines in the
/// node, not just the first) to match RuboCop's `correction_exceeds_line_limit?`.
/// Some repos disable `Layout/LineLength` via `DisabledByDefault: true`, which
/// means the oracle was generated without the guard — causing a small FN delta
/// at HEAD for those repos. This is acceptable in reduce mode.
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
/// `<<`, which missed comparison-based branch patterns.
///
/// FN reduction (2026-04-06): the line-length guard was adding `node_col`
/// (starting column) to the first line's length, double-counting indentation.
/// RuboCop's `longest_line` computes each branch line's length independently
/// of the node's column position. Dropped `node_col` to match.
pub struct ConditionalAssignment;

impl Cop for ConditionalAssignment {
    fn name(&self) -> &'static str {
        "Style/ConditionalAssignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CASE_MATCH_NODE, CASE_NODE, IF_NODE, UNLESS_NODE]
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
        if config.get_str("EnforcedStyle", "assign_to_condition") != "assign_to_condition" {
            return;
        }

        let single_line_only = config.get_bool("SingleLineConditionsOnly", true);
        let include_ternary = config.get_bool("IncludeTernaryExpressions", true);
        let max_line_length = config.get_usize("MaxLineLength", 120);
        let line_length_enabled = config.get_bool("LineLengthEnabled", max_line_length > 0);

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
        // Collect all branches: if body, then traverse elsif chain, then else
        let mut bodies: Vec<Vec<ruby_prism::Node<'_>>> = Vec::new();

        // First branch: the if body
        let if_stmts = match if_node.statements() {
            Some(s) => s,
            None => return,
        };
        bodies.push(if_stmts.body().iter().collect());

        // Traverse the subsequent chain (elsif nodes and final else)
        let mut current_subsequent = if_node.subsequent();
        loop {
            let subsequent = match current_subsequent {
                Some(s) => s,
                None => return, // no else clause at end -> not flaggable
            };

            // Check if it's another if node (elsif)
            if let Some(elsif_node) = subsequent.as_if_node() {
                let stmts = match elsif_node.statements() {
                    Some(s) => s,
                    None => return,
                };
                bodies.push(stmts.body().iter().collect());
                current_subsequent = elsif_node.subsequent();
                continue;
            }

            // Must be an else node (terminal)
            let else_node = match subsequent.as_else_node() {
                Some(e) => e,
                None => return,
            };
            let else_stmts = match else_node.statements() {
                Some(s) => s,
                None => return,
            };
            bodies.push(else_stmts.body().iter().collect());
            break;
        }

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
        let else_clause = match case_node.else_clause() {
            Some(e) => e,
            None => return,
        };

        let mut bodies: Vec<Vec<ruby_prism::Node<'_>>> = Vec::new();
        for condition in case_node.conditions().iter() {
            let when_node = match condition.as_when_node() {
                Some(w) => w,
                None => return,
            };
            match when_node.statements() {
                Some(s) => bodies.push(s.body().iter().collect()),
                None => return,
            }
        }
        match else_clause.statements() {
            Some(s) => bodies.push(s.body().iter().collect()),
            None => return,
        }

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
        let else_clause = match case_match.else_clause() {
            Some(e) => e,
            None => return,
        };

        let mut bodies: Vec<Vec<ruby_prism::Node<'_>>> = Vec::new();
        for condition in case_match.conditions().iter() {
            let in_node = match condition.as_in_node() {
                Some(i) => i,
                None => return,
            };
            match in_node.statements() {
                Some(s) => bodies.push(s.body().iter().collect()),
                None => return,
            }
        }
        match else_clause.statements() {
            Some(s) => bodies.push(s.body().iter().collect()),
            None => return,
        }

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
        // Must have else clause
        let else_clause = match unless_node.else_clause() {
            Some(e) => e,
            None => return,
        };

        let unless_stmts = match unless_node.statements() {
            Some(s) => s,
            None => return,
        };
        let unless_body: Vec<_> = unless_stmts.body().iter().collect();

        let else_stmts = match else_clause.statements() {
            Some(s) => s,
            None => return,
        };
        let else_body: Vec<_> = else_stmts.body().iter().collect();

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

        if line_length_enabled
            && max_line_length > 0
            && exceeds_line_limit(&if_node.location(), &if_info.lhs_text, max_line_length)
        {
            return;
        }

        let loc = if_node.location();
        let (line, col) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(source, line, col, MSG.to_string()));
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
        if line_length_enabled
            && max_line_length > 0
            && !lhs_text.is_empty()
            && exceeds_line_limit(node_loc, &lhs_text, max_line_length)
        {
            return;
        }

        let (line, col) = source.offset_to_line_col(node_loc.start_offset());
        diagnostics.push(self.diagnostic(source, line, col, MSG.to_string()));
    }
}

struct AssignInfo {
    key: String,
    lhs_text: String, // e.g. "x = ", "@foo = ", "obj.method = "
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
        // Comparison/operator sends: ==, !=, <=, >=, =~, !~, <=>, <, >
        // RuboCop's assignment_type? treats these as assignment-like.
        if matches!(
            method,
            b"==" | b"!=" | b"<=" | b">=" | b"=~" | b"!~" | b"<=>" | b"<" | b">"
        ) {
            let recv_src = receiver_source(call.receiver());
            let method_str = String::from_utf8_lossy(method);
            return Some(AssignInfo {
                key: format!("send:{} {}", recv_src, method_str),
                lhs_text: format!("{} {} ", recv_src, method_str),
            });
        }
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
    let node_bytes = node_loc.as_slice();
    let src = match std::str::from_utf8(node_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let lhs_trimmed = lhs_text.trim_end();
    let mut max_remaining = 0;
    for line in src.lines() {
        // Try to remove the LHS from this line (at line start after whitespace)
        let trimmed = line.trim_start();
        let remaining = if let Some(stripped) = trimmed.strip_prefix(lhs_trimmed) {
            let rest = stripped.trim_start();
            let leading_ws = line.len() - trimmed.len();
            leading_ws + rest.len()
        } else {
            line.len()
        };
        if remaining > max_remaining {
            max_remaining = remaining;
        }
    }
    lhs_text.len() + max_remaining > max_line_length
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(ConditionalAssignment, "cops/style/conditional_assignment");
}
