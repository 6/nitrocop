use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Count leading whitespace characters (spaces and tabs) as columns.
/// Unlike `indentation_of()` which only counts spaces, this counts both spaces
/// and tabs as 1 column each, matching `offset_to_line_col()`'s character counting.
fn leading_whitespace_columns(line: &[u8]) -> usize {
    line.iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

fn char_width(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| (b & 0xC0) != 0x80).count()
}

fn config_style_matches(config: &CopConfig, key: &str, target: &str, default: &str) -> bool {
    match config.options.get(key) {
        Some(value) => value.as_str().is_some_and(|style| style == target),
        None => default == target,
    }
}

/// ## Corpus investigation (2026-03-08)
///
/// Corpus oracle reported high FN volume concentrated in closing-brace sites.
///
/// Root cause: this cop only checked the first element of multiline hashes and
/// returned early for empty hashes. RuboCop's `Layout/FirstHashElementIndentation`
/// also enforces right-brace indentation, with the same indent base rules used
/// for the first element:
/// - line start for ordinary hashes
/// - first position after `(` for `special_inside_parentheses`
/// - parent hash key when a hash value has a following sibling pair
/// - left brace column for `align_braces`
///
/// Fix: reuse a shared indent-base calculation for both the first element and
/// the right brace, and keep checking empty multiline hashes so `a << {` / `}`
/// cases are covered.
///
/// ## Corpus investigation (2026-03-14)
///
/// FP=12, FN=14 remaining. Root causes:
///
/// 1. Tab-indented files (WhatWeb, phlex, crowdint, iobridge, puppetlabs): `indentation_of()`
///    only counts spaces, returning 0 for tab-indented lines, while `offset_to_line_col()` counts
///    tabs as column positions. Fix: use `leading_whitespace_columns()` that counts both tabs and
///    spaces as single columns, consistent with `offset_to_line_col()`.
///
/// 2. Splat FP (Shopify/shipit-engine): hashes whose only elements are `**var` (AssocSplatNode)
///    were being checked for first-element indentation, but RuboCop's `hash_node.pairs.first`
///    skips kwsplat nodes. Fix: filter to AssocNode elements only (matching `.pairs`).
///
/// 3. Double-splat FN (vagrant-hostmanager): hashes inside `**{...}` in method args were not
///    found because `find_hash_args_in_call` didn't traverse AssocSplatNode. Fix: add traversal.
///
/// 4. Local var assignment FN (foreigner): hash inside `options = {` in method args. Fix: add
///    LocalVariableWriteNode traversal.
///
/// 5. Ternary FN (jekyll-assets): hash inside `cond ? a : {...}` in method args. Fix: add
///    IfNode traversal for both if_true and if_false branches.
///
/// ## Corpus investigation (2026-03-29)
///
/// Remaining FN=7 came from two parenthesized-argument shapes that RuboCop's
/// `each_argument_node(..., :hash)` reaches through Parser AST, but Prism
/// exposes differently:
///
/// 1. `arg || { ... }` / `arg && { ... }` wrappers produce `OrNode` / `AndNode`
///    instead of a direct hash argument.
/// 2. `call_with_block { { ... } }` keeps the `BlockNode` on `CallNode.block()`
///    rather than wrapping the call in a standalone block AST node.
///
/// Fix: recurse through boolean wrapper nodes and inspect only the attached
/// block body for nested call arguments, while still skipping nested call
/// receivers and argument lists so outer-parenthesis indentation does not leak
/// into unrelated inner sends.
///
/// ## Variant divergence fix (2026-04-08)
///
/// `EnforcedStyle=consistent` still uses RuboCop's `parent_hash_key` indent
/// base for nested hash values in multi-pair hashes; nitrocop had disabled
/// that base for `consistent`, which produced large FN volume on shapes like
/// `options = { parse: { ... }, render: { ... } }`.
///
/// A smaller FP bucket came from separator-aligned hashes under
/// `Layout/HashAlignment`. RuboCop adds the longest-key offset when
/// `EnforcedColonStyle` / `EnforcedHashRocketStyle` is `separator`, even for
/// `FirstHashElementIndentation`. RuboCop only does that when the sibling
/// setting is exactly the string `separator`; mixed-style arrays like
/// `['key', 'separator']` do not trigger the offset here. Nitrocop now mirrors
/// that exact-string check while still inheriting the raw sibling config.
pub struct FirstHashElementIndentation;

