import { test, expect } from "@playwright/test";

test("admin sees Load today's rates and the preview dialog opens", async ({
  page,
}) => {
  await page.goto("/rate-sheets");
  await expect(page.getByRole("heading", { name: "Rate sheets" })).toBeVisible();

  const load = page.getByRole("button", { name: /load today['’]s rates/i });
  await expect(load).toBeVisible();
  await load.click();

  const dialog = page.getByRole("dialog", { name: /load today['’]s rates/i });
  await expect(dialog).toBeVisible();
  // The preview either resolves (real index reachable) or surfaces the
  // upstream error — never a blank dialog.
  await expect(
    dialog
      .getByRole("button", { name: /apply & publish/i })
      .or(dialog.getByRole("alert")),
  ).toBeVisible({ timeout: 20_000 });
});

test("rate-sheets shows last-import provenance", async ({ page }) => {
  // MPE-149 — the panel renders whatever the latest rate_index_imports row
  // holds (CI seeds none → "no imports yet" line; either is valid proof).
  await page.goto("/rate-sheets");
  await expect(page.getByText("Last import")).toBeVisible();
  await expect(
    page
      .getByText(/by .+/)
      .or(page.getByText(/No rate-index imports yet/)),
  ).toBeVisible();
});

test("products list and detail show rate source", async ({ page }) => {
  await page.goto("/products");
  await expect(
    page.getByRole("columnheader", { name: "Rate source" }),
  ).toBeVisible();
  await expect(page.getByText("index — 30 Yr. Fixed")).toBeVisible();

  await page.getByRole("link", { name: "CONV30" }).click();
  await expect(
    page.getByRole("heading", { name: "Rate source" }),
  ).toBeVisible();
});
