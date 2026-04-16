use crate::cop::shared::node_type::{HASH_NODE, KEYWORD_HASH_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::source::SourceFile;
use ruby_prism::Visit;

/// Layout/HashAlignment checks that keys, separators, and values of multi-line
/// hash literals are aligned according to configuration.
///
/// ## Root cause analysis (corpus investigation, 2026-03-09)
///
/// The original implementation only checked key column alignment, missing:
/// - **Separator alignment** for hash rockets: `=>` must be exactly 1 space after key end
///   (in "key" style), or right-aligned (in "separator" style), or table-aligned.
/// - **Value alignment**: value must be exactly 1 space after separator end (in "key" style),
///   or aligned across all pairs (in "table"/"separator" style).
/// - **First pair checking**: even the first pair can have bad separator/value spacing.
/// - **Keyword splat alignment**: `**opts` must be aligned with the rest of the hash keys.
/// - **AllowMultipleStyles / array-valued config**: when EnforcedColonStyle or
///   EnforcedHashRocketStyle is an array (e.g., `[key, table]`), the cop picks the
///   style producing fewer offenses per hash.
///
/// These missing checks accounted for the vast majority of the 94K FN gap.
/// The 26 FPs were likely from edge cases in the key-only check.
///
/// ## FP/FN fixes (2026-03-14)
///
/// 1. **kwsplat-first reference bug (FP):** When a hash starts with `**opts` (keyword
///    splat), the cop was using the kwsplat as the alignment reference for key checks.
///    RuboCop uses `node.pairs.first` (first non-kwsplat pair). This caused spurious
///    key-alignment offenses on pairs that were correctly aligned with each other but
///    at a different column than the kwsplat. Fixed by introducing `first_pair()` helper
///    that skips kwsplats, matching RuboCop's behavior.
///
/// 2. **Table-style rocket value off-by-one (FP):** In table alignment for hash rockets,
///    the expected value column was computed as `key_col + max_key_len + sep_len + 1`,
///    missing the space before `=>`. RuboCop's `max_delimiter_width` for rockets is
///    `" => ".length` = 4 (includes both surrounding spaces). Fixed to use `+ 2` instead
///    of `+ 1` to account for spaces on both sides of `=>`.
///
/// 3. **Kwsplat inline with pairs (FP, 2026-03-14):** When a keyword splat (`**options`)
///    appears on the same line as other keyword args (e.g., `**options, method:,\n collection:,`),
///    `check_kwsplat_alignment()` was incorrectly comparing the kwsplat's column against the
///    first non-kwsplat pair's column. But when they share a line, column alignment is meaningless.
///    Fixed by skipping kwsplats that share a line with any non-kwsplat pair.
///
/// 4. **Remaining gap:** `is_call_arg` heuristic for `EnforcedLastArgumentHashStyle`
///    used `!begins_its_line` as a proxy for "is last argument of call," which was
///    wrong in both directions for explicit hashes:
///    - false negatives: plain hash assignments like `CONST = { ... }` were skipped
///      under `always_ignore` / `ignore_explicit` just because `{` did not begin the line;
///    - false positives: explicit last-argument hashes on their own line inside
///      calls/setters were still inspected, even though RuboCop ignores them.
///      Fixed by checking the Prism parent chain directly and only ignoring hashes
///      that are actually the last argument of a `CallNode`, `SuperNode`, or
///      `YieldNode`.
///
/// 5. **`ignore_explicit` with trailing `&block` (FN, 2026-04-16):** Prism keeps
///    `&block` separate from `arguments()`. That made `is_last_argument_hash()`
///    misclassify `call({ ... }, &block)` and `super({ ... }, &block)` as
///    explicit last-argument hashes, so nitrocop skipped them under
///    `ignore_explicit`. RuboCop's `node.last_argument` treats the block pass as
///    trailing after the hash, so these hashes are still inspected. Fixed by
///    requiring `CallNode` / `SuperNode` to have no trailing `block()` before a
///    hash can be ignored as the last argument.
pub struct HashAlignment;

/// Which alignment style to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AlignStyle {
    Key,
    Separator,
    Table,
}

/// An offense found during alignment checking.
#[derive(Debug)]
struct AlignOffense {
    line: usize,
    col: usize,
    #[allow(dead_code)]
    end_col: usize,
    message: &'static str,
}

const MSG_KEY: &str = "Align the keys of a hash literal if they span more than one line.";
const MSG_SEP: &str = "Align the separators of a hash literal if they span more than one line.";
const MSG_TABLE: &str =
    "Align the keys and values of a hash literal if they span more than one line.";
const MSG_KWSPLAT: &str =
    "Align keyword splats with the rest of the hash if it spans more than one line.";

