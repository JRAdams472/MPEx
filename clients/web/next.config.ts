import type { NextConfig } from "next";

// `next dev` compiles client code with eval(); production builds don't.
const isDev = process.env.NODE_ENV !== "production";

// Next ships inline bootstrap scripts (self.__next_f) and MUI/Emotion injects
// <style> tags, so script-src/style-src keep 'unsafe-inline'. All API traffic
// is same-origin through /bff — connect-src stays 'self'. HSTS is deliberately
// absent: it belongs at the TLS terminator (see docs/security.md).
const csp = [
  "default-src 'self'",
  `script-src 'self' 'unsafe-inline'${isDev ? " 'unsafe-eval'" : ""}`,
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  "font-src 'self' data:",
  "connect-src 'self'",
  "object-src 'none'",
  "base-uri 'self'",
  "form-action 'self'",
  "frame-ancestors 'none'",
].join("; ");

const nextConfig: NextConfig = {
  reactStrictMode: true,
  async headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "Content-Security-Policy", value: csp },
          { key: "X-Content-Type-Options", value: "nosniff" },
          { key: "X-Frame-Options", value: "DENY" },
          { key: "Referrer-Policy", value: "no-referrer" },
        ],
      },
    ];
  },
};

export default nextConfig;
