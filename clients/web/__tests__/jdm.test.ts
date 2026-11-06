import type { DecisionGraphType, DecisionTableType } from "@gorules/jdm-editor";

import { findSimpleTable, patchTableContent, stableStringify } from "@/lib/jdm";

const TABLE: DecisionTableType = {
  hitPolicy: "first",
  inputs: [{ id: "i1", name: "LTV", field: "loan.ltv" }],
  outputs: [{ id: "o1", name: "reason", field: "reasons[]" }],
  rules: [{ _id: "r1", i1: "> 97", o1: "'ltv exceeds 97%'" }],
};

function graph(tables: number): DecisionGraphType {
  const nodes = [
    { id: "in", type: "inputNode", name: "Request", position: { x: 0, y: 0 } },
    ...Array.from({ length: tables }, (_, i) => ({
      id: `tbl${i}`,
      type: "decisionTableNode",
      name: `Rules ${i}`,
      position: { x: 100 * (i + 1), y: 0 },
      content: TABLE,
    })),
    { id: "out", type: "outputNode", name: "Response", position: { x: 400, y: 0 } },
  ];
  return {
    nodes: nodes as DecisionGraphType["nodes"],
    edges: [
      { id: "e1", sourceId: "in", targetId: tables ? "tbl0" : "out" },
      ...(tables ? [{ id: "e2", sourceId: "tbl0", targetId: "out" }] : []),
    ] as DecisionGraphType["edges"],
  };
}

describe("findSimpleTable", () => {
  it("returns the single decisionTableNode", () => {
    const res = findSimpleTable(graph(1));
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.nodeId).toBe("tbl0");
      expect(res.table.hitPolicy).toBe("first");
      expect(res.table.rules).toHaveLength(1);
    }
  });

  it("reports 'none' when no table node exists", () => {
    const res = findSimpleTable(graph(0));
    expect(res).toEqual({ ok: false, reason: "none" });
  });

  it("reports 'multiple' when more than one table node exists", () => {
    const res = findSimpleTable(graph(2));
    expect(res).toEqual({ ok: false, reason: "multiple" });
  });

  it("handles a null graph", () => {
    expect(findSimpleTable(null)).toEqual({ ok: false, reason: "none" });
  });
});

describe("patchTableContent", () => {
  it("replaces only the target node's content", () => {
    const g = graph(1);
    const updated: DecisionTableType = {
      ...TABLE,
      rules: [...TABLE.rules, { _id: "r2", i1: "> 90", o1: "'ltv exceeds 90%'" }],
    };
    const next = patchTableContent(g, "tbl0", updated);

    expect(next.nodes.find((n) => n.id === "tbl0")?.content).toEqual(updated);
    // Everything else is untouched — same references for other nodes/edges.
    expect(next.nodes.find((n) => n.id === "in")).toBe(g.nodes.find((n) => n.id === "in"));
    expect(next.nodes.find((n) => n.id === "out")).toBe(g.nodes.find((n) => n.id === "out"));
    expect(next.edges).toBe(g.edges);
    // The input graph is not mutated.
    expect(g.nodes.find((n) => n.id === "tbl0")?.content).toEqual(TABLE);
  });
});

describe("stableStringify", () => {
  it("treats key order as irrelevant", () => {
    expect(stableStringify({ a: 1, b: { c: 2, d: 3 } })).toBe(
      stableStringify({ b: { d: 3, c: 2 }, a: 1 }),
    );
  });

  it("distinguishes real changes", () => {
    expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: 2 }));
    expect(stableStringify({ a: [1, 2] })).not.toBe(stableStringify({ a: [2, 1] }));
  });
});