fn parse_styles(config: &CopConfig, key: &str, default: &str) -> Vec<AlignStyle> {
    // Check if the value is a YAML sequence (array)
    if let Some(val) = config.options.get(key) {
        if let Some(seq) = val.as_sequence() {
            let mut styles = Vec::new();
            for item in seq {
                if let Some(s) = item.as_str() {
                    match s {
                        "key" => styles.push(AlignStyle::Key),
                        "separator" => styles.push(AlignStyle::Separator),
                        "table" => styles.push(AlignStyle::Table),
                        _ => {}
                    }
                }
            }
            if !styles.is_empty() {
                styles.dedup();
                return styles;
            }
        }
    }
    // Fallback to string
    let s = config.get_str(key, default);
    match s {
        "key" => vec![AlignStyle::Key],
        "separator" => vec![AlignStyle::Separator],
        "table" => vec![AlignStyle::Table],
        _ => vec![AlignStyle::Key],
    }
}

/// Info about a single hash pair extracted from the AST.
struct PairInfo {
    /// Start offset of the entire pair element (key start).
    elem_start: usize,
    /// End offset of the entire pair element (value end or key end for kwsplat).
    elem_end: usize,
    /// Line and column of the key (or kwsplat) start.
    line: usize,
    col: usize,
    /// Whether this element begins its line.
    begins_line: bool,
    /// Whether this is a keyword splat (**foo).
    is_kwsplat: bool,
    /// Whether this uses hash rocket (=>). False for colon style and kwsplats.
    is_rocket: bool,
    /// Key end column (column after last char of key).
    key_end_col: usize,
    /// Separator (=> or :) column, if any. For colon style, this is part of the key.
    sep_col: Option<usize>,
    /// Separator end column (column after last char of separator).
    sep_end_col: Option<usize>,
    /// Value start column, if value exists and is on the same line.
    value_col: Option<usize>,
    /// Whether the value is on a new line relative to the key.
    #[allow(dead_code)]
    value_on_new_line: bool,
    /// Whether this is a value omission pair (e.g., `a:` with no value).
    is_value_omission: bool,
    /// Key source length (for table alignment calculation).
    key_source_len: usize,
    /// Separator source length (for table alignment calculation).
    sep_source_len: usize,
}

fn extract_pair_info(source: &SourceFile, elem: &ruby_prism::Node<'_>) -> Option<PairInfo> {
    let elem_end = elem.location().end_offset();

    if let Some(assoc) = elem.as_assoc_node() {
        let key = assoc.key();
        let value = assoc.value();
        let key_start = key.location().start_offset();
        let key_end = key.location().end_offset();
        let elem_start = key_start;
        let (line, col) = source.offset_to_line_col(elem_start);
        let begins_line = crate::cop::shared::util::begins_its_line(source, elem_start);
        let (_, key_end_col) = source.offset_to_line_col(key_end);
        let key_source_len = key_end - key_start;

        let (is_rocket, sep_col, sep_end_col, sep_source_len) =
            if let Some(op_loc) = assoc.operator_loc() {
                let op_start = op_loc.start_offset();
                let op_end = op_loc.end_offset();
                let (_, sc) = source.offset_to_line_col(op_start);
                let (_, sec) = source.offset_to_line_col(op_end);
                (true, Some(sc), Some(sec), op_end - op_start)
            } else {
                // Colon style: the colon is part of the key (e.g., `a:`)
                // The "separator end" for value spacing purposes is the key end
                (false, None, None, 0)
            };

        let value_start = value.location().start_offset();
        let (value_line, value_col_v) = source.offset_to_line_col(value_start);
        let value_on_new_line = value_line != line;

        // Detect value omission: `a:` with value being same location as key end
        // In Prism, value omission means the value node is an ImplicitNode
        let is_value_omission = value.as_implicit_node().is_some();

        Some(PairInfo {
            elem_start,
            elem_end,
            line,
            col,
            begins_line,
            is_kwsplat: false,
            is_rocket,
            key_end_col,
            sep_col,
            sep_end_col,
            value_col: if !value_on_new_line && !is_value_omission {
                Some(value_col_v)
            } else {
                None
            },
            value_on_new_line,
            is_value_omission,
            key_source_len,
            sep_source_len,
        })
    } else if elem.as_assoc_splat_node().is_some() {
        // **foo keyword splat
        let elem_start = elem.location().start_offset();
        let (line, col) = source.offset_to_line_col(elem_start);
        let begins_line = crate::cop::shared::util::begins_its_line(source, elem_start);
        Some(PairInfo {
            elem_start,
            elem_end,
            line,
            col,
            begins_line,
            is_kwsplat: true,
            is_rocket: false,
            key_end_col: col, // not used for kwsplat
            sep_col: None,
            sep_end_col: None,
            value_col: None,
            value_on_new_line: false,
            is_value_omission: false,
            key_source_len: 0,
            sep_source_len: 0,
        })
    } else {
        None
    }
}

fn is_last_argument_hash(
    node: &ruby_prism::Node<'_>,
    parent: Option<&ruby_prism::Node<'_>>,
) -> bool {
    let Some(parent) = parent else {
        return false;
    };

    let is_last_argument = |arguments: Option<ruby_prism::ArgumentsNode<'_>>| {
        arguments.is_some_and(|args| {
            args.arguments().iter().last().is_some_and(|last_arg| {
                last_arg.location().start_offset() == node.location().start_offset()
                    && last_arg.location().end_offset() == node.location().end_offset()
            })
        })
    };

    if let Some(call) = parent.as_call_node() {
        if call.block().is_some() {
            return false;
        }
        return is_last_argument(call.arguments());
    }
    if let Some(super_node) = parent.as_super_node() {
        if super_node.block().is_some() {
            return false;
        }
        return is_last_argument(super_node.arguments());
    }
    if let Some(yield_node) = parent.as_yield_node() {
        return is_last_argument(yield_node.arguments());
    }

    false
}

