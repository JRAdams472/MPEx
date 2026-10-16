# Project Rules for Devin — MPEx

MPEx is a centralized system for managing **mortgage product eligibility and pricing**.
Backend: Rust (axum + sqlx/Postgres + [Zen Engine](https://gorules.io) `zen-engine = "2"`).
Admin UI: Next.js (App Router) + TypeScript + MUI/Emotion.
Rules are **metadata-driven**: named ZEN expressions composed into decision graphs and
row×column adjustment matrices. All evaluation is idempotent — everything needed is in the
loan-model JSON payload plus a resolved publication snapshot.

## Git Workflow

- Each phase of a plan is developed on its own branch and lands as **one PR per phase subticket**.
- **Ticket-prefixed names (required):** branches and PR titles start with the Linear ticket —
  `MPE-<n>-phase-<m>` / `MPE-<n>: Phase <m> — <scope>`.
- Do not push commits directly to `main`.
- Merge only after build + tests + lint are green **and** the user has approved.

## Linear Workflow (required)

- Team: **MPE** (mortgage pricing engine). Every plan gets a **parent ticket**; once planning is
  done and work begins, the parent moves to **In Progress**.
- Attach the plan document to the parent ticket **before** creating phase sub-issues.
- Every phase gets a **sub-issue** named for its branch with scope + acceptance criteria;
  mark it In Progress when the phase starts.
- On phase completion, post **proof of completion** on the sub-issue: run logs (build/test/lint),
  Playwright screenshots for user-facing changes, and a link to the green GitHub CI run.
- **No merge without the user's explicit approval.**

## Code Conventions

- Rust: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test` must pass.
  Errors map through `AppError` (`src/error.rs`); handlers return `Result<_, AppError>`.
- Migrations: `migrations/NNNN_name.sql`, applied by `sqlx::migrate!` at boot.
- Web: App Router, MUI + Emotion, React Query for server state, typed fetch helpers in `lib/`.
- All rule artifacts (decisions, matrices, rate sheets) are **versioned by publication**;
  draft edits never affect published evaluations. `asOf` on evaluate endpoints selects the
  latest publication with `published_at <= asOf`.
- No PII anywhere in the loan model or persisted payloads.

## Plan Close-Out

After the final phase of a plan merges: delete merged branches, verify no new feature ships
at zero coverage, file improvement tickets, and refresh `README.md`/docs for what shipped.
