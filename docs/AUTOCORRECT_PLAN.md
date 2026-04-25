# Autocorrect Implementation Plan

## Overview

nitrocop supports autocorrect via `-a` (safe) and `-A` (all) flags. As of March 2026, **87 cops** have autocorrect implementations. This document covers the architecture, difficulty assessment, phased implementation roadmap, and conformance testing strategy.

For the complete catalog of every autocorrectable cop (664 cops across 4 gems, with safety classifications and extraction scripts), see **[AUTOCORRECT_COPS.md](AUTOCORRECT_COPS.md)**.

### Scope

| Gem | Total Cops | Autocorrectable | % |
|-----|-----------|----------------|---|
| rubocop (core) | 593 | 458 | 77% |
| rubocop-performance | 52 | 45 | 87% |
| rubocop-rails | 148 | 101 | 68% |
| rubocop-rspec | 114 | 60 | 53% |
| **Total** | **907** | **664** | **73%** |

---

## 1. How RuboCop's Autocorrect Works

### 1.1 Corrector

Each cop gets a `Corrector` instance (extends `Parser::Source::TreeRewriter`) with operations: `replace`, `remove`, `insert_before`, `insert_after`, `wrap`, `swap`.

### 1.2 add_offense with Correction Block

```ruby
add_offense(range, message: 'Prefer single quotes') do |corrector|
  corrector.replace(range, "'hello'")
end
```

The block is only evaluated if autocorrect is requested. Cops must `extend AutoCorrector`.

### 1.3 Safety Model

| Flag | Behavior |
|------|----------|
| `-a` / `--autocorrect` | Safe only: skips cops with `SafeAutoCorrect: false` or `Safe: false` |
| `-A` / `--autocorrect-all` | All corrections, including unsafe ones |

Per-cop config keys: `AutoCorrect` (`'always'`/`'contextual'`/`'disabled'`), `Safe` (bool), `SafeAutoCorrect` (bool).

### 1.4 Correction Merging and Conflicts

Each cop produces corrections independently. RuboCop merges them per-file using these rules:

- The `Corrector` (extends `Parser::Source::TreeRewriter`) is configured with:
  - `different_replacements: :raise` — raises `ClobberingError` if two corrections target the same range with different replacements
  - `swallowed_insertions: :raise` — raises if an insertion would be deleted by another edit
  - `crossing_deletions: :accept` — overlapping deletions are allowed (merged)
- `ClobberingError` is caught and silently ignored in `Team#suppress_clobbering` — the conflicting correction is simply dropped
- Corrections merge in **cop registration order** — first cop merged wins when there's a conflict
- Cops can declare `autocorrect_incompatible_with` — after a cop's correction is merged, all cops it declares incompatible are **skipped** for that iteration (not just for overlapping ranges — skipped entirely)

### 1.5 Iteration Loop

Re-inspects after applying corrections, up to **200 iterations**, until source stabilizes. Detects infinite loops via source checksum comparison.

### 1.6 File Writing

Corrected source written to disk, or to stdout with `--stdin`.

---

## 2. Difficulty Assessment

### 2.1 Overall Difficulty: Moderate-High

The autocorrect system has two distinct layers of difficulty:

**Infrastructure (the hard part):** The correction framework, iteration loop, safety model, and walker integration touch the core linting pipeline. This is architecturally involved because it interacts with rayon parallelism, the `!Send` `ParseResult` constraint, the config system, the formatter, and the cache. However, the design is well-understood — we're replicating RuboCop's proven architecture, not inventing something new.

**Per-cop corrections (the long tail):** Each cop needs its own correction logic. Difficulty varies wildly — from trivial (delete trailing whitespace) to genuinely hard (restructure `if`/`unless` with proper precedence handling). The good news: each cop is independent, so they can be added incrementally without blocking anything.

### 2.2 Difficulty by Component