fn should_ignore_last_argument_hash(
    is_last_argument_hash: bool,
    is_keyword_hash: bool,
    last_arg_style: &str,
) -> bool {
    if !is_last_argument_hash {
        return false;
    }

    match last_arg_style {
        "always_ignore" => true,
        "ignore_explicit" => !is_keyword_hash,
        "ignore_implicit" => is_keyword_hash,
        _ => false,
    }
}

/// Find the first non-kwsplat pair (matching RuboCop's `node.pairs.first`).
fn first_pair(pairs: &[PairInfo]) -> Option<&PairInfo> {
    pairs.iter().find(|p| !p.is_kwsplat)
}

/// Check a hash under the "key" alignment style.
/// Returns offenses for this style.
fn check_key_style(source: &SourceFile, pairs: &[PairInfo]) -> Vec<AlignOffense> {
    let mut offenses = Vec::new();
    if pairs.is_empty() {
        return offenses;
    }

    // Use first non-kwsplat pair as reference (matching RuboCop's `node.pairs.first`)
    let first = match first_pair(pairs) {
        Some(p) => p,
        None => return offenses,
    };

    // Check first pair's separator/value spacing
    if !first.is_kwsplat {
        check_key_style_spacing(source, first, &mut offenses);
    }

    for pair in pairs {
        // Skip the first pair (already checked via check_key_style_spacing above)
        if std::ptr::eq(pair, first) {
            continue;
        }
        if !pair.begins_line {
            continue;
        }

        if pair.is_kwsplat {
            // Keyword splat: just check key alignment
            if pair.col != first.col {
                offenses.push(AlignOffense {
                    line: pair.line,
                    col: pair.col,
                    end_col: source.offset_to_line_col(pair.elem_end).1,
                    message: MSG_KWSPLAT,
                });
            }
            continue;
        }

        // Check key column alignment
        let key_misaligned = pair.col != first.col;

        // Check separator/value spacing
        let spacing_bad = has_bad_key_spacing(pair);

        if key_misaligned || spacing_bad {
            offenses.push(AlignOffense {
                line: pair.line,
                col: pair.col,
                end_col: source.offset_to_line_col(pair.elem_end).1,
                message: MSG_KEY,
            });
        }
    }

    offenses
}

/// Check separator and value spacing for a single pair under "key" style.
fn check_key_style_spacing(
    _source: &SourceFile,
    pair: &PairInfo,
    offenses: &mut Vec<AlignOffense>,
) {
    if has_bad_key_spacing(pair) {
        offenses.push(AlignOffense {
            line: pair.line,
            col: pair.col,
            // We need the end column of the pair
            end_col: pair.col + (pair.elem_end - pair.elem_start),
            message: MSG_KEY,
        });
    }
}

/// Check if a pair has bad separator/value spacing under "key" style.
fn has_bad_key_spacing(pair: &PairInfo) -> bool {
    if pair.is_kwsplat || pair.is_value_omission {
        return false;
    }

    if pair.is_rocket {
        // Hash rocket: separator should be 1 space after key end
        if let Some(sc) = pair.sep_col {
            let expected_sep_col = pair.key_end_col + 1;
            if sc != expected_sep_col {
                return true;
            }
        }
        // Value should be 1 space after separator end
        if let (Some(sec), Some(vc)) = (pair.sep_end_col, pair.value_col) {
            let expected_value_col = sec + 1;
            if vc != expected_value_col {
                return true;
            }
        }
    } else {
        // Colon style: value should be 1 space after key end (which includes the colon)
        if let Some(vc) = pair.value_col {
            let expected_value_col = pair.key_end_col + 1;
            if vc != expected_value_col {
                return true;
            }
        }
    }

    false
}

/// Check a hash under the "separator" alignment style.
fn check_separator_style(source: &SourceFile, pairs: &[PairInfo]) -> Vec<AlignOffense> {
    let mut offenses = Vec::new();
    if pairs.len() < 2 {
        return offenses;
    }

    let first = match first_pair(pairs) {
        Some(p) => p,
        None => return offenses,
    };

    for pair in pairs {
        if std::ptr::eq(pair, first) {
            continue;
        }
        if !pair.begins_line {
            continue;
        }

        if pair.is_kwsplat {
            if pair.col != first.col {
                offenses.push(AlignOffense {
                    line: pair.line,
                    col: pair.col,
                    end_col: source.offset_to_line_col(pair.elem_end).1,
                    message: MSG_KWSPLAT,
                });
            }
            continue;
        }

        let mut bad = false;

        if pair.is_rocket && first.is_rocket {
            // Separator (=>) should be aligned with first pair's separator
            if let (Some(first_sc), Some(pair_sc)) = (first.sep_col, pair.sep_col) {
                if first_sc != pair_sc {
                    bad = true;
                }
            }
            // Key should be right-aligned: key_end_col should match first's key_end_col
            if first.key_end_col != pair.key_end_col {
                bad = true;
            }
            // Value should be aligned with first pair's value
            if let (Some(fv), Some(pv)) = (first.value_col, pair.value_col) {
                if fv != pv {
                    bad = true;
                }
            }
        } else if !pair.is_rocket && !first.is_rocket {
            // Colon style: key end (including colon) should be right-aligned
            if first.key_end_col != pair.key_end_col {
                bad = true;
            }
            // Value should be aligned
            if let (Some(fv), Some(pv)) = (first.value_col, pair.value_col) {
                if fv != pv {
                    bad = true;
                }
            }
        } else {
            // Mixed delimiters — separator style can't check, skip
            continue;
        }

        if bad {
            offenses.push(AlignOffense {
                line: pair.line,
                col: pair.col,
                end_col: source.offset_to_line_col(pair.elem_end).1,
                message: MSG_SEP,
            });
        }
    }

    offenses
}

