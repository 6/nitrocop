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
/// 5. **`&block` after explicit hash (`ignore_explicit` FN, 2026-04-16):** Prism stores
///    block-pass arguments as `BlockArgumentNode` on `call.block()` / `super.block()`,
///    not in `arguments()`. The previous last-argument check only looked at positional
///    arguments, so `foo({a: 1,\n  b: 2}, &block)` incorrectly treated the explicit hash
///    as the last argument and skipped it under `ignore_explicit`. RuboCop still inspects
///    that hash because the trailing `&block` is the actual last argument. Fixed by
///    treating a trailing `BlockArgumentNode` as disqualifying the hash from "last
///    argument hash" handling.
///
/// 6. **Multiline value alignment in `separator` / `table` styles (FN, 2026-04-17):**
///    the cop discarded `value_col` whenever a hash value started on the next line
///    (`"key" =>\n  value`). That matched `key` style, which ignores newline values,
///    but diverged from RuboCop for `separator` and `table`, where `Pair#value.loc.column`
///    is still aligned even across lines. This specifically missed first-pair offenses in
///    table style and multiline-proc/hash variants in the corpus. Fixed by keeping the
///    value column for non-omitted values and only suppressing newline value spacing in
///    `key` style.
///
/// 7. **Bundled `EnforcedStyle` alias + single-pair table hashes (2026-04-17):**
///    local variant validation passes `EnforcedStyle: rocket, colon, last_arg_style`
///    for this cop, but the implementation only read the split RuboCop keys. That left
///    `check_cop.py --style EnforcedStyle=...` effectively on default behavior. RuboCop
///    also checks the first pair in single-pair hashes under `table` style, while the cop
///    returned early unless there were at least two pairs. Fixed by parsing the bundled
///    alias in the cop and letting `check_table_style` inspect single-pair hashes.
///
/// 8. **Table-style alignment must use char width, not byte width (2026-04-17):** The
///    key length fed into `check_table_style`'s `max_key_len`/expected-column math was
///    `key_end - key_start` in bytes, while column positions are UTF-8 codepoint counts.
///    Hashes with multi-byte keys (emoji, CJK) therefore mis-computed the expected
///    separator/value columns and produced FPs whenever RuboCop would have accepted the
///    alignment. Fixed by counting non-continuation bytes (UTF-8 codepoint count) for
///    `key_char_len` so it composes with column math.
///
/// 9. **Variant-only shared-line + separator quirks (2026-04-20):**
///    - `table` / `separator` styles use RuboCop AST's `pairs_on_same_line?`, which treats
///      adjacent pairs as "same line" when a multiline pair's closing line also contains the
///      next pair (for example `}, :conditions => {`). The previous implementation only looked
///      at pair start lines, so `table, table, ignore_implicit` incorrectly flagged hashes that
///      RuboCop marks uncheckable.
///    - Under `separator` with hash rockets, RuboCop 1.84.2 crashes its autocorrection pass
///      (`Parser::ClobberingError`) when the first pair's value starts on the next line but
///      later pairs mix newline and same-line values. The corpus oracle counts that as zero
///      offenses, so nitrocop now suppresses that exact mixed-value shape to match RuboCop's
///      observable output.
///    - Under `separator` with colon pairs, RuboCop 1.84.2 crashes per-pair when the first
///      non-kwsplat pair uses Ruby 3.1 value omission (`foo:`) and a later non-omission
///      colon pair has a *strictly shorter* key. The crash aborts processing the remaining
///      pairs in that hash, but any offenses already emitted by earlier pairs are kept. So
///      `{ token:, uuid: creds[:uuid] }` reports no offenses (`uuid` < `token`), while
///      `{ token:, uuid_long: x }` still reports normally (`uuid_long` >= `token`). Mixed
///      colon/rocket pairs and same-length keys do not crash.
///
/// 10. **Separator-style colon crash with mixed newline values (2026-04-25):**
///     RuboCop 1.84.2 aborts `separator`-style checking for colon pairs when the first
///     non-kwsplat pair's value starts on the next line, a later non-omission pair keeps
///     its value on the same line, AND that later pair's key is strictly shorter than
///     the first pair's. The crash comes from RuboCop's right-align corrector trying to
///     pad the shorter key with leading spaces, whose remove-range overlaps with the
///     separator and value ranges. When all later keys are equal-length or longer,
///     RuboCop emits offenses normally, so the colon suppression is gated on
///     `pair.key_char_len < first.key_char_len`. The rocket suppression is unconditional
///     because RuboCop also clobbers rocket pairs that are already separator-aligned
///     (where key length doesn't matter) — see the `"xml" =>`/`"uiinput" =>` fixture.
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

