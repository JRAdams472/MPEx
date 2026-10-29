# Rules versioning & publications

MPEx stores rule artifacts (decisions, expressions, matrices, rate sheets) as
**append-only rows keyed `(key, status, published_at)`**. Nothing published is
ever updated — history accumulates and `asOf` resolves a point-in-time view
over it.

## Lifecycle

```
 PUT /api/{decisions,expressions,matrices,rate-sheets}/{key}
                                           →  mutable draft (one per key)
 POST /api/publications                   →  atomic publish of every *changed* draft
 GET  ...?asOf=<rfc3339>                  →  published snapshot at a timestamp
```

- **Drafts** are the working state — one mutable row per key, freely
  upsertable/deletable. Drafts never appear in `asOf` resolution.
- **Publish** (`POST /api/publications {label}`) runs in a single transaction:
  insert one `publications` row, copy each draft whose content differs from its
  latest published row into a new immutable `status='published'` row stamped
  with the publication's `published_at` and `publication_id`, commit. Changed
  drafts only — unchanged keys are not re-stored, so history doesn't duplicate
  documents. `409` when nothing changed (no empty publications).
- **`asOf`** resolution is `DISTINCT ON (key) ... WHERE status='published' AND
  published_at <= ts ORDER BY key, published_at DESC` per artifact kind —
  identically for decisions, expressions, and eval snapshots.
- **Snapshot cache** keys on a `(max(updated_at), count)` fingerprint per
  revision; publishing inserts rows, which bumps the fingerprint and
  invalidates the cached loader with no explicit eviction.

## Endpoints

| Route | Purpose |
|---|---|
| `GET /api/decisions[?asOf=]` | Draft list, or published snapshot at `asOf` |
| `GET /api/decisions/{key}` | `{draft, published, history[]}` |
| `PUT /api/decisions/{key}` | Upsert draft (`{kind, productCode?, jdm}`; JDM validated) |
| `DELETE /api/decisions/{key}` | Delete draft only |
| `GET /api/expressions[?asOf=]` | Draft list, or published snapshot at `asOf` |
| `GET /api/expressions/{key}` | `{draft, published, history[]}` |
| `PUT /api/expressions/{key}` | Upsert draft (`{kind?, description?, expression}`; ZEN compile-checked) |
| `DELETE /api/expressions/{key}` | Delete draft only |
| `GET /api/matrices[?asOf=]` | Draft list, or published snapshot at `asOf` |
| `GET /api/matrices/{key}` | `{draft, published, history[]}` |
| `PUT /api/matrices/{key}` | Upsert draft (`{kind?, productCode?, spec}`; spec validated + compiled to JDM at write) |
| `DELETE /api/matrices/{key}` | Delete draft only |
| `POST /api/simulate/matrix` | `{spec, context}` → `{result, cells[]}` editor preview with per-cell match detail |
| `GET /api/rate-sheets[?asOf=]` | Draft list, or published snapshot at `asOf` |
| `GET /api/rate-sheets/{key}` | `{draft, published, history[]}` (`key` = product code) |
| `PUT /api/rate-sheets/{key}` | Upsert draft (`{spec}`; spec validated) |
| `DELETE /api/rate-sheets/{key}` | Delete draft only |
| `POST /api/evaluate/eligibility` | `{loan, asOf?, trace?}` → per-product `eligible`/`reasons` buckets |
| `POST /api/evaluate/pricing` | `{loan, products[], rate?, lockTermDays?, asOf?, trace?}` → per-rate lines + LLPA waterfall |
| `GET /api/publications` | Publications with artifact counts |
| `GET /api/publications/{id}` | Publication detail + stamped keys |
| `POST /api/publications` | `{label}` → atomic publish → `201` or `409` |

## Matrices

`matrices` store a row×column adjustment spec — each row and column is a
standard ZEN rule against the full context; a cell's scalar value applies when
its row AND column rules match. Overlap is legal: every matching cell collects
into `adjustments[]`, combined downstream by `op` (`sum` default | `list` |
`min` | `max`).

```jsonc
// spec
{
  "rows": [{"id": "hi", "expr": "loan.ltv > 80"}],
  "cols": [{"id": "good", "expr": "borrower.creditScore >= 740"}],
  "cells": {"hi:good": 0.25},
  "op": "sum"
}
```

At write time the spec compiles to a decision table: one input column with no
`field` (cells run as standard expressions), each populated cell → a rule
`(<rowExpr>) and (<colExpr>)`, output `adjustments[]` under `first` policy
(engine collect-column semantics — all matches gather while the output stays a
mergeable object). An aggregate node emits `{adjustments, value, op}`. Drafts
and published rows both carry the compiled `jdm`, so previews evaluate exactly
what a publish ships.

## Rate sheets & pricing

`rate_sheets` are per-product data artifacts — `key` is the product code, so a
product has exactly one sheet. The spec is a rate × lock-term price grid:

```jsonc
{
  "rates": [6.5, 6.75],
  "lockTerms": [30, 45],
  "prices": {"6.5:30": 100.0, "6.5:45": 99.75, "6.75:30": 99.5}
}
```

Missing cells mean the combo isn't offered. They version and publish like
every other artifact, but ship no JDM — snapshots resolve their `spec` beside
the decision loader.

`POST /api/evaluate/pricing` prices the listed `products` in parallel:

1. Base price = the product's sheet at (`rate`, `lockTermDays`). With `rate`
   omitted, every offered rate gets a **line**. `lockTermDays` falls back to
   `loan.scenario.lockTermDays`, then the sheet's shortest term.
2. Every `llpa` matrix scoped to the product (`productCode` = the product, or
   NULL = shared) evaluates with `loan.rate` / `loan.scenario.lockTermDays`
   injected — matrix expressions can key on them.
3. `price = basePrice + Σ matrix values`, with an `adjustments[]` waterfall
   entry per matrix (`{matrix, value, adjustments}`).

Products without a sheet (or an offered rate/term) land in `unpriced[]` with a
reason. Responses memoize on `(snapshot fingerprint, loan, products, rate,
lockTerm, trace)` — evaluation is pure, and a publish bumps the fingerprint so
stale entries never serve.

## Expressions

`expressions` are named reusable ZEN fragments (`kind`: `standard` | `unary`),
versioned exactly like decisions. They're validated at save time
(`zen_expression::compile_{,unary_}expression`) and resolve into the eval
snapshot alongside decisions — each expression is wrapped in a synthetic
single-purpose decision, so rule graphs call it via `decisionNode` on its key:

- `standard` (e.g. `borrower.creditScore >= 740 ? 'prime' : 'near-prime'`) evaluates
  against the full context and emits `{result: <value>}`.
- `unary` (e.g. `<= 80`) is wrapped in a one-rule decision table that
  predicates the caller's `input` field and emits `{result: bool}` — set
  `input` upstream (e.g. an expressionNode `input = loan.ltv`) before the
  decisionNode call.

The sub-decision's `{result}` merges into the passing context, so the next
node reads `result` directly. Keep `expr.*` key namespacing — a collision
with a decision/matrix key skips the expression (logged warn). Snapshot
fingerprints cover `expressions`, so republishing one invalidates cached
snapshots and pricing memos like any other artifact.

Matrix axis expressions can't call `decisionNode` — they're plain ZEN
compiled at save time — so matrices don't reference named expressions
directly; wrap the matrix in a decision graph that evaluates them.

## Evaluation revisions (unchanged)

`POST /api/evaluate/eligibility` accepts `asOf`: omitted = latest published
(with draft fallback for keys never published — dev ergonomics), `"draft"` =
drafts only (editor preview), RFC3339 = historic published snapshot.
