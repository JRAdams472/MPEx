import { NextRequest, NextResponse } from "next/server";

import { finishLogin } from "@/lib/auth";

/** IdP redirect target: verify state, exchange code, seal the session. */
export async function GET(req: NextRequest) {
  const code = req.nextUrl.searchParams.get("code");
  const state = req.nextUrl.searchParams.get("state");
  if (!code) {
    return NextResponse.redirect(new URL("/auth/login", req.nextUrl.origin));
  }
  try {
    const next = await finishLogin(code, state);
    // Re-validate the cookie-carried target: the state cookie isn't
    // integrity-protected, so a tampered `//host` must not redirect off-site.
    const safe = next.startsWith("/") && !next.startsWith("//") ? next : "/";
    return NextResponse.redirect(new URL(safe, req.nextUrl.origin));
  } catch (e) {
    return NextResponse.redirect(
      new URL(`/auth/login?error=${encodeURIComponent(`${e}`)}`, req.nextUrl.origin),
    );
  }
}
