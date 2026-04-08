use crate::cop::shared::node_type::CALL_NODE;
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;

/// Layout/ArgumentAlignment cop.
///
/// ## Investigation findings (2026-03-14)
///
/// **FP root cause — `**splat` with aligned continuation kwargs:**
/// When a KeywordHashNode is expanded for alignment checking, `AssocSplatNode`
/// elements must be excluded from both the alignment items and the minimum-count
/// check. RuboCop's `first_arg.pairs` returns only `pair` nodes (not `kwsplat`),
/// and `multiple_arguments?` checks `pairs.count >= 2`. So `**splat` + 1 keyword
/// pair → skip (not enough args). `**splat` + 2+ keyword pairs → check pairs only,
/// using the first pair as the alignment reference (not the splat).
///
/// **FN root cause — block args (`&block`, `&handler`):**
/// In Prism, block arguments (`&block`) are stored on `call_node.block()` as a
/// `BlockArgumentNode`, NOT in `call_node.arguments()`. The cop was only iterating
/// `arguments()`, so block args were invisible to alignment checking. Fix: append
/// the block argument to the effective args list when present.
///
/// ## Investigation findings (2026-03-16)
///
/// **FP root cause — incorrect keyword hash expansion in `with_first_argument` mode:**
/// When multiple arguments exist and the last one is a `KeywordHashNode`, nitrocop
/// was ALWAYS expanding the hash's elements into the effective_args list. But
/// RuboCop's `arguments_or_first_arg_pairs` (used for `with_first_argument` style)
/// only expands when the FIRST argument is a bare keyword hash (sole arg case).
/// For multi-arg calls, it returns `node.arguments` as-is, keeping the
/// KeywordHashNode as a single item. This caused ~19k false positives because
/// continuation keyword pairs (e.g., `branch: "v349"` on line 2 of a `gem` call)
/// were individually checked against the first positional arg's column.
///
/// Fix: only expand the last KeywordHashNode for `with_fixed_indentation` style
/// (which uses `arguments_with_last_arg_pairs`). For `with_first_argument`, keep
/// the kwHash as a single item so its internal pairs aren't individually checked.
///
/// **Previous FN note (2026-03-14) revised:** The earlier finding about expanding
/// keyword hash elements in multi-arg calls was only correct for
/// `with_fixed_indentation` style. For `with_first_argument` (default), expansion
/// was causing massive FPs. The fix keeps expansion only for fixed indentation.
///
/// ## Investigation findings (2026-03-16, second pass)
///
/// **Interim fix — sole keyword hash + block pass:**
/// An earlier revision reduced false positives by preventing a trailing
/// `BlockArgumentNode` from turning a sole keyword-hash call into a checked
/// multi-argument call. A later investigation (2026-03-30 below) refined this
/// to match Parser more closely by treating block pass as part of
/// `node.arguments` first and then applying RuboCop's flattening rules.
///
/// ## Investigation findings (2026-03-30)
///
/// **FP+FN root cause — block-pass parity must match Parser's `node.arguments`:**
/// RuboCop's `node.arguments` includes block-pass arguments for normal calls
/// like `capture(fields, &block)`, but `with_first_argument` still drops the
/// block pass when the first argument is a bare keyword hash by flattening to
/// `first_arg.pairs` only. Our previous "append block after the len gate"
/// shortcut fixed `render(layout: ..., &block)` false positives, but it also
/// hid real offenses in `capture(fields, &block)`. The fix is to build a
/// parser-style argument list that already includes `BlockArgumentNode`, then
/// apply RuboCop's flattening rules to that list instead of special-casing the
/// block pass afterward.
///
/// ## Investigation findings (2026-04-01)
///
/// **FP root cause — display width vs. codepoint count in interpolated strings:**
/// RuboCop uses `Unicode::DisplayWidth` for the base column in
/// `with_first_argument` mode. Our previous implementation used
/// `offset_to_line_col()`, which counts UTF-8 codepoints, so a wide character
/// such as `🌊`, `🌌`, or fullwidth text before `#{...}` made the first
/// argument's expected column too small. The continued argument line contained
/// only spaces, so its visual column matched RuboCop but nitrocop still
/// reported a false positive. Fix: compute the first-argument base column from
/// the display width of the line prefix, while keeping the rest of the
/// alignment logic unchanged.
///
/// ## Investigation findings (2026-04-08)
///
/// **FN root cause — `with_fixed_indentation` first-arg skipping:**
/// The alignment loop used `skip(1)` to always skip the first effective
/// argument. For `with_first_argument` this is correct (the first arg
/// defines the expected column), but for `with_fixed_indentation` the
/// expected column is `call_indentation + indent_width`, so the first
/// continuation-line argument CAN be misaligned. RuboCop's
/// `each_bad_alignment` starts with `prev_line = -1` and checks ALL items.
/// Fix: for `with_fixed_indentation`, iterate all effective args and start
/// with an empty `checked_lines` set.
///
/// **FP root cause — `indentation_of` ignoring tabs:**
/// `indentation_of()` only counted leading spaces. RuboCop uses
/// `/\S.*/`.begin(0)` which counts all leading whitespace bytes (including
/// tabs). Fix: count both spaces and tabs when computing the base
/// indentation for `with_fixed_indentation`.
///
/// **Message text:** `with_fixed_indentation` uses a different message
/// ("Use one level of indentation for arguments following the first line
/// of a multi-line method call.") than the default `with_first_argument`
/// ("Align the arguments of a method call if they span more than one
/// line.").
pub struct ArgumentAlignment;

