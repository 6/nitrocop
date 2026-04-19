use crate::cop::shared::node_type::{CALL_NODE, CALL_OPERATOR_WRITE_NODE};
use crate::cop::{Cop, CopConfig};
use crate::diagnostic::{Diagnostic, Severity};
use crate::parse::source::SourceFile;

/// Rails/TimeZone — checks for Time methods without zone.
///
/// ## Investigation (2026-03-10)
///
/// **FP root cause (qualified constant paths):** `util::constant_name()` extracted
/// just the last segment of a ConstantPathNode, so `Some::Time.now` matched as
/// `Time` and was falsely flagged. Fix: inline the constant check to verify the
/// receiver is either a bare `Time` (ConstantReadNode) or root-qualified `::Time`
/// (ConstantPathNode with parent=None/cbase). Matches RuboCop's `(const {nil? cbase} :Time)`.
///
/// **FN root cause (extra SAFE_METHODS):** `getutc`, `rfc2822`, `rfc822`, `to_r`
/// were in the safe methods list but are NOT in RuboCop's ACCEPTED_METHODS, causing
/// `Time.now.getutc` etc. to be incorrectly exempted. Removed these methods.
///
/// **Remaining gaps (historical note):**
/// - Byte-level chain scanner vs RuboCop's AST parent walking: the scanner works
///   correctly for most cases because `call.location().end_offset()` ends at the
///   closing paren of arguments, so `foo(Time.now).utc` correctly sees `)` (not
///   `.utc`) after `Time.now`. Edge cases with complex nesting may still diverge.
///
/// ## Investigation (2026-03-15): FP=7, FN=59
///
/// Two fixes:
///
/// **1. Nested `Time.now`/`Time.local` inside outer call with safe chain (FP fix, 7 FPs):**
/// `Time.to_mongo(Time.local(...)).zone` — the inner `Time.local(...)` was flagged because
/// `enclosing_call_is_safe` only checked whether the immediate enclosing method was safe
/// (e.g., `to_mongo` — not safe) but didn't scan the chain AFTER the outer call's closing
/// `)`. Also, it only checked the first argument position (`(`), not later arguments
/// (`Time.parse(x, Time.now).iso8601`). Fix: replaced direct `(` check with
/// `find_enclosing_open_paren()` that scans backward through balanced parens to find the
/// containing `(` regardless of argument position. Added `find_matching_close_paren()` to
/// locate the outer call's closing `)`, then `chain_contains_tz_safe_method()` checks the
/// chain continuing after it.
///
/// **2. `String#to_time` detection in both modes (FN fix):**
/// RuboCop's `on_send` fires on `.to_time` unconditionally (no mode check), but ONLY when
/// the receiver is a string literal (`node.receiver&.str_type?`). Previously nitrocop
/// only flagged in strict mode AND flagged variable receivers too. Fixed to:
/// - Fire in both flexible and strict mode (matching RuboCop's unconditional `on_send`)
/// - Only flag when receiver is a string literal node (not variables)
///
/// ## Investigation (2026-03-15): FP=17, FN=82
///
/// Two fixes:
///
/// **1. `.localtime` without args now treated as unsafe (FN fix, ~82 FNs):**
/// RuboCop treats `.localtime` without arguments as an offense (MSG_LOCALTIME) and
/// `.localtime(offset)` as accepted. Previously all `.localtime` was in SAFE_METHODS.
/// Fix: removed `localtime` from `chain_contains_tz_safe_method` SAFE_METHODS and added
/// special handling that only treats it as safe when followed by `(` with arguments.
///
/// **2. `Time.now` inside `Time.at(..., in:)` — REVERTED (was FP fix, ~10 FPs):**
/// Previously added `IN_KEYWORD_METHODS` check in `enclosing_call_is_safe_recursive` to
/// suppress inner `Time.now` when the outer call had `in:` keyword. This was incorrect:
/// RuboCop's `in:` keyword only makes the OUTER `Time.at` call safe (handled at line 250
/// via `has_in_keyword_arg`), but inner `Time.now` arguments should still be flagged.
/// The original "FP" was actually correct behavior — removed the IN_KEYWORD_METHODS check
/// and `enclosing_parens_have_in_keyword` function. Fixes 26 FN in corpus.
///
/// ## Investigation (2026-03-14): FP=25
///
/// Two root causes addressed:
///
/// **1. Interpolated string timezone specifier (ManageIQ/feedjira, ~4 FPs):**
/// `Time.parse("#{ts} UTC", ...)` — the first argument is a dstr (interpolated string)
/// ending with a timezone indicator. RuboCop's `attach_timezone_specifier?` checks
/// `date.respond_to?(:value)`. In RuboCop's AST, dstr nodes for `"#{expr} UTC"` have
/// a last child of `str(" UTC")`. The check implicitly covers this via the last part.
/// Fix: added explicit check of the last string literal part of InterpolatedStringNode.
///
/// **2. Time.now inside Time.utc(...) arguments (ice_cube, ~4 FPs):**
/// `Time.utc(Time.now.year - 1, ...)` — RuboCop's parent-chain walking traverses
/// through the argument position into the enclosing call, making chain = [now, year, -, utc].
/// `utc` is in ACCEPTED_METHODS → not_danger_chain? returns true → no offense.
/// Nitrocop's forward byte scanner stops at the `)` following `Time.now.year-1` and
/// doesn't see the outer `Time.utc(...)` call.
/// Fix: added `enclosing_call_is_safe()` backward scan: if Time.now is directly preceded
/// by `safe_method(`, suppress the offense.
///
/// ## Investigation (2026-03-18): FP=1, FN=361
///
/// **FN root cause (`String#to_time` in flexible mode):**
/// RuboCop's `on_send` fires on `.to_time` in BOTH strict and flexible mode, but only
/// when the receiver is a string literal (`node.receiver&.str_type?`). Nitrocop was only
/// firing in strict mode AND was incorrectly flagging variable receivers (e.g.,
/// `date_str.to_time`). Fixed by: (1) removing the strict-mode gate so `to_time` fires
/// in both modes, (2) checking that the receiver is a string literal node before flagging.
///
/// ## Investigation (2026-03-18): FP=1, FN=334
///
/// **FN root cause (grouping parens treated as method-call parens):**
/// `enclosing_call_is_safe()` found the enclosing `(` via `find_enclosing_open_paren()`,
/// then extracted the "method name" before it. For grouping parens like
/// `(Time.now - 1.day).to_i`, the `(` had no method name — it picked up an identifier
/// from a completely different line/statement. Even when no identifier was found (e.g.,
/// after `=` or `||`), it fell through to check the chain after the closing `)`, where
/// `.to_i` was in SAFE_METHODS and incorrectly suppressed the offense.
///
/// Fix: three guards before treating the enclosing `(` as method-call parens:
/// 1. `method_start > end_of_method` — no identifier at all (preceded by operator/punct)
/// 2. Newline between method name and `(` — identifier is from a different statement
/// 3. Method name is a Ruby keyword (`return`, `if`, etc.) — keywords use grouping parens
///
/// Fixes ~300+ of the 334 FN (grouping paren patterns in corpus).
///
/// ## Investigation (2026-03-18): FP=1, FN=43
///
/// **FN root cause (method name truncation with `?`/`!` suffixes):**
/// `chain_contains_tz_safe_method()` read method names using only alphanumeric + underscore,
/// truncating `utc?` to `utc` which matched SAFE_METHODS and suppressed the offense.
/// Fix: include `?` and `!` in the method name character set so `utc?` is read as `utc?`
/// and does NOT match `utc` in SAFE_METHODS. Fixes 43 FN across jruby, natalie-lang,
/// sidekiq and other corpus repos.
///
/// **FP root cause (deeply nested parens):**
/// `Time.parse(helper_method(Time.now)).utc` — the inner `Time.now` is nested two levels
/// deep. `enclosing_call_is_safe()` only checked the immediate enclosing `(` (from
/// `helper_method`), which was not safe, and the chain after `helper_method(...)` was `)`
/// (not `.utc`). Fix: made `enclosing_call_is_safe()` recursive (up to 3 levels) so it
/// checks the next enclosing `(` (from `Time.parse`), whose chain after `)` is `.utc`.
///
/// ## Investigation (2026-03-19): FN=16
///
/// **1. Grouping parens with space (12 FN — FIXED):** `schedule (Time.now - 60).to_f` —
/// backward scan found `schedule` before `(`, but the space between `schedule` and `(`
/// means the `(` is a subexpression grouping paren. The `.to_f` after `)` chains on the
/// grouped expression, not the `schedule` call. Fix: detect space/tab in the gap between
/// method name and `(`; when present (`is_spaced_paren`), skip the chain-after-closing-paren
/// check in `enclosing_call_is_safe_recursive`.
///
/// **2. Safe navigation `&.` chain break (2+ FN — REVERTED):** `Time.at(val)&.utc` — in
/// RuboCop, `csend` (safe navigation) is not `send_type?`, so `extract_method_chain` stops
/// at `&.utc`. Attempted fix: removed `&.` handling from `chain_contains_tz_safe_method`.
/// REVERTED because corpus rerun showed FP=405: many legitimate `Time.at(x)&.utc` patterns
/// in the corpus ARE accepted by RuboCop (possibly via a different code path or because
/// `csend` handling changed in newer parser versions). Restoring `&.` handling eliminates
/// the 405 FPs while leaving ~2-4 FN from the `&.` pattern. The FN are acceptable since
/// `&.utc` genuinely provides timezone safety.
///
/// Remaining FN after spaced-paren fix: ~4-6 (2+ from `&.` pattern, plus possible
/// other patterns in rack-contrib, rack-cache, TracksApp, flippercloud, hackclub).
///
/// ## Investigation (2026-03-19): FN=7 (corpus oracle)
///
/// Investigated all 7 FN across 5 repos. Two fixes applied:
///
/// **1. Grouping paren recursion (2 FN in rack-cache — FIXED):**
/// `Time.httpdate((Time.now - (60**2)).httpdate)` — the inner `(Time.now - ...)`
/// is a grouping paren. Previously, `enclosing_call_is_safe_recursive` recursed
/// past it and found the outer `Time.httpdate(` whose method `httpdate` was in
/// SAFE_METHODS, incorrectly suppressing the offense. In RuboCop, grouping parens
/// create a `begin` AST node which stops `extract_method_chain`'s parent walk.
/// Fix: when `is_grouping_paren` is true, return false immediately instead of
/// recursing. This matches RuboCop's chain-breaking `begin` node semantics.
///
/// **2. String interpolation boundary (1 FN in flippercloud — FIXED):**
/// `"tolerance zone (#{Time.at(ts)})"` — `find_enclosing_open_paren` scanned
/// backward through the `#{` interpolation boundary and found the literal `(`
/// from the string text `zone (`. The word `zone` before it matched SAFE_METHODS,
/// incorrectly suppressing the offense. Fix: stop backward scan at `#{` boundaries
/// (return None when `bytes[i] == b'{'` and `bytes[i-1] == b'#'`).
///
/// **Previous 4 FN now FIXED (2026-03-19):**
///
/// **1. `&.` breaks chain (feedbin, 2 FN):** RuboCop's `extract_method_chain` uses
/// `node.send_type?` which excludes `csend` (safe navigation). So `Time.at(x)&.utc`
/// does NOT see `utc` in the chain → offense. Previous attempt was reverted due to
/// 405 FP, but the current corpus oracle baseline reflects the correct behavior.
/// Fix: stop `chain_contains_tz_safe_method` at `&.` instead of following it.
///
/// **2. `method_from_time_class?` gate (hackclub, 1 FN + 3 fixture corrections):**
/// `Duration.build(Time.now).seconds.to_i` — `.to_i` is on Duration, not Time.
/// RuboCop's `method_from_time_class?` returns false for non-Time receivers, so
/// `.to_i` isn't added to the chain. Fix: added `receiver_traces_to_time()` helper
/// and gated both the direct method name check AND chain-after-paren check in
/// `enclosing_call_is_safe_recursive`. Also corrected `foo(Time.now).in_time_zone`,
/// `bar(Time.local(...)).to_i`, and `wrap(Time.now).zone` from no_offense to
/// offense — these have non-Time receivers, so the safe chain doesn't apply.
///
/// **3. Non-dangerous Time.XXX in dangerous enclosing Time call (TracksApp, 1 FN):**
/// `Time.zone.local(year, month, Time.days_in_month(month))` — RuboCop's chain
/// walk goes up from inner `Time` through `days_in_month` to enclosing `local`,
/// where `method_from_time_class?` confirms the receiver traces to Time. Chain =
/// `[:days_in_month, :local]`, `:local` is dangerous, no good method → offense.
/// Fix: added `in_dangerous_time_context()` check for non-dangerous Time methods
/// that detects enclosing dangerous Time calls without safe chains.
///
/// ## Investigation (2026-03-19): FP=40
///
/// **GOOD_METHODS neutralize danger in chain (40 FP — FIXED):**
/// `Time.new(2005, 10, 30, 0, 0, 0, Time.zone)` — the inner `Time.zone` call
/// triggered `in_dangerous_time_context()` which found the enclosing `Time.new(`
/// as dangerous. But RuboCop's `extract_method_chain` builds chain `[:zone, :new]`,
/// and `not_danger_chain?` returns true because `:zone` is in GOOD_METHODS. The
/// presence of ANY good method in the chain neutralizes the danger.
/// Fix: skip `in_dangerous_time_context()` when the inner method is a GOOD_METHOD
/// (`zone`, `zone_default`, `find_zone`, `find_zone!`). Also affects patterns like
/// `Time.new(2019, 1, 1, 0, 0, 0, "+03:00")` which already passed via the 7-arg
/// check, and `Time.new(2010, 1, 1, 0, 0, 0, "+10:00")` similarly.
///
/// ## Investigation (2026-03-23): FP=3, FN=4 (extended corpus)
///
/// **FP1: Bracket indexing breaks chain scanner (1 FP — FIXED):**
/// `Time.now.year.to_s[0..1].to_i` — `chain_contains_tz_safe_method` stopped at
/// `[` because it only followed `.method` patterns. The `[0..1]` bracket indexing
/// is not a method call separator but shouldn't break the chain. Added bracket
/// skipping (`[...]`) in the chain scanner loop so `.to_i` after `[0..1]` is found.
///
/// **FP2-3: ACCEPTED_METHODS inside dangerous Time context (2 FP — FIXED):**
/// `Time.zone.at(... || Time.current)` and `Time.zone.parse(Time.utc(...).to_s)` —
/// `in_dangerous_time_context()` found the enclosing dangerous Time call and flagged
/// the inner `Time.current`/`Time.utc`. But in RuboCop's flexible mode, `good_methods`
/// includes `GOOD_METHODS + [:current] + ACCEPTED_METHODS`. The inner method being
/// `current` or `utc` makes `not_danger_chain?` return true. Fix: in the
/// `!is_unsafe_method` branch, skip `in_dangerous_time_context()` when the inner method
/// is in ACCEPTED_METHODS or is `:current` (in flexible mode).
///
/// **FN1-3: Splat breaks chain walk (3 FN — FIXED):**
/// `Time.new(*Time.now.to_a[0..5].reverse[0..4]).to_i` — RuboCop's chain walk from
/// `Time.now` stops at the `*` splat (not `send_type?`), never reaching `Time.new` or
/// `.to_i`. But `enclosing_call_is_safe` found the outer `Time.new(` and chain `.to_i`,
/// incorrectly suppressing. Fix: detect `*` between `(` and the inner call position in
/// `enclosing_call_is_safe_recursive`; when present, return false (chain is broken).
///
/// **FN4: CallOperatorWriteNode (1 FN — FIXED):**
/// `Time.now += secs` — Prism parses this as `CallOperatorWriteNode`, not `CallNode`.
/// The cop only handled `CALL_NODE`. Fix: added `CALL_OPERATOR_WRITE_NODE` to
/// `interested_node_types` and handling for `read_name()` == dangerous method with
/// `Time` receiver.
///
/// ## Investigation (2026-04-17): strict variant FP=35, FN=0
///
/// **Strict mode GOOD_METHODS chain (35 FP — FIXED):**
/// RuboCop's strict style still treats GOOD_METHODS (`zone`, `zone_default`,
/// `find_zone`, `find_zone!`) as neutralizing methods in the extracted parent chain.
/// Nitrocop only applied chain/enclosing-call safety checks in flexible mode, so
/// strict-mode patterns like `Time.now.zone`, `"#{Time.now.zone}"`,
/// `Time.to_mongo(Time.local(...)).zone`, and `Time.new(...).zone` were falsely
/// flagged. Fix: made the chain scanner, enclosing-call walker, and dangerous-context
/// suppression style-aware so strict mode accepts ONLY GOOD_METHODS while flexible
/// mode retains the broader ACCEPTED_METHODS behavior. This keeps `Time.now.to_i`
/// flagged in strict mode while matching RuboCop on `.zone` chains.
pub struct TimeZone;

