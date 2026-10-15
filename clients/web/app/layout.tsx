import type { Metadata } from "next";

import Providers from "./providers";

export const metadata: Metadata = {
  title: "MPEx",
  description: "Mortgage product eligibility & pricing engine",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