impl Cop for FirstHashElementIndentation {
    fn name(&self) -> &'static str {
        "Layout/FirstHashElementIndentation"
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &crate::parse::codemap::CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let style = config.get_str("EnforcedStyle", "special_inside_parentheses");
        let width = config.get_usize("IndentationWidth", 2);
        let mut visitor = HashIndentVisitor {
            cop: self,
            source,
            style,
            width,
            colon_separator_style: config_style_matches(
                config,
                "EnforcedColonStyle",
                "separator",
                "key",
            ),
            hash_rocket_separator_style: config_style_matches(
                config,
                "EnforcedHashRocketStyle",
                "separator",
                "key",
            ),
            diagnostics: Vec::new(),
            handled_hashes: Vec::new(),
            parent_pair_col: None,
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct HashIndentVisitor<'a> {
    cop: &'a FirstHashElementIndentation,
    source: &'a SourceFile,
    style: &'a str,
    width: usize,
    colon_separator_style: bool,
    hash_rocket_separator_style: bool,
    diagnostics: Vec<Diagnostic>,
    /// Start offsets of hash nodes already checked via a parent call with parentheses.
    handled_hashes: Vec<usize>,
    /// When visiting a hash that is a value in a pair (AssocNode), this stores
    /// the pair's column and whether a right sibling begins on a subsequent line.
    parent_pair_col: Option<usize>,
}

#[derive(Clone, Copy)]
enum IndentBaseKind {
    StartOfLine,
    LeftBrace,
    FirstPositionAfterLeftParenthesis,
    ParentHashKey,
}

impl HashIndentVisitor<'_> {
    fn find_hash_args_in_body(
        &mut self,
        body: ruby_prism::Node<'_>,
        paren_line: usize,
        paren_col: usize,
    ) {
        if let Some(statements) = body.as_statements_node() {
            for stmt in statements.body().iter() {
                self.find_hash_args_in_call(&stmt, paren_line, paren_col);
            }
        } else {
            self.find_hash_args_in_call(&body, paren_line, paren_col);
        }
    }

