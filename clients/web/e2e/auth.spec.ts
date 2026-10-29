import { expect, test } from "@playwright/test";

/**
 * The login flow itself — runs WITHOUT storageState (fresh context).
 * Drives the real auth-code+PKCE round trip through Dex and proves the
 * role-gated UI: publish for admin, hidden for viewer.
 */

async function dexLogin(page: import("@playwright/test").Page, user: string) {
  await page.goto("/");
  await expect(page).toHaveURL(/localhost:5556\/dex\/auth/);
  await page.fill('input[name="login"]', `${user}@mpex.dev`);
  await page.fill('input[name="password"]', `${user}123`);
  await page.click('button[type="submit"]');
  await expect(page.getByRole("heading", { name: "Dashboard" })).toBeVisible();
}

test("login through Dex shows identity and role in the shell", async ({
  page,
}) => {
  await dexLogin(page, "admin");
  // AppShell chip shows the resolved principal: email + roles.
  await expect(page.getByText("admin@mpex.dev · admin")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Publish" }),
  ).toBeVisible();
});

test("viewer role hides publish and mutation affordances", async ({
  page,
}) => {
  await dexLogin(page, "viewer");
  await expect(page.getByText("viewer@mpex.dev · viewer")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Publish" }),
  ).toBeHidden();

  // Edit/create affordances are pricer+ — absent for a viewer.
  await page.goto("/decisions");
  await expect(
    page.getByRole("button", { name: "New decision" }),
  ).toBeHidden();
});

test("logout drops the session and bounces to login", async ({ page }) => {
  await dexLogin(page, "viewer");
  await page.getByRole("link", { name: "Sign out" }).click();
  await expect(page).toHaveURL(/localhost:5556\/dex\/auth/);
});

test("unauthenticated page load redirects through the IdP", async ({
  page,
}) => {
  await page.goto("/decisions");
  await expect(page).toHaveURL(/localhost:5556\/dex\/auth/);
});
