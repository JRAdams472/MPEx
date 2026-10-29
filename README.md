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
cp .env.example .env             # then fill in the secret values (see docs/security.md)
docker compose up --build        # db + api + web
# or, API only (hot reload):
docker compose up db -d
cargo run
```

Compose interpolates secrets from `.env` and refuses to start without them;
`.env` is gitignored — never commit it.

Web UI: http://localhost:3000 · API: http://localhost:8080 (`/healthz`, `/api/health`).

## Security

- **AuthN/Z**: every `/api/*` route (except health) requires an OIDC bearer
  token validated against the issuer's JWKS with an algorithm allowlist.
  Roles — `viewer`, `pricer`, `admin`, `service` — resolve per request from
  `role_grants`. Dev runs Dex in compose; humans log in via the web app's
  auth-code+PKCE BFF (tokens sealed in an httpOnly session cookie), services
  use client-credentials (`mpex-svc`, `mpex-ci`).
- **Audit**: sensitive mutations and authenticated 403 denials append
  hash-chained `audit_events` rows inside the mutation's transaction —
  tamper-evident and append-only at the storage layer. Admins read it via
  `GET /api/audit` and re-verify the chain via `GET /api/audit/verify`.
- **PII guard**: `/api/evaluate/*` loan payloads are recursively scanned
  before parsing — denylisted field names and SSN/email value shapes are
  rejected 400 with the JSON path only.
- **Edge**: per-IP rate limits (global + tighter mutation tier → 429 +
  `Retry-After`), 2 MiB body cap, bounded artifact keys/labels/spec sizes,
  allowlist CORS, and a `nosniff`/`DENY`/`no-referrer`/CSP/Permissions-Policy
  header baseline on API and web responses.
- **CI guards**: gitleaks secret scan, `cargo audit` (RUSTSEC), a
  filtered `npm audit` gate (high/critical in prod deps, minus documented
  exceptions in `clients/web/npm-audit-exceptions.json`), and a SonarQube
  Community scan + quality gate (ephemeral per-run instance; sources/
  suppressions in `sonar-project.properties`) run on every PR.
- Secrets, TLS termination, OIDC env, Seq ingest auth, and rotation:
  `docs/security.md`.

## Observability

`docker compose up` also starts **Seq** (logs, http://localhost:5341) and
**Jaeger** (traces, http://localhost:16686). The API ships both over OTLP when
the env flags are set — compose wires them for you:

| Var | Effect |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Trace spans → OTLP/gRPC (compose: `http://jaeger:4317`) |
| `SEQ_URL` | `tracing` events → OTLP logs — full `v1/logs` path (compose: `http://seq/ingest/otlp/v1/logs`) |
| `SEQ_API_KEY` | `x-seq-apikey` header — required by the Seq container's ingest auth |
| `OTEL_SERVICE_NAME` | Resource `service.name` (default `mpex-api`) |

Unset = stdout JSON only (`docker logs mpex-api-1` keeps working either way).
Traces cover request handling (TraceLayer), `snapshot.resolve`/`build` per
revision, `eval.eligibility`/`eval.pricing` fanning out to per-product
`eval.decision`/`eval.product`/`eval.matrix` spans on the eval threads, and
`publish.transaction` with stamped-key counts.

## Verification

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
cd clients/web && npm run lint && npm run typecheck && npm test -- --ci && npm run build

# e2e (Playwright): needs the dev stack up and seeded —
#   docker compose up db -d && RUN_SEED=1 cargo run   (or docker compose up api)
cd clients/web && npm run e2e    # specs in e2e/, auto-starts `next dev`
```

## Layout

- `src/` — Rust binary (`api/`, `domain/`, `rules/`, `pricing/`)
- `migrations/` — sqlx migrations
- `clients/web/` — Next.js admin app
- `seed/` — demo products, LLPA grids, rate sheets, sample loan scenarios
- `docs/` — loan model schema, architecture notes

## Status

All planned phases are in: P1 loan model + eligibility spine, P2 rules
persistence & publications, P3 adjustment matrices, P4 rate sheets + pricing
pipeline, P5/P6 the admin UI (JDM decision editor, expression library,
matrix/rate-sheet grid editors, publications history with `asOf` picker), and
P7 the pricer sandbox (`/sandbox`: scenario editor → eligibility → pricing
with decision traces and the per-line LLPA waterfall) plus Playwright e2e and
determinism/concurrency tests (`tests/hardening.rs`).

`POST /api/publications` atomically publishes changed drafts under a named
publication; `asOf` resolves point-in-time snapshots; matrix specs compile to
Zen decision tables; `POST /api/evaluate/pricing` returns per-rate pricing
lines with an LLPA waterfall per line. See `docs/rules-versioning.md`.
See Linear team **MPE** for the phased plan.
