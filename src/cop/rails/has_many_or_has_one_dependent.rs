use crate::cop::shared::util::keyword_arg_value;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Rails/HasManyOrHasOneDependent -- checks `has_many` and `has_one` associations
/// for a missing `:dependent` option.
///
/// ## Root causes of historical FNs (76 total, 0 FPs):
///
/// 1. **Only checked inside ClassNode bodies** -- the cop used `class_body_calls()`
///    which only found associations directly inside `class ... end` bodies. All 76
///    FNs were associations inside `included do ... end` blocks in concern modules
///    (e.g., `app/models/concerns/*.rb`). Fixed by switching to a visitor-based
///    approach using `check_source` that walks the entire AST, matching
///    `has_many`/`has_one` calls anywhere in the file.
///
/// 2. **Missing `with_options` block support** -- associations inside
///    `with_options dependent: :destroy do ... end` blocks were not recognized.
///    Fixed by tracking a stack of `with_options` contexts during AST traversal.
///
/// 3. **Missing `through: nil` handling** -- `through: nil` should NOT suppress
///    the offense (only non-nil `through:` values suppress). Fixed by checking
///    the value node for NilNode.
///
/// 4. **Missing `readonly?` method check** -- classes with `def readonly?; true; end`
///    should suppress all association offenses. Fixed by scanning class/module
///    bodies for the readonly pattern.
///
/// 5. **Missing `ActiveResource::Base` check** -- subclasses of `ActiveResource::Base`
///    should not be flagged. Fixed by checking the superclass constant path.
///
/// 6. **Missing receiver support** -- `base.has_many :foo` (calls with a receiver)
///    inside modules should also be checked. Fixed by not requiring receiver to
///    be absent for association detection.
///
/// ## Corpus investigation (2026-03-19)
///
/// Corpus oracle reported FP=0, FN=1. The FN was in `nesquena__rabl__50ebc12`
/// at `fixtures/ashared/models/user.rb:2` — `has_many :phone_numbers` inside
/// `class User < ActiveRecord::Base`.
///
/// Root cause: The rabl repo has 7 symlinks (`fixtures/rails{2..6}/app/models`
/// → `../../ashared/models`). RuboCop follows symlinks via `Dir.glob`, so it
/// discovers `fixtures/rails4/app/models/user.rb` which matches the Include
/// pattern `**/app/models/**/*.rb`. But nitrocop's `ignore::WalkBuilder`
/// defaulted to `follow_links(false)`, only finding the canonical path
/// `fixtures/ashared/models/user.rb` which doesn't match Include.
///
/// Fixed by enabling `follow_links(true)` in `src/fs.rs` WalkBuilder.
///
/// ## Extended corpus investigation (2026-03-23)
///
/// Extended corpus reported FP=0, FN=4 across 3 repos. Three root causes:
///
/// 7. **Association extension block with extra args inside `with_options`** --
///    RuboCop's `association_extension_block?` pattern only matches
///    `(block (send nil? :has_many _) ...)` i.e. exactly one argument.
///    When extra keyword args are present (e.g., `has_many :foo, class_name: 'X' do...end`),
///    the pattern fails and RuboCop does not walk up to find the enclosing
///    `with_options`. Fixed by checking if the association call has both a block
///    and extra arguments; if so, `with_options` context is not applied.
///    (akicho8__shogi-extend, 2 FNs)
///
/// 8. **Explicit hash literal before lambda** -- `has_many :photos, {dependent: :destroy},
///    -> { order(...) }` places a HashNode in a non-last argument position. RuboCop's
///    `association_with_options?` pattern requires `(hash ...)` as the last child.
///    Fixed by only checking explicit HashNode arguments when they are the last
///    argument in the call. (jamesknelson__memamug-server, 1 FN)
///
/// 9. **Intermediate blocks break `with_options` context** -- `has_many` inside
///    an `.each do...end` block within a `with_options` block. RuboCop only walks
///    up through `begin` nodes and recognized association extension blocks, not
///    through arbitrary block boundaries. Fixed by clearing the `with_options`
///    stack when entering non-association, non-with_options block bodies.
///    (spree__spree_multi_vendor, 1 FN)
pub struct HasManyOrHasOneDependent;

impl Cop for HasManyOrHasOneDependent {
    fn name(&self) -> &'static str {
        "Rails/HasManyOrHasOneDependent"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_include(&self) -> &'static [&'static str] {
        &["**/app/models/**/*.rb"]
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        _config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        let mut visitor = DependentVisitor {
            cop: self,
            source,
            with_options_stack: Vec::new(),
            diagnostics: Vec::new(),
        };
        visitor.visit(&parse_result.node());
        diagnostics.extend(visitor.diagnostics);
    }
}

struct DependentVisitor<'a> {
    cop: &'a HasManyOrHasOneDependent,
    source: &'a SourceFile,
    /// Stack of with_options contexts tracking whether dependent/through is provided.
    with_options_stack: Vec<WithOptionsContext>,
    diagnostics: Vec<Diagnostic>,
}

