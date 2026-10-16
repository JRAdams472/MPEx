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
  decisions?: string[];
  expressions?: string[];
  matrices?: string[];
  rateSheets?: string[];
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
  publications: () => apiGet<{ publications: Publication[] }>("/api/publications"),
  publish: (label: string) => apiPost<Publication>("/api/publications", { label }),
  evaluateEligibility: (loan: unknown, asOf?: string) =>
    apiPost<EligibilityReport>("/api/evaluate/eligibility", {
      loan,
      asOf,
    }),
};