    fn parent_pair_col_for_child_hash(
        &self,
        elements: &[ruby_prism::Node<'_>],
        index: usize,
        elem: &ruby_prism::Node<'_>,
    ) -> Option<usize> {
        if self.style == "align_braces" {
            return None;
        }

        let assoc = elem.as_assoc_node()?;
        let value = assoc.value();
        let hash = value
            .as_hash_node()
            .filter(|hash| hash.opening_loc().as_slice() == b"{")?;

        let (key_line, _) = self
            .source
            .offset_to_line_col(assoc.key().location().start_offset());
        let (val_line, _) = self
            .source
            .offset_to_line_col(hash.location().start_offset());
        if key_line != val_line {
            return None;
        }

        let next = elements.get(index + 1)?;
        let (pair_last_line, _) = self
            .source
            .offset_to_line_col(elem.location().end_offset().saturating_sub(1));
        let (sibling_line, _) = self
            .source
            .offset_to_line_col(next.location().start_offset());
        if pair_last_line >= sibling_line {
            return None;
        }

        Some(
            self.source
                .offset_to_line_col(elem.location().start_offset())
                .1,
        )
    }

    fn first_pair_offset(
        &self,
        hash_node: &ruby_prism::HashNode<'_>,
        first_pair: &ruby_prism::Node<'_>,
    ) -> usize {
        let Some(first_assoc) = first_pair.as_assoc_node() else {
            return 0;
        };

        let use_separator_style = match first_assoc.operator_loc().map(|loc| loc.as_slice()) {
            Some(b":") => self.colon_separator_style,
            Some(b"=>") => self.hash_rocket_separator_style,
            None => self.colon_separator_style,
            _ => false,
        };
        if !use_separator_style {
            return 0;
        }

        let first_key_width = char_width(first_assoc.key().location().as_slice());
        let max_key_width = hash_node
            .elements()
            .iter()
            .filter_map(|elem| elem.as_assoc_node())
            .map(|assoc| char_width(assoc.key().location().as_slice()))
            .max()
            .unwrap_or(first_key_width);

        max_key_width.saturating_sub(first_key_width)
    }

    fn find_hashes_in_elements(
        &mut self,
        elements: ruby_prism::NodeList<'_>,
        paren_line: usize,
        paren_col: usize,
    ) {
        let elems: Vec<_> = elements.iter().collect();
        for (i, elem) in elems.iter().enumerate() {
            let saved = self.parent_pair_col;
            self.parent_pair_col = self.parent_pair_col_for_child_hash(elems.as_slice(), i, elem);
            self.find_hash_args_in_call(elem, paren_line, paren_col);
            self.parent_pair_col = saved;
        }
    }

    fn indent_base(
        &self,
        opening_loc: ruby_prism::Location<'_>,
        left_paren_col: Option<usize>,
    ) -> (usize, IndentBaseKind) {
        let (open_line, open_col) = self.source.offset_to_line_col(opening_loc.start_offset());
        let open_line_bytes = self.source.lines().nth(open_line - 1).unwrap_or(b"");
        let open_line_indent = leading_whitespace_columns(open_line_bytes);

        match self.style {
            "align_braces" => (open_col, IndentBaseKind::LeftBrace),
            _ => {
                if let Some(pair_col) = self.parent_pair_col {
                    (pair_col, IndentBaseKind::ParentHashKey)
                } else if self.style == "special_inside_parentheses" {
                    if let Some(paren_col) = left_paren_col {
                        (
                            paren_col + 1,
                            IndentBaseKind::FirstPositionAfterLeftParenthesis,
                        )
                    } else {
                        (open_line_indent, IndentBaseKind::StartOfLine)
                    }
                } else {
                    (open_line_indent, IndentBaseKind::StartOfLine)
                }
            }
        }
    }

    fn right_brace_message(&self, base_kind: IndentBaseKind) -> &'static str {
        match base_kind {
            IndentBaseKind::LeftBrace => "Indent the right brace the same as the left brace.",
            IndentBaseKind::FirstPositionAfterLeftParenthesis => {
                "Indent the right brace the same as the first position after the preceding left parenthesis."
            }
            IndentBaseKind::ParentHashKey => {
                "Indent the right brace the same as the parent hash key."
            }
            IndentBaseKind::StartOfLine => {
                "Indent the right brace the same as the start of the line where the left brace is."
            }
        }
    }

    fn check_right_brace(
        &mut self,
        hash_node: &ruby_prism::HashNode<'_>,
        left_paren_col: Option<usize>,
    ) {
        let closing_loc = hash_node.closing_loc();
        if closing_loc.as_slice() != b"}" {
            return;
        }

        let (brace_line, brace_col) = self.source.offset_to_line_col(closing_loc.start_offset());
        let line_start = match self.source.line_col_to_offset(brace_line, 0) {
            Some(offset) => offset,
            None => return,
        };
        let brace_start = closing_loc.start_offset();
        let prefix = &self.source.as_bytes()[line_start..brace_start];

        // Match RuboCop: accept when the right brace shares a line with the
        // last value (there is non-whitespace before the brace).
        if prefix.iter().any(|b| !b.is_ascii_whitespace()) {
            return;
        }

        let (expected_col, base_kind) = self.indent_base(hash_node.opening_loc(), left_paren_col);
        if brace_col != expected_col {
            self.diagnostics.push(self.cop.diagnostic(
                self.source,
                brace_line,
                brace_col,
                self.right_brace_message(base_kind).to_string(),
            ));
        }
    }

