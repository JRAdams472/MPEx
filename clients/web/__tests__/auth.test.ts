/**
 * @jest-environment node
 */

jest.mock("server-only", () => ({}));

// jose is ESM-only and next/jest's transformIgnorePatterns can't be
// subtracted from — fake the seal/decrypt surface instead. What's under
// test is the refresh dedup/revocation/maxAge logic, not the crypto
// (the real JWE path is exercised by the Playwright login specs).
jest.mock("jose", () => {
  const b64 = (o: unknown) =>
    Buffer.from(JSON.stringify(o)).toString("base64url");
  const unb64 = (s: string) =>
    JSON.parse(Buffer.from(s, "base64url").toString());
  return {
    decodeJwt: (t: string) => {
      const parts = t.split(".");
      if (parts.length !== 3) throw new Error("Invalid JWT");
      return unb64(parts[1]);
    },
    EncryptJWT: class {
      private p: Record<string, unknown>;
      constructor(payload: Record<string, unknown>) {
        this.p = { ...payload };
      }
      setProtectedHeader() {
        return this;
      }
      setIssuedAt() {
        this.p.iat = Math.floor(Date.now() / 1000);
        return this;
      }
      async encrypt() {
        return `mpex.${b64(this.p)}`;
      }
    },
    jwtDecrypt: async (raw: string) => {
      if (!raw.startsWith("mpex.")) throw new Error("decryption failed");
      return { payload: unb64(raw.slice(5)) };
    },
  };
});

const mockCookieStore = {
  get: jest.fn(),
  set: jest.fn(),
  delete: jest.fn(),
};
jest.mock("next/headers", () => ({
  cookies: jest.fn(async () => mockCookieStore),
}));

import { EncryptJWT } from "jose";

import type { SessionData } from "@/lib/auth";

process.env.SESSION_SECRET = "test-secret-0123456789abcdef0123456789abcdef";

/** Mint a sealed session cookie the way setSession() does. */
async function seal(s: Partial<SessionData>): Promise<string> {
  return new EncryptJWT(s as Record<string, unknown>)
    .setProtectedHeader({ alg: "dir", enc: "A256GCM" })
    .setIssuedAt()
    .encrypt(new Uint8Array(32));
}

function session(over: Partial<SessionData> = {}): Partial<SessionData> {
  return {
    at: "old-at",
    rt: "rt-1",
    atExp: Date.now() + 10_000, // inside the 30s refresh margin
    email: "a@mpex.dev",
    ...over,
  };
}

function tokenResponse(over: Record<string, unknown> = {}) {
  return Response.json(
    {
      access_token: "new-at",
      refresh_token: "rt-2",
      expires_in: 3600,
      ...over,
    },
    { status: 200 },
  );
}

function deferred() {
  let resolve!: (v: Response) => void;
  const p = new Promise<Response>((res) => (resolve = res));
  return { p, resolve };
}

async function load() {
  return await import("@/lib/auth");
}

function stubCookie(sealed: string) {
  mockCookieStore.get.mockImplementation((name: string) =>
    name === "mpex_session" ? { name, value: sealed } : undefined,
  );
}

describe("getSession refresh hardening (MPE-122)", () => {
  beforeEach(() => {
    jest.resetModules();
    mockCookieStore.get.mockReset();
    mockCookieStore.set.mockReset();
    mockCookieStore.delete.mockReset();
  });

  it("returns a fresh session untouched", async () => {
    const { getSession } = await load();
    stubCookie(await seal(session({ atExp: Date.now() + 3_600_000 })));
    const fetchSpy = jest.spyOn(global, "fetch");

    const s = await getSession();
    expect(s?.at).toBe("old-at");
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(mockCookieStore.set).not.toHaveBeenCalled();
  });

  it("refreshes once for concurrent requests sharing the cookie", async () => {
    const { getSession } = await load();
    stubCookie(await seal(session()));
    const gate = deferred();
    const fetchSpy = jest.spyOn(global, "fetch").mockReturnValue(gate.p);

    const p1 = getSession();
    const p2 = getSession();
    await Promise.resolve();
    gate.resolve(tokenResponse());
    const [s1, s2] = await Promise.all([p1, p2]);

    expect(fetchSpy).toHaveBeenCalledTimes(1);
    expect(s1?.at).toBe("new-at");
    expect(s2?.at).toBe("new-at");
    // Each request seals the refreshed session into its own response.
    expect(mockCookieStore.set).toHaveBeenCalledTimes(2);
  });

  it("replays the outcome to stragglers holding the pre-refresh cookie", async () => {
    const { getSession } = await load();
    stubCookie(await seal(session()));
    const fetchSpy = jest
      .spyOn(global, "fetch")
      .mockResolvedValue(tokenResponse());

    await getSession();
    // A late request still presents the OLD sealed cookie — under rt
    // rotation, re-submitting it would earn an invalid_grant logout.
    const s = await getSession();

    expect(fetchSpy).toHaveBeenCalledTimes(1);
    expect(s?.at).toBe("new-at");
    expect(s?.rt).toBe("rt-2");
  });

  it("clears the cookie on invalid_grant and replays the revocation", async () => {
    const { getSession } = await load();
    stubCookie(await seal(session()));
    const fetchSpy = jest.spyOn(global, "fetch").mockResolvedValue(
      Response.json({ error: "invalid_grant" }, { status: 400 }),
    );

    expect(await getSession()).toBeNull();
    expect(mockCookieStore.delete).toHaveBeenCalledWith("mpex_session");

    // Straggler with the old cookie: same verdict, no second IdP call.
    expect(await getSession()).toBeNull();
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    expect(mockCookieStore.delete).toHaveBeenCalledTimes(2);
  });

  it("keeps the cookie on transient refresh failure", async () => {
    const { getSession } = await load();
    stubCookie(await seal(session()));
    jest
      .spyOn(global, "fetch")
      .mockResolvedValue(Response.json({ error: "temporarily_unavailable" }, { status: 503 }));

    expect(await getSession()).toBeNull();
    expect(mockCookieStore.delete).not.toHaveBeenCalled();
  });

  it("clears a session whose refresh grant has expired", async () => {
    const { getSession } = await load();
    stubCookie(await seal(session({ rtExp: Date.now() - 1_000 })));
    const fetchSpy = jest.spyOn(global, "fetch");

    expect(await getSession()).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(mockCookieStore.delete).toHaveBeenCalledWith("mpex_session");
  });

  it("clears a session that is expired with no refresh token", async () => {
    const { getSession } = await load();
    stubCookie(await seal({ at: "at", atExp: Date.now() - 1_000 }));

    expect(await getSession()).toBeNull();
    expect(mockCookieStore.delete).toHaveBeenCalledWith("mpex_session");
  });

  it("bounds cookie maxAge by the refresh grant when the IdP exposes it", async () => {
    const { getSession } = await load();
    stubCookie(await seal(session()));
    jest
      .spyOn(global, "fetch")
      .mockResolvedValue(tokenResponse({ refresh_expires_in: 600 }));

    await getSession();

    const opts = mockCookieStore.set.mock.calls.at(-1)?.[2] as {
      maxAge: number;
    };
    expect(opts.maxAge).toBeGreaterThan(500);
    expect(opts.maxAge).toBeLessThanOrEqual(600);
  });
});
