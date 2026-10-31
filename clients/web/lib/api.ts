// Same-origin BFF — the access token lives in an httpOnly cookie and the
// /bff route attaches it server-side. A 401 means the session is gone:
// bounce through the login flow rather than surfacing a raw error.
const BFF = "/bff";

function bounceOn401(res: Response) {
  if (res.status === 401 && typeof window !== "undefined") {
    // eslint-disable-next-line @next/next/no-location-assign-relative-destination -- plain lib code (no useRouter); /auth/login is a route handler, needs a document navigation
    window.location.assign(`/auth/login?next=${encodeURIComponent(window.location.pathname)}`);
  }
}

export async function apiGet<T>(path: string): Promise<T> {
  const res = await fetch(`${BFF}${path}`);
  bounceOn401(res);
  if (!res.ok) {
    throw new Error(`API ${path} failed: ${res.status}`);
  }
  return res.json() as Promise<T>;
}

async function apiWrite<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<{ status: number; data: T }> {
  const res = await fetch(`${BFF}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  bounceOn401(res);
  const data = (await res.json().catch(() => ({}))) as T;
  return { status: res.status, data };
}

export const apiPost = <T>(path: string, body?: unknown) =>
  apiWrite<T>("POST", path, body);
export const apiPut = <T>(path: string, body: unknown) =>
  apiWrite<T>("PUT", path, body);
export const apiPatch = <T>(path: string, body: unknown) =>
  apiWrite<T>("PATCH", path, body);
export const apiDelete = (path: string) => apiWrite("DELETE", path);

/** Environment lanes (MPE-153): dev is the working lane; qa/uat/prod hold
 *  promoted published rows only. */
export type Env = "dev" | "qa" | "uat" | "prod";
export const ENVS: Env[] = ["dev", "qa", "uat", "prod"];
export const NEXT_ENV: Partial<Record<Env, Env>> = {
  dev: "qa",
  qa: "uat",
  uat: "prod",
};

const listQuery = (asOf?: string, env?: Env) => {
  const params = new URLSearchParams();
  if (asOf) params.set("asOf", asOf);
  if (env) params.set("env", env);
  const q = params.toString();
  return q ? `?${q}` : "";
};

/** Error message from an apiWrite-backed mutation result, or null when clean. */
export function mutationError(m: {
  data?: { status: number; data: unknown };
  isError: boolean;
  error: unknown;
}): string | null {
  if (m.data && m.data.status >= 400) {
    const data = m.data.data as { error?: string };
    return data.error ?? `HTTP ${m.data.status}`;
  }
  return m.isError ? `${m.error}` : null;
}

export interface Product {
  code: string;
  name: string;
  loanType: string;
  amortization: string;
  termMonths: number;
  description: string;
  active: boolean;
  /** MND index label when this is a base product; null otherwise. */
  mndLabel: string | null;
  /** Base product code when this is a derived product; null otherwise. */
  baseProductCode: string | null;
  /** Points added to the base index rate for derived products. */
  rateAdjustment: number;
}

export interface IndexRate {
  label: string;
  rate: number;
}

export interface ResolvedSheet {
  productCode: string;
  rate: number;
  /** "index:<label>" or "base:<code>". */
  source: string;
  adjustment: number;
}

export interface RateIndexPreview {
  fetchedAt: string;
  sourceUrl: string;
  contentHash: string;
  indexRates: IndexRate[];
  sheets: ResolvedSheet[];
  skipped: string[];
  unmapped: string[];
}

export interface RateIndexImport {
  importId: string;
  fetchedAt: string;
  contentHash: string;
  sheets: ResolvedSheet[];
  skipped: string[];
  unmapped: string[];
  publication: { id: string; label: string; publishedAt: string } | null;
}

export interface RateIndexImportRow {
  id: string;
  fetchedAt: string;
  sourceUrl: string;
  contentHash: string;
  /** Resolved index rates keyed by index label (e.g. "30 Yr. Fixed"). */
  rates: Record<string, number>;
  importedBy: string;
  publicationId: string | null;
  createdAt: string;
}

/** Sparse cell update for `PATCH …/{key}/cells` (MPE-128): cell id ->
 *  numeric value, or null to depopulate/unoffer the cell. */
export type CellPatch = Record<string, number | null>;

export interface ArtifactListItem {
  key: string;
  kind?: string;
  productCode?: string | null;
  description?: string;
  status: "draft" | "published";
  updatedAt?: string;
  publishedAt?: string | null;
  publicationId?: string | null;
  hasPublished?: boolean;
}

export interface ArtifactRow extends ArtifactListItem {
  id: string;
  createdAt: string;
  jdm?: DecisionGraphDoc;
  spec?: unknown;
  expression?: string;
}

export interface DecisionGraphDoc {
  nodes: unknown[];
  edges: unknown[];
  [k: string]: unknown;
}

export interface HistoryEntry {
  publishedAt: string;
  publicationId: string;
  publicationLabel: string | null;
  /** Lane this published row lives in (MPE-153). */
  env: Env;
}

export interface ArtifactDetail {
  key: string;
  draft: ArtifactRow | null;
  published: ArtifactRow | null;
  history: HistoryEntry[];
}

/** One `audit_events` row as returned by `GET /api/audit` (MPE-121). */
export interface AuditEvent {
  id: number;
  occurredAt: string;
  actorSub: string;
  action: string;
  artifactTable: string | null;
  artifactKey: string | null;
  detail: Record<string, unknown>;
  prevHash: string;
  hash: string;
}

/** `GET /api/audit/verify` result — `firstBadId`/`reason` only on a break;
 * `watermarkId` present when the pass resumed from a checkpoint. */
export interface AuditVerify {
  checked: number;
  ok: boolean;
  scope: "full" | "incremental";
  watermarkId?: number;
  firstBadId?: number;
  reason?: string;
}

/** Filters for `GET /api/audit` — mirrors the server's AuditQuery. */
export interface AuditQuery {
  actor?: string;
  action?: string;
  table?: string;
  key?: string;
  /** RFC3339 bounds on occurred_at. */
  from?: string;
  to?: string;
  limit?: number;
  offset?: number;
}

/** Actions the audit chain records (audit::action constants). */
export const AUDIT_ACTIONS = [
  "artifact.put",
  "artifact.delete",
  "publication.create",
  "auth.denied",
  "grant.put",
  "grant.delete",
  "rate_index.import",
  "product.rate_config.put",
  "promotion.create",
] as const;

/** artifact_table values seen in audit detail (promotions audit under
 *  `publications`). */
export const AUDIT_TABLES = [
  "decisions",
  "expressions",
  "matrices",
  "rate_sheets",
  "publications",
  "role_grants",
  "products",
  "rate_index_imports",
] as const;

export type SubjectType = "sub" | "email" | "aud";
export type Role = "viewer" | "pricer" | "admin" | "service";

/** A role_grants row (admin surface). `envs` is null = unrestricted,
 *  [] = no lanes permitted, or a lane list (MPE-153/MPE-159). */
export interface Grant {
  id: number;
  subjectType: SubjectType;
  subject: string;
  role: Role;
  grantedAt: string;
  grantedBy: string;
  envs: Env[] | null;
}

export interface GrantUpsert {
  subjectType: SubjectType;
  subject: string;
  role: Role;
  /** Omit for unrestricted (NULL); [] permits no lanes. */
  envs?: Env[];
}

export interface Publication {
  id: string;
  label: string;
  status: string;
  publishedAt: string;
  /** Lane this publication stamped (MPE-153). */
  env: Env;
  decisionCount?: number;
  expressionCount?: number;
  matrixCount?: number;
  rateSheetCount?: number;
  decisions?: string[];
  expressions?: string[];
  matrices?: string[];
  rateSheets?: string[];
}

export interface MatrixAxis {
  id: string;
  label?: string;
  expr: string;
}

export interface MatrixSpec {
  rows: MatrixAxis[];
  cols: MatrixAxis[];
  cells: Record<string, unknown>;
  op?: string;
}

export interface RateSheetSpec {
  rates: number[];
  lockTerms: number[];
  prices: Record<string, number>;
}

export interface CellMatch {
  row: string;
  col: string;
  rowMatched: boolean;
  colMatched: boolean;
  matched: boolean;
  value: unknown;
}

export interface SimulateResult {
  result: unknown;
  cells: CellMatch[];
  performance?: string;
}

export interface EligibilityOutcome {
  product: { code: string; name: string };
  eligible: boolean;
  reasons: unknown[];
  result?: unknown;
  performance?: string;
  trace?: unknown;
}

export interface EligibilityReport {
  revision: string;
  eligible: EligibilityOutcome[];
  ineligible: EligibilityOutcome[];
  elapsedMs: number;
}

export interface AdjustmentLine {
  matrix: string;
  value?: number;
  adjustments: unknown[];
  performance?: string;
  trace?: unknown;
  error?: string;
}

export interface PricingLine {
  rate: number;
  lockTermDays: number;
  basePrice: number;
  adjustmentTotal: number;
  price: number;
  adjustments: AdjustmentLine[];
}

export interface PricedProduct {
  product: { code: string; name: string };
  rateSheet: string;
  lines: PricingLine[];
}

export interface UnpricedProduct {
  product: { code: string; name: string };
  reason: string;
}

export interface PricingReport {
  revision: string;
  priced: PricedProduct[];
  unpriced: UnpricedProduct[];
  elapsedMs: number;
}

export interface PricingOptions {
  rate?: number;
  lockTermDays?: number;
  asOf?: string;
  env?: Env;
  trace?: boolean;
}

export interface EvalOptions {
  asOf?: string;
  env?: Env;
  trace?: boolean;
}

export const endpoints = {
  products: () => apiGet<{ products: Product[] }>("/api/products"),
  decisions: (asOf?: string, env?: Env) =>
    apiGet<{ decisions: ArtifactListItem[] }>(
      `/api/decisions${listQuery(asOf, env)}`,
    ),
  decision: (key: string) =>
    apiGet<ArtifactDetail>(`/api/decisions/${encodeURIComponent(key)}`),
  saveDecision: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/decisions/${encodeURIComponent(key)}`, body),
  deleteDecision: (key: string) =>
    apiDelete(`/api/decisions/${encodeURIComponent(key)}`),
  expressions: (asOf?: string, env?: Env) =>
    apiGet<{ expressions: ArtifactListItem[] }>(
      `/api/expressions${listQuery(asOf, env)}`,
    ),
  expression: (key: string) =>
    apiGet<ArtifactDetail>(`/api/expressions/${encodeURIComponent(key)}`),
  saveExpression: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/expressions/${encodeURIComponent(key)}`, body),
  deleteExpression: (key: string) =>
    apiDelete(`/api/expressions/${encodeURIComponent(key)}`),
  validateExpression: (expression: string, kind?: string) =>
    apiPost<{ ok: boolean; error?: string }>("/api/validate/expression", {
      expression,
      kind,
    }),
  matrices: (asOf?: string, env?: Env) =>
    apiGet<{ matrices: ArtifactListItem[] }>(
      `/api/matrices${listQuery(asOf, env)}`,
    ),
  matrix: (key: string) =>
    apiGet<ArtifactDetail>(`/api/matrices/${encodeURIComponent(key)}`),
  saveMatrix: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/matrices/${encodeURIComponent(key)}`, body),
  deleteMatrix: (key: string) =>
    apiDelete(`/api/matrices/${encodeURIComponent(key)}`),
  /** Sparse draft cell updates — `"rowId:colId"` -> number | null (MPE-128). */
  patchMatrixCells: (key: string, cells: CellPatch) =>
    apiPatch<ArtifactRow>(`/api/matrices/${encodeURIComponent(key)}/cells`, {
      cells,
    }),
  simulateMatrix: (spec: unknown, context: unknown) =>
    apiPost<SimulateResult>("/api/simulate/matrix", { spec, context }),
  rateSheets: (asOf?: string, env?: Env) =>
    apiGet<{ rateSheets: ArtifactListItem[] }>(
      `/api/rate-sheets${listQuery(asOf, env)}`,
    ),
  rateSheet: (key: string) =>
    apiGet<ArtifactDetail>(`/api/rate-sheets/${encodeURIComponent(key)}`),
  saveRateSheet: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/rate-sheets/${encodeURIComponent(key)}`, body),
  deleteRateSheet: (key: string) =>
    apiDelete(`/api/rate-sheets/${encodeURIComponent(key)}`),
  /** Sparse draft price updates — `"rate:term"` -> number | null (MPE-128). */
  patchRateSheetCells: (key: string, cells: CellPatch) =>
    apiPatch<ArtifactRow>(`/api/rate-sheets/${encodeURIComponent(key)}/cells`, {
      cells,
    }),
  publications: () => apiGet<{ publications: Publication[] }>("/api/publications"),
  publication: (id: string) =>
    apiGet<Publication>(`/api/publications/${encodeURIComponent(id)}`),
  publish: (label: string) => apiPost<Publication>("/api/publications", { label }),
  /** Adjacent-lane promotion (MPE-153): stamps changed artifacts from `from`
   *  into `to` under a new publication. 400 non-adjacent, 409 empty delta. */
  promote: (from: Env, to: Env, label: string) =>
    apiPost<Publication>("/api/admin/promotions", { from, to, label }),
  grants: () => apiGet<{ grants: Grant[] }>("/api/admin/grants"),
  upsertGrant: (body: GrantUpsert) =>
    apiPut<{ grant: Grant; created: boolean }>("/api/admin/grants", body),
  revokeGrant: (id: number) => apiDelete(`/api/admin/grants/${id}`),
  /** Filtered, paginated audit trail (admin; MPE-121). */
  auditEvents: (q: AuditQuery = {}) => {
    const params = new URLSearchParams();
    for (const [k, v] of Object.entries(q)) {
      if (v !== undefined && v !== "") params.set(k, String(v));
    }
    const qs = params.toString();
    return apiGet<{ events: AuditEvent[] }>(
      `/api/audit${qs ? `?${qs}` : ""}`,
    );
  },
  /** Recompute the whole hash chain — `{checked, ok, firstBadId?, reason?}`. */
  auditVerify: () => apiGet<AuditVerify>("/api/audit/verify"),
  rateIndex: () =>
    apiGet<{ indexLabels: string[]; imports: RateIndexImportRow[] }>(
      "/api/rate-index",
    ),
  previewRateIndex: () =>
    apiPost<RateIndexPreview>("/api/admin/rate-index/preview"),
  importRateIndex: () =>
    apiPost<RateIndexImport>("/api/admin/rate-index/import"),
  saveProductRateConfig: (
    code: string,
    body: {
      mndLabel?: string | null;
      baseProductCode?: string | null;
      rateAdjustment?: number;
    },
  ) =>
    apiPut<{ code: string }>(
      `/api/admin/products/${encodeURIComponent(code)}/rate-config`,
      body,
    ),
  evaluateEligibility: (loan: unknown, opts?: EvalOptions) =>
    apiPost<EligibilityReport>("/api/evaluate/eligibility", {
      loan,
      asOf: opts?.asOf,
      env: opts?.env,
      trace: opts?.trace,
    }),
  evaluatePricing: (loan: unknown, products: string[], opts?: PricingOptions) =>
    apiPost<PricingReport>("/api/evaluate/pricing", {
      loan,
      products,
      rate: opts?.rate,
      lockTermDays: opts?.lockTermDays,
      asOf: opts?.asOf,
      env: opts?.env,
      trace: opts?.trace,
    }),
};
