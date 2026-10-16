import type { Metadata } from "next";

import AppShell from "../components/AppShell";
import Providers from "./providers";

export const metadata: Metadata = {
  title: "MPEx",
  description: "Mortgage product eligibility & pricing engine",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <Providers>
          <AppShell>{children}</AppShell>
        </Providers>
      </body>
    </html>
  );
}
