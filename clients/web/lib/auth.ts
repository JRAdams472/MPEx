import "server-only";

import { createHash, randomBytes } from "crypto";
import { cookies } from "next/headers";
import { decodeJwt, EncryptJWT, jwtDecrypt } from "jose";

/**
 * Server-side auth for the BFF: auth-code+PKCE against the OIDC issuer,
 * tokens sealed into an httpOnly JWE cookie. `OIDC_ISSUER` is the
 * browser-facing issuer (login redirects); `OIDC_TOKEN_URL` is where the
 * server exchanges codes — they differ inside compose (dex vs localhost).
 */
const ISSUER = process.env.OIDC_ISSUER ?? "http://localhost:5556/dex";
const TOKEN_URL = process.env.OIDC_TOKEN_URL ?? `${ISSUER}/token`;
export const CLIENT_ID = process.env.OIDC_CLIENT_ID ?? "mpex-web";
export const REDIRECT_URI =
  process.env.OIDC_REDIRECT_URI ?? "http://localhost:3000/auth/callback";
const SECURE = process.env.SESSION_SECURE === "1";
// No fallback: a compiled-in default would let anyone forge session JWEs.
// Evaluated lazily so `next build` (which imports this module) never throws.
const SECRET = process.env.SESSION_SECRET;

const SESSION_COOKIE = "mpex_session";
const PKCE_COOKIE = "mpex_pkce";
const STATE_COOKIE = "mpex_state";
/** Refresh this far before the access-token exp, ms. */
const REFRESH_MARGIN = 30_000;

function sessionKey(): Buffer {
  if (!SECRET) {
    throw new Error(
      "SESSION_SECRET is required — set it in .env / clients/web/.env.local (docs/security.md)",
    );
  }
  return createHash("sha256").update(SECRET).digest();
}

export interface SessionData {
  /** access token — the bearer for API calls */
  at: string;
  /** refresh token, when the IdP issued one (offline_access) */
  rt?: string;
  /** access-token expiry, epoch ms */
  exp: number;
  email?: string;
}

const cookieOpts = {
  httpOnly: true,
  sameSite: "lax" as const,
  secure: SECURE,
  path: "/",
};

function newPkce() {
  const verifier = randomBytes(32).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const state = randomBytes(16).toString("hex");
  return { verifier, challenge, state };
}

/** Build the authorize URL and remember PKCE + state (+ post-login target). */
export async function beginLogin(next: string) {
  const { verifier, challenge, state } = newPkce();
  const store = await cookies();
  store.set(PKCE_COOKIE, verifier, { ...cookieOpts, maxAge: 300 });
  store.set(STATE_COOKIE, `${state}|${next}`, { ...cookieOpts, maxAge: 300 });

  const url = new URL(`${ISSUER}/auth`);
  url.searchParams.set("client_id", CLIENT_ID);
  url.searchParams.set("redirect_uri", REDIRECT_URI);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("scope", "openid email profile offline_access");
  url.searchParams.set("state", state);
  url.searchParams.set("code_challenge", challenge);
  url.searchParams.set("code_challenge_method", "S256");
  return url.toString();
}

/** Exchange the callback code for tokens; returns session + post-login path. */
export async function finishLogin(code: string, state: string | null) {
  const store = await cookies();
  const verifier = store.get(PKCE_COOKIE)?.value;
  const [expected, next] = (store.get(STATE_COOKIE)?.value ?? "|").split("|");
  if (!verifier || !state || state !== expected) {
    throw new Error("bad oauth state");
  }

  const res = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "authorization_code",
      client_id: CLIENT_ID,
      redirect_uri: REDIRECT_URI,
      code,
      code_verifier: verifier,
    }),
  });
  const tokens = await res.json();
  if (!res.ok) {
    throw new Error(`token exchange failed: ${tokens.error ?? res.status}`);
  }
  const session = sessionFromTokens(tokens);
  await setSession(session);
  store.delete(PKCE_COOKIE);
  store.delete(STATE_COOKIE);
  return next || "/";
}

function sessionFromTokens(t: {
  access_token: string;
  refresh_token?: string;
  expires_in?: number;
  id_token?: string;
}): SessionData {
  let email: string | undefined;
  try {
    email = t.id_token ? (decodeJwt(t.id_token).email as string) : undefined;
  } catch {
    /* email is a nicety, not a requirement */
  }
  return {
    at: t.access_token,
    rt: t.refresh_token,
    exp: Date.now() + (t.expires_in ?? 3600) * 1000,
    email,
  };
}

async function setSession(s: SessionData) {
  const sealed = await new EncryptJWT(s as unknown as Record<string, unknown>)
    .setProtectedHeader({ alg: "dir", enc: "A256GCM" })
    .setIssuedAt()
    .encrypt(sessionKey());
  (await cookies()).set(SESSION_COOKIE, sealed, {
    ...cookieOpts,
    maxAge: 60 * 60 * 12,
  });
}

export async function clearSession() {
  (await cookies()).delete(SESSION_COOKIE);
}

/**
 * Current session — refreshes the access token when close to expiry and a
 * refresh token exists. Returns null when there's no usable session.
 * Callable only from route handlers / server actions (it writes cookies).
 */
export async function getSession(): Promise<SessionData | null> {
  const raw = (await cookies()).get(SESSION_COOKIE)?.value;
  if (!raw) return null;
  let s: SessionData;
  try {
    s = (await jwtDecrypt(raw, sessionKey())).payload as unknown as SessionData;
  } catch {
    return null;
  }
  if (Date.now() < s.exp - REFRESH_MARGIN) return s;
  if (!s.rt) return null;

  const res = await fetch(TOKEN_URL, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: new URLSearchParams({
      grant_type: "refresh_token",
      client_id: CLIENT_ID,
      refresh_token: s.rt,
    }),
  });
  if (!res.ok) return null;
  const tokens = await res.json();
  const refreshed: SessionData = {
    ...sessionFromTokens(tokens),
    rt: tokens.refresh_token ?? s.rt,
    email: s.email,
  };
  await setSession(refreshed);
  return refreshed;
}
