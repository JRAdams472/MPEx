"use client";

import { useQuery } from "@tanstack/react-query";

export interface Me {
  sub: string;
  email: string | null;
  roles: string[];
  /** Lanes this principal may evaluate in — null = unrestricted (MPE-159). */
  permittedEnvs: string[] | null;
}

/**
 * The authenticated principal — `/auth/me` proxies the API's whoami so
 * roles always reflect the server-side role_grants truth. `canEdit`
 * (pricer+) gates mutation affordances; `canPublish` (admin) gates publish.
 * While loading, both are false (affordances hide until identity resolves).
 */
export function useMe() {
  const q = useQuery<Me>({
    queryKey: ["me"],
    queryFn: async () => {
      const res = await fetch("/auth/me");
      if (res.status === 401) {
        // eslint-disable-next-line @next/next/no-location-assign-relative-destination -- inside a React Query queryFn (no useRouter); /auth/login is a route handler, needs a document navigation
        window.location.assign("/auth/login");
        throw new Error("unauthenticated");
      }
      if (!res.ok) throw new Error(`/auth/me failed: ${res.status}`);
      return res.json() as Promise<Me>;
    },
    staleTime: 60_000,
  });
  const roles = q.data?.roles ?? [];
  return {
    ...q,
    me: q.data,
    canEdit: roles.includes("pricer") || roles.includes("admin"),
    canPublish: roles.includes("admin"),
    isAdmin: roles.includes("admin"),
  };
}