/// Check a hash under the "table" alignment style.
fn check_table_style(source: &SourceFile, pairs: &[PairInfo]) -> Vec<AlignOffense> {
    let mut offenses = Vec::new();
    if pairs.len() < 2 {
        return offenses;
    }

    // Table style requires all pairs to use the same delimiter.
    // Check for mixed delimiters.
    let non_kwsplat: Vec<&PairInfo> = pairs.iter().filter(|p| !p.is_kwsplat).collect();
    if non_kwsplat.is_empty() {
        return offenses;
    }

    let has_rocket = non_kwsplat.iter().any(|p| p.is_rocket);
    let has_colon = non_kwsplat.iter().any(|p| !p.is_rocket);
    if has_rocket && has_colon {
        // Mixed delimiters — table style is not checkable
        return offenses;
    }

    // Check if any pairs are on the same line (table requires each pair on its own line)
    let mut lines_seen = std::collections::HashSet::new();
    for p in &non_kwsplat {
        if !lines_seen.insert(p.line) {
            // Two pairs on the same line — not checkable for table
            return offenses;
        }
    }

    // Calculate max key width and expected positions
    let max_key_len = non_kwsplat
        .iter()
        .map(|p| p.key_source_len)
        .max()
        .unwrap_or(0);

    let first = match first_pair(pairs) {
        Some(p) => p,
        None => return offenses,
    };

    // For table style, check all pairs including first
    for pair in pairs {
        if !pair.begins_line && !std::ptr::eq(pair, first) {
            continue;
        }

        if pair.is_kwsplat {
            // Keyword splats just need key alignment
            if pair.col != first.col {
                offenses.push(AlignOffense {
                    line: pair.line,
                    col: pair.col,
                    end_col: source.offset_to_line_col(pair.elem_end).1,
                    message: MSG_KWSPLAT,
                });
            }
            continue;
        }

        let mut bad = false;

        // Key must be left-aligned with first key
        if pair.col != first.col {
            bad = true;
        }

        if pair.is_value_omission {
            // Value omission pairs only need key alignment
            if bad {
                offenses.push(AlignOffense {
                    line: pair.line,
                    col: pair.col,
                    end_col: source.offset_to_line_col(pair.elem_end).1,
                    message: MSG_TABLE,
                });
            }
            continue;
        }

        if pair.is_rocket {
            // Hash rocket: separator should be at first.col + max_key_len + 1 (space before =>)
            let expected_sep = first.col + max_key_len + 1;
            if let Some(sc) = pair.sep_col {
                if sc != expected_sep {
                    bad = true;
                }
            }
            // Value should be after separator + 1 space:
            // first.col + max_key_len + 1 (space before =>) + sep_len + 1 (space after =>)
            let expected_value = first.col + max_key_len + pair.sep_source_len + 2;
            if let Some(vc) = pair.value_col {
                if vc != expected_value {
                    bad = true;
                }
            }
        } else {
            // Colon style: value should be at first.col + max_key_len + 1
            let expected_value = first.col + max_key_len + 1;
            if let Some(vc) = pair.value_col {
                if vc != expected_value {
                    bad = true;
                }
            }
        }

        if bad {
            offenses.push(AlignOffense {
                line: pair.line,
                col: pair.col,
                end_col: source.offset_to_line_col(pair.elem_end).1,
                message: MSG_TABLE,
            });
        }
    }

    offenses
}

impl Cop for HashAlignment {
    fn name(&self) -> &'static str {
        "Layout/HashAlignment"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[HASH_NODE, KEYWORD_HASH_NODE]
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
        let mut visitor = HashAlignmentVisitor {
            cop: self,
            source,
            config,
            diagnostics,
            ancestors: Vec::new(),
        };
        visitor.visit(&parse_result.node());
    }

    fn check_node(
        &self,
        _source: &SourceFile,
        _node: &ruby_prism::Node<'_>,
        _parse_result: &ruby_prism::ParseResult<'_>,
        _config: &CopConfig,
        _diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
    }
}

