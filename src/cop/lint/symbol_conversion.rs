use crate::cop::shared::method_identifier_predicates;
use crate::cop::shared::node_type::{
    CALL_NODE, HASH_NODE, HASH_PATTERN_NODE, KEYWORD_HASH_NODE, SYMBOL_NODE,
};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Lint/SymbolConversion checks for uses of literal strings converted to a symbol
/// where a literal symbol could be used instead, and for unnecessarily quoted symbols.
///
/// ## Corpus investigation (2026-03-10)
///
/// Corpus oracle reported FP=331, FN=279 on the March 10, 2026 run.
///
/// The previous regex-based correction logic only understood identifier-like
/// symbols. That caused:
/// - FPs on quoted hash labels that still require quotes, such as `"7_days":`
/// - FNs on quoted operator and variable symbols such as `:"+", :"@ivar"`
///
/// Fix: generate corrections using Ruby-like symbol literal rules instead of
/// assuming every convertible symbol is an identifier. Hash labels now only
/// autocorrect when the key can be written as a bare Ruby label.
///
/// Remaining gap: `EnforcedStyle: consistent` is still not implemented in this
/// port; the corpus regressions fixed here are all strict-style behavior.
///
/// ## Corpus investigation (2026-03-11)
///
/// Corpus oracle reported FP=37, FN=100. Root causes:
///
/// FPs: Rocket-style hash keys with non-identifier-start values (e.g.,
/// `{ :'@ivar' => val }`) were flagged as standalone symbols. RuboCop's
/// `correct_hash_key` skips keys whose value doesn't match `/\A[a-z0-9_]/i`.
/// Fix: detect rocket-style hash keys by looking for `=>` after the symbol
/// location and skip non-identifier-start values.
///
/// FNs: (1) Missing `!=` from `BARE_OPERATOR_SYMBOLS` — symbols like `:"!="`
/// were not flagged. (2) Special global variables (`$1`, `$?`, `$!`, `$~`,
/// `$0`, etc.) were not recognized as valid bare symbols because
/// `is_global_variable_symbol` only handled named globals starting with
/// identifier chars. Fix: expanded to handle numeric globals, single-char
/// special globals, and `$-x` flags.
///
/// Also fixed `normalize_single_quoted_source` which over-escaped backslashes
/// when converting `:'...'` to `:"..."` form (used `escape_double_quoted_symbol`
/// on raw source chars instead of just escaping `"`).
///
/// ## Corpus investigation (2026-03-14)
///
/// Corpus oracle reported FP=14, FN=94 (98.6% match rate).
///
/// Extensive analysis performed comparing RuboCop's `on_sym`/`on_send` logic
/// against nitrocop's `check_symbol_node`/`check_call_node`:
///
/// - Verified: `.to_sym`/`.intern` on single/double-quoted strings, operator
///   symbols, instance/class/global variables, method-name symbols with `?`/`!`/`=`
///   suffixes, colon-style and rocket-style hash keys, `%i`/`%I` arrays, `alias`
///   statements, setter exemptions, escape sequence handling, and
///   `source_matches_correction` normalization all match RuboCop behavior.
///
/// - Added comprehensive test coverage for all verified patterns.
///
/// - Two potential FP sources identified but unconfirmed (no corpus data available):
///   (1) `alias :"foo" :"bar"` — RuboCop has `in_alias?` check to skip alias
///   arguments, nitrocop relies on Prism's `opening_loc` filtering which works
///   for bare `alias foo bar` but may not for explicitly quoted alias arguments.
///   (2) Multi-line rocket-style hash keys where `=>` is on next line — nitrocop's
///   byte scanning only skips spaces/tabs, not newlines, so may miss the `=>`
///   and treat the key as standalone.
///
/// - `EnforcedStyle: consistent` is still not implemented. The corpus baseline
///   uses `strict` (default) so this should not affect corpus FP/FN. However,
///   individual project configs that set `consistent` would have FNs.
///
/// Root cause of remaining 94 FNs is not definitively identified; may require
/// corpus-level debugging with `investigate-cop.py --context` once corpus data
/// with example locations is available.
///
/// ## FP fix (2026-03-15)
///
/// Corpus oracle reported FP=6, FN=94. 6 FPs traced to escape sequences in
/// quoted symbols where `escape_double_quoted_symbol` didn't reproduce the
/// necessary escaping:
/// - Non-printable characters (e.g., `\x00`, `\a`, `\b`, `\e`) were passed
///   through as raw bytes, producing a correction that didn't match the source.
/// - `#` before `{`, `$`, `@` was not escaped, so `:"#{c}"` (with escaped
///   interpolation) would produce `:"#{c}"` (with live interpolation) as
///   the correction, changing semantics.
///
/// Fix: enhanced `escape_double_quoted_symbol` to handle all control characters
/// (`\a`, `\b`, `\e`, `\xNN` for others), high bytes (UTF-8 pass-through),
/// and `#` before interpolation triggers (`\#`). This ensures
/// `source_matches_correction` correctly identifies these symbols as already
/// using the canonical quoting form.
///
/// ## FP fix (2026-03-14)
///
/// Corpus oracle reported FP=14, FN=94. 14 FPs in jruby, natalie, BetterErrors,
/// hexapdf traced to `alias :'method' other` patterns. RuboCop has an `in_alias?`
/// check (`node.parent&.alias_type?`) that skips all symbols inside alias
/// statements. Since nitrocop's cop framework doesn't provide parent node info,
/// implemented `is_in_alias()` which scans source bytes backwards from the
/// symbol position, skipping whitespace and possibly a preceding symbol argument,
/// to detect the `alias` keyword. This matches RuboCop's behavior: alias
/// arguments are not flagged because a symbol requiring quoting is not a valid
/// method identifier.
///
/// ## FN fix (2026-03-18)
///
/// Corpus oracle reported FP=0, FN=92. 92 FNs concentrated in jruby (37),
/// asciidoctor-pdf (33), natalie (18) — all repos with significant UTF-8 content.
///
/// Root cause: `is_identifier_start` and `is_identifier_continue` only recognized
/// ASCII letters/digits/underscore, but Ruby allows any multi-byte UTF-8 character
/// as an identifier character. Symbols like `:"résumé"` can be written as `:résumé`
/// in Ruby, but nitrocop was not flagging them because it didn't recognize UTF-8
/// bytes (>= 0x80) as valid identifier characters.
///
/// Fix: extended `is_identifier_start` and `is_identifier_continue` to accept
/// bytes >= 0x80 (UTF-8 continuation/leading bytes). This correctly handles
/// multi-byte characters since all bytes in a UTF-8 multi-byte sequence are >= 0x80.
/// The `value_starts_with_identifier` function (used for rocket-style hash key
/// filtering) is intentionally NOT changed — it matches RuboCop's `/\A[a-z0-9_]/i`
/// which is ASCII-only.
///
/// ## FP+FN fix (2026-03-18)
///
/// Corpus oracle reported FP=25, FN=36.
///
/// FP=25: All from basecamp/once-campfire using emoji flag sequences as hash keys
/// (e.g., `{ "🇺🇸": "hello" }`). The previous blanket `bytes >= 0x80` check in
/// `is_identifier_start`/`is_identifier_continue` accepted all non-ASCII bytes,
/// including emoji. Emoji (Unicode Symbol/Other categories) are NOT valid Ruby
/// identifiers — `:🇺🇸` is a syntax error.
/// Fix: replaced byte-level checks with char-level Unicode property checks using
/// `char::is_alphabetic()` and `char::is_numeric()`. These correctly allow
/// letters (é, ñ, 日) and digits but reject emoji and other symbol characters.
///
/// FN=33: From asciidoctor-pdf using `%(...)` string notation with interpolation
/// and `.to_sym` (e.g., `%(cover_#{face}_image).to_sym`). The `check_call_node`
/// for `InterpolatedStringNode` only handled double-quoted strings (`"..."`)
/// but `%(...)` notation starts with `%(` and ends with `)`.
/// Fix: extended the inner-content extraction to also handle `%(` prefix and `)`
/// suffix, constructing the correction as `:"inner_content"`.
///
/// ## FP+FN fix (2026-03-18, second pass)
///
/// Corpus oracle reported FP=8, FN=7.
///
/// FP=8: Non-ASCII characters as colon-style hash keys (e.g., `'æ': 'ae'`,
/// `"а": "a"`, `"Ãa1": "true"`) from travis-ci/dpl (6), danbooru (1), jruby (1).
/// RuboCop's `correct_hash_key` only converts keys starting with `/\A[a-z0-9_]/i`
/// (ASCII-only). Non-ASCII-start keys are skipped.
/// Fix: added ASCII-start check to `is_hash_label_symbol` — the first byte must
/// be ASCII alphanumeric or underscore for colon-style label conversion.
///
/// FN=5: Standalone quoted symbols with non-ASCII chars like `:"×"` (U+00D7,
/// multiplication sign) in jruby. Ruby's lexer accepts ALL non-ASCII multibyte
/// characters as identifier characters (via `is_identchar` macro: `!ISASCII(*p)`).
/// The previous `char::is_alphabetic()` check was too restrictive — it rejected
/// characters like `×` (Unicode category `Sm`, not `Alphabetic`).
/// Fix: reverted `is_char_identifier_start`/`is_char_identifier_continue` to
/// accept all non-ASCII characters (`!ch.is_ascii()`), matching Ruby's behavior.
/// Emoji hash keys remain protected by the separate ASCII-start check in
/// `is_hash_label_symbol` and `value_starts_with_identifier`.
///
/// FN=1: `:"$$"` from pdf-reader — `$$` (process ID) is a valid special global
/// variable. Fix: added `$` to `SPECIAL_GLOBAL_CHARS`.
///
/// FN=1: `%s"..."` literal from rouge — `%s` percent-literal symbols were
/// previously not handled. Added `%s` opening support, but this was reverted
/// because RuboCop's `properly_quoted_symbol?` only checks `:"` and `:'`
/// starts, NOT `%s`. RuboCop never flags `%s()` symbols. Accepting `%s`
/// caused 15 FPs in hexapdf/fontcustom/jruby/natalie.
///
/// FN=1: `"\xff".to_sym` from jruby — `symbol_correction` returned None for
/// non-UTF8 content because `from_utf8` failed. Fixed by generating hex-escaped
/// symbol like `:\"\xFF\"` for non-UTF8 bytes.
///
/// ## Corpus investigation (2026-03-21)
///
/// Corpus oracle reported FP=15, FN=0.
///
/// FP=15: All from `%s()` percent-literal symbols (hexapdf 12, fontcustom 1,
/// jruby 1, natalie 1). RuboCop's `properly_quoted_symbol?` only checks for
/// `:"` and `:'` prefixes, never `%s`. Fix: removed `%s` opening support
/// from `check_symbol_node`.
///
/// ## FN fix (2026-03-23)
///
/// Corpus oracle reported FP=0, FN=1. The single FN is `%s"test"` from
/// rouge-ruby/rouge — a `%s` symbol literal using double-quote delimiters.
///
/// Root cause: the previous revert of `%s` support was too broad. RuboCop's
/// `properly_quoted?` returns true early when `!source.match?(/['"]/)` — so
/// `%s(...)` (no quotes) is skipped, but `%s"..."` and `%s'...'` (quote
/// delimiters) ARE flagged because they contain quote characters.
///
/// Fix: accept `%s"` and `%s'` as valid opening patterns in `check_symbol_node`,
/// while still rejecting `%s(`, `%s[`, and other non-quote delimiters.
///
/// ## FN fix (2026-03-28)
///
/// Corpus oracle reported FP=0, FN=21. The remaining misses fell into two
/// narrow buckets:
///
/// - Colon-style hash keys that still require quotes but start with an ASCII
///   identifier character, such as `"string \"\€\"":`. RuboCop's
///   `correct_hash_key` still normalizes these through `node.value.inspect`,
///   only skipping keys whose first character fails `/\A[a-z0-9_]/i`.
/// - `%s(...)` symbol literals whose body contains quote characters, such as
///   `%s(assign (index Class 2) "Class")`. RuboCop treats `%s` literals as
///   "properly quoted" only when the source contains no quote characters at
///   all, so these remain offenses even with non-quote delimiters.
///
/// Fix: make colon-style hash keys reuse the same symbol correction logic as
/// RuboCop's `node.value.inspect.delete_prefix(':')`, comparing the key source
/// without its trailing `:` against the canonical correction, and treat all
/// `%s` openings as candidates while still short-circuiting when the source has
/// no quote characters. This fixes the FN set without broadening `%i/%I`
/// handling or relaxing the existing rocket-key guards.
///
/// ## FP fix (2026-04-03)
///
/// Corpus oracle reported FP=2, FN=0, both in natalie's `to_sym` shared spec.
/// Ruby's `Symbol#inspect` keeps `!@` and `~@` quoted (`:"!@"`, `:"~@"`),
/// unlike `+@` and `-@`, so RuboCop does not rewrite those two unary operator
/// method names. Fix: treat only `!@` and `~@` as quote-required operator
/// symbols when building corrections.
///
/// ## EnforcedStyle: consistent (2026-04-06)
///
/// Variant divergence reported FP=155, FN=3,331 for `EnforcedStyle: consistent`.
/// Default strict style was perfect (0 FP, 0 FN).
///
/// Root cause: `EnforcedStyle: consistent` was read but not implemented.
/// The cop always used strict-mode behavior regardless of the configured style.
///
/// RuboCop's `consistent` style adds hash-level checking via `on_hash`:
/// - If ANY symbol key in a hash requires quoting (value.inspect is `:"..."`
///   or value ends with `=`), ALL unquoted symbol keys are flagged with
///   `MSG_CONSISTENCY` ("Symbol hash key should be quoted for consistency").
/// - If NO key requires quoting, individually quoted keys are flagged the same
///   as strict mode ("Unnecessary symbol conversion").
/// - Additionally, `properly_quoted?` in consistent mode does NOT short-circuit
///   on quote-less sources or setter symbols.
///
/// Fix: added `HASH_NODE` and `KEYWORD_HASH_NODE` to `interested_node_types`.
/// When `EnforcedStyle: consistent`, `check_hash_consistent` collects all
/// symbol keys from the hash, checks if any requires quoting, and either flags
/// bare keys for consistency (case 1) or delegates to strict-mode individual
/// checking (case 2). In `check_symbol_node`, hash keys are skipped in
/// consistent mode to avoid double-flagging (they're handled by the hash-level
/// check). The `properly_quoted_source` function now takes a `strict` parameter
/// to control the early-return behavior.
///
/// ## Consistent variant fix (2026-04-09)
///
/// Variant divergence reported FP=2, FN=6 for `EnforcedStyle: consistent`.
///
/// FP=2: Both in rubocop/rubocop — `%i[... undef unless ...]` patterns where
/// `is_in_undef` byte-scan found the text "undef" inside a `%i` array and
/// falsely treated `unless` as a `undef` argument. Fix: added
/// `is_undef_at_statement_start` check — after finding `undef`, verify it's
/// preceded (past whitespace) by `\n`, `;`, `{`, or start of file.
///
/// FN=2 (:~@ symbols): Bare symbols with `:` opening were hitting `_ => return`
/// in consistent mode. RuboCop's `properly_quoted?` doesn't short-circuit for
/// non-quoted sources in consistent mode, so `:~@` (value `~`, correction `:~`)
/// is flagged as a mismatch. Fix: allow bare symbols through in consistent mode.
///
/// FN=2 (hash patterns): `case/in` hash patterns use `HashPatternNode`, which
/// wasn't registered in `interested_node_types`. Fix: added `HASH_PATTERN_NODE`.
///
/// FN=2 (binary encoding): `# encoding: binary` files where bare `:il_était`
/// should be `:"il_\xC3\xA9tait"`. Not fixed — requires encoding-aware processing.
pub struct SymbolConversion;

