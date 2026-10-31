# Security runbook

MPE-97 hardening: OIDC authN/Z, hash-chained audit, secret hygiene, transport
baselines. This doc is the operator-facing half — how to configure it per
environment and rotate what rotates.

## Secrets

Tracked files contain **no secret values** — `docker-compose.yml` interpolates
`${VAR:?}` from `.env` (gitignored), and `.env.example` ships placeholders
only. Compose fails closed on any unset secret.

| Secret | Used by | Notes |
|---|---|---|
| `POSTGRES_PASSWORD` | `db`, `api` (`DATABASE_URL`) | Any strong value; db creates the role on first boot. |
| `DEX_MPEX_SVC_SECRET` | `dex` (client `mpex-svc`) | Service-token client secret. |
| `DEX_MPEX_CI_SECRET` | `dex` (client `mpex-ci`) | CI/automation client secret. |
| `SESSION_SECRET` | `web` | JWE key for `mpex_session` cookies — `openssl rand -hex 32`. |
| `SESSION_MAX_AGE_S` | `web` | Session-cookie ceiling (default 43200 = 12h); the cookie is further bounded by the refresh grant's expiry when the IdP exposes it. |
| `SEQ_ADMIN_PASSWORD_HASH` | `seq` | First-boot admin seed (`SEQ_FIRSTRUN_*`) — salted hash, not plaintext: `echo 'pw' \| docker run --rm -i datalust/seq config hash`. |
| `SEQ_API_KEY` | `api` | Seq ingest key, `x-seq-apikey` header. Create after first boot. |

- **Local dev**: `cp .env.example .env`, fill values, `docker compose up --build`.
- **CI**: generates all secrets per run (`openssl rand -hex`) — nothing static
  in `ci.yml` either.
- **Production**: inject via your secret manager (k8s secrets, SSM, Vault) as
  env vars — same names. Never bake into images or files.

## TLS

| Env | Termination |
|---|---|
| **dev** | Plain HTTP on loopback only (`localhost`). No TLS — nothing leaves the host. |
| **prod** | TLS at the reverse proxy / ingress (nginx, ALB, Traefik). API and web stay HTTP behind it. Set `SESSION_SECURE=1` (Secure cookies), point `CORS_ORIGIN`/`OIDC_ISSUER`/`OIDC_REDIRECT_URI`/`SEQ_URL` at `https://` endpoints. |

- **HSTS** belongs at the terminator — set `Strict-Transport-Security:
  max-age=31536000; includeSubDomains` in the proxy config, not in the app
  (browsers ignore it over HTTP anyway).
- Internal hops (api→dex JWKS, api→seq, web→api) ride the private network;
  put them on TLS too if the network isn't trusted.

## OIDC

Dev runs Dex (`dex/config.dev.yaml`) — auth-code+PKCE for the UI
(`mpex-web`, public client), client-credentials for machines (`mpex-svc`,
`mpex-ci`; requires dex ≥ v2.46.0 for `clientCredentialsEnabled`). Production
repoints `OIDC_ISSUER`/`OIDC_JWKS_URL` at the real IdP — no code changes.

- `OIDC_AUDIENCE` = comma-separated **client IDs**, not a resource indicator —
  Dex stamps `aud` with the client_id. List every registered client that may
  call the API.
- Roles resolve from `role_grants` keyed by **typed** subject
  (`subject_type` `sub`/`email`/`aud`): humans by `email`, machines by `aud`
  (client_id), plus `sub`. Seed file `seed/002_dev_principals.sql` is the dev
  example. Grant and revoke without SQL via the admin-gated surface
  (`GET`/`PUT /api/admin/grants`, `DELETE /api/admin/grants/{id}`); mutations
  write attributed `grant.put`/`grant.delete` audit rows in-transaction,
  take effect on the caller's next request on the same instance (other
  replicas converge within ~60s), and refuse to revoke the caller's own
  admin grant or the last remaining admin grant.

## Seq ingestion

`SEQ_FIRSTRUN_REQUIREAUTHENTICATIONFORHTTPINGESTION=true` rejects
unauthenticated ingest from first boot. Provision the ingest key once:

```sh
docker exec mpex-seq-1 seqcli apikey create -t mpex-ingest \
  --connect-username admin --connect-password '<admin-password>' --json
# → prints {"Token":"..."} — put it in .env as SEQ_API_KEY
```

(API keys are also creatable in the UI under Settings → API keys.)

