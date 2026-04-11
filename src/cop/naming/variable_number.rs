use crate::cop::shared::node_type::{
    CLASS_VARIABLE_AND_WRITE_NODE, CLASS_VARIABLE_OPERATOR_WRITE_NODE,
    CLASS_VARIABLE_OR_WRITE_NODE, CLASS_VARIABLE_WRITE_NODE, DEF_NODE, FOR_NODE,
    GLOBAL_VARIABLE_AND_WRITE_NODE, GLOBAL_VARIABLE_OPERATOR_WRITE_NODE,
    GLOBAL_VARIABLE_OR_WRITE_NODE, GLOBAL_VARIABLE_WRITE_NODE, INSTANCE_VARIABLE_AND_WRITE_NODE,
    INSTANCE_VARIABLE_OPERATOR_WRITE_NODE, INSTANCE_VARIABLE_OR_WRITE_NODE,
    INSTANCE_VARIABLE_WRITE_NODE, LOCAL_VARIABLE_AND_WRITE_NODE,
    LOCAL_VARIABLE_OPERATOR_WRITE_NODE, LOCAL_VARIABLE_OR_WRITE_NODE, LOCAL_VARIABLE_WRITE_NODE,
    MULTI_WRITE_NODE, REQUIRED_PARAMETER_NODE,
};
use ruby_prism::Visit;

use crate::cop::{Cop, CopConfig};
use crate::diagnostic::Diagnostic;
use crate::parse::codemap::CodeMap;
use crate::parse::source::SourceFile;

