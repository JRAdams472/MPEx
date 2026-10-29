import { getSession } from "@/lib/auth";

const API_URL = process.env.API_URL ?? "http://localhost:8080";

/** Identity + resolved roles for the UI — proxied from the API's whoami. */
export async function GET() {
  const session = await getSession();
  if (!session) {
    return Response.json({ error: "unauthenticated" }, { status: 401 });
  }
  const res = await fetch(`${API_URL}/api/auth/whoami`, {
    headers: { authorization: `Bearer ${session.at}` },
    cache: "no-store",
  });
  return new Response(res.body, {
    status: res.status,
    headers: {
      "content-type": res.headers.get("content-type") ?? "application/json",
    },
  });
}
