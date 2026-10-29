import { NextRequest, NextResponse } from "next/server";

import { beginLogin } from "@/lib/auth";

/** Start auth-code+PKCE: stash verifier/state, bounce to the IdP. */
export async function GET(req: NextRequest) {
  const next = req.nextUrl.searchParams.get("next") ?? "/";
  const safeNext = next.startsWith("/") && !next.startsWith("//") ? next : "/";
  return NextResponse.redirect(await beginLogin(safeNext));
}
