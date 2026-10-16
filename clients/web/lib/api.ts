const API_URL = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

export async function apiGet<T>(path: string): Promise<T> {
  const res = await fetch(`${API_URL}${path}`);
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
  const res = await fetch(`${API_URL}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = (await res.json().catch(() => ({}))) as T;
  return { status: res.status, data };
}

export const apiPost = <T>(path: string, body?: unknown) =>
  apiWrite<T>("POST", path, body);
export const apiPut = <T>(path: string, body: unknown) =>
  apiWrite<T>("PUT", path, body);
export const apiDelete = (path: string) => apiWrite("DELETE", path);

export interface Product {
  code: string;
  name: string;
  loanType: string;
  amortization: string;
  termMonths: number;
  description: string;
  active: boolean;
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
      `/api/decisions${asOf ? `?asOf=${encodeURIComponent(asOf)}` : ""}`,
    ),
  decision: (key: string) =>
    apiGet<ArtifactDetail>(`/api/decisions/${encodeURIComponent(key)}`),
  saveDecision: (key: string, body: unknown) =>
    apiPut<ArtifactRow>(`/api/decisions/${encodeURIComponent(key)}`, body),
  deleteDecision: (key: string) =>
    apiDelete(`/api/decisions/${encodeURIComponent(key)}`),
  expressions: (asOf?: string) =>
    apiGet<{ expressions: ArtifactListItem[] }>(
      `/api/expressions${asOf ? `?asOf=${encodeURIComponent(asOf)}` : ""}`,
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
      `/api/matrices${asOf ? `?asOf=${encodeURIComponent(asOf)}` : ""}`,
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
      `/api/rate-sheets${asOf ? `?asOf=${encodeURIComponent(asOf)}` : ""}`,
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