| Component | Difficulty | Why |
|-----------|-----------|-----|
| `Correction` + `CorrectionSet` data types | **Easy** | Simple structs, well-defined algorithm (sorted non-overlapping edits). Mostly unit-testable in isolation. |
| `SourceFile::line_col_to_offset()` | **Easy** | Inverse of existing `offset_to_line_col`. The `line_starts` array already exists. |
| CLI flags (`-a`, `-A`) | **Easy** | Clap args + enum. Trivial. |
| `CopConfig` extension (Safe/SafeAutoCorrect/AutoCorrect) | **Easy** | Reading 3 more YAML keys. Config system already handles arbitrary keys via `options` HashMap. |
| `Diagnostic.corrected` field | **Easy** | Add one bool field, update Display impl and JSON serializer. |
| Cop trait `corrections` parameter | **Easy** | Add `Option<&mut Vec<Correction>>` param to existing `check_*` methods. All 915 cops get a mechanical signature update; no behavioral change. |
| `BatchedCopWalker` corrections buffer | **Moderate** | Pass `Option<&mut Vec<Correction>>` through to cop dispatch. Must not regress the hot path when autocorrect is off (`None` path). |
| Linter iteration loop | **Moderate** | Re-parse + re-lint per iteration inside rayon workers. Must handle convergence detection, max iterations, and the `!Send` ParseResult constraint. Conceptually straightforward but touches the most performance-sensitive code. |
| File writing | **Easy-Moderate** | `std::fs::write` for the simple case. `--stdin` mode needs stdout output. Edge cases: file permissions, symlinks, encoding. |
| Formatter changes | **Easy** | `[Corrected]` prefix, corrected count. Mechanical. |
| Cache interaction | **Easy** | Disable cache when autocorrect active. One conditional. |
| Correction conflict resolution | **Moderate** | Overlapping edits need deterministic resolution. Sorting by offset + dropping overlaps is simple in theory; edge cases around insertions at the same point need care. |
| Conformance testing harness | **Moderate-Hard** | Copying bench repos, running both tools, diffing output trees, reporting per-cop match rates. Not algorithmically hard, but lots of plumbing. |

### 2.3 Difficulty by Cop Category

| Category | Count | Difficulty | Example |
|----------|-------|-----------|---------|
| Byte-range deletion | ~15 | **Trivial** | `Layout/TrailingWhitespace`: delete bytes from offset A to B |
| Byte-range insertion | ~5 | **Trivial** | `Style/FrozenStringLiteralComment`: insert string at offset 0 |
| Simple token replacement | ~40 | **Easy** | `Style/StringLiterals`: replace `"` with `'`; `Lint/UnifiedInteger`: replace `Fixnum` with `Integer` |
| AST node replacement | ~80 | **Easy-Moderate** | `Style/NilComparison`: replace `x == nil` with `x.nil?` — need node byte ranges |
| Multi-edit corrections | ~50 | **Moderate** | `Style/SymbolProc`: delete block, insert `(&:method)` — two edits that must be coordinated |
| Structural transforms | ~30 | **Moderate-Hard** | `Style/IfUnlessModifier`: rewrite multi-line if to modifier form — need to understand indentation context |
| Operator precedence-aware | ~10 | **Hard** | `Style/AndOr`: `and`/`or` → `&&`/`||` with parenthesization when precedence changes semantics |
| Layout/indentation cops | ~99 | **Hard** | The entire Layout department. Each cop's correction is conceptually simple (add/remove spaces) but must match RuboCop's exact indentation calculation, which involves context-dependent rules. Getting 100% conformance here is the hardest part of the entire project. |
| Config-dependent behavior | ~40 | **Moderate** | Cops with `EnforcedStyle` that changes the correction direction (e.g., `Style/HashSyntax` with `ruby19` vs `hash_rockets`) |

### 2.4 What Makes This Tractable

1. **Incremental.** Infrastructure lands first, then cops are added one at a time. Each cop is independently useful.
2. **No new algorithms.** We're replicating RuboCop's well-documented behavior, not designing from scratch.
3. **Existing detection logic.** Every cop already identifies the offense location. The correction is usually "the obvious fix" for what was detected.
4. **Byte offsets from Prism.** Prism nodes provide `start_offset()` / `end_offset()`, so we don't need a separate range-mapping layer.
5. **Conformance testing.** We can measure correctness against RuboCop at every step.

### 2.5 What Makes This Hard

1. **Layout cops.** RuboCop's indentation/spacing corrections are the most complex autocorrections and account for 99 cops. Matching their behavior exactly requires understanding RuboCop's indentation calculation internals.
2. **Multi-cop interaction.** When multiple cops correct the same file, corrections can conflict or cascade. RuboCop handles this with its iteration loop, which we must replicate exactly.
3. **The long tail.** 664 cops is a lot. Even at 10 cops/day, full coverage is months of work. Prioritization matters.
4. **Performance.** The iteration loop re-parses files multiple times. With rayon this should still be fast, but it's a meaningful change to the hot path.

---

## 3. Architecture Design

### 3.1 Correction Data Structure

```rust
/// A single source-level edit: replace byte range [start..end) with replacement.
#[derive(Debug, Clone)]
pub struct Correction {
    pub start: usize,        // byte offset, inclusive
    pub end: usize,          // byte offset, exclusive
    pub replacement: String, // replacement text (empty = delete)
    pub cop_name: &'static str,
    pub cop_index: usize,    // registry index for deterministic conflict resolution
}
```

