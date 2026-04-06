# nitrocop

Fast Ruby linter in Rust targeting RuboCop compatibility. Uses Prism for parsing and rayon for parallelism.

## Core Principle: 1:1 RuboCop Compatibility

nitrocop is a **drop-in replacement** for RuboCop. Match RuboCop's exact output on every file — including edge-case quirks. The corpus oracle treats any deviation as a regression. When you must replicate a RuboCop quirk, document it in a `///` doc comment on the cop struct.

## Mental Model

- A **cop** is one RuboCop-compatible rule implemented in Rust under `src/cop/`.
- The **corpus** is the real-world repo set (~5,500 repos) used to compare nitrocop against RuboCop.
- The **corpus oracle** is the CI workflow that produces baseline `corpus-results.json` artifacts consumed by the Python investigation and regression tools.
- `bench/` contains benchmark and corpus tooling. In practice, `bench/corpus/` is where the Ruby bundle and corpus helpers live, while `bench_nitrocop` is the full benchmark/conformance driver.
- Most cop work follows this loop: add or update fixtures, make the cop pass targeted tests, then run per-cop corpus checks to confirm no regression.

## Read First

- If `GITHUB_ACTIONS` is set, also read [`docs/agent-ci.md`](docs/agent-ci.md) before making changes.
- For deeper Prism node-shape notes, see [`docs/node_pattern_analysis.md`](docs/node_pattern_analysis.md).
- For gem packaging and release details, see [`docs/rubygem.md`](docs/rubygem.md).
- Read [`docs/corpus-workflow.md`](docs/corpus-workflow.md) only when the task is to run or debug the operator workflows around corpus results, issue backlog management, or regression investigation, not when the task is simply to fix a cop.
- Do not read `docs/corpus-workflow.md` by default for ordinary cop implementation work, `agent-cop-fix`, or routine `agent-pr-repair` runs.

## Setup

```bash
git submodule update --init    # fetch vendor/rubocop and plugin submodules
```

## Dual-Platform Development

This repo is developed on both macOS and Linux. The devcontainer sets `CARGO_TARGET_DIR=target-linux`, so Linux builds go to `target-linux/` while macOS uses `target/`.

Python scripts under `scripts/` resolve the binary path from `CARGO_TARGET_DIR` automatically.

## Commands

```bash
cargo check                 # fast compile check
cargo build                 # full build (includes Prism C FFI)
cargo test                  # run all tests
cargo run -- .              # lint current directory
cargo run -- -a .           # lint + autocorrect (safe cops only)
cargo run -- -A .           # lint + autocorrect (all cops, including unsafe)
cargo run -- --format json . # emit JSON diagnostics
cargo run -- --debug .      # print phase-level timing breakdown
```

## Before Committing

Run these checks before committing when you touched Rust code (`src/**/*.rs`, `bench/**/*.rs`, `Cargo.toml`, `Cargo.lock`):

```bash
cargo fmt -- src/path/to/changed_file.rs    # format only the files you changed
cargo clippy --release -- -D warnings       # incremental when only a few files changed
cargo test --release
```

If the change is non-Rust only, these Rust checks are optional and should not be run by default.

Important:

- Run `cargo fmt` on the specific Rust files you changed, not `cargo fmt` with no args.
- Run `cargo clippy` normally; it benefits from incremental compilation.
- Do not use `git diff` to discover changed files on shared branches. Check the files you intentionally edited.

For iterative cop development, prefer filtered debug tests:

```bash
cargo test --lib -- cop::style::while_until_modifier
cargo test --lib -- cop::style::
```

When you touch Python under `scripts/`, `tests/python/`, or `bench/corpus/`, run Ruff **and** the Python test suite:

```bash
uv run ruff check --fix path/to/changed.py
uv run ruff check path/to/changed.py
uv run pytest tests/python/ --tb=short
```

Use `uv run` as the preferred way to invoke Python tools (`ruff`, `pytest`, etc.) — it manages the virtualenv automatically. Do not use bare `python3 -m` or `pip install`.

## Repo Layout

