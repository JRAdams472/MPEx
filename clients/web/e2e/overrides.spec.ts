import { expect, test } from "@playwright/test";

/** Overrides admin UI (MPE-126): a pricer-authored draft override patches a
 *  scalar inside a target expression. The spec seeds its own target via the
 *  BFF (admin session), drives the list → dialog → detail flow, and cleans
 *  up so reruns stay order-independent. */

const EXPR = "expr.e2e.ovr";
const OVR = "ovr.e2e.flip";

async function api(
  ctx: import("@playwright/test").BrowserContext,
  method: "put" | "delete",
  path: string,
  body?: unknown,
) {
  const res = await ctx.request[method](`http://localhost:3000${path}`, {
    data: body,
  });
  return res;
}

test("overrides: create draft, see it in list + detail, delete restores", async ({
  page,
  browser,
}) => {
  const admin = await browser.newContext({
    storageState: "e2e/.auth/admin.json",
  });
  try {
    // Seed a fresh target expression — the override's PUT validation needs
    // the target to resolve in the working set.
    const seed = await api(admin, "put", `/bff/api/expressions/${EXPR}`, {
      kind: "standard",
      expression: "true",
    });
    expect(seed.status()).toBe(200);
    // Clear any stale draft from a previous run.
    await api(admin, "delete", `/bff/api/overrides/${OVR}`);

    // List page → New override dialog → all fields → Create draft.
    await page.goto("/overrides");
    await expect(
      page.getByRole("heading", { name: "Overrides" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "New override" }).click();

    const dialog = page.getByRole("dialog");
    await dialog.getByRole("textbox", { name: "Key" }).fill(OVR);
    await dialog.getByLabel("Target kind").click();
    await page.getByRole("option", { name: "expression" }).click();
    await dialog.getByRole("combobox", { name: "Target key" }).fill(EXPR);
    await dialog.getByLabel("JSON Pointer").fill("/expression");
    await dialog.getByLabel(/^Value/).fill('"false"');
    await dialog
      .getByRole("button", { name: "Create draft" })
      .click();

    // Lands on the detail page showing the draft.
    await expect(page).toHaveURL(new RegExp(`/overrides/${OVR}`));
    await expect(page.getByText("draft", { exact: true }).first()).toBeVisible();
    await expect(page.getByLabel("Target key")).toHaveValue(EXPR);
    await expect(page.getByLabel("JSON Pointer")).toHaveValue("/expression");

    // Back on the list, the draft row links the target.
    await page.goto("/overrides");
    const row = page.getByRole("row", { name: new RegExp(OVR) });
    await expect(row).toBeVisible();
    await expect(row.getByText(EXPR)).toBeVisible();

    // Validation surfaces verbatim: a pointer to a missing leaf is a 400.
    await row.getByRole("link", { name: OVR }).click();
    await page.getByLabel("JSON Pointer").fill("/nonexistent");
    await page.getByRole("button", { name: "Save draft" }).click();
    await expect(
      page.getByText(/does not resolve in the target document/),
    ).toBeVisible({ timeout: 10_000 });

    // Restore the path, save cleanly, then delete the draft.
    await page.getByLabel("JSON Pointer").fill("/expression");
    await page.getByRole("button", { name: "Save draft" }).click();
    await page.getByRole("button", { name: "Delete draft" }).click();
    await expect(page.getByRole("row", { name: new RegExp(OVR) })).toHaveCount(
      0,
      { timeout: 10_000 },
    );
  } finally {
    await api(admin, "delete", `/bff/api/overrides/${OVR}`);
    await api(admin, "delete", `/bff/api/expressions/${EXPR}`);
    await admin.close();
  }
});