    fn check_hash(&mut self, hash_node: &ruby_prism::HashNode<'_>, left_paren_col: Option<usize>) {
        let opening_loc = hash_node.opening_loc();
        if opening_loc.as_slice() != b"{" {
            return;
        }

        // Match RuboCop's `hash_node.pairs.first` — only consider AssocNode elements,
        // skipping AssocSplatNode (`**var`). RuboCop doesn't check indentation of splats.
        let first_pair = hash_node
            .elements()
            .iter()
            .find(|e| e.as_assoc_node().is_some());
        if let Some(first_element) = first_pair {
            let (open_line, _) = self.source.offset_to_line_col(opening_loc.start_offset());
            let first_loc = first_element.location();
            let (elem_line, elem_col) = self.source.offset_to_line_col(first_loc.start_offset());

            if elem_line == open_line {
                return;
            }

            let (base_indent, _) = self.indent_base(opening_loc, left_paren_col);
            let expected =
                base_indent + self.width + self.first_pair_offset(hash_node, &first_element);

            if elem_col != expected {
                self.diagnostics.push(self.cop.diagnostic(
                    self.source,
                    elem_line,
                    elem_col,
                    format!(
                        "Use {} (not {}) spaces for indentation of the first element.",
                        self.width,
                        elem_col.saturating_sub(base_indent)
                    ),
                ));
            }
        }

        self.check_right_brace(hash_node, left_paren_col);
    }

    fn find_hash_args_in_call(
        &mut self,
        node: &ruby_prism::Node<'_>,
        paren_line: usize,
        paren_col: usize,
    ) {
        if let Some(hash) = node.as_hash_node() {
            let opening_loc = hash.opening_loc();
            if opening_loc.as_slice() == b"{" {
                let (brace_line, _) = self.source.offset_to_line_col(opening_loc.start_offset());
                if brace_line == paren_line {
                    self.handled_hashes.push(hash.location().start_offset());
                    self.check_hash(&hash, Some(paren_col));
                }
            }
            let saved = self.parent_pair_col;
            self.parent_pair_col = None;
            self.find_hashes_in_elements(hash.elements(), paren_line, paren_col);
            self.parent_pair_col = saved;
            return;
        }

        if let Some(and_node) = node.as_and_node() {
            self.find_hash_args_in_call(&and_node.left(), paren_line, paren_col);
            self.find_hash_args_in_call(&and_node.right(), paren_line, paren_col);
            return;
        }

        if let Some(or_node) = node.as_or_node() {
            self.find_hash_args_in_call(&or_node.left(), paren_line, paren_col);
            self.find_hash_args_in_call(&or_node.right(), paren_line, paren_col);
            return;
        }

        if let Some(call) = node.as_call_node() {
            if let Some(block_node) = call.block().and_then(|block| block.as_block_node()) {
                if let Some(body) = block_node.body() {
                    self.find_hash_args_in_body(body, paren_line, paren_col);
                }
            }
            return;
        }

        if let Some(block_node) = node.as_block_node() {
            if let Some(body) = block_node.body() {
                self.find_hash_args_in_body(body, paren_line, paren_col);
            }
            return;
        }

        if let Some(kw_hash) = node.as_keyword_hash_node() {
            let saved = self.parent_pair_col;
            self.parent_pair_col = None;
            self.find_hashes_in_elements(kw_hash.elements(), paren_line, paren_col);
            self.parent_pair_col = saved;
            return;
        }

        if let Some(assoc) = node.as_assoc_node() {
            self.find_hash_args_in_call(&assoc.value(), paren_line, paren_col);
            return;
        }

        if let Some(splat) = node.as_splat_node() {
            if let Some(expr) = splat.expression() {
                self.find_hash_args_in_call(&expr, paren_line, paren_col);
            }
            return;
        }

        // Double-splat `**{...}` — traverse into the hash expression
        if let Some(assoc_splat) = node.as_assoc_splat_node() {
            if let Some(expr) = assoc_splat.value() {
                self.find_hash_args_in_call(&expr, paren_line, paren_col);
            }
            return;
        }

        if let Some(parens) = node.as_parentheses_node() {
            if let Some(body) = parens.body() {
                self.find_hash_args_in_call(&body, paren_line, paren_col);
            }
            return;
        }

        if let Some(array) = node.as_array_node() {
            for elem in array.elements().iter() {
                self.find_hash_args_in_call(&elem, paren_line, paren_col);
            }
            return;
        }

        // Local variable assignment in args: `options = {...}`
        if let Some(write) = node.as_local_variable_write_node() {
            self.find_hash_args_in_call(&write.value(), paren_line, paren_col);
            return;
        }

        // Ternary/if expression in args: `cond ? a : {...}`
        if let Some(if_node) = node.as_if_node() {
            if let Some(if_true) = if_node.statements() {
                for stmt in if_true.body().iter() {
                    self.find_hash_args_in_call(&stmt, paren_line, paren_col);
                }
            }
            if let Some(subsequent) = if_node.subsequent() {
                // The else branch can be an ElseNode — traverse into its statements
                if let Some(else_node) = subsequent.as_else_node() {
                    if let Some(stmts) = else_node.statements() {
                        for stmt in stmts.body().iter() {
                            self.find_hash_args_in_call(&stmt, paren_line, paren_col);
                        }
                    }
                }
            }
        }
    }
}