/// Check if a style is checkable for the given pairs.
/// "key" is always checkable. "separator" and "table" require no pairs on the same line
/// and no mixed delimiters.
fn is_checkable(style: AlignStyle, pairs: &[PairInfo]) -> bool {
    if style == AlignStyle::Key {
        return true;
    }

    let non_kwsplat: Vec<&PairInfo> = pairs.iter().filter(|p| !p.is_kwsplat).collect();
    if non_kwsplat.is_empty() {
        return true;
    }

    // Check mixed delimiters
    let has_rocket = non_kwsplat.iter().any(|p| p.is_rocket);
    let has_colon = non_kwsplat.iter().any(|p| !p.is_rocket);
    if has_rocket && has_colon {
        return false;
    }

    // Check pairs on same line
    let mut lines_seen = std::collections::HashSet::new();
    for p in &non_kwsplat {
        if !lines_seen.insert(p.line) {
            return false;
        }
    }

    true
}

/// Check offenses for the given styles and return the best (fewest offenses).
fn best_offenses_for_styles(
    styles: &[AlignStyle],
    source: &SourceFile,
    pairs: &[PairInfo],
    is_rocket: bool,
) -> Vec<AlignOffense> {
    // Filter to relevant pairs (matching delimiter type) plus first pair for reference
    let relevant: Vec<&PairInfo> = pairs
        .iter()
        .filter(|p| !p.is_kwsplat && p.is_rocket == is_rocket)
        .collect();

    if relevant.is_empty() {
        return Vec::new();
    }

    // For each style, compute offenses and pick the style with fewest
    let mut best: Option<Vec<AlignOffense>> = None;

    for &style in styles {
        let offenses = match style {
            AlignStyle::Key => check_key_style(source, pairs),
            AlignStyle::Separator => check_separator_style(source, pairs),
            AlignStyle::Table => check_table_style(source, pairs),
        };

        // Filter to only offenses on relevant pair types (not kwsplats, matching delimiter)
        let filtered: Vec<AlignOffense> = offenses
            .into_iter()
            .filter(|o| {
                // Keep offenses that are on pairs matching our delimiter type
                pairs.iter().any(|p| {
                    p.line == o.line && p.col == o.col && !p.is_kwsplat && p.is_rocket == is_rocket
                })
            })
            .collect();

        match &best {
            None => best = Some(filtered),
            Some(current_best) => {
                if filtered.len() < current_best.len() {
                    best = Some(filtered);
                }
            }
        }
    }

    best.unwrap_or_default()
}

/// Check keyword splat alignment (always aligned with first non-kwsplat key).
fn check_kwsplat_alignment(source: &SourceFile, pairs: &[PairInfo]) -> Vec<AlignOffense> {
    let mut offenses = Vec::new();

    // Find first non-kwsplat pair for reference column
    let first_ref = match pairs.iter().find(|p| !p.is_kwsplat) {
        Some(p) => p,
        None => return offenses,
    };

    for pair in pairs {
        if !pair.is_kwsplat || !pair.begins_line {
            continue;
        }
        // Skip kwsplats that share a line with a non-kwsplat pair (e.g., `**options, method:,`).
        // Alignment is not meaningful when elements are on the same line.
        let shares_line_with_pair = pairs.iter().any(|p| !p.is_kwsplat && p.line == pair.line);
        if shares_line_with_pair {
            continue;
        }
        if pair.col != first_ref.col {
            offenses.push(AlignOffense {
                line: pair.line,
                col: pair.col,
                end_col: source.offset_to_line_col(pair.elem_end).1,
                message: MSG_KWSPLAT,
            });
        }
    }

    offenses
}

