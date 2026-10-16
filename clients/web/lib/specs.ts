import type { MatrixSpec, RateSheetSpec } from "./api";

export const MATRIX_OPS = ["sum", "list", "min", "max"] as const;

export interface AxisDraft {
  /** Stable React key — stripped on save (spec ids are editable). */
  uid: string;
  id: string;
  label: string;
  expr: string;
}

let uidCounter = 0;
export function nextUid(): string {
  uidCounter += 1;
  return `ax-${uidCounter}`;
}

/** Sparse cell values as typed in the editor (raw text). */
export type CellDrafts = Record<string, string>;

export function cellKey(rowId: string, colId: string) {
  return `${rowId}:${colId}`;
}

/** Spec -> editor state. Cell values serialize back to their JSON text. */
export function specToMatrixDraft(spec: MatrixSpec): {
  rows: AxisDraft[];
  cols: AxisDraft[];
  cells: CellDrafts;
  op: string;
} {
  const cells: CellDrafts = {};
  for (const [k, v] of Object.entries(spec.cells ?? {})) {
    cells[k] = JSON.stringify(v);
  }
  return {
    rows: spec.rows.map((r) => ({
      uid: nextUid(),
      id: r.id,
      label: r.label ?? "",
      expr: r.expr,
    })),
    cols: spec.cols.map((c) => ({
      uid: nextUid(),
      id: c.id,
      label: c.label ?? "",
      expr: c.expr,
    })),
    cells,
    op: spec.op ?? "sum",
  };
}

function coerceScalar(raw: string): unknown {
  const t = raw.trim();
  if (t === "") return undefined;
  try {
    const v: unknown = JSON.parse(t);
    if (
      typeof v === "number" ||
      typeof v === "boolean" ||
      typeof v === "string"
    ) {
      return v;
    }
    return undefined;
  } catch {
    // Not JSON — treat as a bare string (e.g. "discount").
    return t;
  }
}

/** Editor state -> spec. Blank cells are dropped; values coerce to scalars. */
export function matrixDraftToSpec(
  rows: AxisDraft[],
  cols: AxisDraft[],
  cells: CellDrafts,
  op: string,
): MatrixSpec {
  const out: Record<string, unknown> = {};
  for (const [k, raw] of Object.entries(cells)) {
    const v = coerceScalar(raw);
    if (v !== undefined) out[k] = v;
  }
  return {
    rows: rows.map((r) => ({
      id: r.id.trim(),
      ...(r.label.trim() ? { label: r.label.trim() } : {}),
      expr: r.expr.trim(),
    })),
    cols: cols.map((c) => ({
      id: c.id.trim(),
      ...(c.label.trim() ? { label: c.label.trim() } : {}),
      expr: c.expr.trim(),
    })),
    cells: out,
    op,
  };
}

/**
 * Overlap warnings: axis expressions that literally duplicate another axis on
 * the same side are guaranteed to co-fire, so every cell on them overlaps.
 */
export function matrixOverlapWarnings(
  rows: AxisDraft[],
  cols: AxisDraft[],
): string[] {
  const warnings: string[] = [];
  for (const [axes, name] of [
    [rows, "row"],
    [cols, "column"],
  ] as const) {
    const byExpr = new Map<string, string[]>();
    for (const a of axes) {
      const e = a.expr.trim();
      if (!e) continue;
      byExpr.set(e, [...(byExpr.get(e) ?? []), a.id]);
    }
    for (const [expr, ids] of byExpr) {
      if (ids.length > 1) {
        warnings.push(
          `${name}s ${ids.join(", ")} share the same expression (${expr}) — every cell they touch overlaps`,
        );
      }
    }
  }
  return warnings;
}

export interface GridValue {
  /** Stable React key — the text itself is user-editable. */
  uid: string;
  text: string;
}

export interface RateSheetDraft {
  /** Editor text; parsed to numbers on save. */
  rates: GridValue[];
  lockTerms: GridValue[];
  prices: Record<string, string>;
}

export function specToRateSheetDraft(spec: RateSheetSpec): RateSheetDraft {
  const rates = spec.rates.map((r) => ({ uid: nextUid(), text: String(r) }));
  const lockTerms = spec.lockTerms.map((t) => ({
    uid: nextUid(),
    text: String(t),
  }));
  const prices: Record<string, string> = {};
  for (const [k, v] of Object.entries(spec.prices ?? {})) {
    prices[k] = String(v);
  }
  return { rates, lockTerms, prices };
}

/** Editor state -> spec. Returns a spec or throws a user-facing error list. */
export function rateSheetDraftToSpec(draft: RateSheetDraft): {
  spec?: RateSheetSpec;
  errors: string[];
} {
  const errors: string[] = [];
  const rates: number[] = [];
  draft.rates.forEach((r, i) => {
    const n = Number(r.text);
    if (!Number.isFinite(n) || n <= 0)
      errors.push(`rate ${i + 1} ("${r.text}") is not a positive number`);
    else rates.push(n);
  });
  const lockTerms: number[] = [];
  draft.lockTerms.forEach((t, i) => {
    const n = Number(t.text);
    if (!Number.isInteger(n) || n <= 0)
      errors.push(`lock term ${i + 1} ("${t.text}") is not a positive integer`);
    else lockTerms.push(n);
  });
  const prices: Record<string, number> = {};
  for (const [k, raw] of Object.entries(draft.prices)) {
    if (raw.trim() === "") continue;
    const n = Number(raw);
    if (!Number.isFinite(n)) {
      errors.push(`price ${k} ("${raw}") is not a number`);
      continue;
    }
    prices[k] = n;
  }
  if (errors.length > 0) return { errors };
  return { spec: { rates, lockTerms, prices }, errors: [] };
}