- `src/cop/` — cop implementations by department
- `src/cop/shared/` — shared cop infrastructure (util, node types, predicates)
- `tests/fixtures/cops/` — per-cop offense and no-offense fixtures
- `scripts/` — public CLI tools
- `scripts/workflows/` — workflow-only internals
- `scripts/shared/` — shared Python helpers
- `bench/` — corpus and benchmark tooling

## Shared Cop Infrastructure

Before implementing cop logic, check for shared modules — do not reimplement what already exists. Key shared code:

- **`src/cop/shared/util.rs`** — Node helpers (`is_safe_navigation_call`, `unwrap_parentheses`, `is_ternary`, `is_modifier_if`, `double_quotes_required`, etc.).
- **`src/cop/shared/node_type.rs`** — Node type tag constants for O(1) dispatch.
- **`src/cop/shared/method_identifier_predicates.rs`** — Method name classification (mirrors rubocop-ast's `MethodIdentifierPredicates` and `MethodDispatchNode`).
- **`src/cop/shared/literal_predicates.rs`** — Literal node classification (mirrors rubocop-ast's `Node` literal constants).
- **`src/cop/shared/access_modifier_predicates.rs`** — Access modifier detection and `MacroScope` helpers (mirrors rubocop-ast's `MethodDispatchNode` access modifier methods).
- **`src/cop/variable_force/`** — Variable dataflow engine (10 cops via `VariableForceConsumer` trait).
- **Per-department shared modules** — `metrics/method_complexity.rs`, `style/hash_subset.rs`, `style/hash_transform_method.rs`, `style/trailing_comma.rs`, `layout/multiline_literal_brace_layout.rs`.

The canonical reference for RuboCop's AST node predicates is vendored at `vendor/rubocop-ast/`. When adding a shared module consumed by multiple cops, add a `*_CONSUMERS` set to `scripts/dispatch_cops.py` for CI dispatch.

## Key Constraints

- `ruby_prism::ParseResult` is `!Send + !Sync`, so parsing must happen inside each rayon worker thread.
- The `Cop` trait is `Send + Sync`; cops that need mutable visitor state should create a temporary visitor struct internally.
- **No `Mutex` fields for per-file state on shared cop structs.** Cop instances are shared across rayon worker threads. If a cop pre-computes data in `check_source` and reads it in a VariableForce callback (`before_leaving_scope`, `before_declaring_variable`, etc.), storing that data in `Mutex<Vec<...>>` fields creates a TOCTOU race: another thread's `check_source` can overwrite the data before the first thread's VF callback reads it. Use `thread_local!` with `RefCell` instead — within a rayon task, a single file is processed sequentially (`check_source` → VF engine → callbacks).
- Edition 2024 / Rust 1.85+.
- Cops disabled by default in vendor config must override `default_enabled() -> false`, or they will produce false positives when vendored config loading fails. Note: `default_enabled()` has no effect on corpus FP/FN numbers — the corpus baseline config (`baseline_rubocop.yml`) explicitly enables all cops, overriding this. The override only matters for real-world usage.
- Plugin cops depend on the target project's installed gem version, not just the vendored submodule version in this repo. When nitrocop processes `require: [rubocop-rspec]`, it runs `bundle info --path` to find the installed gem and loads that gem's `config/default.yml`. Cops not in the installed gem's config are treated as non-existent (disabled), matching RuboCop's behavior.
- Corpus bundle version matters: `check_cop.py --rerun` requires the corpus bundle installed for the Ruby version in `mise.toml`. If the Ruby version is wrong or gems are missing, config resolution falls back to hardcoded defaults and offense counts will be wildly incorrect (often 5-10x higher). Fix with `cd bench/corpus && BUNDLE_PATH=vendor/bundle bundle install`.
- **Prism block body shapes:** Block bodies are not always `StatementsNode`. When a block has `rescue`, `ensure`, or `else`, Prism wraps the body as a `BeginNode` with `statements()`, `rescue_clause()`, `else_clause()`, and `ensure_clause()`. Code that only checks `body.as_statements_node()` will silently miss the entire block body. Similarly, RuboCop's parser gem produces `itblock`/`numblock` node types for blocks using Ruby 3.4 `it` keyword or numbered parameters — Prism uses regular `BlockNode` with `ItParametersNode`/`NumberedParametersNode` as parameters. Always check both node shapes when processing block bodies.
- **Fixture tests bypass config:** `cargo test` calls `check_source` directly on the cop, bypassing Include patterns, `Enabled: pending` status, and config resolution. A passing test does NOT mean the binary will fire on real files. When debugging "test passes but binary doesn't fire," verify the cop is enabled in the config being used and that the file matches Include patterns.
- **Preview tier gating:** New cops default to `preview` tier (see `src/resources/tiers.json`). Running `cargo run -- .` without `--preview` silently skips preview-tier cops. The corpus oracle uses `--preview` via `bench/corpus/run_nitrocop.py`, which is the single source of truth for oracle-identical invocation. When investigating corpus FPs locally, always use `run_nitrocop.py` rather than raw `cargo run` to avoid tier/config/env mismatches.

## Fixture Rules

- Standard cop fixtures use `tests/fixtures/cops/<dept>/<cop_name>/offense.rb` and `no_offense.rb`.
- Cops that fire once per file or do not fit `^` annotations can use an `offense/` scenario directory plus `no_offense.rb`.
- False positives belong in `no_offense.rb`; missed detections belong in `offense.rb`.
- `# nitrocop-expect:` and `# nitrocop-filename:` are valid fixture directives when standard `^` annotations are not enough.
- Every real behavior fix must add or update fixtures.

## Corpus Quick Reference

Start with cached corpus tools before rerunning expensive checks:

```bash
python3 scripts/investigate_cop.py Department/CopName
python3 scripts/investigate_cop.py Department/CopName --context
python3 scripts/investigate_repo.py rails
python3 scripts/reduce_mismatch.py Department/CopName repo_id path/to/file.rb:line
```

Use `check_cop.py` for aggregate regression checks after a fix:

```bash
python3 scripts/check_cop.py Department/CopName
python3 scripts/check_cop.py Department/CopName --verbose --rerun
python3 scripts/verify_cop_locations.py Department/CopName
python3 scripts/verify_cop_locations.py Department/CopName --style EnforcedStyle=never
```

Important:

- `investigate_cop.py` and `investigate_repo.py` auto-download the latest corpus artifacts. Do not manually download them first.
- `check_cop.py` is count-only; use `verify_cop_locations.py` when you need location-level confirmation. Both support `--style` for variant checks.
- “file-drop noise” is not an excuse for FN gaps. Investigate the actual missed examples.
- `check_cop.py --rerun` depends on the bundle under `bench/corpus/vendor/bundle/`. If needed:

```bash
cd bench/corpus
BUNDLE_PATH=vendor/bundle bundle install
```

- Do not run `cargo run --release --bin bench_nitrocop -- conform` by default during cop-fix loops. Use per-cop corpus gates unless the task explicitly asks for full conformance regeneration.
- **Disk space:** The devcontainer can only hold ~50 corpus repos locally. Never run `bench/corpus/clone_repos.sh` (which syncs all ~5,500 repos). Instead, clone only the repos needed for a specific cop with `python3 scripts/corpus_repo_map.py --clone Department/CopName`, or use `check_cop.py --rerun --clone` which fetches only diverging repos.

## Core Rules

- Use TDD for cop fixes.
- Every real behavior fix must add or update tests.
- Do not remove or move existing fixture cases unless they are factually wrong.
- Do not use `git stash`.
- Do not use broad restore commands like `git restore .` or `git checkout -- .`.
- Commit early and often on shared branches.
- Ignore unrelated working-tree changes unless asked to handle them.
- Document cop investigation findings as `///` doc comments on the cop struct. This includes failed fix attempts: document what was tried, why it regressed (with FP/FN numbers), the commit SHA that was reverted, and what a correct fix would need. This prevents future agents from repeating the same failed approach.
- When editing a skill, check related skills that share conventions or workflows.

## Notes

- All Python files use snake_case names (enforced by ruff N999).
- Workflow internals belong in `scripts/workflows/`.
- Shared Python helpers belong in `scripts/shared/`.
- Keep vendor submodules pinned to release tags, not arbitrary commits.
