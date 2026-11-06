import { expect, test } from "@playwright/test";

/** Simple (business) rule-editing mode (MPE-193): a decision whose graph is
 *  exactly one decisionTableNode gets a table-only editor; the Simple |
 *  Advanced toggle swaps views, table edits patch back into the same draft
 *  save path, and the rest of the graph is preserved byte-for-byte. The spec
 *  seeds its own decision via the BFF and cleans up so reruns stay
 *  order-independent. */

const KEY = "test.e2e.simple";
const TABLE = {
  hitPolicy: "first",
  inputs: [{ id: "i1", name: "LTV", field: "loan.ltv" }],
  outputs: [{ id: "o1", name: "reason", field: "reasons[]" }],
  rules: [{ _id: "r1", i1: "> 97", o1: "'ltv exceeds 97%'" }],
};
const JDM = {
  nodes: [
    { id: "in", type: "inputNode", name: "Request", position: { x: 0, y: 0 } },
    { id: "tbl", type: "decisionTableNode", name: "Rules", position: { x: 200, y: 0 }, content: TABLE },
    { id: "out", type: "outputNode", name: "Response", position: { x: 400, y: 0 } },
  ],
  edges: [
    { id: "e1", sourceId: "in", targetId: "tbl" },
    { id: "e2", sourceId: "tbl", targetId: "out" },
  ],
};

async function api(
  ctx: import("@playwright/test").BrowserContext,
  method: "get" | "put" | "delete",
  path: string,
  body?: unknown,
) {
  // Every e2e request shares one per-IP quota on the API's edge limits; a
  // 429 under parallel workers is contention, not a failure — retry twice
  // with a short backoff before surfacing the status.
  for (let attempt = 0; ; attempt++) {
    const res = await ctx.request[method](`http://localhost:3000${path}`, {
      data: body,
    });
    if (res.status() !== 429 || attempt === 2) return res;
    await new Promise((r) => setTimeout(r, 2500));
  }
}

test("simple mode: table view, advanced toggle, save preserves graph", async ({
  page,
  browser,
}) => {
  const admin = await browser.newContext({
    storageState: "e2e/.auth/admin.json",
  });
  try {
    // Fresh draft with a single-table graph (PUT fully replaces any stale
    // draft from a previous run).
    const seed = await api(admin, "put", `/bff/api/decisions/${KEY}`, {
      kind: "shared",
      productCode: null,
      jdm: JDM,
    });
    expect(seed.status()).toBe(200);

    await page.goto(`/decisions/${encodeURIComponent(KEY)}`);

    // Simple is the default view — the standalone decision table renders
    // the column names from the seed.
    await expect(
      page.getByRole("button", { name: "Simple" }),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator(".grl-dt")).toBeVisible();
    await expect(page.locator(".grl-dt")).toContainText("LTV");
    await expect(page.locator(".grl-dt")).toContainText("reason");
    // Business mode decomposes "> 97" into an operator picker + a numeric
    // input — the guided cell UX this mode exists for.
    await expect(page.getByRole("spinbutton")).toHaveValue("97");
    await expect(page.locator(".grl-dt")).toContainText("ltv exceeds 97%");

    // Advanced shows the full node graph.
    await page.getByRole("button", { name: "Advanced" }).click();
    await expect(page.locator(".grl-dg")).toBeVisible();

    // Back to Simple — add a rule row via the table's "Add row" control,
    // which marks the draft dirty and enables Save.
    await page.getByRole("button", { name: "Simple" }).click();
    await expect(
      page.getByRole("button", { name: "Save draft" }),
    ).toBeDisabled();
    await page.getByRole("button", { name: "Add row" }).click();
    await expect(
      page.getByRole("button", { name: "Save draft" }),
    ).toBeEnabled();
    await page.getByRole("button", { name: "Save draft" }).click();
    await expect(
      page.getByRole("button", { name: "Save draft" }),
    ).toBeDisabled();

    // The persisted draft gained exactly one rule; nodes/edges otherwise
    // identical to the seed.
    const got = await api(admin, "get", `/bff/api/decisions/${encodeURIComponent(KEY)}`);
    const body = (await got.json()) as {
      draft: { jdm: { nodes: { id: string; type: string; content?: typeof TABLE }[]; edges: unknown[] } };
    };
    const tbl = body.draft.jdm.nodes.find((n) => n.id === "tbl");
    expect(tbl?.content?.rules.length).toBe(TABLE.rules.length + 1);
    expect(body.draft.jdm.nodes.map((n) => n.id)).toEqual(["in", "tbl", "out"]);
    expect(body.draft.jdm.edges).toEqual(JDM.edges);
  } finally {
    await api(admin, "delete", `/bff/api/decisions/${KEY}`);
    await admin.close();
  }
});
