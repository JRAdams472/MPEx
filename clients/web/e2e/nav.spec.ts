import { test, expect } from "@playwright/test";

test("dashboard renders and every nav surface loads", async ({ page }) => {
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Dashboard" }),
  ).toBeVisible();

  const surfaces: Array<[string, RegExp | string]> = [
    ["Products", /products/i],
    ["Decisions", /decisions/i],
    ["Expressions", /expressions/i],
    ["Matrices", /matrices/i],
    ["Rate sheets", /rate.?sheets/i],
    ["Publications", /publications/i],
    ["Sandbox", /sandbox|pricer/i],
  ];
  for (const [label, heading] of surfaces) {
    await page.getByRole("link", { name: label, exact: true }).click();
    await expect(
      page.getByRole("heading", { name: heading }).first(),
    ).toBeVisible();
  }
});

test("products list renders the seeded catalog", async ({ page }) => {
  await page.goto("/products");
  await expect(page.getByText("CONV30").first()).toBeVisible();
});

test("decisions list → detail loads the JDM editor", async ({ page }) => {
  await page.goto("/decisions");
  await page.getByRole("link", { name: /eligibility\.CONV30/ }).click();
  // jdm-editor renders a React Flow canvas.
  await expect(page.locator(".react-flow").first()).toBeVisible({
    timeout: 30_000,
  });
});

test("publications page lists publications with artifact counts", async ({
  page,
}) => {
  await page.goto("/publications");
  await expect(
    page.getByRole("columnheader", { name: /matrices/i }),
  ).toBeVisible();
  await expect(page.getByText(/LLPA|publish|v\d/i).first()).toBeVisible();
});
