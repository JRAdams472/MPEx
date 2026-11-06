import type { DecisionGraphType, DecisionTableType } from "@gorules/jdm-editor";

/** "Simple" editing owns a decision that is exactly one decision table —
 *  the surrounding graph (input/output wiring, verdict expressions) is
 *  boilerplate the simple editor must preserve, not expose. */
export type SimpleTable =
  | { ok: true; nodeId: string; table: DecisionTableType }
  | { ok: false; reason: "none" | "multiple" };

export function findSimpleTable(graph: DecisionGraphType | null): SimpleTable {
  const tables = (graph?.nodes ?? []).filter((n) => n.type === "decisionTableNode");
  if (tables.length === 0) return { ok: false, reason: "none" };
  if (tables.length > 1) return { ok: false, reason: "multiple" };
  return { ok: true, nodeId: tables[0].id, table: tables[0].content as DecisionTableType };
}

/** New graph with one node's content replaced; other nodes/edges shared. */
export function patchTableContent(
  graph: DecisionGraphType,
  nodeId: string,
  content: DecisionTableType,
): DecisionGraphType {
  return {
    ...graph,
    nodes: (graph.nodes ?? []).map((n) =>
      n.id === nodeId ? { ...n, content } : n,
    ),
  };
}

/** Key-order-insensitive stringify — the JDM editor emits normalized
 *  documents, so identity checks for "did the user change anything" need
 *  stable ordering rather than JSON.stringify. */
export function stableStringify(value: unknown): string {
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(",")}]`;
  const entries = Object.entries(value as Record<string, unknown>)
    .filter(([, v]) => v !== undefined)
    .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([k, v]) => `${JSON.stringify(k)}:${stableStringify(v)}`);
  return `{${entries.join(",")}}`;
}
