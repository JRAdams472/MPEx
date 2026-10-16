# MPEx

**MPEx** (Mortgage Pricing & Eligibility eXchange) is a centralized system for managing
mortgage product eligibility and pricing. Rules are metadata-driven and executed by the
[GoRules Zen Engine](https://gorules.io) (`zen-engine`, Rust) against a PII-free loan model.

## What it does

- **Eligibility rules** filter which products a loan scenario qualifies for. Each product has
  its own decision graph (shared base rules via sub-decision nodes); evaluation runs in
  parallel across the catalog.
- **Pricing rules** execute in a single pass: base price from the product's rate sheet, then
  LLPA (loan-level price adjustment) matrices apply on top — all products priced in parallel
  in one request.
- **Adjustment matrices** are edited excel-style: each row and column is a ZEN expression,
  and each cell is the adjustment that applies when both its row and column rules are true.
  Matrices compile to Zen decision tables at publish time.
- **Rate sheets** are per-product grids (note-rate rows × lock-term columns → price) tied to
  a **publication** stamped with a publish date/time — so historic pricing can be re-run by
  passing an `asOf` timestamp.
- **All rules are editable in the web UI**: embedded GoRules JDM editor for decision graphs,
  plus purpose-built matrix and rate-sheet grid editors.
- **Idempotent evaluation**: `POST /api/evaluate/eligibility` and `POST /api/evaluate/pricing`
  take the full loan model in the request body and need no external data beyond the resolved
  publication snapshot.

## Stack

| Layer | Technology |
|---|---|
| API | Rust, axum, sqlx, `zen-engine` v2 |
| DB | PostgreSQL (JSONB artifacts, sqlx migrations) |
| Web | Next.js (App Router), TypeScript, MUI + Emotion, React Query |
| Rule editing | `@gorules/jdm-editor` (embedded decision-graph editor) |
| Dev/deploy | Docker Compose |

## Getting started

```bash
docker compose up --build        # db + api + web
# or, API only (hot reload):
docker compose up db -d
cargo run
```

Web UI: http://localhost:3000 · API: http://localhost:8080 (`/healthz`, `/api/health`).

## Layout

- `src/` — Rust binary (`api/`, `domain/`, `rules/`, `pricing/`)
- `migrations/` — sqlx migrations
- `clients/web/` — Next.js admin app
- `seed/` — demo products, LLPA grids, rate sheets, sample loan scenarios
- `docs/` — loan model schema, architecture notes

## Status

P1 (loan model + eligibility spine), P2 (rules persistence & publications),
P3 (adjustment matrices), and P4 (rate sheets + pricing pipeline) are in.
`POST /api/publications` atomically publishes changed drafts under a named
publication; `asOf` resolves point-in-time snapshots; matrix specs compile to
Zen decision tables; `POST /api/evaluate/pricing` returns per-rate pricing
lines with an LLPA waterfall per line. See `docs/rules-versioning.md`.
See Linear team **MPE** for the phased plan.