impl Cop for TimeZone {
    fn name(&self) -> &'static str {
        "Rails/TimeZone"
    }

    fn default_severity(&self) -> Severity {
        Severity::Convention
    }

    fn default_exclude(&self) -> &'static [&'static str] {
        &["**/*.gemspec"]
    }

    fn interested_node_types(&self) -> &'static [u8] {
        &[CALL_NODE, CALL_OPERATOR_WRITE_NODE]
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
        // Handle CallOperatorWriteNode (e.g., `Time.now += secs`).
        // In Prism, `a.b += c` is a CallOperatorWriteNode with read_name=b, receiver=a.
        // RuboCop fires on the "read" part (`Time.now`) as a regular send node, because
        // `a.b += c` expands to `a.b = a.b + c` and `a.b` is a send node.
        if let Some(op_write) = node.as_call_operator_write_node() {
            let method = op_write.read_name().as_slice();
            if matches!(method, b"now" | b"parse" | b"at" | b"new" | b"local") {
                if let Some(recv) = op_write.receiver() {
                    let is_time = if let Some(cr) = recv.as_constant_read_node() {
                        cr.name().as_slice() == b"Time"
                    } else if let Some(cp) = recv.as_constant_path_node() {
                        cp.parent().is_none() && cp.name().map(|n| n.as_slice()) == Some(b"Time")
                    } else {
                        false
                    };
                    if is_time {
                        let loc = match op_write.message_loc() {
                            Some(l) => l,
                            None => op_write.location(),
                        };
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(self.diagnostic(
                            source,
                            line,
                            column,
                            format!(
                                "Use `Time.zone.{}` instead of `Time.{}`.",
                                String::from_utf8_lossy(method),
                                String::from_utf8_lossy(method)
                            ),
                        ));
                    }
                }
            }
            return;
        }

        let call = match node.as_call_node() {
            Some(c) => c,
            None => return,
        };

        let method = call.name().as_slice();

        // String#to_time detection — fires in BOTH strict and flexible mode.
        // RuboCop's on_send fires unconditionally (no mode check) but only when the
        // receiver is a string literal (`node.receiver&.str_type?`). This means
        // `"string".to_time` is always an offense, but `variable.to_time` is not.
        // Also skips when the string has a timezone specifier (e.g., "...Z").
        if method == b"to_time" {
            // Only flag when receiver is a string literal
            if let Some(recv) = call.receiver() {
                if let Some(str_node) = recv.as_string_node() {
                    let content = str_node.unescaped();
                    if !has_timezone_specifier(content) {
                        let loc = call.message_loc().unwrap_or(call.location());
                        let (line, column) = source.offset_to_line_col(loc.start_offset());
                        diagnostics.push(
                            self.diagnostic(
                                source,
                                line,
                                column,
                                "Do not use `String#to_time` without zone. Use `Time.zone.parse` instead."
                                    .to_string(),
                            ),
                        );
                    }
                }
                // Non-string receivers (variables, expressions) are never flagged
            }
            // No receiver (bare `to_time`) — not an offense
            return;
        }

        // Methods that are timezone-unsafe on Time (matches RuboCop's DANGEROUS_METHODS)
        // Note: utc, gm, mktime are NOT dangerous — they already produce UTC times
        let is_unsafe_method = matches!(method, b"now" | b"parse" | b"at" | b"new" | b"local");

        let recv = match call.receiver() {
            Some(r) => r,
            None => return,
        };
        // Handle ConstantReadNode (Time) and ConstantPathNode (::Time) but NOT
        // qualified paths like Some::Time — only unqualified or root-qualified.
        // RuboCop: (const {nil? cbase} :Time)
        let is_time_receiver = if let Some(cr) = recv.as_constant_read_node() {
            cr.name().as_slice() == b"Time"
        } else if let Some(cp) = recv.as_constant_path_node() {
            // ::Time — parent must be None (cbase), not Some::Time
            cp.parent().is_none() && cp.name().map(|n| n.as_slice()) == Some(b"Time")
        } else {
            false
        };
        if !is_time_receiver {
            return;
        }

        let style = config.get_str("EnforcedStyle", "flexible");
        let strict = style == "strict";

        // Non-dangerous method on Time (e.g., Time.days_in_month) — check if it's
        // inside a dangerous enclosing Time call. RuboCop's extract_method_chain walks
        // up through ALL parents, and method_from_time_class? adds the enclosing method
        // to the chain when the receiver traces to Time. So Time.zone.local(year, month,
        // Time.days_in_month(month)) has chain [:days_in_month, :local] and :local is
        // dangerous with no good method → offense.
        //
        // However, if the inner method is a GOOD_METHOD (zone, zone_default, find_zone,
        // find_zone!), it neutralizes the danger — RuboCop's not_danger_chain? returns
        // true when the chain includes any good method. E.g., Time.zone inside
        // Time.new(2005, 10, 30, 0, 0, 0, Time.zone) has chain [:zone, :new], and
        // :zone is good → no offense.
        if !is_unsafe_method {
            // GOOD_METHODS neutralize any dangerous method in the chain (both modes).
            if matches!(
                method,
                b"zone" | b"zone_default" | b"find_zone" | b"find_zone!"
            ) {
                return;
            }
            // In flexible mode, ACCEPTED_METHODS and :current also neutralize danger.
            // RuboCop's good_methods in flexible mode includes GOOD_METHODS + [:current]
            // + ACCEPTED_METHODS. If the inner method is one of these, not_danger_chain?
            // returns true and the offense is suppressed.
            if style == "flexible"
                && matches!(
                    method,
                    b"current"
                        | b"utc"
                        | b"gm"
                        | b"mktime"
                        | b"in_time_zone"
                        | b"getlocal"
                        | b"xmlschema"
                        | b"iso8601"
                        | b"jisx0301"
                        | b"rfc3339"
                        | b"httpdate"
                        | b"to_i"
                        | b"to_f"
                )
            {
                return;
            }
            let bytes = source.as_bytes();
            let start = call.location().start_offset();
            if let Some((dangerous_method, msg_loc)) =
                in_dangerous_time_context(bytes, start, source, strict)
            {
                let (line, column) = source.offset_to_line_col(msg_loc);
                diagnostics.push(self.diagnostic(
                    source,
                    line,
                    column,
                    format!(
                        "Use `Time.zone.{}` instead of `Time.{}`.",
                        dangerous_method, dangerous_method
                    ),
                ));
            }
            return;
        }

        // RuboCop skips Time.parse/new/at when the first string argument already has
        // a timezone specifier (e.g., "2023-05-29 00:00:00 UTC", "2015-03-02T19:05:37Z",
        // "2015-03-02T19:05:37+05:00"). Pattern: /([A-Za-z]|[+-]\d{2}:?\d{2})\z/
        // Also handles interpolated strings like "#{ts} UTC" by checking the last
        // string literal part (RuboCop's `dstr.value` implicitly returns last str part).
        if let Some(args) = call.arguments() {
            let first_arg = args.arguments().iter().next();
            if let Some(arg) = first_arg {
                if let Some(str_node) = arg.as_string_node() {
                    let content = str_node.unescaped();
                    if has_timezone_specifier(content) {
                        return;
                    }
                }
                // Handle interpolated strings: check the last literal string part.
                // `"#{ts} UTC"` has last part " UTC" which ends with a letter → safe.
                if let Some(dstr) = arg.as_interpolated_string_node() {
                    let last_str = dstr
                        .parts()
                        .iter()
                        .filter_map(|p| p.as_string_node())
                        .last();
                    if let Some(last) = last_str {
                        if has_timezone_specifier(last.unescaped()) {
                            return;
                        }
                    }
                }
            }
        }

        // Skip Time.new/at/now with `in:` keyword argument (timezone offset provided)
        if (method == b"at" || method == b"now" || method == b"new") && has_in_keyword_arg(&call) {
            return;
        }
        // Time.new with 7 arguments (last is timezone offset)
        if method == b"new" {
            if let Some(args) = call.arguments() {
                let arg_count = args.arguments().iter().count();
                if arg_count == 7 {
                    return;
                }
            }
        }

        // RuboCop treats a dangerous method as safe when the parent chain contains
        // any style-allowed good method. In flexible mode this includes
        // ACCEPTED_METHODS; in strict mode it is limited to GOOD_METHODS like `.zone`.
        let bytes = source.as_bytes();
        let end = call.location().end_offset();
        if chain_contains_tz_safe_method(bytes, end, strict) {
            return;
        }

        // RuboCop also walks UP via node.parent, which means it considers the
        // enclosing call context. For `Time.utc(Time.now.year - 1, ...)`, the
        // chain becomes [now, year, -, utc] and `utc` makes it safe in flexible mode.
        // In strict mode, only GOOD_METHODS like `.zone` should suppress.
        let start = call.location().start_offset();
        if enclosing_call_is_safe(bytes, start, strict) {
            return;
        }

        let loc = call.message_loc().unwrap_or(call.location());
        let (line, column) = source.offset_to_line_col(loc.start_offset());
        diagnostics.push(self.diagnostic(
            source,
            line,
            column,
            format!(
                "Use `Time.zone.{}` instead of `Time.{}`.",
                String::from_utf8_lossy(method),
                String::from_utf8_lossy(method)
            ),
        ));
    }
}