/// Check if a character is a valid Ruby identifier start character.
/// Ruby allows ASCII letters, underscore, and ALL non-ASCII characters
/// (Ruby's lexer treats any multibyte character as identifier-valid).
fn is_char_identifier_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic() || !ch.is_ascii()
}

/// Check if a character is a valid Ruby identifier continuation character.
/// Ruby allows ASCII alphanumerics, underscore, and ALL non-ASCII characters.
fn is_char_identifier_continue(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric() || !ch.is_ascii()
}

/// Check if a byte is a valid identifier continuation at the byte level.
/// Used for byte-level backward scanning in `is_in_alias` where we scan over
/// identifier chars without needing full UTF-8 decoding. Accepts bytes >= 0x80
/// to handle multi-byte UTF-8 sequences.
fn is_identifier_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Check if a byte slice contains only valid Ruby identifier characters (char-level).
/// The first character must be a valid identifier start, the rest must be valid continuations.
fn is_valid_identifier(value: &[u8]) -> bool {
    let s = match std::str::from_utf8(value) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let mut chars = s.chars();
    match chars.next() {
        Some(ch) if is_char_identifier_start(ch) => chars.all(is_char_identifier_continue),
        _ => false,
    }
}

fn is_method_name_symbol(value: &[u8]) -> bool {
    if value.is_empty() {
        return false;
    }

    let main = match value.last() {
        Some(b'!' | b'?' | b'=') => &value[..value.len() - 1],
        _ => value,
    };

    !main.is_empty() && is_valid_identifier(main)
}