## Rotation

| What | How |
|---|---|
| `DEX_MPEX_*_SECRET` | Update `.env`, `docker compose up -d dex`; outstanding tokens die at `exp` — mint fresh ones. |
| `SESSION_SECRET` | Update + restart `web`. Invalidates every session cookie — users re-login. |
| `POSTGRES_PASSWORD` | `ALTER ROLE mpex WITH PASSWORD '…'`, update `.env`, recreate `api`/`db`. |
| `SEQ_API_KEY` | Create a new key, update `.env`, restart `api`, revoke the old key in Seq. |
| OIDC signing keys | IdP-side rotation only — the validator refreshes JWKS automatically on unknown `kid`. |
| Dex user passwords | `dex/config.dev.yaml` holds bcrypt hashes — regenerate with a bcrypt tool and restart dex (dev only). |

## Edge hardening

- **Rate limits** (per-IP, token bucket): global `RATE_LIMIT_PER_MIN` (default
  120/min) on every request; mutation tier `RATE_LIMIT_MUTATIONS_PER_MIN`
  (default 30/min) on PUT/DELETE and the POSTs that write or reach outside
  (`POST /api/publications`, `POST /api/admin/rate-index/preview`,
  `POST /api/admin/rate-index/import` — the import mutates, and a preview
  flood would amplify an external fetch). Eval/simulate POSTs stay on the
  global tier — compute isn't blocked when mutations exhaust. 429s carry
  `Retry-After`.
- **Client IP**: peer address via `ConnectInfo` by default. Behind a proxy,
  set `RATE_LIMIT_TRUST_XFF=1` to key on `X-Forwarded-For` — **only** when the
  proxy overwrites the header, or clients can reset their own bucket.
- **Body cap**: 2 MiB (`DefaultBodyLimit`) → 413. Semantic caps inside it:
  `{key}` `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`, publication label ≤ 200 chars,
  expression ≤ 64 KiB, serialized spec/jdm ≤ 1 MiB.
- `RATE_LIMIT_DISABLE=1` turns limits off (local dev escape hatch — never prod).

## Rate-index import

The admin rate-index feature fetches an **external, untrusted** HTML page
(`RATE_INDEX_URL`, default MND) server-side — the browser never touches the
source. Treat the fetched content as hostile input: the parser confines
matching to the `mtg-rates` table, requires bare `<th>label</th><td>rate%`
rows for all six allowlisted labels, bounds rates to (0.5, 25), and fails
**all-or-nothing** — markup drift writes nothing. Fetch has a bounded
timeout (`RATE_INDEX_TIMEOUT_SECS`) and upstream failures map to 502, never
500. The import itself is one transaction: draft upserts → a
rate-sheets-**scoped** publication (an unattended import can never sweep
unrelated decision/matrix drafts live) → `rate_index_imports` provenance
(source URL, sha256 content hash, rates, actor, publication) → a
`rate_index.import` audit row. `product.rate_config.put` rows audit
per-product rate-source changes. The daily scheduler
(`RATE_INDEX_DAILY_AT=HH:MM` UTC, unset = off) runs the same path under the
`rate-index-scheduler` synthetic actor — scheduled failures log and never
crash the API.

## PII guard

`POST /api/evaluate/*` loan payloads pass `security::pii_guard` before
`LoanScenario::parse`: a recursive JSON walk rejects denylisted field names
(`ssn`, `email`, `phone`, `dob`, `account_number`, …; camel/snake/kebab
normalized, token-boundary matched so `existingLiens`/`property.state` pass)
and recognizable value shapes (SSN `\d{3}-\d{2}-\d{4}`, bare 9-digit runs,
email). 400s carry the dotted field path only — values never reach errors or
logs. Artifacts beyond the loan model stay PII-free by invariant; the guard
is the enforcement point on the one endpoint family that accepts free-form
loan data.

## Log masking

`security::mask` is an active scrub layer in front of every log sink —
defense-in-depth behind the "never record sensitive fields" rule enforced
at the boundary (PII guard, path-only errors). If a sensitive value slips
through anyway — a Debug impl, a query string, a credential inside an
error message — the serialized line is masked before it leaves the
process. The bias is deliberate over-masking: a scrubbed benign field is a
nuisance, a leaked secret is an incident.

