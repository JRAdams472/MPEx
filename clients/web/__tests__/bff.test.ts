/**
 * @jest-environment node
 */
import { NextRequest } from "next/server";

jest.mock("server-only", () => ({}));

const getSession = jest.fn();
jest.mock("@/lib/auth", () => ({
  getSession: (...args: unknown[]) => getSession(...args),
}));

import { POST } from "@/app/bff/[...path]/route";

const ctx = { params: Promise.resolve({ path: ["api", "products"] }) };
const MB = 1024 * 1024;

function req(body?: BodyInit, headers?: Record<string, string>) {
  return new NextRequest("http://localhost:3000/bff/api/products", {
    method: "POST",
    headers,
    body: body ?? null,
    // @ts-expect-error — required for streaming request bodies
    duplex: "half",
  });
}

describe("BFF proxy", () => {
  beforeEach(() => {
    getSession.mockReset();
    jest.restoreAllMocks();
  });

  it("401s without a session", async () => {
    getSession.mockResolvedValue(null);
    const res = await POST(req(), ctx);
    expect(res.status).toBe(401);
  });

  it("413s on a declared content-length over the cap", async () => {
    getSession.mockResolvedValue({ at: "tok", exp: Date.now() + 60_000 });
    const res = await POST(req(undefined, { "content-length": String(3 * MB) }), ctx);
    expect(res.status).toBe(413);
  });

  it("413s when a chunked body exceeds the cap mid-stream", async () => {
    getSession.mockResolvedValue({ at: "tok", exp: Date.now() + 60_000 });
    const stream = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(new Uint8Array(3 * MB));
        c.close();
      },
    });
    const res = await POST(req(stream), ctx);
    expect(res.status).toBe(413);
  });

  it("proxies with the session bearer token", async () => {
    getSession.mockResolvedValue({ at: "tok", exp: Date.now() + 60_000 });
    const upstream = jest
      .spyOn(global, "fetch")
      .mockResolvedValue(new Response("{}", { status: 200 }));
    const res = await POST(req('{"a":1}'), ctx);
    expect(res.status).toBe(200);
    const [url, init] = upstream.mock.calls[0];
    expect(url).toBe("http://localhost:8080/api/products");
    expect(new Headers(init?.headers).get("authorization")).toBe("Bearer tok");
  });
});