/// FN=160 investigation: nitrocop only handled simple write nodes (e.g.
/// `LocalVariableWriteNode`) but missed compound assignment variants:
/// or-write (`||=`), and-write (`&&=`), operator-write (`+=`, `-=`, etc.),
/// and multi-assignment target nodes. All 16 missing node types have a
/// `.name()` method returning the variable name, same as the write nodes.
/// Fix: register all 16 additional node types and handle them identically.
///
/// ## Corpus investigation (2026-03-10)
///
/// Corpus oracle reported FP=32. All 32 FPs from empty symbols like `:''"`,
/// `:""`  used as hash keys, symbol arguments, etc. Root cause: RuboCop's
/// Parser gem creates `dsym` (dynamic symbol) for empty symbols, not `sym`.
/// The VariableNumber cop only has `on_sym`, NOT `on_dsym`, so RuboCop
/// never checks empty symbols. In Prism, empty symbols are `SymbolNode`
/// with an empty `unescaped()` value, so nitrocop was processing them.
/// Fix: skip empty names early in `check_number_style` (`!has_digit || name.is_empty()`).
///
/// ## Corpus investigation (2026-03-11)
///
/// Corpus oracle reported FP=0, FN=1. No example locations available.
/// The cop handles all variable write/compound-write/target node types,
/// RequiredParameterNode, DefNode (method names), and SymbolNode. RuboCop's
/// `on_arg` covers all parameter types, but optional/keyword/rest/block
/// parameters are not checked in RuboCop's VariableNumber cop (it only has
/// `on_arg`, not `on_optarg`/`on_kwarg`/`on_kwoptarg`/`on_restarg`/`on_blockarg`).
/// FN=1 is likely a corpus artifact (CI file discovery, encoding, or stale cache)
/// given 16,625 matches with 99.99% match rate. Local `check-cop.py --rerun`
/// needed to confirm.
///
/// ## Corpus fix (2026-03-13)
///
/// Corpus oracle reported FN=1 (confirmed fresh, not stale). Root cause:
/// the implicit-param exemption (`_1`, `_2`, etc.) was applied after sigil
/// stripping, so `@_1`, `@@_1`, `$_1` were incorrectly exempted. RuboCop's
/// `\A_\d+\z` implicit_param regex is applied to the FULL name including
/// sigils (`@_1` starts with `@`, not `_`, so it doesn't match). Fix: only
/// apply the implicit-param exemption to bare names (local variables and
/// parameters), not to sigiled variables.
///
/// ## Corpus investigation (2026-03-14)
///
/// Corpus oracle reported FP=0, FN=1 on hexapdf test_serializer.rb:101.
/// The offense is on `"":` (empty string hash key). With TargetRubyVersion: 4.0
/// (the corpus baseline), Parser gem treats `"":` as `:sym` (not `:dsym`),
/// causing RuboCop's `on_sym` to fire. The normalcase regex doesn't match
/// empty strings, so it flags them.
///
/// Fix: stop skipping empty names in `check_number_style`, but only for
/// hash-key symbols (no colon-prefix opening in Prism). Standalone empty
/// symbols (`:""`, `:''`) still have `:dsym` in Parser gem and are not
/// checked by RuboCop, so we skip those by checking `opening_loc` for a
/// colon prefix.
///
/// ## Corpus investigation (2026-03-14) — batch 2
///
/// Corpus oracle reported FP=1 on opal/opal `$$` global variable.
/// Root cause: `trim_start_matches('$')` strips BOTH `$` chars from `$$`,
/// leaving empty bare name `""`. The empty name fails the normalcase regex.
/// RuboCop doesn't fire on `$$` because Parser gem handles it differently.
/// Fix: skip variables with empty bare names after sigil stripping.
///
/// ## Corpus investigation (2026-03-23) — extended corpus
///
/// Extended corpus reported FP=39 across 2 repos. All FPs from pattern matching
/// variable bindings (`in [a_1, b_2]`, `value => result_1`, `obj => { key: val_1 }`).
/// In Parser gem, pattern matching creates `match_var` nodes, so `on_lvasgn` never
/// fires. In Prism, the same syntax creates `LocalVariableTargetNode`, which was
/// registered as an interested node type. Fix: removed all `*TargetNode` types from
/// interested_node_types and instead handle them through `MultiWriteNode` (multi-
/// assignment), `ForNode` (for-loop), and `RescueNode` (rescue exception variable),
/// which are the only non-pattern-matching contexts where target nodes appear.
///
/// ## Corpus investigation (2026-03-23) — extended corpus, batch 2
///
/// Extended corpus reported FP=1 on `halostatue__color__3299b65` for
/// `weight => k_1:, k_2:, k_l:` (pattern matching deconstruction).
/// In Prism, hash pattern keys like `k_1:` are SymbolNode inside HashPatternNode.
/// In Parser gem, these become `match_var` nodes, so RuboCop's `on_sym` never fires.
/// The previous fix removed `*TargetNode` types but still processed SymbolNode in
/// `check_node`, which has no parent context to detect pattern matching.
/// Fix: moved symbol checking from `check_node` to the `check_source` visitor, which
/// overrides `visit_hash_pattern_node` to skip SymbolNode keys while still visiting
/// assoc values.
///
/// ## Variant style fix (2026-04-05) — snake_case and non_integer
///
/// Variant styles had divergence: snake_case had 45 FN, non_integer had 61 FP + 60 FN.
/// Root cause: RuboCop's `\A\d+\z` regex alternative (for all-digit names like `:"42"`)
/// checks the FULL name including sigils. Nitrocop strips sigils before checking, so
/// `$0` became bare `"0"` which incorrectly matched the all-digits pattern as valid.
/// In RuboCop, `$0` starts with `$` so `\A\d+\z` doesn't match → offense.
/// Fix: `is_valid_snake_case` and `is_valid_non_integer` now accept `is_bare_name`
/// and only apply the all-digits exemption for truly bare names (locals, symbols,
/// methods), not for sigil-stripped variables.
///
/// ## Variant style fix (2026-04-06) — remaining FN in non_integer and snake_case
///
/// After the sigil fix, remaining divergence: non_integer 61 FP + 3 FN,
/// snake_case 0 FP + 1 FN.
///
/// **FN fix 1: Nested multi-assignment targets.** `(a,(b1,b2)),c = [...]`
/// creates `MultiTargetNode` inside `MultiWriteNode`. The previous code only
/// processed immediate targets of `MultiWriteNode`, not nested
/// `MultiTargetNode` children. `check_target_variable` now recursively
/// descends into `MultiTargetNode` to find all target variables. This fixes
/// 2 FN in jruby test_assignment.rb under non_integer.
///
/// **FN fix 2: Hash pattern keys with explicit values.** In `in { md5: String }`,
/// Parser gem creates `:sym` for the key, so RuboCop's `on_sym` fires. In
/// `in { k_1: }` (bare binding), Parser creates `match_var`, so `on_sym`
/// doesn't fire. The previous `visit_hash_pattern_node` skipped ALL keys.
/// Now it only skips keys whose value is `ImplicitNode` (bare bindings) and
/// visits keys with explicit values. This fixes 1 FN in danbooru under both
/// non_integer and snake_case.
///
/// ## Variant style fix (2026-04-07) — remaining 61 FP in non_integer
///
/// All 61 FPs from jruby repo. Two root causes:
///
/// **FP fix 1: Non-UTF-8 encoding files (59 FPs).** Files with encoding
/// magic comments like `# coding: US-ASCII` or `# encoding:windows-1252`
/// cause `Prism::Translation::Parser` to crash or produce fatal syntax
/// errors. RuboCop catches the crash and reports 0 offenses for the file.
/// Nitrocop's native Prism parser handles these files fine, producing
/// offenses that are FPs relative to RuboCop. Fix: detect non-UTF-8
/// encoding magic comments in `check_node`/`check_source` and skip the
/// file entirely. UTF-8 and binary/ASCII-8BIT encodings are still
/// processed normally (RuboCop handles those without crashing).
///
/// **FP fix 2: `%s()` empty symbols (2 FPs).** `%s()` creates an empty
/// symbol `:""`'. Parser gem treats `%s()` as `:dsym` (dynamic symbol),
/// so RuboCop's `on_sym` never fires. Non-empty `%s(foo)` is `:sym` and
/// IS checked. Fix: in `visit_symbol_node`, also skip empty symbols whose
/// opening starts with `%s` (not just `:`-prefixed standalone symbols).
///
/// ## Variant fix (2026-04-08) — all variants, US-ASCII encoding
///
/// All three variants (default 4 FN, snake_case 14 FN, non_integer 1,317 FN)
/// had FNs from files with `# encoding: US-ASCII` or `# coding: us-ascii`
/// magic comments. The `has_non_utf8_encoding_comment` function was skipping
/// these files, but US-ASCII is a strict 7-bit subset of UTF-8. RuboCop's
/// `Prism::Translation::Parser` handles US-ASCII files without crashing and
/// reports offenses normally. Fix: add US-ASCII to the allow-list alongside
/// UTF-8 and binary/ASCII-8BIT.
///
/// **Known residual FP (31 snake_case, 52 non_integer):** jruby's
/// `test/mri/ruby/test_regexp.rb` is a US-ASCII file containing `\u`
/// Unicode escapes inside regex literals (e.g. `/\u3042/`). RuboCop's
/// `Prism::Translation::Parser` crashes with `RegexpError` on this file
/// (multibyte bytes incompatible with US-ASCII encoding), and the corpus
/// `rescue_parser_crashes.rb` monkey-patch catches the crash → 0 offenses.
/// Nitrocop's native Prism handles it fine → FP. This is the only known
/// crash file in ~5,500 corpus repos. A correct fix would use Prism's AST
/// to detect `RegularExpressionNode` with `\u` escapes in US-ASCII files,
/// rather than raw-byte heuristics which are fragile around `/` ambiguity.
///
/// ## Variant fix (2026-04-11) — non_integer 3 FN in windows-1251 file
///
/// 3 FN in jruby `test/mri/ruby/enc/test_windows_1251.rb` (lines 7, 9, 10):
/// `test_windows_1251` (method), `c1`, `c2` (variables). The file declares
/// `# encoding:windows-1251` but contains only ASCII bytes. The previous
/// `has_non_utf8_encoding_comment` check blanket-skipped ALL files with
/// non-UTF-8 encoding declarations, but Translation::Parser only crashes
/// when actual non-ASCII bytes (>= 0x80) are present. ASCII-only files
/// are parsed fine regardless of declared encoding.
/// Fix: renamed to `has_non_utf8_encoding_with_non_ascii_bytes` and added
/// a check for bytes >= 0x80. Files with non-UTF-8 declarations but only
/// ASCII content are now processed normally.
pub struct VariableNumber;

const DEFAULT_ALLOWED: &[&str] = &[
    "TLS1_1",
    "TLS1_2",
    "capture3",
    "iso8601",
    "rfc1123_date",
    "rfc822",
    "rfc2822",
    "rfc3339",
    "x86_64",
];

