# Rules versioning & publications

MPEx stores rule artifacts (decisions, expressions, later matrices and rate
sheets) as **append-only rows keyed `(key, status, published_at)`**. Nothing
published is ever updated — history accumulates and `asOf` resolves a
point-in-time view over it.

## Lifecycle

```
 PUT /api/{decisions,expressions}/{key}   →  mutable draft (one per key)
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
| `GET /api/publications` | Publications with artifact counts |
| `GET /api/publications/{id}` | Publication detail + stamped keys |
| `POST /api/publications` | `{label}` → atomic publish → `201` or `409` |

## Expressions

`expressions` are named reusable ZEN fragments (`kind`: `standard` | `unary`),
versioned exactly like decisions. They're validated at save time
(`zen_expression::compile_{,unary_}expression`) and consumed by rule editors /
matrix compilers — they're not loaded into the eval snapshot yet.

## Evaluation revisions (unchanged)

`POST /api/evaluate/eligibility` accepts `asOf`: omitted = latest published
(with draft fallback for keys never published — dev ergonomics), `"draft"` =
drafts only (editor preview), RFC3339 = historic published snapshot.