fn parse_style_value(value: &str) -> Option<AlignStyle> {
    match value.trim() {
        "key" => Some(AlignStyle::Key),
        "separator" => Some(AlignStyle::Separator),
        "table" => Some(AlignStyle::Table),
        _ => None,
    }
}

fn parse_enforced_style_alias(
    config: &CopConfig,
) -> Option<(Vec<AlignStyle>, Vec<AlignStyle>, String)> {
    let value = config.options.get("EnforcedStyle")?.as_str()?;
    let parts: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }

    Some((
        vec![parse_style_value(parts[0])?],
        vec![parse_style_value(parts[1])?],
        parts[2].to_string(),
    ))
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
    /// Last line touched by the element.
    last_line: usize,
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
    /// Value start column, if the pair has an explicit value.
    value_col: Option<usize>,
    /// Whether the value is on a new line relative to the key.
    #[allow(dead_code)]
    value_on_new_line: bool,
    /// Whether this is a value omission pair (e.g., `a:` with no value).
    is_value_omission: bool,
    /// Key character width (for table alignment calculation). Must be counted in
    /// UTF-8 codepoints, not bytes, since it is added to character columns.
    key_char_len: usize,
    /// Separator source length (for table alignment calculation).
    sep_source_len: usize,
}

fn extract_pair_info(source: &SourceFile, elem: &ruby_prism::Node<'_>) -> Option<PairInfo> {
    let elem_end = elem.location().end_offset();
    let last_line = source.offset_to_line_col(elem_end.saturating_sub(1)).0;

    if let Some(assoc) = elem.as_assoc_node() {
        let key = assoc.key();
        let value = assoc.value();
        let key_start = key.location().start_offset();
        let key_end = key.location().end_offset();
        let elem_start = key_start;
        let (line, col) = source.offset_to_line_col(elem_start);
        let begins_line = crate::cop::shared::util::begins_its_line(source, elem_start);
        let (_, key_end_col) = source.offset_to_line_col(key_end);
        // Count characters (UTF-8 codepoints), not bytes, so this matches the
        // character-based column math used to derive expected alignment positions.
        let key_char_len = source.content[key_start..key_end]
            .iter()
            .filter(|&&b| (b & 0xC0) != 0x80)
            .count();

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
            last_line,
            begins_line,
            is_kwsplat: false,
            is_rocket,
            key_end_col,
            sep_col,
            sep_end_col,
            value_col: if !is_value_omission {
                Some(value_col_v)
            } else {
                None
            },
            value_on_new_line,
            is_value_omission,
            key_char_len,
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
            last_line,
            begins_line,
            is_kwsplat: true,
            is_rocket: false,
            key_end_col: col, // not used for kwsplat
            sep_col: None,
            sep_end_col: None,
            value_col: None,
            value_on_new_line: false,
            is_value_omission: false,
            key_char_len: 0,
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
        return is_last_argument(call.arguments())
            && call
                .block()
                .is_none_or(|block| block.as_block_argument_node().is_none());
    }
    if let Some(super_node) = parent.as_super_node() {
        return is_last_argument(super_node.arguments())
            && super_node
                .block()
                .is_none_or(|block| block.as_block_argument_node().is_none());
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

fn pairs_share_line(first: &PairInfo, second: &PairInfo) -> bool {
    first.last_line == second.line || first.line == second.last_line
}

fn adjacent_pairs_share_line<'a, I>(pairs: I) -> bool
where
    I: IntoIterator<Item = &'a PairInfo>,
{
    let pairs: Vec<&PairInfo> = pairs.into_iter().collect();
    pairs
        .windows(2)
        .any(|window| pairs_share_line(window[0], window[1]))
}

fn separator_style_rubocop_clobber_quirk(pairs: &[PairInfo]) -> bool {
    let Some(first) = first_pair(pairs) else {
        return false;
    };

    if !first.value_on_new_line {
        return false;
    }

    pairs
        .iter()
        .filter(|pair| !pair.is_kwsplat && !std::ptr::eq(*pair, first))
        .any(|pair| {
            if pair.is_value_omission || pair.value_on_new_line {
                return false;
            }
            // Rocket-style triggers the clobber whenever the first pair's value
            // is on a new line and a later pair keeps its value on the same
            // line; the corrector's separator/value range overlaps regardless
            // of key length.
            //
            // Colon-style only crashes when RuboCop's right-aligned key
            // correction needs to pad a strictly shorter key. When the later
            // pair's key is the same length or longer than the first pair's,
            // RuboCop emits offenses normally — suppressing those produces
            // FNs in the corpus.
            first.is_rocket || pair.key_char_len < first.key_char_len
        })
}

/// RuboCop 1.84.2 crashes when checking a colon-style pair whose key is strictly shorter
/// than the first pair's key, and the first pair is a Ruby 3.1 value omission (`foo:`).
/// The crash aborts the hash's remaining pair checks; any offenses emitted by earlier
/// pairs are preserved. We model the abort by `break`ing the per-pair loop on this trigger.
fn rubocop_crashes_on_omission_pair(first: &PairInfo, pair: &PairInfo) -> bool {
    first.is_value_omission
        && !pair.is_kwsplat
        && !pair.is_value_omission
        && !pair.is_rocket
        && pair.key_char_len < first.key_char_len
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
        if !pair.value_on_new_line {
            if let (Some(sec), Some(vc)) = (pair.sep_end_col, pair.value_col) {
                let expected_value_col = sec + 1;
                if vc != expected_value_col {
                    return true;
                }
            }
        }
    } else {
        // Colon style: value should be 1 space after key end (which includes the colon)
        if !pair.value_on_new_line {
            if let Some(vc) = pair.value_col {
                let expected_value_col = pair.key_end_col + 1;
                if vc != expected_value_col {
                    return true;
                }
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

    if separator_style_rubocop_clobber_quirk(pairs) {
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
        if rubocop_crashes_on_omission_pair(first, pair) {
            // RuboCop crashes here and stops checking the rest of this hash.
            break;
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
    if pairs.is_empty() {
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
    if adjacent_pairs_share_line(non_kwsplat.iter().copied()) {
        // Two adjacent pairs share a line — not checkable for table
        return offenses;
    }

    // Calculate max key width and expected positions
    let max_key_len = non_kwsplat
        .iter()
        .map(|p| p.key_char_len)
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
    !adjacent_pairs_share_line(non_kwsplat.iter().copied())
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
        let (rocket_styles, colon_styles, last_arg_style) =
            if let Some((rocket_styles, colon_styles, last_arg_style)) =
                parse_enforced_style_alias(config)
            {
                (rocket_styles, colon_styles, last_arg_style)
            } else {
                (
                    parse_styles(config, "EnforcedHashRocketStyle", "key"),
                    parse_styles(config, "EnforcedColonStyle", "key"),
                    config
                        .get_str("EnforcedLastArgumentHashStyle", "always_inspect")
                        .to_string(),
                )
            };
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
        if should_ignore_last_argument_hash(
            is_last_argument_hash,
            is_keyword_hash,
            last_arg_style.as_str(),
        ) {
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
    fn separator_always_ignore_inline_first_pair_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/always_ignore_separator_inline_first_pair_offense.rb"
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
    fn ignore_explicit_block_argument_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/offense.ignore_explicit.rb"
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
    fn table_ignore_implicit_inline_first_pair_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/table_ignore_implicit_inline_first_pair_offense.rb"
            ),
            variant_config("table", "table", "ignore_implicit"),
        );
    }

    #[test]
    fn table_multibyte_key_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/table_multibyte_key_no_offense.rb"
            ),
            variant_config("table", "table", "ignore_implicit"),
        );
    }

    #[test]
    fn table_ignore_implicit_shared_line_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/table_ignore_implicit_shared_line_no_offense.rb"
            ),
            variant_config("table", "table", "ignore_implicit"),
        );
    }

    #[test]
    fn separator_always_ignore_mixed_newline_values_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/always_ignore_separator_mixed_newline_values_no_offense.rb"
            ),
            variant_config("separator", "separator", "always_ignore"),
        );
    }

    #[test]
    fn separator_always_ignore_first_value_newline_equal_key_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/always_ignore_separator_first_value_newline_equal_key_offense.rb"
            ),
            variant_config("separator", "separator", "always_ignore"),
        );
    }

    #[test]
    fn separator_always_ignore_value_omission_first_pair_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &HashAlignment,
            include_bytes!(
                "../../../tests/fixtures/cops/layout/hash_alignment/always_ignore_separator_value_omission_first_pair_no_offense.rb"
            ),
            variant_config("separator", "separator", "always_ignore"),
        );
    }

    #[test]
    fn enforced_style_alias_is_respected_for_variant_checks() {
        use crate::testutil::run_cop_full_with_config;
        use std::collections::HashMap;

        let config = CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".into(),
                serde_yml::Value::String("separator, separator, always_ignore".into()),
            )]),
            ..CopConfig::default()
        };
        let src = b"data = {\n  aa: 0,\n  b: 1,\n}\n";
        let diags = run_cop_full_with_config(&HashAlignment, src, config);
        assert_eq!(
            diags.len(),
            1,
            "bundled EnforcedStyle alias should configure separator alignment"
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