/// Check if a file has a non-UTF-8 encoding magic comment AND contains
/// non-ASCII bytes. Both conditions must be true for Translation::Parser
/// to crash. Files declaring a non-UTF-8 encoding but containing only
/// ASCII bytes (all bytes < 0x80) are parsed fine because ASCII is a
/// valid subset of every encoding.
fn has_non_utf8_encoding_with_non_ascii_bytes(bytes: &[u8]) -> bool {
    // Scan up to 3 lines (shebang + possible encoding comment)
    let mut start = 0;
    for _ in 0..3 {
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| start + p)
            .unwrap_or(bytes.len());
        let line = &bytes[start..end];
        // Skip leading whitespace
        let trimmed: Vec<u8> = line.iter().copied().filter(|b| *b != b'\r').collect();
        if trimmed.starts_with(b"#") {
            let lower: Vec<u8> = trimmed.iter().map(|b| b.to_ascii_lowercase()).collect();
            // Look for encoding/coding keywords in the comment
            if let Some(pos) = find_subsequence(&lower, b"encoding")
                .or_else(|| find_subsequence(&lower, b"coding"))
            {
                // Extract the encoding value after the keyword
                let after = &lower[pos..];
                // Skip the keyword and any separator (: = etc.)
                let value_start = after
                    .iter()
                    .position(|&b| b == b':' || b == b'=')
                    .map(|p| p + 1)
                    .unwrap_or(after.len());
                let value = &after[value_start..];
                // Trim whitespace and extract the encoding name
                let value_trimmed: Vec<u8> =
                    value.iter().copied().skip_while(|b| *b == b' ').collect();
                // Take alphanumeric + hyphens + underscores (for names like utf-8)
                let enc_end = value_trimmed
                    .iter()
                    .position(|b| !b.is_ascii_alphanumeric() && *b != b'-' && *b != b'_')
                    .unwrap_or(value_trimmed.len());
                let enc_name = &value_trimmed[..enc_end];
                // UTF-8 variants are fine
                if enc_name == b"utf"
                    || enc_name == b"utf8"
                    || enc_name.starts_with(b"utf-8")
                    || enc_name.starts_with(b"utf_8")
                {
                    return false;
                }
                // binary / ASCII-8BIT are fine — RuboCop's Translation::Parser
                // handles them without crashing. Common in files dealing with
                // binary data (packetfu, puppetlabs, etc.).
                if enc_name == b"binary"
                    || enc_name.starts_with(b"ascii-8bit")
                    || enc_name.starts_with(b"ascii_8bit")
                {
                    return false;
                }
                // US-ASCII is fine — it's a strict subset of UTF-8, so
                // Translation::Parser handles it without crashing. RuboCop
                // reports offenses normally for US-ASCII files.
                if enc_name == b"us-ascii" || enc_name == b"ascii" {
                    return false;
                }
                // Other non-UTF-8 encodings (windows-1252, iso-8859-1,
                // etc.) cause Translation::Parser crashes, but ONLY if the
                // file contains actual non-ASCII bytes. ASCII-only files are
                // parsed fine regardless of declared encoding.
                if !enc_name.is_empty() {
                    return bytes.iter().any(|&b| b >= 0x80);
                }
            }
        }
        start = end + 1;
        if start >= bytes.len() {
            break;
        }
    }
    false
}

/// Find the position of a subsequence in a byte slice.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

impl Cop for VariableNumber {
    fn name(&self) -> &'static str {
        "Naming/VariableNumber"
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CLASS_VARIABLE_AND_WRITE_NODE,
            CLASS_VARIABLE_OPERATOR_WRITE_NODE,
            CLASS_VARIABLE_OR_WRITE_NODE,
            CLASS_VARIABLE_WRITE_NODE,
            DEF_NODE,
            FOR_NODE,
            GLOBAL_VARIABLE_AND_WRITE_NODE,
            GLOBAL_VARIABLE_OPERATOR_WRITE_NODE,
            GLOBAL_VARIABLE_OR_WRITE_NODE,
            GLOBAL_VARIABLE_WRITE_NODE,
            INSTANCE_VARIABLE_AND_WRITE_NODE,
            INSTANCE_VARIABLE_OPERATOR_WRITE_NODE,
            INSTANCE_VARIABLE_OR_WRITE_NODE,
            INSTANCE_VARIABLE_WRITE_NODE,
            LOCAL_VARIABLE_AND_WRITE_NODE,
            LOCAL_VARIABLE_OPERATOR_WRITE_NODE,
            LOCAL_VARIABLE_OR_WRITE_NODE,
            LOCAL_VARIABLE_WRITE_NODE,
            MULTI_WRITE_NODE,
            REQUIRED_PARAMETER_NODE,
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
        // Skip files with non-UTF-8 encoding magic comments. RuboCop's
        // Translation::Parser crashes or produces fatal syntax errors on
        // these files, so no Naming cops run → 0 offenses.
        if has_non_utf8_encoding_with_non_ascii_bytes(source.as_bytes()) {
            return;
        }

        let enforced_style = config.get_str("EnforcedStyle", "normalcase");
        let check_method_names = config.get_bool("CheckMethodNames", true);
        let allowed = config.get_string_array("AllowedIdentifiers");
        let allowed_patterns = config.get_string_array("AllowedPatterns");

        let allowed_ids: Vec<String> =
            allowed.unwrap_or_else(|| DEFAULT_ALLOWED.iter().map(|s| s.to_string()).collect());

        let allowed_pats: Vec<String> = allowed_patterns.unwrap_or_default();