fn is_instance_variable_symbol(value: &[u8]) -> bool {
    value.len() > 1 && value[0] == b'@' && is_valid_identifier(&value[1..])
}

fn is_class_variable_symbol(value: &[u8]) -> bool {
    value.len() > 2 && value.starts_with(b"@@") && is_valid_identifier(&value[2..])
}

/// Characters recognized by Ruby as single-char special global variables
/// (e.g., `$?`, `$!`, `$~`, `$@`, `$;`, `$,`, `$/`, `$\`, `$=`, `$<`, `$>`,
/// `$.`, `$*`, `$:`, `$+`, `$&`, `` $` ``, `$'`, `$"`, `$0`, `$$`).
const SPECIAL_GLOBAL_CHARS: &[u8] = b"?!~@;,/\\=<>.*:+&`'\"0$";

fn is_global_variable_symbol(value: &[u8]) -> bool {
    if value.len() < 2 || value[0] != b'$' {
        return false;
    }

    // Named globals: $foo, $LOAD_PATH, $_
    if value.len() > 1 && is_valid_identifier(&value[1..]) {
        return true;
    }

    // Numeric globals: $1, $2, ..., $9 (and possibly multi-digit like $10)
    if value[1].is_ascii_digit() {
        return value[2..].iter().copied().all(|b| b.is_ascii_digit());
    }

    // Single-char special globals: $?, $!, $~, etc.
    if value.len() == 2 && SPECIAL_GLOBAL_CHARS.contains(&value[1]) {
        return true;
    }

    // $-x flags (e.g., $-w, $-v, $-a)
    if value.len() == 3 && value[1] == b'-' && value[2].is_ascii_alphabetic() {
        return true;
    }

    false
}

fn is_operator_symbol(value: &[u8]) -> bool {
    method_identifier_predicates::is_operator_method(value)
}

fn is_quote_required_operator_symbol(value: &[u8]) -> bool {
    matches!(value, b"!@" | b"~@")
}

fn can_be_unquoted_symbol(value: &[u8]) -> bool {
    is_method_name_symbol(value)
        || is_instance_variable_symbol(value)
        || is_class_variable_symbol(value)
        || is_global_variable_symbol(value)
        || (is_operator_symbol(value) && !is_quote_required_operator_symbol(value))
}

/// Check if a symbol key "requires quoting" for the purposes of consistent-style
/// hash key checking. Matches RuboCop's `requires_quotes?` which checks:
/// `sym_node.value.inspect.match?(/^:".*?"|=$/)`.
/// Returns true if the symbol's inspect form would be :"..." (quoted) or ends with =.
fn requires_quotes(value: &[u8]) -> bool {
    !can_be_unquoted_symbol(value) || value.ends_with(b"=")
}

fn escape_double_quoted_symbol(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\\' => escaped.push_str("\\\\"),
            b'"' => escaped.push_str("\\\""),
            b'\n' => escaped.push_str("\\n"),
            b'\r' => escaped.push_str("\\r"),
            b'\t' => escaped.push_str("\\t"),
            0x0C => escaped.push_str("\\f"),
            0x07 => escaped.push_str("\\a"),
            0x08 => escaped.push_str("\\b"),
            0x0B => escaped.push_str("\\v"),
            0x1B => escaped.push_str("\\e"),
            // Escape # before {, $, @ to prevent interpolation
            b'#' if i + 1 < bytes.len()
                && (bytes[i + 1] == b'{' || bytes[i + 1] == b'$' || bytes[i + 1] == b'@') =>
            {
                escaped.push_str("\\#");
            }
            // Non-printable characters: control chars (0x00-0x1F, 0x7F) and high bytes
            _ if b < 0x20 || b == 0x7F => {
                escaped.push_str(&format!("\\x{b:02X}"));
            }
            _ if b > 0x7F => {
                // For multi-byte UTF-8, push the character as-is
                if let Some(ch) = value[i..].chars().next() {
                    escaped.push(ch);
                    i += ch.len_utf8();
                    continue;
                }
                // Fallback: hex escape the byte
                escaped.push_str(&format!("\\x{b:02X}"));
            }
            _ => escaped.push(b as char),
        }
        i += 1;
    }
    escaped
}

/// Compute the correction string for a symbol value.
/// Returns Ruby-like symbol literal syntax, e.g. `:+`, `:@ivar`, or `:"foo-bar"`.
fn symbol_correction(value: &[u8]) -> Option<String> {
    if let Ok(value_str) = std::str::from_utf8(value) {
        if can_be_unquoted_symbol(value) {
            Some(format!(":{value_str}"))
        } else {
            Some(format!(":\"{}\"", escape_double_quoted_symbol(value_str)))
        }
    } else {
        // Non-UTF8 content: generate hex-escaped symbol like :"\xFF"
        let mut escaped = String::new();
        for &b in value {
            if b.is_ascii_graphic() || b == b' ' {
                escaped.push(b as char);
            } else {
                escaped.push_str(&format!("\\x{:02X}", b));
            }
        }
        Some(format!(":\"{escaped}\""))
    }
}