struct WithOptionsContext {
    has_dependent: bool,
    has_through: bool,
}

impl DependentVisitor<'_> {
    /// Check if a keyword arg exists and its value is not nil.
    /// Matches RuboCop's `(pair (sym :key) !nil)` pattern.
    fn has_keyword_arg_not_nil(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> bool {
        match keyword_arg_value(call, key) {
            Some(val) => val.as_nil_node().is_none(),
            None => false,
        }
    }

    /// Check if a keyword arg exists (regardless of value, including nil).
    fn has_keyword_arg_any(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> bool {
        keyword_arg_value(call, key).is_some()
    }

    /// Check if the call has `dependent:` via double splat hash literal like `**{dependent: :destroy}`.
    fn has_dependent_in_kwsplat(call: &ruby_prism::CallNode<'_>) -> bool {
        let Some(args) = call.arguments() else {
            return false;
        };
        for arg in args.arguments().iter() {
            if let Some(kw) = arg.as_keyword_hash_node() {
                for elem in kw.elements().iter() {
                    if let Some(splat) = elem.as_assoc_splat_node() {
                        if let Some(value) = splat.value() {
                            if let Some(hash) = value.as_hash_node() {
                                for pair in hash.elements().iter() {
                                    if let Some(assoc) = pair.as_assoc_node() {
                                        if let Some(sym) = assoc.key().as_symbol_node() {
                                            if sym.unescaped() == b"dependent" {
                                                return true;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }

    fn extract_with_options_context(call: &ruby_prism::CallNode<'_>) -> WithOptionsContext {
        WithOptionsContext {
            has_dependent: Self::has_keyword_arg_any(call, b"dependent"),
            has_through: Self::has_keyword_arg_not_nil(call, b"through"),
        }
    }

    fn is_association_call(node: &ruby_prism::CallNode<'_>) -> bool {
        let name = node.name().as_slice();
        name == b"has_many" || name == b"has_one"
    }

    /// Check if a class node extends ActiveResource::Base.
    fn is_active_resource_class(class_node: &ruby_prism::ClassNode<'_>) -> bool {
        let Some(superclass) = class_node.superclass() else {
            return false;
        };
        // Match `ActiveResource::Base` or `::ActiveResource::Base`
        if let Some(const_path) = superclass.as_constant_path_node() {
            if let Some(name) = const_path.name() {
                if name.as_slice() != b"Base" {
                    return false;
                }
            }
            if let Some(parent) = const_path.parent() {
                // ActiveResource (ConstantReadNode) or ::ActiveResource (ConstantPathNode)
                if let Some(cr) = parent.as_constant_read_node() {
                    return cr.name().as_slice() == b"ActiveResource";
                }
                if let Some(cp) = parent.as_constant_path_node() {
                    // ::ActiveResource -- parent is None (cbase), name is ActiveResource
                    if cp.parent().is_none() {
                        if let Some(name) = cp.name() {
                            return name.as_slice() == b"ActiveResource";
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if a body contains `def readonly?; true; end`.
    fn has_readonly_true(body: &ruby_prism::Node<'_>) -> bool {
        let stmts = if let Some(s) = body.as_statements_node() {
            s
        } else {
            return false;
        };
        for stmt in stmts.body().iter() {
            if let Some(def_node) = stmt.as_def_node() {
                if def_node.name().as_slice() == b"readonly?" {
                    // Check that the body is just `true`
                    if let Some(def_body) = def_node.body() {
                        if let Some(def_stmts) = def_body.as_statements_node() {
                            let body_stmts: Vec<_> = def_stmts.body().iter().collect();
                            if body_stmts.len() == 1 && body_stmts[0].as_true_node().is_some() {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Check if the association call has more than one argument (i.e., has keyword
    /// options or extra positional args beyond the association name).
    fn has_extra_args(call: &ruby_prism::CallNode<'_>) -> bool {
        let Some(args) = call.arguments() else {
            return false;
        };
        args.arguments().len() > 1
    }

    /// Check if a HashNode argument is the last argument in the call.
    /// Explicit hash literals like `{dependent: :destroy}` that are NOT the last
    /// argument should not be treated as the options hash (matching RuboCop behavior).
    fn has_keyword_arg_not_nil_strict(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> bool {
        let Some(args) = call.arguments() else {
            return false;
        };
        let args_list: Vec<_> = args.arguments().iter().collect();
        let last_idx = args_list.len().saturating_sub(1);
        for (i, arg) in args_list.iter().enumerate() {
            // Direct keyword hash pairs in arguments (always valid)
            if let Some(kw) = arg.as_keyword_hash_node() {
                for elem in kw.elements().iter() {
                    if let Some(assoc) = elem.as_assoc_node() {
                        if let Some(sym) = assoc.key().as_symbol_node() {
                            if sym.unescaped() == key && assoc.value().as_nil_node().is_none() {
                                return true;
                            }
                        }
                    }
                }
            }
            // Hash literal -- only check if it's the last argument
            if i == last_idx {
                if let Some(hash) = arg.as_hash_node() {
                    for elem in hash.elements().iter() {
                        if let Some(assoc) = elem.as_assoc_node() {
                            if let Some(sym) = assoc.key().as_symbol_node() {
                                if sym.unescaped() == key && assoc.value().as_nil_node().is_none() {
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

    fn has_keyword_arg_any_strict(call: &ruby_prism::CallNode<'_>, key: &[u8]) -> bool {
        let Some(args) = call.arguments() else {
            return false;
        };
        let args_list: Vec<_> = args.arguments().iter().collect();
        let last_idx = args_list.len().saturating_sub(1);
        for (i, arg) in args_list.iter().enumerate() {
            if let Some(kw) = arg.as_keyword_hash_node() {
                for elem in kw.elements().iter() {
                    if let Some(assoc) = elem.as_assoc_node() {
                        if let Some(sym) = assoc.key().as_symbol_node() {
                            if sym.unescaped() == key {
                                return true;
                            }
                        }
                    }
                }
            }
            // Hash literal -- only check if it's the last argument
            if i == last_idx {
                if let Some(hash) = arg.as_hash_node() {
                    for elem in hash.elements().iter() {
                        if let Some(assoc) = elem.as_assoc_node() {
                            if let Some(sym) = assoc.key().as_symbol_node() {
                                if sym.unescaped() == key {
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

    fn check_association(&mut self, call: &ruby_prism::CallNode<'_>) {
        // Skip if :through is specified with non-nil value (directly or via with_options)
        let has_through = Self::has_keyword_arg_not_nil_strict(call, b"through")
            || self.with_options_stack.iter().any(|ctx| ctx.has_through);
        if has_through {
            return;
        }

        // Skip if :dependent is specified (directly or via **{dependent: ...})
        let has_dependent = Self::has_keyword_arg_any_strict(call, b"dependent")
            || Self::has_dependent_in_kwsplat(call);

        // with_options context only applies when the association call does NOT have
        // a block with extra arguments. RuboCop's association_extension_block? pattern
        // only matches `has_many :name` (exactly one arg), so when extra args are
        // present with a block, RuboCop fails to walk up to with_options.
        let with_options_applies = !(call.block().is_some() && Self::has_extra_args(call));

        let has_dependent = has_dependent
            || (with_options_applies
                && self.with_options_stack.iter().any(|ctx| ctx.has_dependent));
        if has_dependent {
            return;
        }

        // Skip dynamic kwsplat (**opts) -- may contain dependent
        // RuboCop flags these, but we match RuboCop behavior: flag even with **var
        // Actually looking at the spec: "registers an offense when a variable passed with double splat"
        // So we DO flag **var, we only skip **{hash literal with dependent}

        let loc = call.message_loc().unwrap_or(call.location());
        let (line, column) = self.source.offset_to_line_col(loc.start_offset());
        self.diagnostics.push(self.cop.diagnostic(
            self.source,
            line,
            column,
            "Specify a `:dependent` option.".to_string(),
        ));
    }
}

impl<'pr> Visit<'pr> for DependentVisitor<'_> {
    fn visit_class_node(&mut self, node: &ruby_prism::ClassNode<'pr>) {
        // Skip ActiveResource::Base subclasses entirely
        if Self::is_active_resource_class(node) {
            return;
        }

        // Check for readonly? method returning true -- suppresses all offenses in this class
        if let Some(body) = node.body() {
            if Self::has_readonly_true(&body) {
                return;
            }
        }

        // Continue default traversal into class body
        if let Some(body) = node.body() {
            self.visit(&body);
        }
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        // Check if this is a with_options block call
        if node.receiver().is_none()
            && node.name().as_slice() == b"with_options"
            && node.block().is_some()
        {
            let ctx = Self::extract_with_options_context(node);
            self.with_options_stack.push(ctx);

            // Visit children (the block body) with the context pushed
            if let Some(block) = node.block() {
                if let Some(block_node) = block.as_block_node() {
                    if let Some(body) = block_node.body() {
                        self.visit(&body);
                    }
                }
            }

            self.with_options_stack.pop();
            return;
        }

        // Check if this is an association call (with or without receiver)
        if Self::is_association_call(node) {
            self.check_association(node);
        }

        // Continue visiting children (e.g., `included do ... end` blocks)
        if let Some(args) = node.arguments() {
            self.visit(&args.as_node());
        }
        if let Some(block) = node.block() {
            // Non-with_options blocks (e.g., `each do...end`, `tap do...end`)
            // break the with_options context chain. RuboCop only walks up
            // through begin nodes and association extension blocks, not through
            // arbitrary block boundaries. Save and clear the stack for the block body.
            if !self.with_options_stack.is_empty() && !Self::is_association_call(node) {
                let saved = std::mem::take(&mut self.with_options_stack);
                self.visit(&block);
                self.with_options_stack = saved;
            } else {
                self.visit(&block);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(
        HasManyOrHasOneDependent,
        "cops/rails/has_many_or_has_one_dependent"
    );
}