        // Extract (name_bytes, location) from any variable write/compound-write/target node
        let var_info: Option<(&[u8], ruby_prism::Location<'_>)> =
            // Local variables (no sigil to strip)
            if let Some(n) = node.as_local_variable_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_local_variable_or_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_local_variable_and_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_local_variable_operator_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            }
            // Instance variables (strip @)
            else if let Some(n) = node.as_instance_variable_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_instance_variable_or_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_instance_variable_and_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_instance_variable_operator_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            }
            // Class variables (strip @@)
            else if let Some(n) = node.as_class_variable_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_class_variable_or_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_class_variable_and_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_class_variable_operator_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            }
            // Global variables (strip $)
            else if let Some(n) = node.as_global_variable_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_global_variable_or_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_global_variable_and_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else if let Some(n) = node.as_global_variable_operator_write_node() {
                Some((n.name().as_slice(), n.name_loc()))
            } else {
                None
            };

        if let Some((name_bytes, loc)) = var_info {
            let name_str = std::str::from_utf8(name_bytes).unwrap_or("");
            // Strip sigils: @@ for class vars, @ for instance vars, $ for globals
            let bare = name_str.trim_start_matches('@').trim_start_matches('$');
            let is_bare = bare.len() == name_str.len(); // no sigil stripped
            // Skip variables whose entire name IS the sigil (e.g., $$ → bare "").
            // RuboCop's Parser gem doesn't produce gvasgn for $$ in the same way,
            // so these are never checked.
            if bare.is_empty() {
                return;
            }
            if !is_allowed(bare, &allowed_ids, &allowed_pats) {
                if let Some(diag) = check_number_style(
                    self,
                    source,
                    bare,
                    &loc,
                    enforced_style,
                    "variable",
                    is_bare,
                ) {
                    diagnostics.push(diag);
                }
            }
            return;
        }

        // Check method names (def)
        if check_method_names {
            if let Some(def_node) = node.as_def_node() {
                let name = def_node.name().as_slice();
                let name_str = std::str::from_utf8(name).unwrap_or("");
                if !is_allowed(name_str, &allowed_ids, &allowed_pats) {
                    if let Some(diag) = check_number_style(
                        self,
                        source,
                        name_str,
                        &def_node.name_loc(),
                        enforced_style,
                        "method name",
                        true,
                    ) {
                        diagnostics.push(diag);
                    }
                }
            }
        }

        // Check method parameters
        if let Some(param) = node.as_required_parameter_node() {
            let name = param.name().as_slice();
            let name_str = std::str::from_utf8(name).unwrap_or("");
            if !is_allowed(name_str, &allowed_ids, &allowed_pats) {
                if let Some(diag) = check_number_style(
                    self,
                    source,
                    name_str,
                    &param.location(),
                    enforced_style,
                    "variable",
                    true,
                ) {
                    diagnostics.push(diag);
                }
            }
        }

        // Multi-assignment targets: `val_1, val_2 = arr`
        // In Prism, *TargetNode types appear in both multi-assignment and pattern matching.
        // RuboCop's on_lvasgn fires for multi-assignment (Parser creates lvasgn children in
        // mlhs), but NOT for pattern matching (Parser creates match_var nodes). By handling
        // only MultiWriteNode targets here (instead of registering *TargetNode types
        // directly), we correctly skip pattern matching variable bindings.
        if let Some(mw) = node.as_multi_write_node() {
            for target in mw.lefts().iter() {
                self.check_target_variable(
                    source,
                    &target,
                    enforced_style,
                    &allowed_ids,
                    &allowed_pats,
                    diagnostics,
                );
            }
            // Check the rest target (splat) if present
            if let Some(rest) = mw.rest() {
                if let Some(splat) = rest.as_splat_node() {
                    if let Some(expr) = splat.expression() {
                        self.check_target_variable(
                            source,
                            &expr,
                            enforced_style,
                            &allowed_ids,
                            &allowed_pats,
                            diagnostics,
                        );
                    }
                }
            }
            for target in mw.rights().iter() {
                self.check_target_variable(
                    source,
                    &target,
                    enforced_style,
                    &allowed_ids,
                    &allowed_pats,
                    diagnostics,
                );
            }
        }

        // For-loop index: `for val_1 in collection`
        if let Some(for_node) = node.as_for_node() {
            let index = for_node.index();
            self.check_target_variable(
                source,
                &index,
                enforced_style,
                &allowed_ids,
                &allowed_pats,
                diagnostics,
            );
        }
    }

    fn check_source(
        &self,
        source: &SourceFile,
        parse_result: &ruby_prism::ParseResult<'_>,
        _code_map: &CodeMap,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        _corrections: Option<&mut Vec<crate::correction::Correction>>,
    ) {
        // Skip files with non-UTF-8 encoding magic comments (see check_node).
        if has_non_utf8_encoding_with_non_ascii_bytes(source.as_bytes()) {
            return;
        }

        // This visitor handles two cases that require tree-walking context:
        //
        // 1. Rescue exception variables (`rescue => error_2`): Prism's Visit trait
        //    calls visit_rescue_node directly from visit_begin_node, bypassing
        //    visit_branch_node_enter, so check_node never sees RescueNode.
        //
        // 2. Symbol checking: In pattern matching (`value => k_1:, k_2:`), Prism
        //    creates SymbolNode keys inside HashPatternNode. Parser gem creates
        //    match_var nodes instead, so RuboCop's on_sym never fires. The visitor
        //    skips SymbolNode children of HashPatternNode to avoid false positives.
        let enforced_style = config.get_str("EnforcedStyle", "normalcase");
        let check_symbols = config.get_bool("CheckSymbols", true);
        let allowed = config.get_string_array("AllowedIdentifiers");
        let allowed_patterns = config.get_string_array("AllowedPatterns");
        let allowed_ids: Vec<String> =
            allowed.unwrap_or_else(|| DEFAULT_ALLOWED.iter().map(|s| s.to_string()).collect());
        let allowed_pats: Vec<String> = allowed_patterns.unwrap_or_default();

        let mut visitor = VariableNumberVisitor {
            cop: self,
            source,
            enforced_style,
            check_symbols,
            allowed_ids: &allowed_ids,
            allowed_pats: &allowed_pats,
            diagnostics,
        };
        visitor.visit(&parse_result.node());
    }
}