impl Cop for ArgumentAlignment {
    fn name(&self) -> &'static str {
        "Layout/ArgumentAlignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE]
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
        let style = config.get_str("EnforcedStyle", "with_first_argument");
        let indent_width = config.get_usize("IndentationWidth", 2);
        let call_node = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        // RuboCop skips []= calls (bracket assignment)
        if call_node.name().as_slice() == b"[]=" {
            return;
        }

        let arguments = match call_node.arguments() {
            Some(args) => args,
            None => return,
        };

        let arg_list = arguments.arguments();
        if arg_list.is_empty() && call_node.block().is_none() {
            return;
        }

        // Build a Parser-style argument list. In RuboCop/Parser, block-pass
        // arguments (`&block`) are part of `node.arguments`; in Prism they live
        // on `call.block()`.
        let mut args_vec: Vec<ruby_prism::Node<'_>> = arg_list.iter().collect();
        if let Some(block) = call_node.block() {
            if block.as_block_argument_node().is_some() {
                args_vec.push(block);
            }
        }

        if args_vec.is_empty() {
            return;
        }

        // Collect effective arguments, matching RuboCop's behavior:
        //
        // with_first_argument style (arguments_or_first_arg_pairs):
        //   - If the first arg is a bare KeywordHashNode, expand to its .pairs
        //     only (excludes AssocSplatNode). This also drops any trailing
        //     block-pass arg, matching Parser's `first_arg.pairs` behavior.
        //   - Otherwise, use node.arguments as-is (including block pass).
        //
        // with_fixed_indentation style (arguments_with_last_arg_pairs):
        //   - All args except last, plus last arg's KeywordHashNode .pairs
        //     (or the last arg itself if not a keyword hash).

        let mut effective_args: Vec<ruby_prism::Node<'_>> = Vec::new();