/// Check if a call has an `in:` keyword argument (for timezone offset).
fn has_in_keyword_arg(call: &ruby_prism::CallNode<'_>) -> bool {
    let args = match call.arguments() {
        Some(a) => a,
        None => return false,
    };

    // Check the last argument for a keyword hash with `in:` key
    let last_arg = args.arguments().iter().last();
    if let Some(arg) = last_arg {
        // Keyword hash argument (keyword args in method calls)
        if let Some(kw_hash) = arg.as_keyword_hash_node() {
            for elem in kw_hash.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == b"in" {
                            // Value must not be nil
                            if assoc.value().as_nil_node().is_none() {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        // Regular hash argument
        if let Some(hash) = arg.as_hash_node() {
            for elem in hash.elements().iter() {
                if let Some(assoc) = elem.as_assoc_node() {
                    if let Some(sym) = assoc.key().as_symbol_node() {
                        if sym.unescaped() == b"in" && assoc.value().as_nil_node().is_none() {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

/// Check if a string value ends with a timezone specifier.
/// Matches RuboCop's TIMEZONE_SPECIFIER: /([A-Za-z]|[+-]\d{2}:?\d{2})\z/
fn has_timezone_specifier(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    let last = bytes[bytes.len() - 1];
    // Ends with a letter (e.g., "UTC", "Z", "EST")
    if last.is_ascii_alphabetic() {
        return true;
    }
    // Ends with +/-HH:MM or +/-HHMM pattern
    // Check for pattern: [+-]\d{2}:?\d{2} at end
    let len = bytes.len();
    // +/-HHMM (5 chars) or +/-HH:MM (6 chars)
    if len >= 6 {
        let s = &bytes[len - 6..];
        if (s[0] == b'+' || s[0] == b'-')
            && s[1].is_ascii_digit()
            && s[2].is_ascii_digit()
            && s[3] == b':'
            && s[4].is_ascii_digit()
            && s[5].is_ascii_digit()
        {
            return true;
        }
    }
    if len >= 5 {
        let s = &bytes[len - 5..];
        if (s[0] == b'+' || s[0] == b'-')
            && s[1].is_ascii_digit()
            && s[2].is_ascii_digit()
            && s[3].is_ascii_digit()
            && s[4].is_ascii_digit()
        {
            return true;
        }
    }
    false
}

fn is_good_method(method: &[u8]) -> bool {
    matches!(
        method,
        b"zone" | b"zone_default" | b"find_zone" | b"find_zone!"
    )
}

fn is_style_safe_method(method: &[u8], has_args: bool, strict: bool) -> bool {
    if is_good_method(method) {
        return true;
    }

    if strict {
        return false;
    }

    if matches!(
        method,
        b"utc"
            | b"getlocal"
            | b"in_time_zone"
            | b"iso8601"
            | b"xmlschema"
            | b"jisx0301"
            | b"rfc3339"
            | b"httpdate"
            | b"to_i"
            | b"to_f"
            | b"current"
    ) {
        return true;
    }

    method == b"localtime" && has_args
}

/// Check if the byte at `start` (beginning of `Time.now` etc.) is immediately
/// inside the argument list of a timezone-safe method call.
///
/// This handles the case where RuboCop's parent-chain walking finds a safe method
/// in the enclosing context. For `Time.utc(Time.now.year - 1, ...)`:
/// - Walking backwards from `Time.now` finds `(` preceded by `utc`
/// - `utc` is in the safe methods list → suppress offense
///
/// This matches RuboCop's behavior where `not_danger_chain?` returns true when
/// the parent-chain (now, year, -, utc) includes an ACCEPTED_METHOD.
fn enclosing_call_is_safe(bytes: &[u8], start: usize, strict: bool) -> bool {
    // Check up to 3 levels of nesting to handle cases like:
    // Time.parse(helper_method(Time.now)).utc
    // Level 1: helper_method( — not safe, chain after ) is ) — not safe
    // Level 2: Time.parse( — not safe itself, but chain after ) is .utc — safe!
    enclosing_call_is_safe_recursive(bytes, start, 3, strict)
}

fn enclosing_call_is_safe_recursive(
    bytes: &[u8],
    start: usize,
    max_depth: u8,
    strict: bool,
) -> bool {
    if start == 0 || max_depth == 0 {
        return false;
    }

    // Find the opening `(` of the enclosing call by scanning backward.
    // Time.now may be the first argument (preceded by `(`) or a later argument
    // (preceded by `, ` or similar). We scan backward, tracking parenthesis depth,
    // to find the matching opening `(`.
    let paren_pos = match find_enclosing_open_paren(bytes, start) {
        Some(p) => p,
        None => return false,
    };

    // Check for splat `*` between the opening paren and the inner call position.
    // A splat argument (e.g., `Time.new(*Time.now.to_a...)`) creates a `splat` AST
    // node which is NOT `send_type?`, so RuboCop's chain walk stops at the splat
    // boundary and never reaches the outer call or its subsequent chain.
    // Scan backward from `start` to `paren_pos` looking for `*` that indicates splat.
    {
        let mut j = start.saturating_sub(1);
        // Skip whitespace
        while j > paren_pos && bytes[j].is_ascii_whitespace() {
            j -= 1;
        }
        // Skip past comma-separated earlier args to find `*` right before our position
        if j > paren_pos && bytes[j] == b'*' && (j == 0 || bytes[j - 1] != b'*') {
            return false;
        }
    }

    if paren_pos == 0 {
        return false;
    }
    let mut i = paren_pos - 1;

    // Skip whitespace before `(`
    while i > 0 && bytes[i].is_ascii_whitespace() {
        i -= 1;
    }

    // Read method name backwards (alphanumeric + underscore + ? + !)
    let end_of_method = i;
    while i > 0
        && (bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'_'
            || bytes[i] == b'?'
            || bytes[i] == b'!')
    {
        i -= 1;
    }
    // Adjust for the loop decrement
    let method_start = if bytes[i].is_ascii_alphanumeric()
        || bytes[i] == b'_'
        || bytes[i] == b'?'
        || bytes[i] == b'!'
    {
        i
    } else {
        i + 1
    };
    let method_name = &bytes[method_start..=end_of_method];

    // Check if the `(` is actually a method-call argument paren vs a grouping paren.
    // Grouping parens: `(Time.now - 1.day).to_i`, `return (Time.now).to_i`
    // Method-call parens: `foo(Time.now)`, `Time.utc(Time.now)`
    //
    // Two checks:
    // 1. No newlines between the method name and `(` — prevents picking up
    //    identifiers from a completely different statement/line.
    // 2. The method name is not a Ruby keyword (return, if, unless, etc.) —
    //    keywords followed by `(` create grouping parens, not method calls.
    let gap = &bytes[end_of_method + 1..paren_pos];
    let has_newline = gap.contains(&b'\n');
    let has_space = gap.iter().any(|&b| b == b' ' || b == b'\t');
    let is_keyword = matches!(
        method_name,
        b"return"
            | b"if"
            | b"unless"
            | b"while"
            | b"until"
            | b"and"
            | b"or"
            | b"not"
            | b"when"
            | b"case"
            | b"elsif"
            | b"yield"
    );
    let is_grouping_paren = method_start > end_of_method || has_newline || is_keyword;
    // When there's a space between the method name and `(`, the `(` starts a
    // grouped subexpression within the argument list: `schedule (Time.now - 60).to_f`.
    // Any chain after `)` (like `.to_f`) is on the grouped expression, NOT on the
    // enclosing call. We must not check chain_contains_tz_safe_method after `)`.
    let is_spaced_paren = has_space && !is_grouping_paren;

    // Grouping parens (no method name, keyword parens, etc.) act as chain-breaking
    // boundaries, analogous to RuboCop's `begin` AST node which stops the chain walk.
    // Do not recurse past them or check chains through them.
    if is_grouping_paren {
        return false;
    }

    // RuboCop's method_from_time_class? gate: only count the enclosing call as
    // relevant when its receiver chain traces back to `Time`. This prevents
    // `Duration.build(Time.now).seconds.to_i` from being suppressed (receiver
    // is Duration, not Time). But `Time.utc(Time.now)` IS suppressed.
    let receiver_is_time = receiver_traces_to_time(bytes, method_start);

    if receiver_is_time && is_style_safe_method(method_name, true, strict) {
        return true;
    }

    // The enclosing function itself isn't safe, but check if the CHAIN AFTER
    // the enclosing call's closing `)` contains a safe method.
    // E.g., `Time.to_mongo(Time.local(...)).zone` — `to_mongo` is not safe,
    // but `.zone` after `Time.to_mongo(...)` IS safe.
    // Find the closing `)` that matches the `(` at paren_pos, then scan forward.
    //
    // Skip this check when there's a space between method name and `(`:
    // `schedule (Time.now - 60).to_f` — `.to_f` chains on the grouped
    // expression `(Time.now - 60)`, not on the `schedule` call.
    //
    // Only check when receiver traces to Time (method_from_time_class? gate).
    if receiver_is_time && !is_spaced_paren {
        let closing_paren = find_matching_close_paren(bytes, paren_pos);
        if let Some(close_pos) = closing_paren {
            if chain_contains_tz_safe_method(bytes, close_pos + 1, strict) {
                return true;
            }
        }
    }

    // Not safe at this level — try the next enclosing level.
    // E.g., Time.parse(helper_method(Time.now)).utc
    // At level 1: helper_method( is not safe, chain after helper_method(...) is ) — not safe
    // At level 2: Time.parse( is checked, chain after Time.parse(...) is .utc — safe!
    enclosing_call_is_safe_recursive(bytes, paren_pos, max_depth - 1, strict)
}

/// Find the opening `(` that encloses the position `pos` in the source.
/// Scans backward, tracking nested parens/brackets/braces, to find the
/// unmatched `(` that contains this position as an argument.
fn find_enclosing_open_paren(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut i = pos.saturating_sub(1);
    let mut depth = 0u32; // tracks nested closers we need to skip

    while i < bytes.len() {
        match bytes[i] {
            b'(' if depth == 0 => return Some(i),
            b'(' => depth -= 1,
            b')' => depth += 1,
            // Stop at string interpolation boundary #{...}
            // The `{` from `#{` is an expression boundary — any `(` outside it
            // is literal string content, not a Ruby method-call paren.
            b'{' if i > 0 && bytes[i - 1] == b'#' => return None,
            b'\'' | b'"' => {
                // Skip backward past string literals
                if i == 0 {
                    return None;
                }
                let quote = bytes[i];
                i -= 1;
                while i > 0 && bytes[i] != quote {
                    // Handle escaped quotes: if we see the quote preceded by \, keep going
                    if bytes[i] == quote && i > 0 && bytes[i - 1] == b'\\' {
                        i -= 1;
                    }
                    i -= 1;
                }
                // i is now at the opening quote
            }
            _ => {}
        }
        if i == 0 {
            // Check the byte at position 0
            if bytes[0] == b'(' && depth == 0 {
                return Some(0);
            }
            return None;
        }
        i -= 1;
    }
    None
}

/// Find the position of the closing `)` that matches the opening `(` at `open_pos`.
fn find_matching_close_paren(bytes: &[u8], open_pos: usize) -> Option<usize> {
    let mut pos = open_pos + 1;
    let mut depth = 1u32;
    while pos < bytes.len() && depth > 0 {
        match bytes[pos] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            b'\'' | b'"' => {
                let quote = bytes[pos];
                pos += 1;
                while pos < bytes.len() && bytes[pos] != quote {
                    if bytes[pos] == b'\\' {
                        pos += 1;
                    }
                    pos += 1;
                }
            }
            _ => {}
        }
        pos += 1;
    }
    None
}

/// Check if a non-dangerous Time method call (e.g., Time.days_in_month) is inside
/// the argument list of a dangerous Time method (e.g., Time.zone.local). Returns
/// the dangerous method name and the offset of the inner call's message_loc for
/// the offense location.
///
/// RuboCop's extract_method_chain walks up ALL parents, and method_from_time_class?
/// adds methods when receiver traces to Time. So Time.zone.local(year, month,
/// Time.days_in_month(month)) has chain [:days_in_month, :local] — :local is
/// dangerous with no good method → offense on the inner method selector.
fn in_dangerous_time_context(
    bytes: &[u8],
    start: usize,
    source: &SourceFile,
    strict: bool,
) -> Option<(String, usize)> {
    const DANGEROUS_METHODS: &[&[u8]] = &[b"now", b"parse", b"at", b"new", b"local"];

    let paren_pos = find_enclosing_open_paren(bytes, start)?;
    if paren_pos == 0 {
        return None;
    }

    let mut i = paren_pos - 1;
    // Skip whitespace
    while i > 0 && bytes[i].is_ascii_whitespace() {
        i -= 1;
    }

    // Read method name backwards
    let end_of_method = i;
    while i > 0
        && (bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'_'
            || bytes[i] == b'?'
            || bytes[i] == b'!')
    {
        i -= 1;
    }
    let method_start = if bytes[i].is_ascii_alphanumeric()
        || bytes[i] == b'_'
        || bytes[i] == b'?'
        || bytes[i] == b'!'
    {
        i
    } else {
        i + 1
    };
    if method_start > end_of_method {
        return None;
    }
    let method_name = &bytes[method_start..=end_of_method];

    // Check gap for newlines/keywords (grouping paren detection)
    let gap = &bytes[end_of_method + 1..paren_pos];
    if gap.contains(&b'\n') {
        return None;
    }

    // The enclosing method must be dangerous
    if !DANGEROUS_METHODS.contains(&method_name) {
        return None;
    }

    // The enclosing method's receiver must trace to Time
    if !receiver_traces_to_time(bytes, method_start) {
        return None;
    }

    // Check if the chain after the enclosing call's closing paren has a good method
    // If it does, suppress (e.g., Time.zone.local(..., Time.days_in_month(month)).utc)
    let closing_paren = find_matching_close_paren(bytes, paren_pos);
    if let Some(close_pos) = closing_paren {
        if chain_contains_tz_safe_method(bytes, close_pos + 1, strict) {
            return None;
        }
    }

    // Find the message_loc — use the inner call's message_loc (the method selector
    // of the non-dangerous method). `start` is the beginning of the inner `Time.XXX`
    // call. We need to find the `.method_name` part — scan from start past `Time.`
    // to find the method name.
    let _ = source; // source not needed for offset calculation
    let mut msg_pos = start;
    // Skip past `Time` or `::Time`
    if msg_pos < bytes.len() && bytes[msg_pos] == b':' {
        msg_pos += 2; // skip `::`
    }
    // Skip `Time`
    while msg_pos < bytes.len() && bytes[msg_pos].is_ascii_alphanumeric() {
        msg_pos += 1;
    }
    // Skip `.`
    if msg_pos < bytes.len() && bytes[msg_pos] == b'.' {
        msg_pos += 1;
    }
    // msg_pos now points to the method name start

    let dangerous_name = String::from_utf8_lossy(method_name).to_string();
    Some((dangerous_name, msg_pos))
}

/// Check if the receiver chain before a method traces back to `Time` as the root.
/// Starting at `method_start` (the first byte of the method name), scans backward
/// past `.method` chains to find the root receiver. Returns true only if the root
/// is `Time` (or `::Time`).
///
/// Examples:
/// - `Time.utc(` → traces: `.utc` ← `Time` → true
/// - `Time.zone.local(` → traces: `.local` ← `.zone` ← `Time` → true
/// - `Duration.build(` → traces: `.build` ← `Duration` → false
/// - `foo(` → no `.` before `foo` → false
fn receiver_traces_to_time(bytes: &[u8], method_start: usize) -> bool {
    if method_start == 0 {
        return false;
    }
    let mut i = method_start - 1;

    // Skip whitespace
    while i > 0 && bytes[i].is_ascii_whitespace() {
        i -= 1;
    }

    // Must see `.` before the method name to indicate a receiver chain
    if bytes[i] != b'.' {
        return false;
    }

    // Walk backward through `.method` segments
    loop {
        if i == 0 {
            return false;
        }
        i -= 1; // skip the `.`

        // Skip whitespace
        while i > 0 && bytes[i].is_ascii_whitespace() {
            i -= 1;
        }

        // Read identifier backwards
        let end_of_ident = i;
        while i > 0
            && (bytes[i].is_ascii_alphanumeric()
                || bytes[i] == b'_'
                || bytes[i] == b'?'
                || bytes[i] == b'!')
        {
            i -= 1;
        }
        let ident_start = if bytes[i].is_ascii_alphanumeric()
            || bytes[i] == b'_'
            || bytes[i] == b'?'
            || bytes[i] == b'!'
        {
            i
        } else {
            i + 1
        };

        if ident_start > end_of_ident {
            return false; // no identifier found
        }
        let ident = &bytes[ident_start..=end_of_ident];

        // Check what precedes this identifier
        let before = if ident_start > 0 {
            bytes[ident_start - 1]
        } else {
            b'\0' // start of file
        };

        if before == b'.' {
            // Another `.method` in the chain — continue walking
            i = ident_start - 1;
            continue;
        }

        // Check if this identifier is `Time`
        if ident == b"Time" {
            // Preceded by start-of-file, whitespace, `(`, `,`, `=`, `::`, operators, etc.
            return true;
        }
        // Not `Time` and no more `.` chain — not a Time receiver
        return false;
    }
}

/// Scan forward through a method chain starting at `pos` in `bytes`, returning
/// true if any method in the chain is a timezone-safe method. Handles chains
/// like `.to_datetime.in_time_zone(...)` by following `.method(args)` segments.
fn chain_contains_tz_safe_method(bytes: &[u8], start: usize, strict: bool) -> bool {
    let mut pos = start;
    loop {
        // Skip whitespace (including newlines for multi-line chains)
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        // Must see '.' to continue the chain.
        // Safe navigation `&.` (csend) breaks the chain — RuboCop's extract_method_chain
        // uses `node.send_type?` which excludes csend nodes, so `Time.at(x)&.utc` does
        // NOT see `utc` in the chain and still flags the offense.
        if pos >= bytes.len() || (bytes[pos] != b'.' && bytes[pos] != b'&') {
            return false;
        }
        if bytes[pos] == b'&' {
            // `&.` is safe navigation (csend) — stops the chain walk
            return false;
        }
        pos += 1; // skip the '.'

        // Skip whitespace after dot
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        // Read the method name (include ? and ! suffixes)
        let method_start = pos;
        while pos < bytes.len()
            && (bytes[pos].is_ascii_alphanumeric()
                || bytes[pos] == b'_'
                || bytes[pos] == b'?'
                || bytes[pos] == b'!')
        {
            pos += 1;
        }
        if pos == method_start {
            return false; // no method name found
        }
        let method = &bytes[method_start..pos];

        // Skip past arguments if present: balanced parentheses, track if args exist
        // Skip whitespace first
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip bracket indexing like [0..1] — these are not method calls but
        // should not break the chain. E.g., Time.now.to_a[0..5].to_i
        while pos < bytes.len() && bytes[pos] == b'[' {
            let mut depth = 1u32;
            pos += 1;
            while pos < bytes.len() && depth > 0 {
                match bytes[pos] {
                    b'[' => depth += 1,
                    b']' => depth -= 1,
                    _ => {}
                }
                pos += 1;
            }
            // Skip whitespace after ]
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
        }
        let has_args = if pos < bytes.len() && bytes[pos] == b'(' {
            let mut depth = 1u32;
            pos += 1;
            // Skip whitespace after opening paren
            let mut content_start = pos;
            while content_start < bytes.len() && bytes[content_start].is_ascii_whitespace() {
                content_start += 1;
            }
            // If we immediately hit ')', there are no arguments
            let has_content = content_start < bytes.len() && bytes[content_start] != b')';
            while pos < bytes.len() && depth > 0 {
                match bytes[pos] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'\'' | b'"' => {
                        // Skip string literals to avoid counting parens inside strings
                        let quote = bytes[pos];
                        pos += 1;
                        while pos < bytes.len() && bytes[pos] != quote {
                            if bytes[pos] == b'\\' {
                                pos += 1; // skip escaped char
                            }
                            pos += 1;
                        }
                        // pos is at closing quote, will be incremented below
                    }
                    _ => {}
                }
                pos += 1;
            }
            has_content
        } else {
            false
        };

        if is_style_safe_method(method, has_args, strict) {
            return true;
        }

        // Continue to check next chain element
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::cop_fixture_tests!(TimeZone, "cops/rails/time_zone");

    fn strict_config() -> CopConfig {
        use std::collections::HashMap;

        let mut options = HashMap::new();
        options.insert(
            "EnforcedStyle".to_string(),
            serde_yml::Value::String("strict".to_string()),
        );
        CopConfig {
            options,
            ..CopConfig::default()
        }
    }

    #[test]
    fn to_time_flagged_in_strict_mode() {
        // In strict mode, string literal receivers are flagged.
        // Non-string receivers (date_str.to_time) are NOT flagged — RuboCop requires str_type?.
        let fixture = b"\"2005-02-27 23:50\".to_time\n                   ^^^^^^^ Rails/TimeZone: Do not use `String#to_time` without zone. Use `Time.zone.parse` instead.\n\"2005-02-27 23:50\".to_time(:utc)\n                   ^^^^^^^ Rails/TimeZone: Do not use `String#to_time` without zone. Use `Time.zone.parse` instead.\n";
        crate::testutil::assert_cop_offenses_full_with_config(&TimeZone, fixture, strict_config());
    }

    #[test]
    fn to_time_flagged_in_flexible_mode() {
        // RuboCop fires on String#to_time in BOTH strict and flexible mode.
        // Variable receivers are NOT flagged (RuboCop requires str_type? receiver).
        let fixture = b"\"2005-02-27 23:50\".to_time\n                   ^^^^^^^ Rails/TimeZone: Do not use `String#to_time` without zone. Use `Time.zone.parse` instead.\n\"2005-02-27 23:50\".to_time(:utc)\n                   ^^^^^^^ Rails/TimeZone: Do not use `String#to_time` without zone. Use `Time.zone.parse` instead.\n";
        crate::testutil::assert_cop_offenses_full(&TimeZone, fixture);
    }

    #[test]
    fn to_time_not_flagged_for_non_string_receivers() {
        // RuboCop only flags string literal receivers, not variable.to_time
        let source = b"date_str.to_time\nmy_var.to_time\nto_time\n";
        crate::testutil::assert_cop_no_offenses_full(&TimeZone, source);
    }

    #[test]
    fn strict_mode_zone_chain_no_offense_fixture() {
        crate::testutil::assert_cop_no_offenses_full_with_config(
            &TimeZone,
            include_bytes!("../../../tests/fixtures/cops/rails/time_zone/no_offense.strict.rb"),
            strict_config(),
        );
    }

    #[test]
    fn strict_mode_still_flags_accepted_methods() {
        let fixture =
            b"Time.now.to_i\n     ^^^ Rails/TimeZone: Use `Time.zone.now` instead of `Time.now`.\n";
        crate::testutil::assert_cop_offenses_full_with_config(&TimeZone, fixture, strict_config());
    }
}
