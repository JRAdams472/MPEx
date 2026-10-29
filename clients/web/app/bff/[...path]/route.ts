import { NextRequest } from "next/server";

import { getSession } from "@/lib/auth";

const API_URL = process.env.API_URL ?? "http://localhost:8080";
// Mirror the API's body ceiling — the BFF buffers bodies before proxying,
// so an oversized upload must be rejected before `arrayBuffer()` runs.
const MAX_BODY = 2 * 1024 * 1024;
const TOO_LARGE = Symbol("too-large");

/** Read the request body up to MAX_BODY; TOO_LARGE sentinel past it. */
async function boundedBody(req: NextRequest): Promise<ArrayBuffer | typeof TOO_LARGE> {
  const reader = req.body?.getReader();
  if (!reader) return new ArrayBuffer(0);
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    total += value.byteLength;
    if (total > MAX_BODY) {
      await reader.cancel();
      return TOO_LARGE;
    }
    chunks.push(value);
  }
  const out = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.byteLength;
  }
  return out.buffer;
}

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
  const declared = Number(req.headers.get("content-length") ?? 0);
  if (declared > MAX_BODY) {
    return Response.json({ error: "body too large" }, { status: 413 });
  }
  const { path } = await ctx.params;
  // content-length can be absent/lie under chunked transfer — bound the
  // actual read too.
  const body = ["GET", "HEAD"].includes(req.method)
    ? undefined
    : await boundedBody(req);
  if (body === TOO_LARGE) {
    return Response.json({ error: "body too large" }, { status: 413 });
  }
  const upstream = `${API_URL}/${path.join("/")}${req.nextUrl.search}`;
  const headers = new Headers();
  headers.set("authorization", `Bearer ${session.at}`);
  const contentType = req.headers.get("content-type");
  if (contentType) headers.set("content-type", contentType);

  const res = await fetch(upstream, {
    method: req.method,
    headers,
    body,
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