/// Visitor that handles rescue exception variables and symbol checking.
///
/// Rescue: Prism's visit_begin_node calls visit_rescue_node directly,
/// bypassing visit_branch_node_enter, so RescueNode is invisible to check_node.
///
/// Symbols: In pattern matching (`value => k_1:`), Prism creates SymbolNode
/// keys inside HashPatternNode. Parser gem creates match_var nodes instead,
/// so RuboCop's on_sym never fires. This visitor skips HashPatternNode
/// subtrees entirely for symbol checking to match RuboCop behavior.
struct VariableNumberVisitor<'a> {
    cop: &'a VariableNumber,
    source: &'a SourceFile,
    enforced_style: &'a str,
    check_symbols: bool,
    allowed_ids: &'a [String],
    allowed_pats: &'a [String],
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<'pr> ruby_prism::Visit<'pr> for VariableNumberVisitor<'_> {
    fn visit_rescue_node(&mut self, node: &ruby_prism::RescueNode<'pr>) {
        if let Some(reference) = node.reference() {
            self.cop.check_target_variable(
                self.source,
                &reference,
                self.enforced_style,
                self.allowed_ids,
                self.allowed_pats,
                self.diagnostics,
            );
        }
        // Continue walking children (subsequent rescue clauses, etc.)
        ruby_prism::visit_rescue_node(self, node);
    }

    fn visit_symbol_node(&mut self, node: &ruby_prism::SymbolNode<'pr>) {
        if !self.check_symbols {
            return;
        }
        let name = node.unescaped();
        let name_str = std::str::from_utf8(name).unwrap_or("");
        // Skip standalone empty symbols (:'' and :""). In Parser gem
        // with TargetRubyVersion >= 4.0, these are :dsym (not :sym),
        // so RuboCop's on_sym never fires. Only hash-key empty symbols
        // ("": val) become :sym in Parser 4.0. In Prism, standalone
        // symbols have a colon-prefix opening, while hash-key symbols don't.
        if name_str.is_empty() {
            // Skip standalone empty symbols (:'' and :"") — Parser gem creates
            // :dsym, so RuboCop's on_sym never fires.
            // Also skip %s() empty symbols — Parser gem creates :dsym for these
            // too. Non-empty %s(foo) IS :sym and IS checked.
            let is_standalone = node
                .opening_loc()
                .is_some_and(|loc| loc.as_slice().starts_with(b":"));
            let is_percent_s = node
                .opening_loc()
                .is_some_and(|loc| loc.as_slice().starts_with(b"%s"));
            if is_standalone || is_percent_s {
                return;
            }
        }
        if !is_allowed(name_str, self.allowed_ids, self.allowed_pats) {
            // For empty-value symbols like :"", value_loc() may return
            // a zero-length range at an incorrect offset. Use the full
            // symbol location instead when value_loc has zero length.
            let loc = match node.value_loc() {
                Some(vloc) if !vloc.as_slice().is_empty() => vloc,
                _ => node.location(),
            };
            if let Some(diag) = check_number_style(
                self.cop,
                self.source,
                name_str,
                &loc,
                self.enforced_style,
                "symbol",
                true,
            ) {
                self.diagnostics.push(diag);
            }
        }
        // SymbolNode is a leaf — no children to visit.
    }

    fn visit_hash_pattern_node(&mut self, node: &ruby_prism::HashPatternNode<'pr>) {
        // In pattern matching, Prism creates SymbolNode keys inside HashPatternNode.
        // Parser gem behavior differs based on whether the key has an explicit value:
        //
        // - `in { k_1: }` (bare binding): Parser creates match_var, NOT sym.
        //   RuboCop's on_sym never fires. In Prism, the value is an ImplicitNode.
        //   → Skip the key symbol.
        //
        // - `in { md5: String }` (key with value): Parser creates sym(:md5).
        //   RuboCop's on_sym DOES fire. In Prism, the value is NOT ImplicitNode.
        //   → Visit the key symbol.
        for assoc in node.elements().iter() {
            if let Some(assoc_node) = assoc.as_assoc_node() {
                let value = assoc_node.value();
                // If the value is ImplicitNode, this is a bare binding (match_var
                // in Parser) — skip the key. Otherwise, the key is a real symbol.
                if value.as_implicit_node().is_none() {
                    let key = assoc_node.key();
                    self.visit(&key);
                }
                self.visit(&value);
            } else if let Some(splat) = assoc.as_assoc_splat_node() {
                // **rest pattern — visit the expression
                if let Some(value) = splat.value() {
                    self.visit(&value);
                }
            }
        }
        // Visit the rest node if present (e.g., `in { **rest }`)
        if let Some(rest) = node.rest() {
            self.visit(&rest);
        }
    }
}

impl VariableNumber {
    /// Check a target variable node from MultiWriteNode, MultiTargetNode, or ForNode.
    /// Handles LocalVariableTargetNode, InstanceVariableTargetNode,
    /// ClassVariableTargetNode, GlobalVariableTargetNode, and recursively
    /// handles nested MultiTargetNode (e.g., `(a,(b1,b2)),c = ...`).
    fn check_target_variable(
        &self,
        source: &SourceFile,
        target: &ruby_prism::Node<'_>,
        enforced_style: &str,
        allowed_ids: &[String],
        allowed_pats: &[String],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // Nested destructuring: (a,(b1,b2)) creates a MultiTargetNode
        // containing further target nodes. Recurse into it.
        if let Some(mt) = target.as_multi_target_node() {
            for child in mt.lefts().iter() {
                self.check_target_variable(
                    source,
                    &child,
                    enforced_style,
                    allowed_ids,
                    allowed_pats,
                    diagnostics,
                );
            }
            if let Some(rest) = mt.rest() {
                if let Some(splat) = rest.as_splat_node() {
                    if let Some(expr) = splat.expression() {
                        self.check_target_variable(
                            source,
                            &expr,
                            enforced_style,
                            allowed_ids,
                            allowed_pats,
                            diagnostics,
                        );
                    }
                }
            }
            for child in mt.rights().iter() {
                self.check_target_variable(
                    source,
                    &child,
                    enforced_style,
                    allowed_ids,
                    allowed_pats,
                    diagnostics,
                );
            }
            return;
        }

        let (name_bytes, loc) = if let Some(n) = target.as_local_variable_target_node() {
            (n.name().as_slice(), n.location())
        } else if let Some(n) = target.as_instance_variable_target_node() {
            (n.name().as_slice(), n.location())
        } else if let Some(n) = target.as_class_variable_target_node() {
            (n.name().as_slice(), n.location())
        } else if let Some(n) = target.as_global_variable_target_node() {
            (n.name().as_slice(), n.location())
        } else {
            return;
        };

        let name_str = std::str::from_utf8(name_bytes).unwrap_or("");
        let bare = name_str.trim_start_matches('@').trim_start_matches('$');
        let is_bare = bare.len() == name_str.len();
        if bare.is_empty() {
            return;
        }
        if !is_allowed(bare, allowed_ids, allowed_pats) {
            if let Some(diag) = check_number_style(
                self,
                source,
                bare,
                &loc,
                enforced_style,
                "variable",
                is_bare,
            ) {
                diagnostics.push(diag);
            }
        }
    }
}

