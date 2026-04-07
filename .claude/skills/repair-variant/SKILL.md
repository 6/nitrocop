---
name: repair-variant
description: Fix variant (EnforcedStyle) FP/FN on a cop — either a CI regression on a bot PR, or outstanding divergence from the corpus oracle. Auto-reads docs/corpus.md for examples.
---

# Fix Variant FP/FN

Fix FP or FN for a cop's variant or default config. Works for two scenarios:

1. **CI regression on a bot PR** — the user provides a PR URL and/or CI failure details
2. **Outstanding divergence** — the user provides the cop name (and optionally variant)

## Workflow

### Step 0: Identify the cop, variant, and FP/FN examples

**Parse the user's input.** They may provide:
- Just a cop name: `Layout/EmptyLinesAroundBlockBody`
- A cop name + variant: `Layout/EmptyLinesAroundBlockBody (empty_lines)`
- A PR URL with CI failure details
- Explicit FP/FN lines

**Auto-read `docs/corpus.md`** to find the FP/FN examples for this cop:
```bash
# Find the cop's section in corpus.md and extract variant info
grep -A50 '<strong>Department/CopName</strong>' docs/corpus.md
```

The format in `docs/corpus.md` is:
```
<summary><strong>Layout/EmptyLinesAroundBlockBody</strong> — 62,599 matches, 720,389 FP, 1,054,572 FN (100.0%)</summary>

**empty_lines** (720,389 FP, 1,054,572 FN):

- FP: `24pullrequests__24pullrequests__381028d: Gemfile:38  [Empty line missing at block body end.]`
- FN: `2016rshah__githubchart-api__639b3ff: app.rb:13  [Empty line missing at block body end.]`
```

**If the user didn't specify a variant**, check which variants have outstanding FP/FN
(including `(default)`) and ask the user which one to fix. Present the options like:

> Which variant should I fix?
> 1. **default** — 0 FP, 0 FN (already perfect)
> 2. **empty_lines** — 720,389 FP, 1,054,572 FN
>
> (Enter a number or variant name)

Skip the prompt if there's only one diverging variant, or if the user already specified one.

### Step 1: Gather FP/FN source lines

Extract the FP/FN example lines from `docs/corpus.md`. These give repo_id, file path,
line number, and message.

Then fetch the actual Ruby source for a representative sample of FP and FN locations.

Do NOT clone repos. Use `gh api` to fetch specific files.

1. Look up each repo in `bench/corpus/manifest.jsonl` to get the GitHub `repo_url` and `sha`:
   ```bash
   grep '<repo_id>' bench/corpus/manifest.jsonl
   ```

2. Fetch the file content at the pinned commit via GitHub API:
   ```bash
   gh api "repos/{owner}/{repo}/contents/{path}?ref={sha}" --jq '.content' | base64 -d | sed -n '{start},{end}p'
   ```
   **Note:** Always double-quote the `gh api` URL argument — zsh interprets bare `?` as a glob.
   Fetch ~15 lines of context around the failing line.

**For CI count-only failures** (repo ID + FP/FN delta, no file paths):

Use `investigate_cop.py --context` to find the specific FP/FN locations:
```bash
python3 scripts/investigate_cop.py Department/CopName --context 2>&1 | grep -A5 '<repo_id>'
```

