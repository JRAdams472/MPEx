import { NextRequest, NextResponse } from "next/server";

/**
 * Bounce unauthenticated page loads to the login flow. Presence-only check —
 * the JWE is verified server-side in the BFF/auth routes (a stale cookie
 * fails there and the client bounces back to login on the 401).
 */
export function proxy(req: NextRequest) {
  if (req.cookies.has("mpex_session")) {
    return NextResponse.next();
  }
  const url = req.nextUrl.clone();
  url.pathname = "/auth/login";
  url.search = `?next=${encodeURIComponent(req.nextUrl.pathname)}`;
  return NextResponse.redirect(url);
}

export const config = {
  matcher: ["/((?!auth|bff|_next/static|_next/image|favicon.ico).*)"],
};
