"use client";

import Box from "@mui/material/Box";
import Card from "@mui/material/Card";
import CardActionArea from "@mui/material/CardActionArea";
import CardContent from "@mui/material/CardContent";
import Typography from "@mui/material/Typography";
import Link from "next/link";
import { useQueries } from "@tanstack/react-query";

import { endpoints } from "@/lib/api";

const SECTIONS = [
  {
    href: "/products",
    title: "Products",
    desc: "Loan products in the catalog — eligibility and pricing target these codes.",
  },
  {
    href: "/decisions",
    title: "Decisions",
    desc: "Zen decision graphs — eligibility rules per product, editable in the JDM editor.",
  },
  {
    href: "/expressions",
    title: "Expressions",
    desc: "Named reusable ZEN fragments, validated on save.",
  },
  {
    href: "/matrices",
    title: "Matrices",
    desc: "Row×column LLPA adjustment grids, compiled to decision tables.",
  },
  {
    href: "/rate-sheets",
    title: "Rate sheets",
    desc: "Per-product rate × lock-term price grids.",
  },
  {
    href: "/publications",
    title: "Publications",
    desc: "Named point-in-time snapshots of every published artifact.",
  },
];

export default function Home() {
  const [products, decisions, expressions, matrices, rateSheets, publications] =
    useQueries({
      queries: [
        { queryKey: ["products"], queryFn: endpoints.products },
        { queryKey: ["decisions"], queryFn: () => endpoints.decisions() },
        { queryKey: ["expressions"], queryFn: () => endpoints.expressions() },
        { queryKey: ["matrices"], queryFn: () => endpoints.matrices() },
        { queryKey: ["rateSheets"], queryFn: () => endpoints.rateSheets() },
        { queryKey: ["publications"], queryFn: endpoints.publications },
      ],
    });

  const counts: Record<string, number | undefined> = {
    "/products": products.data?.products.length,
    "/decisions": decisions.data?.decisions.length,
    "/expressions": expressions.data?.expressions.length,
    "/matrices": matrices.data?.matrices.length,
    "/rate-sheets": rateSheets.data?.rateSheets.length,
    "/publications": publications.data?.publications.length,
  };

  return (
    <Box>
      <Typography variant="h4" component="h1" gutterBottom>
        Dashboard
      </Typography>
      <Typography variant="body1" color="text.secondary" sx={{ mb: 4 }}>
        Centralized mortgage product eligibility &amp; pricing. Edit drafts,
        publish a named snapshot, evaluate deterministically at any `asOf`.
      </Typography>
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(260px, 1fr))",
          gap: 2,
        }}
      >
        {SECTIONS.map((s) => (
          <Card key={s.href} variant="outlined" sx={{ bgcolor: "background.paper" }}>
            <CardActionArea component={Link} href={s.href}>
              <CardContent>
                <Typography variant="h6" color="primary.main">
                  {s.title}
                  {counts[s.href] !== undefined && (
                    <Typography
                      component="span"
                      variant="h6"
                      color="text.secondary"
                      sx={{ ml: 1 }}
                    >
                      ({counts[s.href]})
                    </Typography>
                  )}
                </Typography>
                <Typography variant="body2" color="text.secondary">
                  {s.desc}
                </Typography>
              </CardContent>
            </CardActionArea>
          </Card>
        ))}
      </Box>
    </Box>
  );
}