impl HashAlignment {
    fn check_hash_node(
        &self,
        source: &SourceFile,
        node: &ruby_prism::Node<'_>,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        is_last_argument_hash: bool,
    ) {
        let _allow_multiple = config.get_bool("AllowMultipleStyles", true);
        let rocket_styles = parse_styles(config, "EnforcedHashRocketStyle", "key");
        let colon_styles = parse_styles(config, "EnforcedColonStyle", "key");
        let last_arg_style = config.get_str("EnforcedLastArgumentHashStyle", "always_inspect");
        let arg_alignment_style = config.get_str("ArgumentAlignmentStyle", "with_first_argument");
        let fixed_indentation = arg_alignment_style == "with_fixed_indentation";

        // Handle both HashNode (literal `{}`) and KeywordHashNode (keyword args `foo(a: 1)`)
        let is_keyword_hash = node.as_keyword_hash_node().is_some();
        let (elements, hash_node_start) = if let Some(hash_node) = node.as_hash_node() {
            (hash_node.elements(), hash_node.location().start_offset())
        } else if let Some(kw_hash_node) = node.as_keyword_hash_node() {
            (
                kw_hash_node.elements(),
                kw_hash_node.location().start_offset(),
            )
        } else {
            return;
        };

        // Need at least 2 elements OR at least 1 element where we check spacing.
        // RuboCop's on_hash requires node.pairs.empty? to be false and node.single_line? to be false.
        // For single-element hashes, only separator/value spacing is checked (via first pair).
        let elem_count = elements.len();
        if elem_count == 0 {
            return;
        }

        // Check if hash is single-line — skip if so
        let hash_start_line = source.offset_to_line_col(hash_node_start).0;
        let hash_end_offset = if let Some(hash_node) = node.as_hash_node() {
            hash_node.location().end_offset()
        } else if let Some(kw_hash_node) = node.as_keyword_hash_node() {
            kw_hash_node.location().end_offset()
        } else {
            return;
        };
        let hash_end_line = source.offset_to_line_col(hash_end_offset).0;
        if hash_start_line == hash_end_line {
            return;
        }

        // Match RuboCop's `on_send` / `ignore_hash_argument?`: only ignore a hash
        // when this node is actually the last argument of a call-like node.
        if should_ignore_last_argument_hash(is_last_argument_hash, is_keyword_hash, last_arg_style)
        {
            return;
        }

        // Extract pair info for all elements
        let pairs: Vec<PairInfo> = elements
            .iter()
            .filter_map(|elem| extract_pair_info(source, &elem))
            .collect();

        if pairs.is_empty() {
            return;
        }

        // Use first non-kwsplat pair as reference (matching RuboCop's `node.pairs.first`)
        let first = match first_pair(&pairs) {
            Some(p) => p,
            None => return,
        };

        // autocorrect_incompatible_with_other_cops? check
        if fixed_indentation {
            if is_keyword_hash {
                if !first.begins_line {
                    return;
                }
            } else {
                let hash_begins_line =
                    crate::cop::shared::util::begins_its_line(source, hash_node_start);
                if !hash_begins_line && !first.begins_line {
                    return;
                }
            }
        }

        // Determine which styles apply based on pair types present
        let has_rocket = pairs.iter().any(|p| !p.is_kwsplat && p.is_rocket);
        let has_colon = pairs.iter().any(|p| !p.is_kwsplat && !p.is_rocket);

        // Check if any style combination is valid for the hash
        // RuboCop checks alignment_for_hash_rockets.any?(checkable_layout?) &&
        //   alignment_for_colons.any?(checkable_layout?)
        // For "key" style, checkable_layout? is always true.
        // For separator/table, it requires !pairs_on_same_line? && !mixed_delimiters?
        let rocket_checkable = rocket_styles.iter().any(|s| is_checkable(*s, &pairs));
        let colon_checkable = colon_styles.iter().any(|s| is_checkable(*s, &pairs));

        if has_rocket && !rocket_checkable {
            return;
        }
        if has_colon && !colon_checkable {
            return;
        }
        // If both are present, both must be checkable
        if has_rocket && has_colon && (!rocket_checkable || !colon_checkable) {
            return;
        }

        // For each pair, determine which style applies (based on whether it's rocket or colon)
        // and check alignment. When multiple styles are allowed, pick the one with fewest offenses.

        // We need to check the entire hash under each applicable style combination
        // and report the one with fewest offenses.

        // Collect offenses per style for rocket pairs and colon pairs separately,
        // then combine.
        let rocket_pair_offenses = if has_rocket {
            best_offenses_for_styles(&rocket_styles, source, &pairs, true)
        } else {
            Vec::new()
        };

        let colon_pair_offenses = if has_colon {
            best_offenses_for_styles(&colon_styles, source, &pairs, false)
        } else {
            Vec::new()
        };

        // Also check keyword splat offenses (always use key alignment for splats)
        let kwsplat_offenses = check_kwsplat_alignment(source, &pairs);

        // Emit diagnostics
        for offense in rocket_pair_offenses
            .iter()
            .chain(colon_pair_offenses.iter())
            .chain(kwsplat_offenses.iter())
        {
            diagnostics.push(self.diagnostic(
                source,
                offense.line,
                offense.col,
                offense.message.to_string(),
            ));
        }
    }
}

struct HashAlignmentVisitor<'a, 'src, 'pr> {
    cop: &'a HashAlignment,
    source: &'src SourceFile,
    config: &'a CopConfig,
    diagnostics: &'a mut Vec<Diagnostic>,
    ancestors: Vec<ruby_prism::Node<'pr>>,
}

impl<'a, 'src, 'pr> HashAlignmentVisitor<'a, 'src, 'pr> {
    fn current_parent(&self) -> Option<&ruby_prism::Node<'pr>> {
        self.ancestors.iter().rev().nth(1)
    }
}

