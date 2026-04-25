use ruby_prism::Visit;

use crate::cop::shared::node_type::{NODE_TYPE_COUNT, node_type_tag};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

pub struct CopWalker<'a, 'pr> {
    pub cop: &'a dyn Cop,
    pub source: &'a SourceFile,
    pub parse_result: &'a ruby_prism::ParseResult<'pr>,
    pub cop_config: &'a CopConfig,
    pub diagnostics: Vec<Diagnostic>,
    pub corrections: Option<Vec<crate::correction::Correction>>,
}

impl<'pr> Visit<'pr> for CopWalker<'_, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.cop.check_node(
            self.source,
            &node,
            self.parse_result,
            self.cop_config,
            &mut self.diagnostics,
            self.corrections.as_mut(),
        );
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.cop.check_node(
            self.source,
            &node,
            self.parse_result,
            self.cop_config,
            &mut self.diagnostics,
            self.corrections.as_mut(),
        );
    }
}

/// Walks the AST once and dispatches each node only to cops that declared
/// interest in that node type. Cops that haven't declared interest (empty
/// `interested_node_types()`) are called for every node (universal dispatch).
///
/// Each entry in `cops` is `(registry_index, cop, config)`. The registry index
/// is stamped onto every correction the cop produces so `CorrectionSet` can
/// resolve same-offset ties deterministically by registration order. This
/// avoids touching every per-cop `Correction { cop_index: 0, .. }` site —
/// the walker overwrites the field after dispatch.
pub struct BatchedCopWalker<'a, 'pr> {
    /// Cops that haven't declared node type interest — called for every node.
    universal_cops: Vec<(usize, &'a dyn Cop, &'a CopConfig)>,
    /// Dispatch table: indexed by node type tag, each entry = cops for that type.
    dispatch_table: [Vec<(usize, &'a dyn Cop, &'a CopConfig)>; NODE_TYPE_COUNT],
    pub source: &'a SourceFile,
    pub parse_result: &'a ruby_prism::ParseResult<'pr>,
    pub diagnostics: Vec<Diagnostic>,
    corrections: Option<Vec<crate::correction::Correction>>,
}

impl<'a, 'pr> BatchedCopWalker<'a, 'pr> {
    pub fn new(
        cops: Vec<(usize, &'a dyn Cop, &'a CopConfig)>,
        source: &'a SourceFile,
        parse_result: &'a ruby_prism::ParseResult<'pr>,
    ) -> Self {
        let mut universal = Vec::new();
        let mut table: [Vec<(usize, &'a dyn Cop, &'a CopConfig)>; NODE_TYPE_COUNT] =
            std::array::from_fn(|_| Vec::new());

        for (idx, cop, config) in cops {
            let types = cop.interested_node_types();
            if types.is_empty() {
                universal.push((idx, cop, config));
            } else {
                for &t in types {
                    table[t as usize].push((idx, cop, config));
                }
            }
        }

        Self {
            universal_cops: universal,
            dispatch_table: table,
            source,
            parse_result,
            diagnostics: Vec::new(),
            corrections: None,
        }
    }

    /// Enable corrections collection for this walker.
    pub fn with_corrections(mut self) -> Self {
        self.corrections = Some(Vec::new());
        self
    }

    /// Consume the walker and return (diagnostics, corrections).
    pub fn into_results(self) -> (Vec<Diagnostic>, Option<Vec<crate::correction::Correction>>) {
        (self.diagnostics, self.corrections)
    }

    /// Run a cop's `check_node` and stamp `cop_index` onto any corrections it
    /// pushed. Most cops hardcode `cop_index: 0`; this overwrite is the source
    /// of truth for tiebreaking in `CorrectionSet`.
    #[inline]
    fn run_with_index(
        &mut self,
        cop_index: usize,
        cop: &dyn Cop,
        cop_config: &CopConfig,
        node: &ruby_prism::Node<'pr>,
    ) {
        let pre_len = self.corrections.as_ref().map_or(0, |c| c.len());
        cop.check_node(
            self.source,
            node,
            self.parse_result,
            cop_config,
            &mut self.diagnostics,
            self.corrections.as_mut(),
        );
        if let Some(corr) = self.corrections.as_mut() {
            for c in &mut corr[pre_len..] {
                c.cop_index = cop_index;
            }
        }
    }

    #[inline]
    fn dispatch(&mut self, node: &ruby_prism::Node<'pr>) {
        let tag = node_type_tag(node) as usize;

        // Iterate by index to avoid borrowing self immutably while we also
        // need &mut self in run_with_index.
        for i in 0..self.universal_cops.len() {
            let (idx, cop, cop_config) = self.universal_cops[i];
            self.run_with_index(idx, cop, cop_config, node);
        }

        if tag < NODE_TYPE_COUNT {
            for i in 0..self.dispatch_table[tag].len() {
                let (idx, cop, cop_config) = self.dispatch_table[tag][i];
                self.run_with_index(idx, cop, cop_config, node);
            }
        }
    }
}

impl<'pr> Visit<'pr> for BatchedCopWalker<'_, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.dispatch(&node);
    }

    fn visit_leaf_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.dispatch(&node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correction::Correction;
    use std::path::PathBuf;

    /// Fake cop that pushes a single dummy correction with `cop_index: 0` per
    /// node it sees. Lets us assert the walker overwrites `cop_index` to the
    /// registry index it was constructed with.
    struct StampCop {
        name: &'static str,
    }

    impl Cop for StampCop {
        fn name(&self) -> &'static str {
            self.name
        }

        fn supports_autocorrect(&self) -> bool {
            true
        }

        fn check_node(
            &self,
            _source: &SourceFile,
            node: &ruby_prism::Node<'_>,
            _parse_result: &ruby_prism::ParseResult<'_>,
            _config: &CopConfig,
            _diagnostics: &mut Vec<crate::diagnostic::Diagnostic>,
            corrections: Option<&mut Vec<Correction>>,
        ) {
            // Only fire on the program node so we get one correction per cop.
            if node.as_program_node().is_none() {
                return;
            }
            if let Some(corr) = corrections {
                corr.push(Correction {
                    start: 0,
                    end: 0,
                    replacement: String::new(),
                    cop_name: self.name,
                    cop_index: 0,
                });
            }
        }
    }

    #[test]
    fn dispatch_stamps_cop_index_onto_corrections() {
        let cop_a = StampCop { name: "Test/A" };
        let cop_b = StampCop { name: "Test/B" };
        let config = CopConfig::default();

        let source = SourceFile::from_string(PathBuf::from("test.rb"), String::from("x = 1\n"));
        let parse_result = crate::parse::parse_source(source.as_bytes());

        let ast_cops: Vec<(usize, &dyn Cop, &CopConfig)> = vec![
            // Use distinctive non-zero indices so we can tell whether the
            // walker actually wrote them.
            (7, &cop_a, &config),
            (42, &cop_b, &config),
        ];
        let mut walker = BatchedCopWalker::new(ast_cops, &source, &parse_result).with_corrections();
        walker.visit(&parse_result.node());

        let (_diags, corrections) = walker.into_results();
        let corrections = corrections.expect("with_corrections enabled");

        let by_name: std::collections::HashMap<&str, usize> = corrections
            .iter()
            .map(|c| (c.cop_name, c.cop_index))
            .collect();
        assert_eq!(by_name.get("Test/A"), Some(&7));
        assert_eq!(by_name.get("Test/B"), Some(&42));
    }
}
