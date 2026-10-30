import { expect, test } from "@playwright/test";

/** Grants admin UI (MPE-159): an admin restricts viewer@mpex.dev to the prod
 *  lane through the grants editor, then the viewer logs in and gets a clear
 *  lane-denial on dev eval. The grant is restored in `finally` so other specs
 *  (auth.spec's viewer test) stay order-independent. */

async function dexLogin(page: import("@playwright/test").Page, user: string) {
  await page.goto("/");
  await expect(page).toHaveURL(/localhost:5556\/dex\/auth/);
  await page.fill('input[name="login"]', `${user}@mpex.dev`);
  await page.fill('input[name="password"]', `${user}123`);
  await page.click('button[type="submit"]');
  await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible();
}

test("grants: restricting viewer to prod blocks dev eval with a clear message", async ({
  page,
  browser,
}) => {
  try {
    // 1. Admin edits viewer@mpex.dev's grant: env switch OFF, prod only.
    await page.goto("/grants");
    const row = page.getByRole("row", { name: /viewer@mpex\.dev/ });
    await expect(row).toBeVisible({ timeout: 30_000 });
    await row.getByRole("button", { name: "Edit" }).click();

    const dialog = page.getByRole("dialog");
    await dialog
      .getByLabel("Unrestricted — all lanes")
      .click(); // switch OFF
    await dialog.getByRole("checkbox", { name: "prod" }).check();
    await dialog.getByRole("button", { name: "Save" }).click();

    // Row now shows a prod env chip instead of "all lanes".
    await expect(
      row.getByText("prod", { exact: true }),
    ).toBeVisible({ timeout: 10_000 });

    // 2. Log in as the restricted viewer — dev eval is denied with the
    //    API's lane message, not a bare HTTP 403.
    await page.context().clearCookies();
    await dexLogin(page, "viewer");
    await page.goto("/sandbox");
    // The selector hint shows the restricted lanes.
    await expect(page.getByText(/your grants permit: prod/)).toBeVisible({
      timeout: 30_000,
    });
    await page.getByRole("button", { name: /run eligibility/i }).click();
    await expect(
      page.getByText(/outside your permitted lanes/),
    ).toBeVisible({ timeout: 30_000 });
  } finally {
    // 3. Restore the grant to unrestricted (envs omitted → NULL) via the BFF
    //    as admin — leaves auth.spec's viewer test order-independent.
    const ctx = await browser.newContext({
      storageState: "e2e/.auth/admin.json",
    });
    const res = await ctx.request.put(
      "http://localhost:3000/bff/api/admin/grants",
      {
        data: {
          subjectType: "email",
          subject: "viewer@mpex.dev",
          role: "viewer",
        },
      },
    );
    expect(res.ok()).toBeTruthy();
    await ctx.close();
  }
});