fn is_allowed(name: &str, allowed_ids: &[String], allowed_pats: &[String]) -> bool {
    if allowed_ids.iter().any(|a| a == name) {
        return true;
    }
    for pattern in allowed_pats {
        if let Ok(re) = regex::Regex::new(pattern) {
            if re.is_match(name) {
                return true;
            }
        }
    }
    false
}

fn check_number_style(
    cop: &VariableNumber,
    source: &SourceFile,
    name: &str,
    loc: &ruby_prism::Location<'_>,
    enforced_style: &str,
    identifier_type: &str,
    is_bare_name: bool,
) -> Option<Diagnostic> {
    // Skip names without digits — the style regex always matches non-empty
    // strings ending with a non-digit character. But empty names (e.g. `:""`
    // from `"":` hash key syntax) DON'T match any style regex. With
    // TargetRubyVersion >= 4.0, Parser gem creates :sym for `"":` (instead
    // of :dsym in older versions), so RuboCop's on_sym fires and the regex
    // check fails on the empty string → offense. Prism always creates
    // SymbolNode for these, so we match RuboCop 4.0 behavior by not skipping
    // empty names.
    let has_digit = name.bytes().any(|b| b.is_ascii_digit());
    if !has_digit && !name.is_empty() {
        return None;
    }

    // Implicit params like _1, _2 are always allowed, but only for bare names
    // (local variables, parameters). Instance/class/global variables like @_1
    // are NOT implicit params — RuboCop's regex checks the full name including
    // sigil, so \A_\d+\z won't match @_1.
    if is_bare_name && name.starts_with('_') && name[1..].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    // RuboCop checks the END of the identifier against a format regex.
    // The name is checked INCLUDING trailing `?` or `!` suffixes — these
    // count as non-digit characters that satisfy the \D alternative.
    //
    // normalcase:  /(?:\D|[^_\d]\d+|\A\d+)\z/ — trailing digits must NOT be preceded by _
    // snake_case:  /(?:\D|_\d+|\A\d+)\z/      — trailing digits MUST be preceded by _
    // non_integer: /(\D|\A\d+)\z/              — no trailing digits allowed
    let valid = match enforced_style {
        "normalcase" => is_valid_normalcase(name),
        "snake_case" => is_valid_snake_case(name, is_bare_name),
        "non_integer" => is_valid_non_integer(name, is_bare_name),
        _ => true,
    };

    if !valid {
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        return Some(cop.diagnostic(
            source,
            line,
            column,
            format!("Use {enforced_style} for {identifier_type} numbers."),
        ));
    }

    None
}

/// normalcase: /(?:\D|[^_\d]\d+|\A\d+)\z/
/// Valid if: ends with non-digit, OR ends with digits NOT preceded by _, OR is all digits.
/// Empty names are invalid (regex doesn't match empty string).
fn is_valid_normalcase(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let last = bytes[bytes.len() - 1];
    // Ends with non-digit → OK
    if !last.is_ascii_digit() {
        return true;
    }
    // Ends with digits. Find where the trailing digit run starts.
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    // If trailing digits span the whole string → OK (all digits)
    if i == 0 {
        return true;
    }
    // The character before the trailing digits must NOT be underscore
    bytes[i - 1] != b'_'
}

/// snake_case: /(?:\D|_\d+|\A\d+)\z/
/// Valid if: ends with non-digit, OR ends with digits preceded by _, OR is all digits.
/// Empty names are invalid (regex doesn't match empty string).
///
/// `is_bare_name` is false for sigiled variables (@, @@, $). RuboCop checks the
/// FULL name including sigils, so `\A\d+\z` never matches sigiled names like `$0`.
/// We check the bare name after stripping sigils, so we need `is_bare_name` to
/// avoid incorrectly treating all-digit bare names (e.g., `$0` → `"0"`) as valid.
fn is_valid_snake_case(name: &str, is_bare_name: bool) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let last = bytes[bytes.len() - 1];
    if !last.is_ascii_digit() {
        return true;
    }
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == 0 {
        // All digits in bare name. For bare names (locals, symbols, methods),
        // this is truly all-digit (e.g., :"42") → valid. For sigiled names,
        // RuboCop checks the full name (e.g., "$0") where \A\d+\z doesn't
        // match because of the sigil prefix → invalid.
        return is_bare_name;
    }
    // The character before the trailing digits MUST be underscore
    bytes[i - 1] == b'_'
}