        if style != "with_fixed_indentation" && args_vec[0].as_keyword_hash_node().is_some() {
            // with_first_argument: expand first arg's pairs only
            let kw_hash = args_vec[0].as_keyword_hash_node().unwrap();
            for elem in kw_hash.elements().iter() {
                // Only include AssocNode (pair), skip AssocSplatNode
                if elem.as_assoc_splat_node().is_none() {
                    effective_args.push(elem);
                }
            }
        } else {
            // Expand the last arg if it's a KeywordHashNode.
            //
            // RuboCop behavior differs by style:
            // - with_first_argument uses `arguments_or_first_arg_pairs` which only
            //   expands the FIRST arg when it's a sole keyword hash (handled above).
            //   For multi-arg calls, it returns `node.arguments` as-is, keeping the
            //   KeywordHashNode as a single item.
            // - with_fixed_indentation uses `arguments_with_last_arg_pairs` which
            //   always expands the last arg's keyword hash elements.
            let last_idx = args_vec.len() - 1;
            for (i, arg) in args_vec.into_iter().enumerate() {
                if i == last_idx && style == "with_fixed_indentation" {
                    if let Some(kw_hash) = arg.as_keyword_hash_node() {
                        for elem in kw_hash.elements().iter() {
                            // Only include AssocNode (pair), skip AssocSplatNode
                            if elem.as_assoc_splat_node().is_none() {
                                effective_args.push(elem);
                            }
                        }
                        continue;
                    }
                }
                effective_args.push(arg);
            }
        }

        if effective_args.len() < 2 {
            return;
        }

        let fixed_indentation = style == "with_fixed_indentation";

        let first_arg = &effective_args[0];
        let first_start = first_arg.location().start_offset();
        let (first_line, first_col) = source.offset_to_line_col(first_start);

        let mut checked_lines = std::collections::HashSet::new();
        // For with_first_argument, the first arg defines the expected column,
        // so it can never be misaligned — skip its line.
        // For with_fixed_indentation, the reference is call indentation + indent_width,
        // so the first arg CAN be misaligned — start with empty checked_lines
        // (matching RuboCop's prev_line = -1 in each_bad_alignment).
        if !fixed_indentation {
            checked_lines.insert(first_line);
        }

        // For "with_fixed_indentation", the expected column is the call line's
        // indentation + indent_width
        let expected_col = match style {
            "with_fixed_indentation" => {
                // Use the line containing the method selector (or opening paren),
                // NOT the full call expression start (which includes the receiver
                // chain). For chained calls like `Foo.bar.baz("str", arg)`, the
                // call node starts at `Foo` but we want the indentation of the
                // line containing `.baz(`.
                let base_line = if let Some(open_loc) = call_node.opening_loc() {
                    source.offset_to_line_col(open_loc.start_offset()).0
                } else if let Some(msg_loc) = call_node.message_loc() {
                    source.offset_to_line_col(msg_loc.start_offset()).0
                } else {
                    source
                        .offset_to_line_col(call_node.location().start_offset())
                        .0
                };
                let base_line_bytes = source.lines().nth(base_line - 1).unwrap_or(b"");
                // Count all leading whitespace (spaces + tabs), matching RuboCop's
                // /\S.*/.match(line).begin(0) which finds the byte offset of the
                // first non-whitespace character.
                let indentation = base_line_bytes
                    .iter()
                    .take_while(|&&b| b == b' ' || b == b'\t')
                    .count();
                indentation + indent_width
            }
            _ => display_column(source, first_start).unwrap_or(first_col),
        };

        let message = if fixed_indentation {
            "Use one level of indentation for arguments following the first line of a multi-line method call."
        } else {
            "Align the arguments of a method call if they span more than one line."
        };

        // For with_fixed_indentation, check ALL items (including the first);
        // for with_first_argument, skip the first (it defines the expected column).
        let skip_count = if fixed_indentation { 0 } else { 1 };