impl<'a, 'src, 'pr> ruby_prism::Visit<'pr> for HashAlignmentVisitor<'a, 'src, 'pr> {
    fn visit_branch_node_enter(&mut self, node: ruby_prism::Node<'pr>) {
        self.ancestors.push(node);
    }

    fn visit_branch_node_leave(&mut self) {
        self.ancestors.pop();
    }

    fn visit_leaf_node_enter(&mut self, _node: ruby_prism::Node<'pr>) {}

    fn visit_hash_node(&mut self, node: &ruby_prism::HashNode<'pr>) {
        let generic = node.as_node();
        let is_last_arg = is_last_argument_hash(&generic, self.current_parent());
        self.cop.check_hash_node(
            self.source,
            &generic,
            self.config,
            self.diagnostics,
            is_last_arg,
        );
        ruby_prism::visit_hash_node(self, node);
    }

    fn visit_keyword_hash_node(&mut self, node: &ruby_prism::KeywordHashNode<'pr>) {
        let generic = node.as_node();
        let is_last_arg = is_last_argument_hash(&generic, self.current_parent());
        self.cop.check_hash_node(
            self.source,
            &generic,
            self.config,
            self.diagnostics,
            is_last_arg,
        );
        ruby_prism::visit_keyword_hash_node(self, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::run_cop_full;

    crate::cop_fixture_tests!(HashAlignment, "cops/layout/hash_alignment");

    #[test]
    fn single_line_hash_no_offense() {
        let source = b"x = { a: 1, b: 2 }\n";
        let diags = run_cop_full(&HashAlignment, source);
        assert!(diags.is_empty());
    }

    #[test]
    fn config_options_are_read() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedHashRocketStyle".into(),
                    serde_yml::Value::String("key".into()),
                ),
                (
                    "EnforcedColonStyle".into(),
                    serde_yml::Value::String("key".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        // Key-aligned hash should be accepted
        let src = b"x = {\n  a: 1,\n  b: 2\n}\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(diags.is_empty(), "key-aligned hash should be accepted");
    }

    #[test]
    fn fixed_indentation_skips_keyword_hash_on_same_line() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "ArgumentAlignmentStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"render html: \"hello\",\n  layout: \"application\"\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(
            diags.is_empty(),
            "keyword hash on same line as call should be skipped with fixed indentation"
        );
    }

    #[test]
    fn fixed_indentation_still_checks_keyword_hash_on_own_line() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "ArgumentAlignmentStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"render(\n  html: \"hello\",\n    layout: \"application\"\n)\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert_eq!(
            diags.len(),
            1,
            "keyword hash on own line should still be checked with fixed indentation"
        );
    }

    fn variant_config(
        hash_rocket_style: &'static str,
        colon_style: &'static str,
        last_arg_style: &'static str,
    ) -> CopConfig {
        use std::collections::HashMap;

        CopConfig {
            options: HashMap::from([
                (
                    "EnforcedHashRocketStyle".into(),
                    serde_yml::Value::String(hash_rocket_style.into()),
                ),
                (
                    "EnforcedColonStyle".into(),
                    serde_yml::Value::String(colon_style.into()),
                ),
                (
                    "EnforcedLastArgumentHashStyle".into(),
                    serde_yml::Value::String(last_arg_style.into()),
                ),
            ]),
            ..CopConfig::default()
        }
    }

    #[test]
    fn separator_always_ignore_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/always_ignore_separator_offense.rb"
            ),
            variant_config("separator", "separator", "always_ignore"),
        );
    }

    #[test]
    fn ignore_explicit_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/ignore_explicit_offense.rb"
            ),
            variant_config("key", "key", "ignore_explicit"),
        );
    }

    #[test]
    fn explicit_last_arg_no_offense_fixture_for_always_ignore() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/explicit_last_arg_no_offense.rb"
            ),
            variant_config("separator", "separator", "always_ignore"),
        );
    }

    #[test]
    fn explicit_last_arg_no_offense_fixture_for_ignore_explicit() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/explicit_last_arg_no_offense.rb"
            ),
            variant_config("key", "key", "ignore_explicit"),
        );
    }

    #[test]
    fn table_ignore_implicit_flags_first_pair_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/table_ignore_implicit_first_pair_offense.rb"
            ),
            variant_config("table", "table", "ignore_implicit"),
        );
    }

    #[test]
    fn default_config_flags_keyword_hash_on_same_line() {
        let src = b"render html: \"hello\",\n  layout: \"application\"\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(
            diags.len(),
            1,
            "keyword hash should be flagged without fixed indentation"
        );
    }

    #[test]
    fn always_ignore_skips_keyword_hash() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedLastArgumentHashStyle".into(),
                serde_yml::Value::String("always_ignore".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"render html: \"hello\",\n  layout: \"application\"\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(
            diags.is_empty(),
            "always_ignore should skip keyword hash args"
        );
    }

    #[test]
    fn ignore_implicit_skips_keyword_hash() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedLastArgumentHashStyle".into(),
                serde_yml::Value::String("ignore_implicit".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"render html: \"hello\",\n  layout: \"application\"\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(
            diags.is_empty(),
            "ignore_implicit should skip implicit keyword hash args"
        );
    }

    #[test]
    fn key_style_flags_extra_spaces_after_colon() {
        let src = b"hash = {\n  a:   0,\n  bb: 1,\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(diags.len(), 1, "extra spaces after colon should be flagged");
        assert_eq!(diags[0].location.line, 2);
    }

    #[test]
    fn key_style_flags_zero_spaces_after_colon() {
        let src = b"hash = {\n  a:0,\n  bb: 1,\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(diags.len(), 1, "zero spaces after colon should be flagged");
        assert_eq!(diags[0].location.line, 2);
    }

    #[test]
    fn key_style_flags_bad_rocket_spacing() {
        let src = b"hash = {\n  'ccc'=> 2,\n  'dddd' => 3\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(diags.len(), 1, "missing space before => should be flagged");
        assert_eq!(diags[0].location.line, 2);
    }

    #[test]
    fn key_style_flags_extra_space_after_rocket() {
        let src = b"hash = {\n  'a' =>  0,\n  'bbb' => 1\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(diags.len(), 1, "extra space after => should be flagged");
        assert_eq!(diags[0].location.line, 2);
    }

    #[test]
    fn key_style_accepts_correct_spacing() {
        let src = b"hash = {\n  :a => 0,\n  :bb => 1\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert!(diags.is_empty(), "correctly spaced rockets should pass");
    }

    #[test]
    fn key_style_first_pair_bad_spacing() {
        let src = b"hash = {\n  :a   => 0,\n  :bb => 1,\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(
            diags.len(),
            1,
            "first pair with extra spaces before => should be flagged"
        );
        assert_eq!(diags[0].location.line, 2);
    }

    #[test]
    fn kwsplat_alignment() {
        let src = b"{foo: 'bar',\n       **extra\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(diags.len(), 1, "misaligned kwsplat should be flagged");
        assert!(diags[0].message.contains("keyword splats"));
    }

    #[test]
    fn kwsplat_aligned_no_offense() {
        let src = b"{foo: 'bar',\n **extra}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert!(diags.is_empty(), "aligned kwsplat should pass");
    }

    #[test]
    fn value_on_new_line_no_offense() {
        let src = b"hash = {\n  'a' =>\n    0,\n  'bbb' => 1\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert!(diags.is_empty(), "value on new line should not be flagged");
    }

    #[test]
    fn several_pairs_per_line_no_offense() {
        let src = b"func(a: 1, bb: 2,\n     ccc: 3, dddd: 4)\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert!(
            diags.is_empty(),
            "several pairs per line should not be flagged"
        );
    }

    #[test]
    fn table_style_accepts_aligned() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedColonStyle".into(),
                    serde_yml::Value::String("table".into()),
                ),
                (
                    "EnforcedHashRocketStyle".into(),
                    serde_yml::Value::String("table".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        let src = b"hash = {\n  a:   0,\n  bbb: 1\n}\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(diags.is_empty(), "table-aligned hash should pass");
    }

    #[test]
    fn table_style_flags_misaligned() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedColonStyle".into(),
                    serde_yml::Value::String("table".into()),
                ),
                (
                    "EnforcedHashRocketStyle".into(),
                    serde_yml::Value::String("table".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        let src = b"hash = {\n  a: 0,\n  bbb: 1\n}\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(
            !diags.is_empty(),
            "non-table-aligned hash should be flagged"
        );
    }

    #[test]
    fn fixed_indentation_table_aligned_kwargs() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        // With with_fixed_indentation, table-aligned kwargs where first key is on same line as
        // the method call should be skipped
        let config = CopConfig {
            options: HashMap::from([(
                "ArgumentAlignmentStyle".into(),
                serde_yml::Value::String("with_fixed_indentation".into()),
            )]),
            ..CopConfig::default()
        };
        let src =
            b"config.fog_credentials_as_kwargs(\n  provider: 'AWS',\n  aws_access_key_id: ENV['S3_ACCESS_KEY'],\n)\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(
            diags.is_empty(),
            "kwargs on own line with fixed indentation should pass"
        );
    }

    #[test]
    fn kwsplat_first_pairs_aligned_no_offense() {
        // When kwsplat is the first element, pairs should be checked against the
        // first non-kwsplat pair (matching RuboCop's `node.pairs.first`), not the kwsplat.
        // Here pairs are aligned with each other but at a different column than kwsplat.
        // Only the kwsplat misalignment should be reported.
        let src = b"{\n  **opts,\n    a: 1,\n    b: 2\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert_eq!(
            diags.len(),
            1,
            "only kwsplat misalignment should be reported, not key offenses: {:?}",
            diags
        );
        assert!(
            diags[0].message.contains("keyword splats"),
            "offense should be kwsplat alignment: {}",
            diags[0].message
        );
    }

    #[test]
    fn table_style_rocket_correct_alignment() {
        // Table style for rockets: values should be aligned at max_key_width + " => " width
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([
                (
                    "EnforcedHashRocketStyle".into(),
                    serde_yml::Value::String("table".into()),
                ),
                (
                    "EnforcedColonStyle".into(),
                    serde_yml::Value::String("table".into()),
                ),
            ]),
            ..CopConfig::default()
        };
        // Correctly table-aligned:
        //   :a   => 0
        //   :bbb => 1
        // max_key_width = 4 (`:bbb`), delimiter = ` => ` (4 chars)
        // values at col 2 + 4 + 4 = 10
        let src = b"hash = {\n  :a   => 0,\n  :bbb => 1\n}\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert!(
            diags.is_empty(),
            "correctly table-aligned rockets should pass: {:?}",
            diags
        );
    }

    #[test]
    fn kwsplat_first_all_aligned_no_offense() {
        // When kwsplat is first and everything is at the same column, no offense
        let src = b"{\n  **opts,\n  a: 1,\n  b: 2\n}\n";
        let diags = run_cop_full(&HashAlignment, src);
        assert!(
            diags.is_empty(),
            "all elements at same column should pass: {:?}",
            diags
        );
    }
}
