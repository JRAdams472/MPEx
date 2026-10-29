import { NextRequest, NextResponse } from "next/server";

import { clearSession } from "@/lib/auth";

/** Drop the session cookie and bounce back through the login flow. */
export async function GET(req: NextRequest) {
  await clearSession();
  return NextResponse.redirect(new URL("/auth/login", req.nextUrl.origin));
}
