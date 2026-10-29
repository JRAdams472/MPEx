import { NextRequest } from "next/server";

import { getSession } from "@/lib/auth";

const API_URL = process.env.API_URL ?? "http://localhost:8080";

/**
 * Same-origin proxy for every API call the browser makes — the access token
 * never reaches the client. `/bff/api/...` → `${API_URL}/api/...`.
 */
async function proxy(
  req: NextRequest,
  ctx: { params: Promise<{ path: string[] }> },
) {
  const session = await getSession();
  if (!session) {
    return Response.json({ error: "unauthenticated" }, { status: 401 });
  }
  const { path } = await ctx.params;
  const upstream = `${API_URL}/${path.join("/")}${req.nextUrl.search}`;
  const headers = new Headers();
  headers.set("authorization", `Bearer ${session.at}`);
  const contentType = req.headers.get("content-type");
  if (contentType) headers.set("content-type", contentType);

  const res = await fetch(upstream, {
    method: req.method,
    headers,
    body: ["GET", "HEAD"].includes(req.method)
      ? undefined
      : await req.arrayBuffer(),
    // @ts-expect-error — required for streaming request bodies in undici
    duplex: "half",
  });
  return new Response(res.body, {
    status: res.status,
    headers: {
      "content-type": res.headers.get("content-type") ?? "application/json",
    },
  });
}

export {
  proxy as DELETE,
  proxy as GET,
  proxy as PATCH,
  proxy as POST,
  proxy as PUT,
};