**Design decision: byte offsets.** Prism provides `start_offset()`/`end_offset()`. Avoids lossy line:col round-trips.

**Note on cop_index:** The field exists in the struct and `CorrectionSet` sorts by `(start, cop_index)` for tiebreaking, but as of March 2026 all 181 correction sites hardcode `cop_index: 0`. See Section 8.2 for the plan to wire this up.

### 3.2 CorrectionSet

```rust
pub struct CorrectionSet {
    corrections: Vec<Correction>, // sorted by start, non-overlapping
}
```

Apply algorithm — single O(n) linear scan:
```
cursor = 0
for each correction c (sorted by start):
    copy source[cursor..c.start]
    copy c.replacement
    cursor = c.end
copy source[cursor..]
```

**Design decision: sorted-edits, not tree-rewriter.** Simpler than RuboCop's `TreeRewriter` since we collect all corrections upfront.

**Conflict resolution rules** (matching RuboCop's "first merged wins" behavior):
- Sort corrections by `start` offset ascending
- When two corrections overlap (second's `start` < first's `end`), drop the second
- When two corrections start at the same offset, the one from the earlier cop in registry order wins
- Registry order is stable (deterministic cop registration in `default_registry()`)

### 3.3 Cop Trait Extension

```rust
pub trait Cop: Send + Sync {
    // existing methods unchanged...

    fn supports_autocorrect(&self) -> bool { false }
    fn safe_autocorrect(&self) -> bool { true }

    fn check_lines(
        &self,
        source: &SourceFile,
        config: &CopConfig,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<Correction>>,  // NEW param
    );
    fn check_source(
        &self, ...,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<Correction>>,  // NEW param
    );
    fn check_node(
        &self, ...,
        diagnostics: &mut Vec<Diagnostic>,
        corrections: Option<&mut Vec<Correction>>,  // NEW param
    );
}
```

**Why `Option` parameter instead of dual methods?** Adding `*_with_corrections` variants would mean 3 new trait methods with 915 default impls that just delegate. Instead, we add one `Option<&mut Vec<Correction>>` parameter to the existing methods. Callers pass `None` when autocorrect is off — cops that don't support autocorrect simply ignore the parameter. This keeps the trait surface flat and avoids duplicating every walker call site.

### 3.4 Other Changes

- **Diagnostic**: Add `corrected: bool` field
- **CLI**: `-a`/`-A` flags, `AutocorrectMode` enum
- **CopConfig**: Read `Safe`, `SafeAutoCorrect`, `AutoCorrect` keys
- **BatchedCopWalker**: Add `corrections: Vec<Correction>` buffer, conditional dispatch
- **Linter**: Iteration loop (max 200), file writing
- **Formatters**: `[Corrected]` prefix, corrected count summary, JSON fields
- **Cache**: Disabled when autocorrect active

### 3.5 Linter Iteration Loop

```
for each file (parallel via rayon):
    source = read file
    loop (max 200 iterations):
        (diagnostics, corrections) = lint_with_corrections(source)
        if corrections.is_empty(): break
        source = corrections.apply(source)
        if source unchanged: break
    write corrected source to disk
    collect remaining diagnostics
```

Files are independent — rayon parallelism at file level. Per-file iterations are sequential. `ParseResult` being `!Send` is fine since we re-parse inside the same rayon worker.

### 3.6 Post-Correction Syntax Validation ✅ COMPLETE

After the iteration loop converges (or hits max iterations), the final corrected bytes are validated by re-parsing with Prism before writing to disk. If `parse_result.errors()` reports any syntax errors, the corrections are **discarded entirely** and the original file is preserved, with a warning emitted to stderr.

This is stricter than RuboCop, which writes the (potentially broken) corrected source to disk and only stops further iterations on parse failure. Since nitrocop holds corrected bytes in memory until the loop finishes, it can avoid writing invalid Ruby at no additional risk.

**Cost:** One Prism parse per corrected file. Prism parses typical Ruby files in microseconds, so this is negligible relative to the multi-iteration lint passes that precede it.

**Implementation:** `validate_corrected_bytes()` helper in `src/linter.rs`, called at both return points in `lint_source_inner`.

---

## 4. Phased Implementation Roadmap

### Phase 0: Infrastructure ✅ COMPLETE

**Goal:** The autocorrect framework works end-to-end with zero cops. Running `nitrocop -a .` accepts the flag, applies no corrections (since no cop implements `supports_autocorrect`), and exits cleanly.

**Status:** Implemented in commit `b370fa5`. 935 files changed (1507 insertions, 145 deletions). All tests pass.

| File | Change | Status |
|------|--------|--------|
| `src/correction.rs` | **NEW**: `Correction`, `CorrectionSet` + 13 unit tests | ✅ |
| `src/diagnostic.rs` | Add `corrected: bool` with `#[serde(default)]` | ✅ |
| `src/cop/mod.rs` | Add `supports_autocorrect`, `safe_autocorrect`; add `corrections` param; add config helpers (`is_safe`, `is_safe_autocorrect`, `autocorrect_setting`, `should_autocorrect`) | ✅ |
| `src/cop/**/*.rs` | All 915 cop signatures updated with `_corrections` parameter | ✅ |
| `src/cli.rs` | Add `-a`, `-A` flags, `AutocorrectMode` enum | ✅ |
| `src/cop/walker.rs` | Pass `None` through `CopWalker` and `BatchedCopWalker` dispatch | ✅ |
| `src/cop/mod.rs` (CopConfig) | Read `Safe`, `SafeAutoCorrect`, `AutoCorrect` keys | ✅ |
| `src/linter.rs` | `corrected_count` in `LintResult`, cache disabled during autocorrect | ✅ |
| `src/lib.rs` | Autocorrect mode debug output, stdin+autocorrect warning | ✅ |
| `src/formatter/text.rs` | Corrected count in summary when > 0 | ✅ |
| `src/formatter/progress.rs` | Corrected count in summary when > 0 | ✅ |
| `src/formatter/pacman.rs` | Corrected count in summary when > 0 | ✅ |
| `src/formatter/json.rs` | `corrected` field on offenses, `corrected_count` in metadata | ✅ |
| `src/parse/source.rs` | `line_col_to_offset()` with roundtrip proptest | ✅ |
| `src/testutil.rs` | Updated all call sites with `None` | ✅ |
| `src/cache.rs` | Updated `test_args()` and `to_diagnostic()` | ✅ |
| `tests/integration.rs` | Updated `default_args()` and Diagnostic literals | ✅ |

### Phase 1: Trivial Cops (byte-range deletions and insertions) ✅ COMPLETE

**Goal:** First cops that actually correct files. Validates the full pipeline: detection → correction → file write → re-lint convergence.

**Status:** All 6 cops implemented. Iteration loop, file writing, and `BatchedCopWalker` corrections buffer implemented.

| Cop | Correction | Status |
|-----|-----------|--------|
| `Layout/TrailingWhitespace` | Delete trailing whitespace bytes | ✅ |
| `Layout/TrailingEmptyLines` | Add/remove trailing newlines | ✅ |
| `Layout/LeadingEmptyLines` | Remove leading blank lines | ✅ |
| `Style/FrozenStringLiteralComment` | Insert `# frozen_string_literal: true\n` at top | ✅ |
| `Layout/IndentationStyle` | Replace tabs with spaces (or spaces with tabs) | ✅ |
| `Style/Encoding` | Remove unnecessary encoding comment | ✅ |

### Phase 1b: Layout Spacing + Line Cops ✅ COMPLETE

**Goal:** Extend autocorrect to simple spacing and line-deletion cops.

**Status:** 13 additional cops implemented across two batches.

| Cop | Correction | Status |
|-----|-----------|--------|
| `Layout/SpaceAfterComma` | Insert space after `,` | ✅ |
| `Layout/SpaceAfterSemicolon` | Insert space after `;` | ✅ |
| `Layout/SpaceBeforeComma` | Remove space before `,` | ✅ |
| `Layout/SpaceBeforeSemicolon` | Remove space before `;` | ✅ |
| `Layout/EmptyLines` | Delete consecutive blank lines | ✅ |
| `Layout/EndOfLine` | Normalize line endings | ✅ |
| `Layout/InitialIndentation` | Remove initial indentation | ✅ |
| `Layout/EmptyLineAfterMagicComment` | Insert blank line after magic comment | ✅ |
| `Layout/LeadingCommentSpace` | Insert space after `#` | ✅ |
| `Layout/SpaceBeforeComment` | Insert space before inline `#` | ✅ |
| `Layout/SpaceAroundKeyword` | Insert space after keyword | ✅ |
| `Layout/EmptyComment` | Delete empty comment lines | ✅ |
| `Lint/DuplicateMagicComment` | Delete duplicate magic comment lines | ✅ |

### Phase 2: Simple Token Replacements ✅ COMPLETE

**Goal:** Cops that replace one token/string with another. Builds confidence in AST-based corrections.

**Status:** All originally listed cops implemented, plus additional token-replacement cops.

| Cop | Correction | Status |
|-----|-----------|--------|
| `Lint/UnifiedInteger` | `Fixnum`/`Bignum` → `Integer` | ✅ |
| `Style/NilComparison` | `x == nil` → `x.nil?` | ✅ |
| `Style/Not` | `not x` → `!x` | ✅ |
| `Style/ColonMethodDefinition` | `self::method` → `self.method` | ✅ |
| `Style/Proc` | `Proc.new` → `proc` | ✅ |
| `Style/Attr` | `attr` → `attr_reader` | ✅ |
| `Lint/RedundantStringCoercion` | Remove `.to_s` in interpolation | ✅ |
| `Style/CharacterLiteral` | `?c` → `'c'` | ✅ |
| `Lint/EmptyInterpolation` | Remove empty `#{}` | ✅ |
| `Lint/RedundantWithIndex` | Remove redundant `.with_index` | ✅ |
| `Lint/RedundantWithObject` | Remove redundant `.with_object` | ✅ |
| `Style/EvenOdd` | `x % 2 == 0` → `x.even?` | ✅ |
| `Style/Strip` | `.lstrip.rstrip` → `.strip` | ✅ |
| `Style/StringChars` | `.split('').first` → `.chars.first` | ✅ |
| `Style/RedundantSortBy` | `.sort_by { \|x\| x.foo }` → `.sort_by(&:foo)` | ✅ |
| `Style/RedundantCapitalW` | `%W()` → `%w()` when no interpolation | ✅ |
| `Style/RedundantFileExtensionInRequire` | Remove `.rb` from `require` | ✅ |
| `Style/RedundantException` | Remove redundant `RuntimeError` | ✅ |
| `Style/DirEmpty` | `Dir.entries(x).size == 2` → `Dir.empty?(x)` | ✅ |
| `Style/NumericPredicate` | `x > 0` → `x.positive?` | ✅ |
| `Style/SpecialGlobalVars` | `$!` → `$ERROR_INFO` etc. | ✅ |
| `Style/FrozenStringLiteralComment` | Insert/remove magic comment | ✅ |
| `Style/Encoding` | Remove unnecessary encoding comment | ✅ |
| `Naming/HeredocDelimiterCase` | Fix heredoc delimiter case | ✅ |
| `Naming/RescuedExceptionsVariableName` | Rename rescue variable | ✅ |
| `Naming/BlockForwarding` | Anonymous block forwarding | ✅ |
| `Bundler/OrderedGems` | Reorder gems | ✅ |
| `Bundler/InsecureProtocolSource` | Fix insecure protocol | ✅ |
| `Gemspec/AddRuntimeDependency` | Fix dependency method | ✅ |
| `Gemspec/DeprecatedAttributeAssignment` | Fix deprecated assignment | ✅ |
| `Gemspec/OrderedDependencies` | Reorder dependencies | ✅ |
| `Gemspec/RequireMFA` | Add MFA requirement | ✅ |

### Phase 3: Conformance Testing Harness

**Goal:** Automated way to measure how well nitrocop's autocorrect matches RuboCop, per-cop and per-file. See also **Section 8.3** for the lighter-weight per-cop comparison script.

**Difficulty: Moderate.** Not algorithmically hard, but substantial plumbing (temp dirs, running both tools, diffing, reporting).

- Add `autocorrect-conform` subcommand to `bench_nitrocop`
- For each bench repo: copy to two temp dirs, run `rubocop -A` on one and `nitrocop -A` on the other, diff results
- Report per-cop match rate and overall file-level match rate
- Integrate into the conformance pipeline alongside existing detection conformance

**Verification:** Running `cargo run --release --bin bench_nitrocop -- autocorrect-conform` produces a report.

### Phase 4: AST-Based Corrections (Moderate) — Partially Complete

**Goal:** Cops that require understanding AST structure for correct replacement. Covers the high-value middle ground.

| Cop | Correction | Status |
|-----|-----------|--------|
| `Style/NegatedIf` | Flip condition, swap if/unless | ✅ |
| `Style/NegatedUnless` | Flip condition, swap unless/if | ✅ |
| `Style/NegatedWhile` | Flip condition, swap while/until | ✅ |
| `Style/AndOr` | `and`/`or` → `&&`/`\|\|` (with parenthesization) | ✅ |
| `Style/WhenThen` | `when x then` → `when x\n` | ✅ |
| `Style/MultilineWhenThen` | Remove `then` from multiline `when` | ✅ |
| `Style/MultilineIfThen` | Remove `then` from multiline `if` | ✅ |
| `Style/UnlessElse` | Flip unless/else to if/else | ✅ |
| `Style/EmptyElse` | Remove empty else clause | ✅ |
| `Style/ParenthesesAroundCondition` | Remove unnecessary parens around condition | ✅ |
| `Style/SymbolProc` | `{ \|x\| x.foo }` → `(&:foo)` | |
| `Style/HashSyntax` | Convert between `ruby19`/`hash_rockets` styles | |
| `Style/BlockDelimiters` | Switch between `do..end` and `{..}` | |
| `Style/Lambda` | `lambda { }` ↔ `-> { }` | |
| `Style/RedundantReturn` | Remove `return` keyword | |
| `Style/RedundantBegin` | Remove redundant `begin..end` | |
| `Style/RedundantParentheses` | Remove unnecessary parens | |

**Verification:** `corrected.rb` fixtures + autocorrect-conform runs.

### Phase 5: Layout Spacing Cops — Largely Complete

**Goal:** The ~30 most common spacing cops. This is the highest-impact but hardest-to-match category.

**Status:** Most spacing and empty-line cops now have autocorrect. Indentation cops remain unimplemented.

| Cop | Status |
|-----|--------|
| `Layout/SpaceAroundOperators` | ✅ |
| `Layout/SpaceAroundKeyword` | ✅ |
| `Layout/SpaceAroundBlockParameters` | ✅ |
| `Layout/SpaceAroundEqualsInParameterDefault` | ✅ |
| `Layout/SpaceBeforeBlockBraces` | ✅ |
| `Layout/SpaceInsideBlockBraces` | ✅ |
| `Layout/SpaceInsideHashLiteralBraces` | ✅ |
| `Layout/SpaceInsideParens` | ✅ |
| `Layout/SpaceInsideArrayLiteralBrackets` | ✅ |
| `Layout/SpaceInsideReferenceBrackets` | ✅ |
| `Layout/SpaceInsideStringInterpolation` | ✅ |
| `Layout/SpaceInsideRangeLiteral` | ✅ |
| `Layout/SpaceInsidePercentLiteralDelimiters` | ✅ |
| `Layout/SpaceInsideArrayPercentLiteral` | ✅ |
| `Layout/SpaceAfterColon` | ✅ |
| `Layout/SpaceAfterNot` | ✅ |
| `Layout/SpaceAfterMethodName` | ✅ |
| `Layout/ExtraSpacing` | ✅ |
| `Layout/EmptyLineBetweenDefs` | ✅ |
| `Layout/EmptyLineAfterGuardClause` | ✅ |
| `Layout/EmptyLineAfterMagicComment` | ✅ |
| `Layout/EmptyLinesAroundMethodBody` | ✅ |
| `Layout/EmptyLinesAroundClassBody` | ✅ |
| `Layout/EmptyLinesAroundModuleBody` | ✅ |
| `Layout/EmptyLinesAroundBlockBody` | ✅ |
| `Layout/EmptyLinesAroundBeginBody` | ✅ |
| `Layout/EmptyLinesAroundAccessModifier` | ✅ |
| `Layout/EmptyLinesAroundExceptionHandlingKeywords` | ✅ |
| `Layout/EmptyLinesAroundAttributeAccessor` | ✅ |
| `Layout/EmptyLinesAfterModuleInclusion` | ✅ |
| `Layout/IndentationWidth` | |
| `Layout/IndentationConsistency` | |
| `Layout/HashAlignment` | |
| `Layout/ArgumentAlignment` | |

**Verification:** Autocorrect-conform runs are critical here. Layout cops are where conformance divergence is most likely.

### Phase 6: Long Tail + Full Conformance

**Goal:** Remaining cops, driven by conformance data. Prioritize cops that cause the most conformance failures in bench repos.

**Difficulty: Varies.** Some are trivial cops that just weren't prioritized earlier. Some are genuinely complex (e.g., `Style/IfUnlessModifier`, `Style/ConditionalAssignment`, `Style/GuardClause`).

- Use autocorrect-conform data to identify highest-impact missing corrections
- Implement `autocorrect_incompatible_with` mechanism for cop conflicts
- Target specific conformance rates per bench repo
- Consider `--disable-uncorrectable` support

---

## 5. Conformance Testing Strategy

### 5.1 Unit Tests: corrected.rb Fixtures

Extend the existing fixture format:

```
testdata/cops/style/string_literals/
  offense.rb       # source with ^ annotations (existing)
  no_offense.rb    # clean source (existing)
  corrected.rb     # expected output after autocorrect (NEW)
```

New macro `cop_autocorrect_fixture_tests!` in `src/cop/mod.rs`:
- Strips `^` annotations from `offense.rb` to get clean source
- Runs the cop in autocorrect mode (`corrections: Some(&mut vec)`)
- Applies `CorrectionSet` to produce corrected source
- Asserts output matches `corrected.rb` byte-for-byte

If `corrected.rb` doesn't exist, the macro silently skips (backward-compatible). This means autocorrect tests are opt-in per cop — just add `corrected.rb`.

### 5.2 Integration Tests: autocorrect-conform

**Single-cop isolation approach.** Rather than comparing full `nitrocop -A` vs `rubocop -A` output (which conflates all cops and makes it hard to attribute failures), the harness tests autocorrect conformance **one cop at a time**:

```bash
cargo run --release --bin bench_nitrocop -- autocorrect-conform
```

For each bench repo, for each cop that `supports_autocorrect`:

1. Copy the repo to a temp directory
2. Run `rubocop -A --only Department/CopName --format json` on the copy
3. Record which files were corrected and the corrected content
4. Reset the copy (restore originals)
5. Run `nitrocop -A --only Department/CopName` on the same copy
6. Diff the corrected files

**Why single-cop?**
- No conflict resolution noise — isolates each cop's behavior
- Easy to attribute failures to specific cops
- Matches how conformance testing works for detection (per-cop stats)
- Slow but thorough — can run overnight, cache results

**Output:** `bench/autocorrect_conform.json` with per-cop, per-repo stats:
```json
{
  "mastodon": {
    "Style/StringLiterals": {
      "files_corrected_rubocop": 42,
      "files_corrected_nitrocop": 42,
      "files_match": 41,
      "files_differ": 1,
      "match_rate": 97.6
    }
  }
}
```

### 5.3 Golden-File Tests (from RuboCop specs)

For each autocorrectable cop, extract `expect_correction` blocks from vendor RuboCop specs:

```
testdata/cops/style/string_literals/
  corrected.rb     # derived from vendor spec expect_correction blocks
```

RuboCop specs contain explicit before/after pairs:
```ruby
expect_offense(<<~RUBY)
  x = "hello"
      ^^^^^^^ Prefer single-quoted strings
RUBY

expect_correction(<<~RUBY)
  x = 'hello'
RUBY
```

Extraction process:
1. Parse vendor spec for `expect_correction` blocks
2. Combine with corresponding `expect_offense` blocks to get input → expected output pairs
3. Write as `corrected.rb` (the expected output after correction)

This gives us RuboCop's own test expectations as our ground truth.

### 5.4 Conflict Resolution Tests

Test that overlapping corrections from multiple cops are handled identically to RuboCop:
- Files that trigger corrections from 2+ cops on overlapping ranges
- Verify the same corrections win/lose in both tools
- Multi-cop golden-file tests in `testdata/autocorrect/multi_cop/` with input.rb + expected.rb

---

## 6. Open Questions

1. ~~**Iteration limit**~~: Resolved — using 200, matching RuboCop. Implemented in `src/linter.rs`.

2. **Atomic writes**: Write corrected files atomically (temp + rename)? RuboCop doesn't, but it's safer.

3. **`--disable-uncorrectable`**: Defer to a later phase? It adds `# rubocop:todo` comments, which is useful but orthogonal to the core autocorrect flow.

4. **Priority ordering**: When bench conformance data is available, should we prioritize (a) most-triggered cops in bench repos, (b) simplest corrections, or (c) highest conformance impact? Likely (c).

5. ~~**Layout cop strategy**~~: Resolved — implement from scratch per Phase 5. The phased approach allows measuring conformance incrementally and deprioritizing cops where RuboCop's behavior is too complex to match exactly.

---

## 7. Files Modified Summary

| File | Change |
|------|--------|
| `src/correction.rs` | **NEW**: `Correction`, `CorrectionSet` types |
| `src/diagnostic.rs` | Add `corrected: bool` field |
| `src/cop/mod.rs` | Add `supports_autocorrect`, `safe_autocorrect`; add `corrections: Option<&mut Vec<Correction>>` param to `check_*` methods |
| `src/cop/walker.rs` | Pass `Option<&mut Vec<Correction>>` through `BatchedCopWalker` dispatch |
| `src/cli.rs` | Add `-a`, `-A` flags, `AutocorrectMode` |
| `src/config/mod.rs` | Read `Safe`, `SafeAutoCorrect`, `AutoCorrect` |
| `src/linter.rs` | Correction-aware lint path, iteration loop, file writing |
| `src/lib.rs` | Branch on autocorrect mode |
| `src/formatter/text.rs` | `[Corrected]` prefix, corrected count |
| `src/formatter/json.rs` | `corrected`/`correctable` fields |
| `src/parse/source.rs` | `line_col_to_offset()` helper |
| Per-cop files | Use `corrections` param in `check_*` methods to produce `Correction` values |

---

## 8. Autocorrect Conflict Parity

### 8.1 Current State

**What's implemented:**
- Byte-range overlap detection via `CorrectionSet` — corrections sorted by `(start, cop_index)`, overlapping edits dropped
- 200-iteration convergence loop in `lint_source_inner` (`src/linter.rs`)
- Source-equality convergence check (bail if corrections don't change the source)
- Post-correction syntax validation (re-parse with Prism, discard if invalid)

**What's missing:**
- **Functional `cop_index`**: ✅ Wired in `BatchedCopWalker`. Per-cop sites still hardcode `cop_index: 0`, but the walker stamps the registry index onto every correction post-dispatch (see `run_with_index` in `src/cop/walker.rs`). Tiebreaks in `CorrectionSet` are now deterministic by registration order.
- **`autocorrect_incompatible_with`**: RuboCop lets cops declare other cops whose corrections should be entirely skipped when theirs are applied. Not implemented. (~10 cops use this in RuboCop.)
- **Two-pass cop ordering**: RuboCop partitions cops into autocorrect vs non-autocorrect, runs autocorrect cops first, and skips non-autocorrect cops if corrections were made. Nitrocop runs all cops in a single pass (universal → pattern → AST order). This is an optimization, not a correctness issue — without it, nitrocop just does slightly more work per iteration.
- **Checksum-based infinite loop detection**: RuboCop detects cycles by tracking source checksums across iterations. Nitrocop uses source-equality (bail if source didn't change), which catches fixed points but not cycles (A→B→A). In practice, cycles are rare because `CorrectionSet` deterministically resolves conflicts the same way each iteration.

**Why this matters:** When multiple cops produce corrections that overlap or interact, the conflict resolution strategy determines which corrections survive. Differences here can cause nitrocop's autocorrected output to diverge from RuboCop's even when each cop's correction logic is identical in isolation.

**Why it's not urgent yet:** The iteration loop handles most conflicts through convergence — corrections that were dropped in one pass get applied in the next. The gaps above affect edge cases (same-offset tiebreaking, cop-pair conflicts). Measuring actual divergence should precede investing in conflict-layer sophistication.

### 8.2 Wire cop_index ✅ COMPLETE

The `cop_index` field in `Correction` is now functional. `BatchedCopWalker::new` accepts `(usize, &dyn Cop, &CopConfig)` tuples — the registry index. After each `check_node` call, the walker overwrites `cop_index` on any corrections that cop just pushed (`run_with_index` in `src/cop/walker.rs`). Same-offset ties in `CorrectionSet` now resolve deterministically by registration order, matching RuboCop's "first merged wins".

**Why this approach (vs. updating 181 cop sites):** Stamping at the walker layer keeps cop_index a registry-owned concept rather than something every cop has to remember to thread through `CopConfig`. The 181 hardcoded `cop_index: 0` sites are dead writes — harmless, since the walker overwrites them — but can be cleaned up opportunistically.

**Test:** `cop::walker::tests::dispatch_stamps_cop_index_onto_corrections` constructs a walker with two cops at distinct registry indices and asserts the stamp lands on the right correction.

### 8.3 Autocorrect comparison harness (next step)

Before investing in `autocorrect_incompatible_with` or two-pass ordering, measure how much nitrocop's autocorrect actually diverges from RuboCop's on real-world code.

**Approach:**
- Lightweight Python script under `scripts/` (not the full `bench_nitrocop` conform harness from Phase 3)
- Input: a cop name
- For files in the corpus that trigger offenses for that cop:
  1. Run `rubocop -A --only Department/CopName` and capture corrected output
  2. Run `nitrocop -A --only Department/CopName` and capture corrected output
  3. Diff the results
- Output: files matched, files diverged, example diffs
- Prioritize cops that already have autocorrect (87 as of March 2026)

**Why a script, not bench_nitrocop?** The full conform harness (Phase 3) runs all cops across all repos — expensive and conflates cop-interaction effects. A per-cop script isolates each cop's correction behavior, which is what we need to measure before worrying about multi-cop conflicts.

### 8.4 Deferred (implement only if harness shows divergence)

- **`autocorrect_incompatible_with`**: Only ~10 RuboCop cops declare it. The specific pairs: `SpaceBeforeBlockBraces ↔ SymbolProc`, `SpaceInsideBlockBraces ↔ BlockDelimiters`, and a few others. Implement if the comparison harness shows these pairs produce different output.
- **Two-pass cop ordering**: An optimization that reduces wasted work per iteration. Not a correctness issue. Implement if performance profiling shows autocorrect iterations are a bottleneck.
- **Checksum-based cycle detection**: Replace or augment the source-equality check with a checksum history to detect A→B→A cycles. Implement if infinite-loop reports surface in practice.