fn hash_key_correction(value: &[u8]) -> Option<String> {
    if !value_starts_with_identifier(value) {
        return None;
    }

    let correction = symbol_correction(value)?;
    Some(correction.strip_prefix(':')?.to_string())
}

/// Escape only double-quote characters in raw source content when converting
/// from single-quoted to double-quoted form. Unlike `escape_double_quoted_symbol`
/// (which escapes unescaped values), this works on raw source text where
/// backslashes are already in their source-level escaped form.
/// Matches RuboCop's `source.gsub('"', '\"').tr("'", '"')` behavior.
fn escape_quotes_for_normalization(inner: &str) -> String {
    let mut result = String::with_capacity(inner.len());
    for ch in inner.chars() {
        if ch == '"' {
            result.push('\\');
            result.push('"');
        } else {
            result.push(ch);
        }
    }
    result
}

fn normalize_single_quoted_source(source: &str) -> Option<String> {
    if source.starts_with(":'") && source.ends_with('\'') {
        let inner = source.strip_prefix(":'")?.strip_suffix('\'')?;
        return Some(format!(":\"{}\"", escape_quotes_for_normalization(inner)));
    }

    if source.starts_with('\'') && source.ends_with('\'') {
        let inner = source.strip_prefix('\'')?.strip_suffix('\'')?;
        return Some(format!("\"{}\"", escape_quotes_for_normalization(inner)));
    }

    None
}

