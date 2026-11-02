import { chromium } from "@playwright/test";
const browser = await chromium.launch();
const ctx = await browser.newContext({ storageState: "e2e/.auth/admin.json" });
await ctx.request.put("http://localhost:3000/bff/api/expressions/expr.shot.demo", {
  data: { kind: "standard", expression: "true" },
});
await ctx.request.put("http://localhost:3000/bff/api/overrides/ovr.shot.demo", {
  data: { targetKind: "expression", targetKey: "expr.shot.demo",
          valuePath: "/expression", value: "false",
          description: "e2e proof shot" },
});
const page = await ctx.newPage();
await page.goto("http://localhost:3000/overrides", { waitUntil: "networkidle" });
await page.screenshot({ path: "/tmp/overrides-list.png", fullPage: true });
await page.getByRole("button", { name: "New override" }).click();
await page.screenshot({ path: "/tmp/overrides-dialog.png" });
await page.keyboard.press("Escape");
await page.goto("http://localhost:3000/overrides/ovr.shot.demo", { waitUntil: "networkidle" });
await page.screenshot({ path: "/tmp/overrides-detail.png", fullPage: true });
await ctx.request.delete("http://localhost:3000/bff/api/overrides/ovr.shot.demo");
await ctx.request.delete("http://localhost:3000/bff/api/expressions/expr.shot.demo");
await browser.close();
console.log("done");
