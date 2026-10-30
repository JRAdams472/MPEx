// Same-origin BFF — the access token lives in an httpOnly cookie and the
// /bff route attaches it server-side. A 401 means the session is gone:
// bounce through the login flow rather than surfacing a raw error.
const BFF = "/bff";

function bounceOn401(res: Response) {
  if (res.status === 401 && typeof window !== "undefined") {
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
export const apiDelete = (path: string) => apiWrite("DELETE", path);

const asOfQuery = (asOf?: string) =>
  asOf ? `?asOf=${encodeURIComponent(asOf)}` : "";

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
  importedBy: string;
  publicationId: string | null;
  createdAt: string;
}

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
}

export interface ArtifactDetail {
  key: string;
  draft: ArtifactRow | null;
  published: ArtifactRow | null;
  history: HistoryEntry[];
}

export interface Publication {
  id: string;
  label: string;
  status: string;
  publishedAt: string;
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
  trace?: boolean;
}

export const endpoints = {
  products: () => apiGet<{ products: Product[] }>("/api/products"),
  decisions: (asOf?: string) =>
    apiGet<{ decisions: ArtifactListItem[] }>(
      `/api/decisions${asOfQuery(asOf)}`,
    ),
  decision: (key: string) =>
    apiGet<ArtifactDetail>(`/api/decisions/${encodeURIComponent(key)}`),
  saveDecision: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/decisions/${encodeURIComponent(key)}`, body),
  deleteDecision: (key: string) =>
    apiDelete(`/api/decisions/${encodeURIComponent(key)}`),
  expressions: (asOf?: string) =>
    apiGet<{ expressions: ArtifactListItem[] }>(
      `/api/expressions${asOfQuery(asOf)}`,
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
  matrices: (asOf?: string) =>
    apiGet<{ matrices: ArtifactListItem[] }>(
      `/api/matrices${asOfQuery(asOf)}`,
    ),
  matrix: (key: string) =>
    apiGet<ArtifactDetail>(`/api/matrices/${encodeURIComponent(key)}`),
  saveMatrix: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/matrices/${encodeURIComponent(key)}`, body),
  deleteMatrix: (key: string) =>
    apiDelete(`/api/matrices/${encodeURIComponent(key)}`),
  simulateMatrix: (spec: unknown, context: unknown) =>
    apiPost<SimulateResult>("/api/simulate/matrix", { spec, context }),
  rateSheets: (asOf?: string) =>
    apiGet<{ rateSheets: ArtifactListItem[] }>(
      `/api/rate-sheets${asOfQuery(asOf)}`,
    ),
  rateSheet: (key: string) =>
    apiGet<ArtifactDetail>(`/api/rate-sheets/${encodeURIComponent(key)}`),
  saveRateSheet: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/rate-sheets/${encodeURIComponent(key)}`, body),
  deleteRateSheet: (key: string) =>
    apiDelete(`/api/rate-sheets/${encodeURIComponent(key)}`),
  publications: () => apiGet<{ publications: Publication[] }>("/api/publications"),
  publication: (id: string) =>
    apiGet<Publication>(`/api/publications/${encodeURIComponent(id)}`),
  publish: (label: string) => apiPost<Publication>("/api/publications", { label }),
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
  evaluateEligibility: (loan: unknown, asOf?: string, trace?: boolean) =>
    apiPost<EligibilityReport>("/api/evaluate/eligibility", {
      loan,
      asOf,
      trace,
    }),
  evaluatePricing: (loan: unknown, products: string[], opts?: PricingOptions) =>
    apiPost<PricingReport>("/api/evaluate/pricing", {
      loan,
      products,
      rate: opts?.rate,
      lockTermDays: opts?.lockTermDays,
      asOf: opts?.asOf,
      trace: opts?.trace,
    }),
};