/// Check if the value starts with an alphanumeric or underscore character.
/// Matches RuboCop's `/\A[a-z0-9_]/i` check in `correct_hash_key`.
fn value_starts_with_identifier(value: &[u8]) -> bool {
    value
        .first()
        .is_some_and(|&b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Check if `undef` at a given position is actually a keyword (at statement start),
/// not just the text "undef" inside a `%i` array or similar context.
/// Returns true if preceded (after skipping whitespace) by a statement boundary:
/// newline, `;`, `{` (block opening), or start of file.
fn is_undef_at_statement_start(src: &[u8], undef_start: usize) -> bool {
    if undef_start == 0 {
        return true;
    }
    let mut p = undef_start;
    while p > 0 && matches!(src[p - 1], b' ' | b'\t') {
        p -= 1;
    }
    p == 0 || matches!(src[p - 1], b'\n' | b'\r' | b';' | b'{')
}

/// Check if a symbol node is inside a `undef` statement.
/// In `EnforcedStyle: consistent`, RuboCop's `properly_quoted?` doesn't
/// short-circuit on quote-less sources, so bare symbols in `undef` are flagged
/// (e.g., `undef is_a?` → "use `:is_a?` instead").
///
/// Handles both simple `undef foo` and comma-separated `undef foo, bar`.
fn is_in_undef(source: &SourceFile, sym: &ruby_prism::SymbolNode<'_>) -> bool {
    let start = sym.location().start_offset();
    let src = source.as_bytes();
    if start < 6 {
        return false;
    }

    let mut pos = start;

    // Skip whitespace
    while pos > 0 && matches!(src[pos - 1], b' ' | b'\t') {
        pos -= 1;
    }

    // Direct `undef foo`
    if pos >= 5 && &src[pos - 5..pos] == b"undef" {
        let undef_start = pos - 5;
        return (pos == 5 || !is_identifier_continue(src[pos - 6]))
            && is_undef_at_statement_start(src, undef_start);
    }

    // `undef foo, bar` — skip back over comma and previous arguments
    while pos > 0 && src[pos - 1] == b',' {
        pos -= 1; // skip comma

        // Skip whitespace (including newlines for multi-line undef)
        while pos > 0 && matches!(src[pos - 1], b' ' | b'\t' | b'\n' | b'\r') {
            pos -= 1;
        }

        // Skip a bare symbol backward: optional suffix (?/!/=), then identifier body
        let start_pos = pos;
        if pos > 0 && matches!(src[pos - 1], b'?' | b'!' | b'=') {
            pos -= 1;
        }
        while pos > 0 && is_identifier_continue(src[pos - 1]) {
            pos -= 1;
        }

        if pos == start_pos {
            return false;
        }

        // Skip whitespace
        while pos > 0 && matches!(src[pos - 1], b' ' | b'\t') {
            pos -= 1;
        }

        // Check for `undef`
        if pos >= 5 && &src[pos - 5..pos] == b"undef" {
            let undef_start = pos - 5;
            return (pos == 5 || !is_identifier_continue(src[pos - 6]))
                && is_undef_at_statement_start(src, undef_start);
        }
    }

    false
}

/// Check if a symbol node is an argument to the `alias` keyword.
/// RuboCop skips alias arguments because a symbol requiring quoting is not a
/// valid method identifier, so flagging it would be unhelpful.
///
/// Scans backwards from the symbol's start offset, skipping whitespace and
/// possibly one preceding quoted/bare symbol (the first alias argument),
/// to see if the `alias` keyword precedes it.
fn is_in_alias(source: &SourceFile, sym: &ruby_prism::SymbolNode<'_>) -> bool {
    let start = sym.location().start_offset();
    let src = source.as_bytes();
    if start == 0 {
        return false;
    }

    // Skip backwards over whitespace
    let mut pos = start;
    while pos > 0 && (src[pos - 1] == b' ' || src[pos - 1] == b'\t') {
        pos -= 1;
    }

    // Check if preceded directly by `alias` (this is the first alias argument)
    if pos >= 5 && &src[pos - 5..pos] == b"alias" {
        // Make sure `alias` is at start of token (not part of a longer identifier)
        return pos == 5 || !is_identifier_continue(src[pos - 6]);
    }

    // Maybe this is the second alias argument — skip over the first argument
    // First argument can be: bare symbol (identifier chars), or quoted symbol (:'...' or :"...")
    if pos > 0 {
        let end_char = src[pos - 1];
        if end_char == b'\'' || end_char == b'"' {
            // Quoted symbol: scan back to find matching :'  or :"
            let quote = end_char;
            if pos < 3 {
                return false;
            }
            // Find the opening :' or :"
            let mut p = pos - 2; // skip closing quote
            while p > 0 && src[p] != quote {
                p -= 1;
            }
            // p should now be at the opening quote, preceded by ':'
            if p > 0 && src[p] == quote && src[p - 1] == b':' {
                pos = p - 1; // move before ':'
            } else {
                return false;
            }
        } else if is_identifier_continue(end_char) {
            // Bare symbol (identifier): scan back over identifier chars
            let mut p = pos - 1;
            while p > 0 && is_identifier_continue(src[p - 1]) {
                p -= 1;
            }
            pos = p;
        } else {
            return false;
        }

        // Skip whitespace again
        while pos > 0 && (src[pos - 1] == b' ' || src[pos - 1] == b'\t') {
            pos -= 1;
        }

        // Now check for `alias`
        if pos >= 5 && &src[pos - 5..pos] == b"alias" {
            return pos == 5 || !is_identifier_continue(src[pos - 6]);
        }
    }

    false
}

/// Check if a symbol node is used as a rocket-style hash key (e.g., `:'foo' => val`).
/// Looks at the source bytes after the symbol to see if `=>` follows (after whitespace).
fn is_rocket_hash_key(source: &SourceFile, sym: &ruby_prism::SymbolNode<'_>) -> bool {
    let end_offset = sym.location().end_offset();
    let src = source.as_bytes();
    if end_offset >= src.len() {
        return false;
    }
    let rest = &src[end_offset..];
    // Skip whitespace, then check for =>
    let trimmed = rest.iter().position(|&b| b != b' ' && b != b'\t');
    match trimmed {
        Some(pos) => rest[pos..].starts_with(b"=>"),
        None => false,
    }
}

fn source_matches_correction(source: &[u8], correction: &str) -> bool {
    let Ok(source_str) = std::str::from_utf8(source) else {
        return false;
    };

    source_str == correction
        || normalize_single_quoted_source(source_str)
            .as_deref()
            .is_some_and(|normalized| normalized == correction)
}

fn source_contains_quote_chars(source: &[u8]) -> bool {
    source.contains(&b'\'') || source.contains(&b'"')
}

fn properly_quoted_source(source: &[u8], correction: &str, strict: bool) -> bool {
    if strict && (!source_contains_quote_chars(source) || correction.ends_with('=')) {
        return true;
    }
    source_matches_correction(source, correction)
}

impl Cop for SymbolConversion {
    fn name(&self) -> &'static str {
        "Lint/SymbolConversion"
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[
            CALL_NODE,
            HASH_NODE,
            HASH_PATTERN_NODE,
            KEYWORD_HASH_NODE,
            SYMBOL_NODE,
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
        let style = config.get_str("EnforcedStyle", "strict");

        // Check HashNode / KeywordHashNode / HashPatternNode: consistent style hash key checking
        if style == "consistent" {
            if let Some(hash) = node.as_hash_node() {
                self.check_hash_consistent(source, hash.elements(), diagnostics);
                return;
            }
            if let Some(kw_hash) = node.as_keyword_hash_node() {
                self.check_hash_consistent(source, kw_hash.elements(), diagnostics);
                return;
            }
            if let Some(hash_pattern) = node.as_hash_pattern_node() {
                self.check_hash_consistent(source, hash_pattern.elements(), diagnostics);
                return;
            }
        }

        // Check SymbolNode: quoted symbols (standalone or hash keys)
        if let Some(sym) = node.as_symbol_node() {
            self.check_symbol_node(source, &sym, style, diagnostics);
            return;
        }

        // Check CallNode: .to_sym / .intern patterns
        if let Some(call) = node.as_call_node() {
            self.check_call_node(source, &call, diagnostics);
        }
    }
}

impl SymbolConversion {
    /// Check a SymbolNode for unnecessary quoting.
    /// Handles:
    /// - Standalone quoted symbols: `:"foo"`, `:'bar'`
    /// - Hash keys (colon-style): `'foo': val`, `"foo": val` (strict mode only;
    ///   consistent mode handles these in `check_hash_consistent`)
    /// - Hash keys/values (rocket-style): `:'foo' => val`, `{ foo: :'bar' }`
    fn check_symbol_node(
        &self,
        source: &SourceFile,
        sym: &ruby_prism::SymbolNode<'_>,
        style: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let src = sym.location().as_slice();
        let value = sym.unescaped();
        let is_strict = style == "strict";

        // Determine what kind of symbol this is based on source representation
        let opening = sym.opening_loc().map(|l| l.as_slice());

        // Check if this is a hash key in colon-style: 'foo': or "foo":
        // In Prism, these have opening_loc of "'" or "\"" and source ends with ':
        let is_colon_hash_key = match opening {
            Some(b"'" | b"\"") => src.ends_with(b"':") || src.ends_with(b"\":"),
            _ => false,
        };

        // In consistent mode, all hash keys are handled by check_hash_consistent
        if !is_strict {
            if is_colon_hash_key {
                return;
            }
            // Also skip rocket-style hash keys
            if matches!(opening, Some(b":\"" | b":'")) && is_rocket_hash_key(source, sym) {
                return;
            }
            // Skip bare hash keys (e.g., `foo:` in `{ foo: 1 }`) — source ends with `:`
            if opening.is_none() && src.ends_with(b":") {
                return;
            }
        }

        if is_colon_hash_key {
            // Hash key with colon style: 'foo': val or "foo": val
            // RuboCop normalizes any key whose value starts with /[a-z0-9_]/i,
            // even when the canonical form still needs quotes.
            if let Some(value_str) = hash_key_correction(value) {
                let source_without_colon = src.strip_suffix(b":").unwrap_or(src);
                if properly_quoted_source(source_without_colon, &value_str, true) {
                    return;
                }

                let loc = sym.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!("Unnecessary symbol conversion; use `{value_str}:` instead."),
                ));
            }
            return;
        }

        // For standalone symbols or rocket-style hash keys/values:
        // Check if the symbol is unnecessarily quoted.
        // Opening must be :" or :' (quoted symbol syntax), OR %s with any
        // delimiter. RuboCop then decides whether it is already "properly
        // quoted" based on whether the source contains any quote characters.
        let is_percent_s = matches!(opening, Some(o) if o.starts_with(b"%s"));
        match opening {
            Some(b":\"" | b":'") => {}
            _ if is_percent_s => {}
            // In consistent mode, bare symbols in `undef` should be checked.
            // RuboCop's `properly_quoted?` doesn't short-circuit for quote-less
            // sources, so `undef is_a?` fires ("use `:is_a?` instead").
            _ if !is_strict && opening.is_none() && is_in_undef(source, sym) => {}
            // In consistent mode, bare symbols with `:` prefix are also checked
            // because `properly_quoted?` doesn't short-circuit. Source `:~@`
            // (value `~`, correction `:~`) → mismatch → offense.
            _ if !is_strict && matches!(opening, Some(b":")) => {}
            _ => return,
        }

        // Skip multi-line symbols (source contains a literal newline between quotes).
        // RuboCop does not flag these because they cannot be simplified without
        // changing the visual representation.
        if src.contains(&b'\n') {
            return;
        }

        // Skip symbols that are arguments to `alias` — a symbol requiring
        // quoting is not a valid method identifier, so flagging it is unhelpful.
        // Matches RuboCop's `in_alias?` check.
        if is_in_alias(source, sym) {
            return;
        }

        // Check if this is a rocket-style hash key (:'foo' => val).
        // RuboCop's correct_hash_key skips keys whose value doesn't start with
        // /\A[a-z0-9_]/i, so we must do the same to avoid false positives.
        if is_rocket_hash_key(source, sym) && !value_starts_with_identifier(value) {
            return;
        }

        let correction = match symbol_correction(value) {
            Some(c) => c,
            None => return,
        };

        if properly_quoted_source(src, &correction, is_strict) {
            return;
        }

        let loc = sym.location();
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!("Unnecessary symbol conversion; use `{correction}` instead."),
        ));
    }

    /// Check hash keys for consistent quoting (EnforcedStyle: consistent).
    ///
    /// RuboCop's `on_hash` for consistent style:
    /// - If any symbol key requires quoting, flag all unquoted keys
    ///   ("Symbol hash key should be quoted for consistency")
    /// - If no key requires quoting, flag individually quoted keys
    ///   (same as strict mode)
    fn check_hash_consistent(
        &self,
        source: &SourceFile,
        elements: ruby_prism::NodeList<'_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        // Collect all symbol keys from hash elements
        let mut sym_keys: Vec<ruby_prism::SymbolNode<'_>> = Vec::new();
        for element in elements.iter() {
            if let Some(assoc) = element.as_assoc_node() {
                if let Some(sym) = assoc.key().as_symbol_node() {
                    sym_keys.push(sym);
                }
            }
        }

        if sym_keys.is_empty() {
            return;
        }

        // Check if any key requires quoting
        let any_requires_quoting = sym_keys.iter().any(|sym| requires_quotes(sym.unescaped()));

        if any_requires_quoting {
            // Case 1: flag all unquoted/bare keys for consistency
            for sym in &sym_keys {
                let value = sym.unescaped();

                // Skip keys that require quoting (they're already properly quoted)
                if requires_quotes(value) {
                    continue;
                }

                let value_str = match std::str::from_utf8(value) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let correction = format!("\"{}\"", value_str);

                // Check if the key source already matches the correction
                let src = sym.location().as_slice();
                // Strip trailing : for colon-style keys (Prism includes it)
                let check_src = src.strip_suffix(b":").unwrap_or(src);

                // In consistent mode, properly_quoted doesn't have the early return
                if source_matches_correction(check_src, &correction) {
                    continue;
                }

                let loc = sym.location();
                let (line, column) = source.offset_to_line_col(loc.start_offset());
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!(
                        "Symbol hash key should be quoted for consistency; use `{}:` instead.",
                        correction
                    ),
                ));
            }
        } else {
            // Case 2: no keys require quoting — flag individually quoted keys
            // Same behavior as strict mode. We call check_symbol_node with
            // style "strict" to reuse the existing hash key checking logic.
            for sym in &sym_keys {
                self.check_symbol_node(source, sym, "strict", diagnostics);
            }
        }
    }

    /// Check a CallNode for .to_sym / .intern on string/symbol/dstr receivers.
    fn check_call_node(
        &self,
        source: &SourceFile,
        call: &ruby_prism::CallNode<'_>,
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        let method_name = call.name().as_slice();

        // Must be .to_sym or .intern
        if method_name != b"to_sym" && method_name != b"intern" {
            return;
        }

        // Must have no arguments
        if call.arguments().is_some() {
            return;
        }

        let recv = match call.receiver() {
            Some(r) => r,
            None => return,
        };

        // String receiver: "foo".to_sym
        if let Some(str_node) = recv.as_string_node() {
            let value = str_node.unescaped();
            let correction = match symbol_correction(value) {
                Some(c) => c,
                None => return,
            };
            let loc = call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!("Unnecessary symbol conversion; use `{correction}` instead."),
            ));
            return;
        }

        // Symbol receiver: :foo.to_sym
        if let Some(sym_node) = recv.as_symbol_node() {
            let value = sym_node.unescaped();
            let correction = match symbol_correction(value) {
                Some(c) => c,
                None => return,
            };
            let loc = call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!("Unnecessary symbol conversion; use `{correction}` instead."),
            ));
            return;
        }

        // Interpolated string receiver: "foo-#{bar}".to_sym
        if let Some(dstr) = recv.as_interpolated_string_node() {
            // Reconstruct the interpolated content from source
            let dstr_loc = dstr.location();
            let dstr_src = dstr_loc.as_slice();
            // Strip the surrounding quotes/delimiters from the dstr source
            let inner = if dstr_src.starts_with(b"\"") && dstr_src.ends_with(b"\"") {
                &dstr_src[1..dstr_src.len() - 1]
            } else if dstr_src.starts_with(b"%(") && dstr_src.ends_with(b")") {
                &dstr_src[2..dstr_src.len() - 1]
            } else {
                return; // heredoc or other form, skip
            };
            let inner_str = match std::str::from_utf8(inner) {
                Ok(s) => s,
                Err(_) => return,
            };
            let correction = format!(":\"{}\"", inner_str);
            let loc = call.location();
            let (line, column) = source.offset_to_line_col(loc.start_offset());
            diagnostics.push(self.diagnostic(
                source,
                line,
                column,
                format!("Unnecessary symbol conversion; use `{correction}` instead."),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(SymbolConversion, "cops/lint/symbol_conversion");

    #[test]
    fn rocket_style_hash_keys_with_non_identifier_start() {
        let cop = SymbolConversion;
        // RuboCop skips rocket-style hash keys where value doesn't start
        // with /[a-z0-9_]/i — these should NOT be flagged.
        let no_offense_cases = [
            r#"{ :'@ivar' => 1 }"#,
            r#"{ :"@ivar" => 1 }"#,
            r#"{ :'$global' => 1 }"#,
            r#"{ :'+' => 1 }"#,
            r#"{ :'==' => 1 }"#,
            r#"{ :'@@cvar' => 1 }"#,
        ];
        for source in &no_offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                diags.is_empty(),
                "Expected no offense for {:?} but got: {:?}",
                source,
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }

        // But standalone versions of these symbols SHOULD be flagged
        let offense_cases = [r#":'@ivar'"#, r#":"@ivar""#, r#":'$global'"#, r#":'+'"#];
        for source in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
        }

        // Rocket-style with identifier-start values SHOULD still be flagged
        let flagged_rocket_cases = [r#"{ :'foo' => 1 }"#, r#"{ :"foo" => 1 }"#];
        for source in &flagged_rocket_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
        }
    }

    #[test]
    fn single_quoted_string_to_sym() {
        let cop = SymbolConversion;
        // Single-quoted strings with .to_sym should be flagged
        let offense_cases = [
            ("'foo'.to_sym", ":foo"),
            ("'foo_bar'.to_sym", ":foo_bar"),
            ("'foo-bar'.to_sym", ":\"foo-bar\""),
            ("'foo'.intern", ":foo"),
        ];
        for (source, expected) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected correction {} in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn to_sym_with_empty_parens() {
        let cop = SymbolConversion;
        // .to_sym() with empty parens should still be flagged
        let offense_cases = [
            (r#""foo".to_sym()"#, ":foo"),
            (r#"'bar'.intern()"#, ":bar"),
            (":baz.to_sym()", ":baz"),
        ];
        for (source, expected) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected correction {} in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn quote_required_unary_operator_symbols() {
        let cop = SymbolConversion;

        for source in [
            r#"obj.send(:"!@")"#,
            r#"obj.send(:"~@")"#,
            r#""!@".to_sym"#,
            r#""~@".to_sym"#,
        ] {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            let corrections = diags.iter().map(|d| d.message.as_str()).collect::<Vec<_>>();

            if source.ends_with(".to_sym") {
                assert!(
                    corrections
                        .iter()
                        .any(|message| message.contains("`:\"!@\"`"))
                        || corrections
                            .iter()
                            .any(|message| message.contains("`:\"~@\"`")),
                    "Expected quoted correction for {:?}, got {:?}",
                    source,
                    corrections
                );
            } else {
                assert!(
                    diags.is_empty(),
                    "Expected no offense for {:?} but got {:?}",
                    source,
                    corrections
                );
            }
        }

        for (source, expected) in [(r#"obj.send(:"+@")"#, ":+@"), (r#"obj.send(:"-@")"#, ":-@")] {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected correction {} in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn comprehensive_offense_check() {
        let cop = SymbolConversion;
        // Comprehensive list of patterns that should be offenses
        let offense_cases: Vec<(&str, &str)> = vec![
            // to_sym on string literals (single and double quoted)
            (r#""foo".to_sym"#, ":foo"),
            ("'foo'.to_sym", ":foo"),
            // intern
            (r#""foo".intern"#, ":foo"),
            ("'foo'.intern", ":foo"),
            // to_sym on symbol literal (redundant)
            (":foo.to_sym", ":foo"),
            (":foo.intern", ":foo"),
            // Quoted standalone symbols
            (":\"foo\"", ":foo"),
            (":'foo'", ":foo"),
            (":\"foo_bar\"", ":foo_bar"),
            (":'foo_bar'", ":foo_bar"),
            // Operator symbols that can be unquoted (excluding those ending with =)
            (":\"<<\"", ":<<"),
            (":'<<'", ":<<"),
            (":\"[]\"", ":[]"),
            (":'[]'", ":[]"),
            (":\"+\"", ":+"),
            (":'+'", ":+"),
            (":\"-\"", ":-"),
            (":'-'", ":-"),
            (":\"**\"", ":**"),
            (":'**'", ":**"),
            // Instance/class variable symbols
            (":\"@foo\"", ":@foo"),
            (":'@foo'", ":@foo"),
            (":\"@@foo\"", ":@@foo"),
            (":'@@foo'", ":@@foo"),
            // Global variable symbols
            (":\"$foo\"", ":$foo"),
            (":'$foo'", ":$foo"),
            // Colon-style hash keys
            ("{ 'foo': 1 }", "foo:"),
            ("{ \"foo\": 1 }", "foo:"),
            ("{ 'foo_bar': 1 }", "foo_bar:"),
            // Rocket-style hash keys with identifier start
            ("{ :'foo' => 1 }", ":foo"),
            ("{ :\"foo\" => 1 }", ":foo"),
            // Method-like symbols
            (":\"foo?\"", ":foo?"),
            (":'foo?'", ":foo?"),
            (":\"foo!\"", ":foo!"),
            (":'foo!'", ":foo!"),
            // Hash keys ending with ? or !
            ("{ 'foo?': 1 }", "foo?:"),
            ("{ 'foo!': 1 }", "foo!:"),
        ];
        for (source, expected_contains) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected_contains}`")),
                "Expected `{}` in message for {:?}, got: {}",
                expected_contains,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn symbol_in_various_contexts() {
        let cop = SymbolConversion;
        // Symbols in various contexts should all be flagged
        let offense_cases = [
            // In array
            (r#"[:"foo", :"bar"]"#, 2),
            // In method arguments
            (r#"method(:"foo")"#, 1),
            // In assignment
            (r#"x = :"foo""#, 1),
            // In conditional
            (r#"if x == :"foo" then 1 end"#, 1),
            // In case/when
            (r#"case x; when :"foo" then 1; end"#, 1),
            // Nested in hash value
            (r#"{ key: :"foo" }"#, 1),
            // Multiple in same line
            (r#"[:"foo", :'bar', "baz".to_sym]"#, 3),
            // In string interpolation context
            (r#"send(:"foo")"#, 1),
            // Method receiver
            (r#":"foo".to_s"#, 1),
        ];
        for (source, expected_count) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert_eq!(
                diags.len(),
                *expected_count,
                "Expected {} offense(s) for {:?} but got {}: {:?}",
                expected_count,
                source,
                diags.len(),
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn to_sym_with_special_strings() {
        let cop = SymbolConversion;
        // Various string forms with .to_sym
        let offense_cases = [
            // Method-like strings
            ("'foo?'.to_sym", ":foo?"),
            ("'foo!'.to_sym", ":foo!"),
            ("'foo='.to_sym", ":foo="),
            // Operator strings
            ("'+'.to_sym", ":+"),
            ("'<<'.to_sym", ":<<"),
            ("'[]'.to_sym", ":[]"),
            ("'[]='.to_sym", ":[]="),
            ("'<=>'.to_sym", ":<=>"),
            // Instance variable strings
            ("'@foo'.to_sym", ":@foo"),
            ("'@@foo'.to_sym", ":@@foo"),
            // Global variable strings
            ("'$foo'.to_sym", ":$foo"),
            // Strings that need quoting as symbols
            ("'foo-bar'.to_sym", ":\"foo-bar\""),
            ("'foo bar'.to_sym", ":\"foo bar\""),
        ];
        for (source, expected) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected `{}` in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn double_quoted_hash_key_with_suffix() {
        let cop = SymbolConversion;
        // Double-quoted hash keys with ? and ! suffix
        let offense_cases = [
            ("{ \"foo?\": 1 }", "foo?:"),
            ("{ \"foo!\": 1 }", "foo!:"),
            ("{ \"Foo\": 1 }", "Foo:"),
            ("{ \"_foo\": 1 }", "_foo:"),
        ];
        for (source, expected) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected `{}` in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn comprehensive_no_offense_check() {
        let cop = SymbolConversion;
        let no_offense_cases: Vec<&str> = vec![
            // Bare symbols
            ":foo",
            ":foo_bar",
            ":Foo",
            // Symbols that need quoting
            ":\"foo-bar\"",
            ":'foo-bar'",
            ":\"foo bar\"",
            ":'foo bar'",
            ":\"foo:bar\"",
            // Setter symbols (properly quoted)
            ":\"foo=\"",
            ":'foo='",
            // Bare hash keys
            "{ foo: 1 }",
            // Hash keys that need quoting
            "{ 'foo-bar': 1 }",
            "{ 'foo bar': 1 }",
            "{ '==': 1 }",
            "{ 'foo=': 1 }",
            // Empty symbol
            ":\"\"",
            // Percent arrays
            "%i(foo bar)",
            "%I(foo bar)",
            // Alias
            "alias foo bar",
            // to_sym on non-literal
            "name.to_sym",
            "x.to_sym",
            // to_sym with args
            "\"foo\".to_sym(1)",
            // Escape-needed symbols
            ":\"\\n\"",
            ":\"\\t\"",
            // Rocket-style with non-identifier start
            "{ :'@ivar' => 1 }",
            "{ :'+' => 1 }",
            // Numeric-start hash key (needs quotes)
            "{ '7_days': 1 }",
            "{ \"7_days\": 1 }",
            // Alias arguments — RuboCop skips these (in_alias? check)
            "alias :'foo' bar",
            "alias :\"foo\" bar",
            "alias bar :'foo'",
            "alias bar :\"foo\"",
            "alias :'foo' :'bar'",
            "alias :\"foo\" :\"bar\"",
        ];
        for source in &no_offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                diags.is_empty(),
                "Expected no offense for {:?} but got: {:?}",
                source,
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn utf8_symbol_conversion() {
        let cop = SymbolConversion;
        // Ruby allows multi-byte (UTF-8) identifiers, so quoted UTF-8 symbols
        // that consist entirely of valid identifier characters should be flagged.
        let offense_cases = [
            // Standalone quoted UTF-8 symbols
            (":\"résumé\"", ":résumé"),
            (":'résumé'", ":résumé"),
            (":\"café\"", ":café"),
            (":'naïve'", ":naïve"),
            (":\"日本語\"", ":日本語"),
            // Colon-style hash keys with UTF-8
            ("{ 'résumé': 1 }", "résumé:"),
            ("{ \"café\": 1 }", "café:"),
            // .to_sym on UTF-8 string
            ("\"résumé\".to_sym", ":résumé"),
            ("'café'.to_sym", ":café"),
        ];
        for (source, expected) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected `{}` in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }

        // UTF-8 symbols that still require quoting (non-identifier chars mixed in)
        let no_offense_cases = [
            ":\"foo-café\"",   // hyphen
            ":\"résumé bar\"", // space
        ];
        for source in &no_offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                diags.is_empty(),
                "Expected no offense for {:?} but got: {:?}",
                source,
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn emoji_hash_keys_not_flagged() {
        let cop = SymbolConversion;
        // Emoji hash keys are not flagged because RuboCop's correct_hash_key
        // only converts keys starting with /\A[a-z0-9_]/i (ASCII only).
        let no_offense_cases = [
            "{ \"\u{1F1FA}\u{1F1F8}\": \"hello\" }", // 🇺🇸
            "{ \"\u{1F3E0}\": \"house\" }",          // 🏠
            "{ \"\u{1F389}\": \"party\" }",          // 🎉
        ];
        for source in &no_offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                diags.is_empty(),
                "Expected no offense for {:?} but got: {:?}",
                source,
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn percent_string_to_sym() {
        let cop = SymbolConversion;
        // %(...)  notation with interpolation and .to_sym should be flagged
        let offense_cases = [
            ("%(cover_#{face}_image).to_sym", ":\"cover_#{face}_image\""),
            ("%(#{periphery}_bg).to_sym", ":\"#{periphery}_bg\""),
            ("%(prefix_#{name}).intern", ":\"prefix_#{name}\""),
        ];
        for (source, expected) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected `{}` in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }

        // Non-interpolated %(hello).to_sym is a StringNode, handled by existing code
        let simple_cases = [
            ("%(hello).to_sym", ":hello"),
            ("%(foo_bar).to_sym", ":foo_bar"),
        ];
        for (source, expected) in &simple_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected `{}` in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn special_global_variable_symbols() {
        let cop = SymbolConversion;
        // These should be flagged — they can be unquoted
        let offense_cases = [
            (r#":"$1""#, ":$1"),
            (r#":"$?""#, ":$?"),
            (r#":"$!""#, ":$!"),
            (r#":"$0""#, ":$0"),
            (r#":"$~""#, ":$~"),
        ];
        for (source, expected_correction) in &offense_cases {
            let diags = crate::testutil::run_cop_full(&cop, source.as_bytes());
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0]
                    .message
                    .contains(&format!("`{expected_correction}`")),
                "Expected correction {} in message for {:?}, got: {}",
                expected_correction,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn consistent_offense_fixture() {
        crate::testutil::assert_cop_offenses_full_with_config(
            &SymbolConversion,
            include_bytes!(
                "../../../tests/fixtures/cops/lint/symbol_conversion/consistent_offense.rb"
            ),
            consistent_config(),
        );
    }

    #[test]
    fn consistent_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &SymbolConversion,
            include_bytes!(
                "../../../tests/fixtures/cops/lint/symbol_conversion/consistent_no_offense.rb"
            ),
            consistent_config(),
        );
    }

    fn consistent_config() -> crate::cop::CopConfig {
        use std::collections::HashMap;
        crate::cop::CopConfig {
            options: HashMap::from([(
                "EnforcedStyle".to_string(),
                serde_yml::Value::String("consistent".to_string()),
            )]),
            ..Default::default()
        }
    }

    #[test]
    fn consistent_style_bare_key_with_quoting_required_sibling() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // When any key requires quoting, bare keys should be flagged for consistency
        let source = b"{ a: 1, 'b-c': 2 }";
        let diags = crate::testutil::run_cop_full_with_config(&cop, source, config);
        assert_eq!(diags.len(), 1, "Expected 1 offense but got {:?}", diags);
        assert!(
            diags[0]
                .message
                .contains("Symbol hash key should be quoted for consistency"),
            "Expected consistency message, got: {}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains(r#"`"a":`"#),
            "Expected `\"a\":` in message, got: {}",
            diags[0].message
        );
    }

    #[test]
    fn consistent_style_already_quoted_is_fine() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // Already quoted keys alongside a key requiring quoting → no offense
        let no_offense_cases: &[&[u8]] = &[
            b"{ 'a': 1, 'b-c': 2 }",
            b"{ \"a\": 1, 'b-c': 2 }",
            b"{ 'a': 1, \"b\": 2, 'c-d': 3 }",
        ];
        for source in no_offense_cases {
            let diags =
                crate::testutil::run_cop_full_with_config(&cop, *source, consistent_config());
            assert!(
                diags.is_empty(),
                "Expected no offense for {:?} but got: {:?}",
                std::str::from_utf8(source).unwrap(),
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn consistent_style_all_bare_no_quoting_needed() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // All bare keys, no key requires quoting → no offense
        let source = b"{ a: 1, b: 2 }";
        let diags = crate::testutil::run_cop_full_with_config(&cop, source, config);
        assert!(diags.is_empty(), "Expected no offense but got: {:?}", diags);
    }

    #[test]
    fn consistent_style_all_quoted_none_required() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // All quoted keys but none require quoting → flag each (like strict)
        let source = b"{ 'a': 1, 'b': 2 }";
        let diags = crate::testutil::run_cop_full_with_config(&cop, source, config);
        assert_eq!(diags.len(), 2, "Expected 2 offenses but got {:?}", diags);
        assert!(diags[0].message.contains("`a:`"));
        assert!(diags[1].message.contains("`b:`"));
    }

    #[test]
    fn consistent_style_multiple_bare_keys_one_requires() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // Multiple bare keys, one requires quoting → flag all bare
        let source = b"{ a: 1, b: 2, 'c-d': 3 }";
        let diags = crate::testutil::run_cop_full_with_config(&cop, source, config);
        assert_eq!(diags.len(), 2, "Expected 2 offenses but got {:?}", diags);
        assert!(diags[0].message.contains("quoted for consistency"));
        assert!(diags[0].message.contains(r#"`"a":`"#));
        assert!(diags[1].message.contains("quoted for consistency"));
        assert!(diags[1].message.contains(r#"`"b":`"#));
    }

    #[test]
    fn consistent_style_keyword_hash() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // Keyword hash (method args) should also be checked
        let source = b"method(a: 1, 'b-c': 2)";
        let diags = crate::testutil::run_cop_full_with_config(&cop, source, config);
        assert_eq!(diags.len(), 1, "Expected 1 offense but got {:?}", diags);
        assert!(diags[0].message.contains("quoted for consistency"));
        assert!(diags[0].message.contains(r#"`"a":`"#));
    }

    #[test]
    fn consistent_style_standalone_symbols_unchanged() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // Standalone symbols (not hash keys) should still be flagged
        let offense_cases = [
            (r#":"foo""#, ":foo"),
            (r#":'bar'"#, ":bar"),
            (r#""hello".to_sym"#, ":hello"),
        ];
        for (source, expected) in &offense_cases {
            let diags = crate::testutil::run_cop_full_with_config(
                &cop,
                source.as_bytes(),
                consistent_config(),
            );
            assert!(
                !diags.is_empty(),
                "Expected offense for {:?} but got none",
                source
            );
            assert!(
                diags[0].message.contains(&format!("`{expected}`")),
                "Expected `{}` in message for {:?}, got: {}",
                expected,
                source,
                diags[0].message
            );
        }
    }

    #[test]
    fn consistent_style_setter_key_triggers_consistency() {
        let cop = SymbolConversion;
        let config = consistent_config();
        // A setter key (foo=) requires quoting → flag bare siblings
        let source = b"{ 'foo=': 1, bar: 2 }";
        let diags = crate::testutil::run_cop_full_with_config(&cop, source, config);
        assert_eq!(diags.len(), 1, "Expected 1 offense but got {:?}", diags);
        assert!(diags[0].message.contains("quoted for consistency"));
        assert!(diags[0].message.contains(r#"`"bar":`"#));
    }
}