- **Stdout**: the JSON fmt layer writes through `MaskingStdout`
  (`telemetry.rs`), buffering each event to the newline before rewriting
  `"key":"value"` pairs (scalars, nulls, and nested objects included),
  bare `key=value` / `key: value` forms, DSN passwords
  (`postgres://user:pass@host`), and value shapes (SSN `\d{3}-\d{2}-\d{4}`,
  bare 9-digit account numbers, compact JWTs) to `[MASKED]`.
- **Key matching** mirrors the PII guard's tokenization: the denylisted
  field names plus credential keys (`password`, `secret`, `token`,
  `api_key`, `authorization`, `cookie`, …) hit inside camel/snake/kebab
  compounds (`refreshToken`, `borrower_ssn`, `x-api-key`) while
  `existingLiens`, `settings`, `filename`, and actor emails pass.
- **OTLP logs** (Seq): `MaskingProcessor` wraps the batch log processor
  and masks each record's *body* before export — including Map/List
  values under sensitive keys. **Caveat**: opentelemetry-sdk 0.33 exposes
  structured record attributes read-only, so they can't be rewritten at
  that seam; attributes rely on the never-record guarantee
  (`tests/pii.rs`). This is the documented residual of the OTLP path.
- **Traces** carry span metadata only (durations, counts) — no payload
  fields — so they're not a masking surface.

## Warm-path latency

Zara-parity performance target: warm-path evaluation < 15 ms.
`tests/pricing.rs`'s `warm_path_pricing_under_15ms` seeds and publishes a
catalog, then prices 50 requests with loan variation defeating
memoization. It reports min/median/p95 for the response `elapsedMs`
(snapshot resolve + pure engine eval) and full-request wall time, and
asserts the `elapsedMs` p95 stays under 15 ms. Wall time includes JWT
verification and middleware on a debug build, so it is context, not the
asserted bound.

## CI dependency scans

`cargo audit` (RUSTSEC advisories) + a filtered `npm audit` gate
(`clients/web/scripts/audit.mjs`, run via `npm run audit:gate`) gate every
PR. The npm gate fails on un-excepted high/critical advisories in production
dependencies; dev-only findings are reported but do not gate. Justified,
unreachable-path ignores live in `.cargo/audit.toml` (Rust) and
`clients/web/npm-audit-exceptions.json` (npm) with the reasoning and review
trigger inline — e.g. jdm-editor's `zod@3` pin has no stable fix and a zod@4
override would break the editor (MPE-123); the exception is removed when
upstream ships fixed transitives.

## SonarQube (code quality)

A `sonarqube` CI job boots an ephemeral `sonarqube:community` container in
the run, scans the checkout (`sonar-project.properties` scopes sources/tests
and holds durable suppressions), and enforces the quality gate via
`sonar.qualitygate.wait=true`. Community Build's Rust analyzer covers `src/`
natively; TypeScript/JS analysis is native; the Clippy sensor is skipped
(cargo isn't on the scanner's PATH).

Local parity: `docker compose -f docker-compose.sonarqube.yml up -d` (or
reuse the LOLA instance on :9000 — projects are keyed `mpex` vs `lena2`),
wait for `"status":"UP"` at `http://localhost:9000/api/system/status`, then:

```sh
docker run --rm -e SONAR_TOKEN=<ui-generated-token> \
  -e SONAR_HOST_URL=http://host.docker.internal:9000 \
  -v "$(pwd -W):/usr/src" sonarsource/sonar-scanner-cli
```

UI at `http://localhost:9000` (admin/admin on first boot). The `sonarqube`
MCP server in `.devin/mcp_config.local.json` queries issues/hotspots/quality
gate — point `SONARQUBE_TOKEN` at a token generated in the UI.

## Audit

`audit_events` is append-only (storage-level trigger) and hash-chained;
`GET /api/audit/verify` (admin) checks the chain — run it after restores
or suspected tampering. Each successful pass appends an
`audit_verify_watermarks` row recording the verified tail, so routine
verifies cost O(new events): the checkpoint row is recomputed once, then
only rows after it are walked. A rewritten history breaks the checkpoint
comparison (a consistent rewrite must propagate forward to the watermarked
tail) or a post-checkpoint link. The residual boundary: a *single-point*
content edit before the checkpoint that leaves the chain locally
inconsistent is invisible to an incremental pass — `GET
/api/audit/verify?full=true` forces a from-genesis walk for deep sweeps.
`auth.denied` rows record 403s; 401s are log-only by design (audit rows
for bad tokens are a DoS vector).
