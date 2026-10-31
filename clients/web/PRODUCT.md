# MPEx Admin Console — product context

**What it is:** the operations surface for MPEx's mortgage product
eligibility & pricing rule lifecycle. Pricers author decisions,
expressions, adjustment matrices, and rate sheets as drafts and publish
them into immutable revisions; publications promote a point-in-time
resolution `dev→qa→uat→prod`; a daily rate-index import seeds base
rates; admins manage grants and read the hash-chained audit trail. A
thin BFF over the Rust API — the browser never sees an access token.

**Mode:** Operate. Scanability, consistency, and role clarity outrank
expression. These are internal users doing repeated technical work —
the interface recedes.

**Audience:** three UI personas plus one machine role:

| Persona | Access |
|---|---|
| admin | everything — publish/promote, grants, audit trail, rate-index import |
| pricer | draft + edit artifacts, evaluate in sandbox; no publish/admin |
| viewer | read artifacts, evaluate in sandbox |
| service | client-credentials tokens; no UI session |

The shell makes authority legible at a glance: the header shows the
signed-in identity + role, admin-only nav items hide for lesser roles,
and the env selector names the current lane.

**Visual world:** GitHub-dark per sibling convention (MPEx/LOLA) —
near-black surfaces (`#0d1117`/`#161b22`), laser-blue accent (`#58a6ff`),
`#30363d` borders, MUI/Emotion components, compact `fontSize: 13`,
`borderRadius: 6`, no-transform buttons. Restrained by design: the
palette is a locked house style, not a choice to revisit.

**Structure:** `/` dashboard, `/products`, `/decisions`, `/expressions`,
`/matrices`, `/rate-sheets` (each with `[key]` editor routes),
`/publications`, `/sandbox` (env-scoped evaluation), `/grants` and
`/audit` (admin-only). Env is an in-shell selector + `env` query param
— deep links carry the lane; switching lanes never mutates state.

**Sibling rule-editor anatomy** (house convention — keep identical in
MPEx and Zara PRODUCT.md; edit in both):

- **List → detail routes.** `/artifacts` renders a dense table (key,
  kind/status, timestamps) with filter inputs above; row →
  `/artifacts/{key}` detail (metadata + spec/graph). Deep links stay
  valid.
- **Revision state is a chip, not a guess.** `StatusChip` renders
  `published` as a success-outlined green chip and `draft` /
  `draft (edited)` as warning-outlined amber. Drafts are mutable
  working copies; published rows are immutable and versioned.
- **Lifecycle verbs are uniform.** `PUT` writes the draft, `Delete`
  removes the draft only, `Publish`/`Promote` mints the immutable
  revision. Nothing ever "edits published" — the UI offers
  new-draft-from-published instead.
- **Density with reveal.** Tables stay compact; detail lives behind
  row expanders (audit events, JSON detail) and per-surface filters,
  not behind page hops.
- **Errors and empties name the next action.** `ErrorPanel` (or an
  Alert) shows the API's message verbatim; empty states say what to do
  first, not just that nothing exists.
- **Authority is legible.** Role-restricted affordances hide from the
  nav, direct access renders a restriction message (never a blank or a
  bounce), and the shell always shows the signed-in identity + role and
  the current environment lane.
- **Env legibility.** Zara encodes the lane as `[env]` URL segments
  with locked-env chips; MPEx uses an in-shell selector. Either way
  the active lane is always visible and switching never mutates data.
- **Keys and hashes are mono.** Artifact keys, content hashes, and
  JSON details render in monospace; everything else uses the default
  compact type.
