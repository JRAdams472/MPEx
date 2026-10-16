# Project Rules for Devin — MPEx

MPEx is a centralized system for managing **mortgage product eligibility and pricing**.
Backend: Rust (axum + sqlx/Postgres + `zen-engine = "2"`, `zen-expression = "2"`).
Admin UI: Next.js (App Router) + TypeScript + MUI/Emotion + React Query.
No mobile client. Dark UI theme: GitHub-dark palette (near-black surfaces, laser-blue accent).

Non-negotiable invariants:

- **Idempotent evaluation** — `/api/evaluate/*` responses depend only on the request body
  plus the resolved rule revision. No clock, no randomness, no I/O inside the engine path.
- **No PII** — the loan model and every persisted artifact carry loan/scenario attributes
  only (no names, SSNs, addresses, account numbers).
- **Immutable artifacts** — decisions, matrices, and rate sheets are append-only rows keyed
  `(key, status, published_at)`. A publication is a point-in-time resolution over those rows;
  historic evaluation resolves each key's latest row `<= asOf`. Never UPDATE published rows.
- **No DB I/O inside the engine** — the eval path runs on `!Send` (`Rc`-based `Variable`)
  worker threads with a pre-resolved snapshot loader. All DB access happens on the async
  runtime before eval threads start. High-volume path: fingerprint-cached snapshots +
  `CachedLoader` (compiled-once graphs); never regress this.

## Git Workflow

- Each phase of a plan is developed on its own ticket-prefixed branch: `MPE-<n>-phase-<m>`
  (e.g. `MPE-11-phase-1`). Descriptive slugs allowed after the ticket: `MPE-11-llpa-grid`.
- **One PR per phase subticket (required)** — never batch phase tickets into a single PR.
- **Do not push commits directly to `main`.** Branches and PR titles must start with the
  Linear ticket number: `MPE-13: Phase 3 — adjustment matrices`.
- **Do not open a PR without a Linear ticket.**
- Before opening a PR: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, and web `lint`/`typecheck`/`test`/`build` all green. Commit `Cargo.lock`
  and `clients/web/package-lock.json` changes — CI fails when they drift.
- **No PR or phase merges without the user's explicit approval.** Present the PR link +
  proof, then wait.

## Linear Workflow (required)

- Team **MPE**, project **MPEx** (`P-LEN-2`). Every plan gets a **parent ticket**; once
  planning is done and work begins, the parent moves to **In Progress**.
- **Attach the plan document to the parent ticket before creating phase sub-issues** —
  the megaplan goes on the parent first, then the subtickets.
- Every phase gets a **sub-issue** under the parent named for its branch, with scope +
  acceptance criteria; mark it **In Progress** when its phase starts.
- On phase completion, post **proof of completion** on the sub-issue before asking for
  merge approval:
  - Run logs (build/test/lint output).
  - **Playwright screenshots** for any user-facing change (`playwright` MCP — capture real
    screens, don't describe them).
  - Link/summary of the green **GitHub CI run**.
- Linear attachments: `prepare_attachment_upload` → PUT the file to the signed URL with the
  exact signed headers → `create_attachment_from_upload`. `SignatureDoesNotMatch` means the
  headers drifted — request a fresh URL and PUT immediately (URLs expire in ~60s).

## Code Conventions

- Rust: handlers return `Result<_, AppError>`; errors map in `src/error.rs`. Migrations are
  append-only `migrations/NNNN_name.sql`, applied by `sqlx::migrate!` at boot.
- Rule artifacts resolve through `SnapshotStore` (`src/rules/snapshot.rs`): one
  `DISTINCT ON (key)` query per request, then pure eval on `spawn_blocking` current-thread
  runtimes — keep it that way.
- Web: App Router + MUI/Emotion + React Query; typed fetch helpers in `lib/`; the GoRules
  `@gorules/jdm-editor` loads client-only (`ssr: false`).
- Boy-scout rule: fix trivial adjacent issues in a touched file when low-risk; never let it
  balloon the diff.

## Evidence Gathering

- Raw logs: `docker logs mpex-api-1 --since 30m` (JSON `tracing` output).
- Observability stack (Seq/Jaeger/SonarQube) can be added later — file a ticket when needed
  rather than silently shipping without it.

## Plan Close-Out

After the final phase of a plan merges:

1. Delete merged phase branches locally and on GitHub (verify PR state with `gh` first).
2. No new feature left at zero coverage — every new service, handler, or page gets at least
   one unit or integration test.
3. File Linear tickets (project **MPEx**) for improvements or new features inspired by the
   completed work.
4. Update `README.md` + `docs/` for what shipped — sweep for stale claims (endpoint lists,
   env vars, workflow names).
5. **Size & coverage report** on the parent ticket at close-out: `tokei -e node_modules -e
   target src clients/web/app clients/web/lib migrations tests`, source/test split via
   `git ls-files`, `cargo test` results, `npm test -- --coverage` numbers.

## Verification Commands

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd clients/web && npm run lint && npm run typecheck && npm test -- --ci && npm run build
docker compose up --build        # db + api + web; api seeds when RUN_SEED=1
```