/// non_integer: /(\D|\A\d+)\z/
/// Valid if: ends with non-digit, OR is all digits.
/// Empty names are invalid (regex doesn't match empty string).
///
/// `is_bare_name` is false for sigiled variables (@, @@, $). See
/// `is_valid_snake_case` for the rationale on the all-digits check.
fn is_valid_non_integer(name: &str, is_bare_name: bool) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let last = bytes[bytes.len() - 1];
    if !last.is_ascii_digit() {
        return true;
    }
    // Only valid if ALL digits AND bare name (no sigil prefix in original)
    is_bare_name && bytes.iter().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    crate::cop_fixture_tests!(VariableNumber, "cops/naming/variable_number");

    #[test]
    fn instance_var_implicit_param_name_is_offense() {
        // RuboCop's implicit_param regex (\A_\d+\z) only matches bare _1, not @_1.
        // So @_1 should be flagged as an offense in normalcase.
        let diags = crate::testutil::run_cop_full(&VariableNumber, b"@_1 = 1\n");
        assert_eq!(diags.len(), 1, "expected @_1 to be flagged");
    }

    #[test]
    fn class_var_implicit_param_name_is_offense() {
        let diags = crate::testutil::run_cop_full(&VariableNumber, b"@@_1 = 1\n");
        assert_eq!(diags.len(), 1, "expected @@_1 to be flagged");
    }

    #[test]
    fn global_var_implicit_param_name_is_offense() {
        let diags = crate::testutil::run_cop_full(&VariableNumber, b"$_1 = 1\n");
        assert_eq!(diags.len(), 1, "expected $_1 to be flagged");
    }

    #[test]
    fn rescue_variable_with_number() {
        let diags =
            crate::testutil::run_cop_full(&VariableNumber, b"begin\nrescue => error_2\nend\n");
        assert_eq!(
            diags.len(),
            1,
            "expected rescue variable error_2 to be flagged"
        );
    }

    #[test]
    fn local_var_implicit_param_is_no_offense() {
        // Bare _1 is an implicit param and should NOT be flagged
        let diags = crate::testutil::run_cop_full(&VariableNumber, b"_1 = 1\n");
        assert_eq!(diags.len(), 0, "expected _1 to NOT be flagged");
    }

    #[test]
    fn empty_hash_key_symbol_is_offense() {
        // With TargetRubyVersion >= 4.0, hash-key empty symbols ("": val)
        // are :sym in Parser gem, so RuboCop's on_sym fires and the normalcase
        // regex fails on empty strings. Prism creates SymbolNode without
        // colon opening for hash keys.
        let diags = crate::testutil::run_cop_full(&VariableNumber, b"{\"\":1}\n");
        assert_eq!(
            diags.len(),
            1,
            "expected hash-key empty symbol to be flagged"
        );
    }

    #[test]
    fn standalone_empty_symbol_is_no_offense() {
        // Standalone empty symbols (:'' and :"") are :dsym in Parser gem
        // (even with Ruby 4.0), so RuboCop's on_sym never fires.
        let diags = crate::testutil::run_cop_full(&VariableNumber, b":\"\"\n");
        assert_eq!(diags.len(), 0, "standalone :\"\" should NOT be flagged");
        let diags = crate::testutil::run_cop_full(&VariableNumber, b":''\n");
        assert_eq!(diags.len(), 0, "standalone :'' should NOT be flagged");
    }

    fn snake_case_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("snake_case".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    fn non_integer_config() -> crate::cop::CopConfig {
        let mut options = std::collections::HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("non_integer".to_string()),
        );
        crate::cop::CopConfig {
            options,
            ..crate::cop::CopConfig::default()
        }
    }

    #[test]
    fn dollar_zero_is_offense_under_snake_case() {
        // $0 under snake_case: RuboCop checks "$0" against /(?:\D|_\d+|\A\d+)\z/.
        // "$0" doesn't match \A\d+\z (starts with $), _\d+\z ($ is not _),
        // or \D\z (0 is digit) → invalid → offense.
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"$0 = 'myapp'\n",
            snake_case_config(),
        );
        assert_eq!(diags.len(), 1, "expected $0 to be flagged under snake_case");
    }

    #[test]
    fn dollar_zero_is_offense_under_non_integer() {
        // $0 under non_integer: same reasoning — sigil prevents \A\d+\z match.
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"$0 = 'myapp'\n",
            non_integer_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "expected $0 to be flagged under non_integer"
        );
    }

    #[test]
    fn dollar_zero_is_no_offense_under_normalcase() {
        // $0 under normalcase: [^_\d]\d+\z matches "$0" because $ is [^_\d].
        // After sigil stripping, bare "0" is all digits → also valid.
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"$0 = 'myapp'\n",
            crate::cop::CopConfig::default(),
        );
        assert_eq!(
            diags.len(),
            0,
            "expected $0 to NOT be flagged under normalcase"
        );
    }

    #[test]
    fn integer_symbol_valid_under_all_styles() {
        // :"42" and %i[1 2 3] should be valid under all styles.
        // The bare name is all-digits (truly, no sigil) → \A\d+\z matches.
        for config in [
            crate::cop::CopConfig::default(),
            snake_case_config(),
            non_integer_config(),
        ] {
            let diags = crate::testutil::run_cop_full_with_config(
                &VariableNumber,
                b":\"42\"\n",
                config.clone(),
            );
            assert_eq!(diags.len(), 0, "integer symbol :\"42\" should be valid");
        }
    }

    #[test]
    fn snake_case_flags_normalcase_names() {
        // Under snake_case, foo1 is an offense (digits not preceded by _)
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"foo1 = 1\n",
            snake_case_config(),
        );
        assert_eq!(diags.len(), 1, "expected foo1 flagged under snake_case");

        // Under snake_case, foo_1 is valid
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"foo_1 = 1\n",
            snake_case_config(),
        );
        assert_eq!(diags.len(), 0, "expected foo_1 valid under snake_case");
    }

    #[test]
    fn non_integer_flags_all_trailing_digits() {
        // Under non_integer, both foo1 and foo_1 are offenses
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"foo1 = 1\n",
            non_integer_config(),
        );
        assert_eq!(diags.len(), 1, "expected foo1 flagged under non_integer");

        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"foo_1 = 1\n",
            non_integer_config(),
        );
        assert_eq!(diags.len(), 1, "expected foo_1 flagged under non_integer");
    }

    #[test]
    fn nested_multi_assignment_under_non_integer() {
        // Nested multi-assignment: (a,(b1,b2)),c = [[1,2],3]
        // Under non_integer, b1 and b2 end with digits → offense.
        // Prism creates MultiTargetNode inside MultiWriteNode for nested patterns.
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"(a,(b1,b2)),c = [[1,2],3]\n",
            non_integer_config(),
        );
        assert_eq!(
            diags.len(),
            2,
            "expected b1 and b2 flagged in nested multi-assignment under non_integer"
        );
    }

    #[test]
    fn hash_pattern_key_with_value_is_checked() {
        // In `in { md5: String }`, the key :md5 is a real symbol (not a binding).
        // Parser gem creates :sym for it, so RuboCop's on_sym fires.
        // Under non_integer, md5 ends with digits → offense.
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"case obj\nin { md5: String }\n  nil\nend\n",
            non_integer_config(),
        );
        assert_eq!(
            diags.len(),
            1,
            "expected :md5 flagged in hash pattern under non_integer"
        );
    }

    #[test]
    fn hash_pattern_bare_binding_not_checked() {
        // In `in { k_1: }`, Parser gem creates match_var (not sym),
        // so RuboCop's on_sym never fires. Nitrocop should not flag it.
        let diags = crate::testutil::run_cop_full_with_config(
            &VariableNumber,
            b"case obj\nin { k_1: }\n  k_1\nend\n",
            non_integer_config(),
        );
        assert_eq!(
            diags.len(),
            0,
            "expected bare binding k_1: to NOT be flagged in hash pattern"
        );
    }

    #[test]
    fn percent_s_empty_symbol_not_checked() {
        // %s() creates an empty symbol. Parser gem treats it as :dsym,
        // so RuboCop's on_sym never fires. Non-empty %s(foo) IS checked.
        let diags = crate::testutil::run_cop_full(&VariableNumber, b"x = %s()\n");
        assert_eq!(diags.len(), 0, "expected %s() empty symbol NOT flagged");

        // Non-empty %s(foo_1) SHOULD be flagged
        let diags = crate::testutil::run_cop_full(&VariableNumber, b"x = %s(foo_1)\n");
        assert_eq!(diags.len(), 1, "expected %s(foo_1) to be flagged");
    }

    #[test]
    fn non_utf8_encoding_file_with_non_ascii_bytes_skipped() {
        // Files with non-UTF-8 encoding comments AND non-ASCII bytes should
        // be skipped. RuboCop's Translation::Parser crashes on these.
        let diags = crate::testutil::run_cop_full(
            &VariableNumber,
            b"# encoding:windows-1252\nfoo_1 = 1\n\x80\n",
        );
        assert_eq!(
            diags.len(),
            0,
            "expected windows-1252 file with non-ASCII bytes to be skipped"
        );

        // But ASCII-only files with non-UTF-8 encoding comments should NOT
        // be skipped — Translation::Parser handles them fine.
        let diags =
            crate::testutil::run_cop_full(&VariableNumber, b"# encoding:windows-1252\nfoo_1 = 1\n");
        assert_eq!(
            diags.len(),
            1,
            "expected ASCII-only windows-1252 file to NOT be skipped"
        );

        // UTF-8 encoding should NOT be skipped
        let diags =
            crate::testutil::run_cop_full(&VariableNumber, b"# encoding: utf-8\nfoo_1 = 1\n");
        assert_eq!(diags.len(), 1, "expected utf-8 file to NOT be skipped");

        // binary / ASCII-8BIT should NOT be skipped — RuboCop handles them fine
        let diags = crate::testutil::run_cop_full(
            &VariableNumber,
            b"# -*- coding: binary -*-\nfoo_1 = 1\n",
        );
        assert_eq!(diags.len(), 1, "expected binary file to NOT be skipped");

        let diags =
            crate::testutil::run_cop_full(&VariableNumber, b"# encoding: ASCII-8BIT\nfoo_1 = 1\n");
        assert_eq!(diags.len(), 1, "expected ASCII-8BIT file to NOT be skipped");

        // US-ASCII should NOT be skipped — it's a strict subset of UTF-8,
        // RuboCop's Parser handles it without crashing.
        let diags =
            crate::testutil::run_cop_full(&VariableNumber, b"# coding: US-ASCII\nfoo_1 = 1\n");
        assert_eq!(diags.len(), 1, "expected US-ASCII file to NOT be skipped");

        let diags = crate::testutil::run_cop_full(
            &VariableNumber,
            b"# -*- coding: us-ascii -*-\nfoo_1 = 1\n",
        );
        assert_eq!(diags.len(), 1, "expected us-ascii file to NOT be skipped");
    }

    #[test]
    fn non_integer_variant_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &VariableNumber,
            include_bytes!(
                "../../../tests/fixtures/cops/naming/variable_number/non_integer_offense.rb"
            ),
            non_integer_config(),
        );
    }

    #[test]
    fn non_integer_variant_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &VariableNumber,
            include_bytes!(
                "../../../tests/fixtures/cops/naming/variable_number/non_integer_no_offense.rb"
            ),
            non_integer_config(),
        );
    }

    // --- has_non_utf8_encoding_with_non_ascii_bytes unit tests ---

    #[test]
    fn encoding_us_ascii_not_skipped() {
        // US-ASCII is a subset of UTF-8 — RuboCop handles it fine
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# coding: US-ASCII\nfoo\n"
        ));
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# -*- coding: us-ascii -*-\nfoo\n"
        ));
    }

    #[test]
    fn encoding_detect_windows_1252_with_non_ascii() {
        // windows-1252 with non-ASCII bytes → skip
        assert!(has_non_utf8_encoding_with_non_ascii_bytes(
            b"# encoding:windows-1252\nfoo\n\x80\n"
        ));
        // windows-1252 with only ASCII bytes → don't skip
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# encoding:windows-1252\nfoo\n"
        ));
    }

    #[test]
    fn encoding_detect_after_shebang() {
        // Non-UTF-8 encoding after shebang with non-ASCII bytes → skip
        assert!(has_non_utf8_encoding_with_non_ascii_bytes(
            b"#!/usr/bin/env ruby\n# coding: ISO-8859-1\nfoo\n\xff\n"
        ));
        // Non-UTF-8 encoding after shebang with only ASCII → don't skip
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"#!/usr/bin/env ruby\n# coding: ISO-8859-1\nfoo\n"
        ));
        // US-ASCII after shebang should NOT be detected (it's UTF-8 compatible)
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"#!/usr/bin/env ruby\n# coding: US-ASCII\nfoo\n"
        ));
    }

    #[test]
    fn encoding_utf8_not_skipped() {
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# encoding: utf-8\nfoo\n"
        ));
    }

    #[test]
    fn encoding_no_comment_not_skipped() {
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"foo_1 = 1\nbar_2 = 2\n"
        ));
    }

    #[test]
    fn encoding_frozen_string_not_skipped() {
        // frozen_string_literal comment should NOT trigger encoding detection
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# frozen_string_literal: true\nfoo\n"
        ));
    }

    #[test]
    fn encoding_binary_not_skipped() {
        // binary / ASCII-8BIT encodings are handled fine by RuboCop
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# -*- coding: binary -*-\nfoo\n"
        ));
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# encoding: ASCII-8BIT\nfoo\n"
        ));
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# -*- encoding : ascii-8bit -*-\nfoo\n"
        ));
    }

    #[test]
    fn encoding_windows_1251_ascii_only_not_skipped() {
        // windows-1251 with only ASCII bytes — Translation::Parser handles fine
        assert!(!has_non_utf8_encoding_with_non_ascii_bytes(
            b"# encoding:windows-1251\ndef test_windows_1251; end\n"
        ));
    }
}