        for arg in effective_args.iter().skip(skip_count) {
            let (arg_line, arg_col) = source.offset_to_line_col(arg.location().start_offset());
            // Only check the FIRST argument on each new line
            if !checked_lines.contains(&arg_line) {
                checked_lines.insert(arg_line);
                // Skip arguments that don't begin their line (matching RuboCop's
                // begins_its_line? check). For example, in `}, suffix: :action`
                // the suffix: argument is not first on its line.
                if !crate::cop::shared::util::begins_its_line(source, arg.location().start_offset())
                {
                    continue;
                }
                if arg_col != expected_col {
                    diagnostics.push(self.diagnostic(
                        source,
                        arg_line,
                        arg_col,
                        message.to_string(),
                    ));
                }
            }
        }
    }
}

fn display_column(source: &SourceFile, byte_offset: usize) -> Option<usize> {
    let (line, fallback_col) = source.offset_to_line_col(byte_offset);
    let line_start = source.line_start_offset(line);
    let prefix = source.try_byte_slice(line_start, byte_offset)?;
    Some(display_width(prefix).max(fallback_col))
}

fn display_width(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut width = 0;
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if is_zero_width(ch) {
            i += 1;
            continue;
        }

        if is_regional_indicator(ch)
            && chars
                .get(i + 1)
                .is_some_and(|next| is_regional_indicator(*next))
        {
            width += 2;
            i += 2;
            continue;
        }

        let base_width =
            if is_wide(ch) || (is_emoji_symbol(ch) && chars.get(i + 1) == Some(&'\u{FE0F}')) {
                2
            } else {
                1
            };
        width += base_width;
        i += 1;

        while chars
            .get(i)
            .is_some_and(|next| is_combining_or_variation(*next))
        {
            i += 1;
        }

        // Treat emoji ZWJ sequences as a single grapheme cluster of the base width.
        while chars.get(i) == Some(&'\u{200D}') && chars.get(i + 1).is_some() {
            i += 2;
            while chars
                .get(i)
                .is_some_and(|next| is_combining_or_variation(*next))
            {
                i += 1;
            }
        }
    }

    width
}

fn is_zero_width(ch: char) -> bool {
    ch == '\u{200C}' || ch == '\u{200D}' || is_combining_or_variation(ch)
}

fn is_combining_or_variation(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE00..=0xFE0F
            | 0xFE20..=0xFE2F
            | 0x1F3FB..=0x1F3FF
            | 0xE0100..=0xE01EF
    )
}

fn is_regional_indicator(ch: char) -> bool {
    matches!(ch as u32, 0x1F1E6..=0x1F1FF)
}

fn is_emoji_symbol(ch: char) -> bool {
    matches!(ch as u32, 0x2600..=0x27BF)
}

fn is_wide(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x115F
            | 0x2329..=0x232A
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1FAFF
            | 0x20000..=0x3FFFD
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(ArgumentAlignment, "cops/layout/argument_alignment");

    #[test]
    fn single_line_call_no_offense() {
        let source = b"foo(1, 2, 3)\n";
        let diags = run_cop_full(&ArgumentAlignment, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn with_fixed_indentation_style() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        };
        // Args aligned with first arg (column 4) but with_fixed_indentation expects column 2
        let src = b"foo(1,\n    2)\n";
        let diags = run_cop_full_with_config(&ArgumentAlignment, src, config.clone());
        assert_eq!(
            diags.len(),
            1,
            "with_fixed_indentation should flag args aligned with first arg"
        );

        // Args at fixed indentation (2 spaces from call)
        let src2 = b"foo(1,\n  2)\n";
        let diags2 = run_cop_full_with_config(&ArgumentAlignment, src2, config);
        assert!(
            diags2.is_empty(),
            "with_fixed_indentation should accept fixed-indent args"
        );
    }

    fn fixed_indent_config() -> CopConfig {
        use std::collections::HashMap;
        CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn with_fixed_indentation_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &ArgumentAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/argument_alignment/with_fixed_indentation_offense.rb"
            ),
            fixed_indent_config(),
        );
    }

    #[test]
    fn with_fixed_indentation_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &ArgumentAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/argument_alignment/with_fixed_indentation_no_offense.rb"
            ),
            fixed_indent_config(),
        );
    }
}