**For each location, record:**
- The actual Ruby code at that line
- Whether it's FP (nitrocop fires but RuboCop doesn't) or FN (RuboCop fires but nitrocop doesn't)
- What pattern is triggering the mismatch

### Step 2: Get on the right branch

**If fixing a CI regression on a bot PR:**
```bash
gh pr checkout <pr-number>
```

**If fixing outstanding divergence (no PR):**
Create a new branch from main:
```bash
git checkout -b fix/<dept>-<cop_snake>-variant-<variant_name> main
```

### Step 3: Understand root cause

Read the cop source and the vendor RuboCop implementation:
```bash
# Cop source
cat src/cop/<dept>/<cop_name>.rs

# RuboCop reference
find vendor/rubocop* -path "*/<cop_name_snake>*" -name "*.rb" | head -5
```

Compare behavior. Common root causes for variant divergence:
- Variant style not implemented at all (most common for outstanding divergence)
- Shared utility function doesn't handle the variant's inverted logic
- Recursive visitor walking too deep (should only check direct children)
- Missing style-specific branching
- Overly broad pattern matching
- Config option not being respected

### Step 4: Add fixtures from the failing lines

For each failing location, distill the pattern into a minimal fixture.

**Variant fixtures** use `# nitrocop-config:` directives:
- Check if variant-specific fixtures already exist
  (e.g., `empty_lines_offense.rb`, `empty_lines_no_offense.rb`)
- If yes, add to them
- If no, create them with a `# nitrocop-config:` directive at the top:
  ```ruby
  # nitrocop-config: EnforcedStyle: <variant>
  ```
- FP locations -> `<variant>_no_offense.rb`
- FN locations -> `<variant>_offense.rb`

**Default config fixtures** (no variant):
- FP locations -> `tests/fixtures/cops/<dept>/<cop_name>/no_offense.rb`
- FN locations -> `tests/fixtures/cops/<dept>/<cop_name>/offense.rb`

Each fixture case should have a brief comment referencing the repo pattern:
```ruby
# FP fix: modifier if nested inside method call, not a direct child
"#{h(x) + ("..." if cond)}"
```

### Step 5: Run tests (expect failure)

```bash
cargo test --lib -- cop::<dept>::<cop_name_snake>
```

Confirm the new fixtures fail (TDD). If they already pass, the issue
may be in config resolution, not detection logic — investigate further
before moving on.

### Step 6: Fix the cop

Edit `src/cop/<dept>/<cop_name>.rs` to fix the divergence. Keep the fix
minimal and targeted.

Run tests until they pass:
```bash
cargo test --lib -- cop::<dept>::<cop_name_snake>
```

### Step 7: Pre-commit checks

```bash
cargo fmt -- src/cop/<dept>/<cop_name>.rs
cargo clippy --release -- -D warnings
cargo test --release -p nitrocop --lib -- cop::<dept>::<cop_name_snake>
```

### Step 8: Commit and push

```bash
git add src/cop/<dept>/<cop_name>.rs tests/fixtures/cops/<dept>/<cop_name>/
git commit -m "Fix <variant> variant: <brief description>

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
git push origin HEAD
```

CI validates the fix. Do NOT run `check_cop.py` locally — let CI do it.

**If fixing outstanding divergence (no PR yet)**, create one after pushing:
```bash
gh pr create --title "Fix <Dept>/<CopName> <variant> variant" --body "$(cat <<'EOF'
## Summary
- Fix FP/FN for `EnforcedStyle=<variant>` on `<Dept>/<CopName>`
- <brief description of root cause and fix>

## Test plan
- [ ] New variant fixtures pass locally
- [ ] CI cop-check-gate passes (including `--check-variants`)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

## Key principles

- **CI does the heavy lifting.** Don't run corpus checks locally. Push and let CI validate.
- **Fetch, don't clone.** Use `gh api` + manifest.jsonl to grab specific files at pinned commits.
- **Every failing line becomes a fixture.** Turn each FP/FN example into a test case.
- **Start minimal, refactor if needed.** Try the narrowest fix first, but if the
  divergence reveals a fundamentally wrong approach (e.g., recursive visitor when
  RuboCop only checks direct children), replace the approach rather than patching
  around it.
- **Don't run corpus tooling to find issues.** When examples are available from
  `docs/corpus.md` or CI, that IS the diagnosis. Fetching the source at those
  lines and reading the cop/RuboCop source is all you need. `check_cop.py --style`
  and `verify_cop_locations.py --style` exist but only help verify a fix after the
  fact — CI already does that when you push. Don't burn time running them locally.
- **All variants must pass.** The CI gate (`--check-variants`) runs ALL variant styles
  automatically. Fixing one variant must not break another or the default config.
