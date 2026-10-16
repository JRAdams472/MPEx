import { test, expect } from "@playwright/test";

// Full sandbox flow against the seeded dev catalog: edit the loan scenario,
// run eligibility, price the eligible products, and inspect the waterfall.
test("sandbox: scenario → eligibility → pricing waterfall", async ({
  page,
}) => {
  await page.goto("/sandbox");

  // Sample scenario is conventional/purchase/FICO 760 — eligible products
  // should appear as selectable chips after the run.
  await page.getByRole("button", { name: /run eligibility/i }).click();
  await expect(page.getByText(/Eligibility — revision/).first()).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByRole("button", { name: /price \d+ product/i })).toBeVisible();

  await page.getByRole("button", { name: /price \d+ product/i }).click();
  await expect(page.getByText(/Pricing — revision/).first()).toBeVisible({
    timeout: 30_000,
  });

  // Waterfall: base price → adjustments → final price.
  await expect(page.getByText(/Base price/).first()).toBeVisible();
  await expect(page.getByText(/Final price/).first()).toBeVisible();
});

test("sandbox: deterministic repeat — same request twice, same prices", async ({
  page,
}) => {
  await page.goto("/sandbox");
  await page.getByRole("button", { name: /run eligibility/i }).click();
  const priceBtn = page.getByRole("button", { name: /price \d+ product/i });
  await expect(priceBtn).toBeVisible({ timeout: 30_000 });

  await priceBtn.click();
  await expect(page.getByText(/Final price/).first()).toBeVisible({
    timeout: 30_000,
  });
  const first = await page
    .getByText(/Final price/)
    .first()
    .locator("..")
    .textContent();

  await priceBtn.click();
  await expect(page.getByText(/Final price/).first()).toBeVisible();
  const second = await page
    .getByText(/Final price/)
    .first()
    .locator("..")
    .textContent();
  expect(second).toBe(first);
});
