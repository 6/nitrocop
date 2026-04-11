# Agent CI Rules

These rules apply when `GITHUB_ACTIONS` is set and the workflow is driving the agent, such as `agent-cop-fix` and `agent-pr-repair`.

## Branch And Git Rules

- Work only in the current checked-out branch. The workflow has already switched to the target PR branch for you.
- Do not create extra branches or `git worktree`s.
- Do not use `git stash`.
- Do not revert the branch to `origin/main` or collapse the PR into an empty diff.
- Do not run `git commit`. The workflow's finalize step commits all changes after the agent exits.
- Do not push. You do not have push permission. The workflow's finalize step handles all git push operations after the agent exits.

## Scope Rules

- Stay within the file scope implied by the workflow route.
- `agent-cop-fix` is limited to cop implementation and corpus-fixture files.
- `agent-pr-repair` is limited by the failing route:
  - Rust/test repairs: Rust sources, tests, and bench files.
  - Python/workflow repairs: `scripts/`, `tests/python/`, workflow YAML, and Python metadata.
  - `cop-check` repairs: cop sources, cop fixtures, `tests/integration.rs`, and `bench/corpus/`.
  - smoke/systemic repairs: broad source, test, bench, and script paths only when the failure truly requires that scope.
- The workflow enforces these scopes after the agent runs. Edits outside the allowed scope will fail the run.

## How To Work

- Read the task prompt first, then inspect the existing PR diff if this is a repair.
- Prefer the provided helper scripts over ad hoc corpus debugging when they directly answer the question.
- Use local corpus artifacts and cached repos when they are already present in the prompt or runtime files.
- **Match RuboCop exactly.** nitrocop is a drop-in replacement. Replicate RuboCop's behavior even when it seems wrong — a "more correct" result that diverges from RuboCop is a corpus regression. Document replicated quirks in `///` doc comments.
- Keep fixes narrow. The workflow prefers a small correct fix over a broad cleanup.
- Add or update tests with every real behavior fix.

## Time Budget

Agent runs have a hard timeout. Plan your work to finish well within it — a partial fix that passes tests is better than an unfinished fix that times out with no commit.

- **Do not rebuild binaries you already have.** The workflow pre-builds a release binary and pre-runs `check_cop.py` before the agent starts. The diagnosis packet already contains the corpus regression data — do not re-derive it from scratch.
- **Do not build `origin/main` from source.** Use the pre-computed diagnosis packet or `investigate_cop.py` with the corpus artifact for baseline data.
- **Do not run full corpus reruns as verification.** `check_cop.py --rerun --clone` against the full diverging-repo set takes 20+ minutes. Use targeted spot-checks on specific repos/files instead. The workflow runs the full gate after you exit.
- **Minimize cargo release builds.** Use `cargo test --lib` (debug, incremental) for fast iteration. Budget for at most one release build after your code fix.
- **Verify with RuboCop on specific patterns first.** Before writing code, confirm what RuboCop does on the exact pattern from the corpus example. Use `bundle exec rubocop --only Department/CopName /tmp/test.rb` (from `bench/corpus/`). This prevents fixing a "regression" that isn't actually a mismatch.
- **Use `investigate_cop.py --context` to understand the pattern.** The context shows the enclosing code structure around each FP/FN, which usually reveals the exact condition you need to add.
- **Don't run `check_cop.py --rerun --clone` more than once.** It takes 5-10 minutes. Use `cargo test --lib` and targeted `cargo run` on test files for iteration. Save the one corpus check for final validation.

## Helper Script Conventions

- Public helper CLIs live in `scripts/`.
- Workflow internals live in `scripts/workflows/`.
- Shared importable helpers live in `scripts/shared/`.
- Use the stable top-level CLI paths shown in the prompt, for example:
  - `python3 scripts/check_cop.py Department/CopName --verbose --rerun --clone`
  - `python3 scripts/investigate_cop.py Department/CopName --context`
  - `python3 scripts/dispatch_cops.py changed --base origin/main --head HEAD`

## Variant Style Testing

Many cops have configurable `EnforcedStyle` parameters. The CI gate tests all supported style values against the corpus, not just the default. A fix that works for the default style but breaks a variant will fail CI.

- The task prompt's **Style Variant Divergence** section lists which variants diverge and by how much.
- Add fixture tests for non-default styles using the `# nitrocop-config: EnforcedStyle: value` directive.
- Spot-check a specific variant locally: `python3 scripts/check_cop.py Cop --rerun --clone --sample 5 --style EnforcedStyle=value`
- The full CI gate runs `--check-variants` automatically, which tests all variant styles.
- When the variant check fails, it shows which repos diverge with FP/FN locations (e.g., `repo_a(FP:file.rb:7) repo_b(FN:file.rb:33)`). Use these to inspect the exact file.
- **Do not assume a default-config fix works for all variants.** Always check the variant section in the task prompt and test each diverging style.

## Failure Handling

- If the only plausible resolution is a full revert of the PR, stop and say so clearly instead of doing the revert.
- If required context is missing, explain the blocker in the final message rather than improvising a broad change.

## Reporting Findings When You Cannot Fix

Your final message is posted to the tracker issue so future agents (and humans) can learn from your attempt. Vague summaries like "tried a fix but it regressed" waste that opportunity. Write findings that prevent the next agent from repeating your work.

**Always include in your final message:**

1. **What you changed** — name the file, function, and logic change (e.g., "added an early return in `check_spacing()` when the operator is inside a string interpolation node")
2. **What happened** — exact regression numbers from `check_cop.py` (e.g., "+70 FP, +149 FN") and which repos/patterns regressed
3. **Why it failed** — root cause of the regression (e.g., "the early return also skipped legitimate `+=` spacing checks because InterpolatedStringNode wraps the entire heredoc body")
4. **What a correct fix needs** — concrete guidance for the next attempt (e.g., "need to check the immediate parent node type, not ancestors, to distinguish interpolation from heredoc context")
5. **Key corpus examples** — specific repo:file:line references that demonstrate the problem

**Even if you made zero code changes**, document what you investigated and why no approach was viable. "I found no fix" with no detail is not acceptable — explain what patterns you examined and why they were not addressable.