impl HashIndentVisitor<'_> {
    /// For each pair element whose value is a HashNode starting with `{`,
    /// check RuboCop's parent_hash_key indentation condition: if the pair's
    /// key and value start on the same line AND the pair has a right sibling
    /// on a subsequent line, set `parent_pair_col` so the child hash uses
    /// the pair's column as indent base.
    fn visit_pairs_with_hash_values(&mut self, elements: ruby_prism::NodeList<'_>) {
        let elems: Vec<_> = elements.iter().collect();
        for (i, elem) in elems.iter().enumerate() {
            let saved = self.parent_pair_col;
            self.parent_pair_col = self.parent_pair_col_for_child_hash(elems.as_slice(), i, elem);
            self.visit(elem);
            self.parent_pair_col = saved;
        }
    }
}

impl Visit<'_> for HashIndentVisitor<'_> {
    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'_>) {
        if let Some(open_paren_loc) = node.opening_loc() {
            if open_paren_loc.as_slice() == b"(" {
                let (paren_line, paren_col) = self
                    .source
                    .offset_to_line_col(open_paren_loc.start_offset());
                if let Some(args) = node.arguments() {
                    for arg in args.arguments().iter() {
                        self.find_hash_args_in_call(&arg, paren_line, paren_col);
                    }
                }
            }
        }
        ruby_prism::visit_call_node(self, node);
    }

    fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'_>) {
        let offset = node.location().start_offset();
        if !self.handled_hashes.contains(&offset) {
            self.check_hash(node, None);
        }
        // Clear parent_pair_col after the immediate hash uses it,
        // so that nested hashes inside this one don't inherit it.
        let saved = self.parent_pair_col;
        self.parent_pair_col = None;
        // Before visiting children, check if any element is a pair whose value
        // is a hash. If so, set parent_pair_col for that child hash.
        self.visit_pairs_with_hash_values(node.elements());
        self.parent_pair_col = saved;
    }

    fn visit_keyword_hash_node(&mut self, node: &ruby_prism::KeywordHashNode<'_>) {
        // keyword hashes can also contain pairs whose values are hashes
        self.visit_pairs_with_hash_values(node.elements());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(
        FirstHashElementIndentation,
        "cops/layout/first_hash_element_indentation"
    );

    #[test]
    fn same_line_elements_ignored() {
        let source = b"x = { a: 1, b: 2 }\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn align_braces_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("align_braces".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"x = {\n      a: 1\n    }\n";
        let diags = run_cop_full_with_config(&FirstHashElementIndentation, src, config.clone());
        assert!(
            diags.is_empty(),
            "align_braces should accept element at brace column + width"
        );

        let src2 = b"x = {\n  a: 1\n    }\n";
        let diags2 = run_cop_full_with_config(&FirstHashElementIndentation, src2, config);
        assert_eq!(
            diags2.len(),
            1,
            "align_braces should flag element not at brace column + width"
        );
    }

    #[test]
    fn align_braces_flags_right_brace() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("align_braces".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"x = {\n      a: 1\n}\n";
        let diags = run_cop_full_with_config(&FirstHashElementIndentation, src, config);
        assert_eq!(
            diags.len(),
            1,
            "align_braces should flag misaligned right brace"
        );
    }

    #[test]
    fn special_inside_parentheses_method_call() {
        let source = b"func({\n       a: 1\n     })\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert!(
            diags.is_empty(),
            "should accept special indentation inside parentheses"
        );
    }

    #[test]
    fn special_inside_parentheses_flags_consistent_indent() {
        let source = b"func({\n  a: 1\n     })\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert_eq!(
            diags.len(),
            1,
            "should flag consistent indentation inside parentheses"
        );
    }

    #[test]
    fn special_inside_parentheses_flags_right_brace() {
        let source = b"func({\n       a: 1\n})\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert_eq!(
            diags.len(),
            1,
            "should flag right brace indentation inside parentheses"
        );
    }

    #[test]
    fn special_inside_parentheses_with_second_arg() {
        let source = b"func(x, {\n       a: 1\n     })\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert!(
            diags.is_empty(),
            "should accept special indentation for second hash arg"
        );
    }

    #[test]
    fn brace_not_on_same_line_as_paren_uses_line_indent() {
        let source = b"func(\n  {\n    a: 1\n  }\n)\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert!(
            diags.is_empty(),
            "brace on different line from paren should use line indent"
        );
    }

    #[test]
    fn safe_navigation_with_hash_arg() {
        let source = b"receiver&.func({\n                 a: 1\n               })\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert!(
            diags.is_empty(),
            "should handle safe navigation with hash arg"
        );
    }

    #[test]
    fn index_assignment_not_treated_as_paren() {
        let source = b"    config['key'] = {\n      val: 1\n    }\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert!(
            diags.is_empty(),
            "index assignment should not use paren context"
        );
    }

    #[test]
    fn nested_hash_in_keyword_arg() {
        let source = b"Config.new('Key' => {\n             val: 1\n           })\n";
        let diags = run_cop_full(&FirstHashElementIndentation, source);
        assert!(
            diags.is_empty(),
            "nested hash in keyword arg should use paren context"
        );
    }

    fn consistent_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("consistent".into()),
            )]),
            ..CopConfig::default()
        }
    }

    fn consistent_separator_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("consistent".into()),
                ),
                (
                    "EnforcedColonStyle".into(),
                    serde_yml::Value::String("separator".into()),
                ),
                (
                    "EnforcedHashRocketStyle".into(),
                    serde_yml::Value::String("key".into()),
                ),
            ]),
            ..CopConfig::default()
        }
    }

    fn consistent_separator_sequence_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([
                (
                    "EnforcedStyle".into(),
                    serde_yml::Value::String("consistent".into()),
                ),
                (
                    "EnforcedColonStyle".into(),
                    serde_yml::Value::Sequence(vec![
                        serde_yml::Value::String("key".into()),
                        serde_yml::Value::String("separator".into()),
                    ]),
                ),
                (
                    "EnforcedHashRocketStyle".into(),
                    serde_yml::Value::String("key".into()),
                ),
            ]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn offense_consistent_fixture() {
        use crate::testutil::assert_cop_offenses_full_with_config;
        assert_cop_offenses_full_with_config(
            &FirstHashElementIndentation,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/first_hash_element_indentation/offense.consistent.rb"
            ),
            consistent_config(),
        );
    }

    #[test]
    fn no_offense_consistent_fixture() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        assert_cop_no_offenses_full_with_config(
            &FirstHashElementIndentation,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/first_hash_element_indentation/no_offense.consistent.rb"
            ),
            consistent_config(),
        );
    }

    #[test]
    fn no_offense_consistent_separator_fixture() {
        use crate::testutil::assert_cop_no_offenses_full_with_config;
        assert_cop_no_offenses_full_with_config(
            &FirstHashElementIndentation,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/first_hash_element_indentation/no_offense.consistent.separator.rb"
            ),
            consistent_separator_config(),
        );
    }

    #[test]
    fn mixed_separator_style_array_does_not_add_first_pair_offset() {
        use crate::testutil::run_cop_full_with_config;

        let source = include_bytes!(
            "../../../tests/fixtures/cops/layout/first_hash_element_indentation/no_offense.consistent.separator.rb"
        );
        let diags = run_cop_full_with_config(
            &FirstHashElementIndentation,
            source,
            consistent_separator_sequence_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "RuboCop ignores mixed-style arrays here and still expects consistent indentation"
        );
    }
}
