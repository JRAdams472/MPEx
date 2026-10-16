import {
  cellKey,
  matrixDraftToSpec,
  matrixOverlapWarnings,
  rateSheetDraftToSpec,
  specToMatrixDraft,
  specToRateSheetDraft,
  type AxisDraft,
} from "@/lib/specs";

const ax = (id: string, expr: string, label = ""): AxisDraft => ({
  uid: `u-${id}`,
  id,
  label,
  expr,
});

describe("matrix spec round-trip", () => {
  it("coerces cell text to scalars and drops blanks", () => {
    const spec = matrixDraftToSpec(
      [ax("r1", "borrower.fico >= 740")],
      [ax("c1", "loan.ltv <= 80"), ax("c2", "loan.ltv > 80")],
      { "r1:c1": "-0.25", "r1:c2": "  ", "r1:c3": "ignored" },
      "sum",
    );
    expect(spec.cells["r1:c1"]).toBe(-0.25);
    expect(spec.cells).not.toHaveProperty("r1:c2");
    // Unknown col ids pass through — the server validates them.
    expect(spec.cells["r1:c3"]).toBe("ignored");
    expect(spec.op).toBe("sum");
  });

  it("round-trips spec -> draft -> spec preserving scalars", () => {
    const spec = specToMatrixDraft({
      rows: [{ id: "r1", label: "Prime", expr: "borrower.fico >= 740" }],
      cols: [{ id: "c1", expr: "loan.ltv <= 80" }],
      cells: { "r1:c1": 0.5 },
      op: "max",
    });
    const out = matrixDraftToSpec(spec.rows, spec.cols, spec.cells, spec.op);
    expect(out.cells["r1:c1"]).toBe(0.5);
    expect(out.rows[0]).toEqual({ id: "r1", label: "Prime", expr: "borrower.fico >= 740" });
    expect(out.cols[0]).toEqual({ id: "c1", expr: "loan.ltv <= 80" });
    expect(out.op).toBe("max");
  });

  it("omits blank axis labels", () => {
    const spec = matrixDraftToSpec([ax("r1", "true")], [ax("c1", "true")], {}, "sum");
    expect(spec.rows[0]).toEqual({ id: "r1", expr: "true" });
  });

  it("keeps bare non-JSON text as a string scalar", () => {
    const spec = matrixDraftToSpec([ax("r1", "true")], [ax("c1", "true")], {
      "r1:c1": "discount",
    }, "sum");
    expect(spec.cells["r1:c1"]).toBe("discount");
  });
});

describe("matrix overlap warnings", () => {
  it("flags duplicate expressions on the same axis", () => {
    const warnings = matrixOverlapWarnings(
      [ax("r1", "loan.ltv <= 80"), ax("r2", "loan.ltv <= 80")],
      [ax("c1", "borrower.fico >= 700")],
    );
    expect(warnings).toHaveLength(1);
    expect(warnings[0]).toContain("rows r1, r2");
  });

  it("is quiet for distinct expressions", () => {
    expect(
      matrixOverlapWarnings(
        [ax("r1", "loan.ltv <= 80")],
        [ax("c1", "borrower.fico >= 700")],
      ),
    ).toEqual([]);
  });
});

describe("rate sheet draft -> spec", () => {
  it("collects per-field errors without emitting a spec", () => {
    const { spec, errors } = rateSheetDraftToSpec({
      rates: [{ uid: "a", text: "abc" }],
      lockTerms: [{ uid: "b", text: "30" }],
      prices: { "abc:30": "x" },
    });
    expect(spec).toBeUndefined();
    expect(errors.join(" ")).toContain("rate 1");
    expect(errors.join(" ")).toContain("price abc:30");
  });

  it("converts a valid draft", () => {
    const { spec, errors } = rateSheetDraftToSpec({
      rates: [{ uid: "a", text: "6.5" }],
      lockTerms: [
        { uid: "b", text: "30" },
        { uid: "c", text: "45" },
      ],
      prices: { "6.5:30": "99.875", "6.5:45": "99.75" },
    });
    expect(errors).toEqual([]);
    expect(spec).toEqual({
      rates: [6.5],
      lockTerms: [30, 45],
      prices: { "6.5:30": 99.875, "6.5:45": 99.75 },
    });
  });

  it("round-trips spec -> draft -> spec", () => {
    const draft = specToRateSheetDraft({
      rates: [6.25, 6.5],
      lockTerms: [30, 45],
      prices: { "6.25:30": 100.125 },
    });
    expect(draft.prices["6.25:30"]).toBe("100.125");
    const { spec } = rateSheetDraftToSpec(draft);
    expect(spec?.prices["6.25:30"]).toBe(100.125);
  });
});

describe("cellKey", () => {
  it("joins row and col ids", () => {
    expect(cellKey("r1", "c2")).toBe("r1:c2");
  });
});
