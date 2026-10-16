"use client";

import Typography from "@mui/material/Typography";
import Paper from "@mui/material/Paper";
import Alert from "@mui/material/Alert";
import Chip from "@mui/material/Chip";
import Box from "@mui/material/Box";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";

import { endpoints } from "@/lib/api";
import StatusChip from "@/components/StatusChip";

export default function ProductDetailPage() {
  const { code } = useParams<{ code: string }>();

  const products = useQuery({ queryKey: ["products"], queryFn: endpoints.products });
  const decisions = useQuery({
    queryKey: ["decisions"],
    queryFn: () => endpoints.decisions(),
  });

  const product = products.data?.products.find((p) => p.code === code);
  const productDecisions =
    decisions.data?.decisions.filter((d) => d.productCode === code) ?? [];

  if (products.error) return <Alert severity="error">{`${products.error}`}</Alert>;
  if (products.data && !product)
    return <Alert severity="warning">Unknown product {code}</Alert>;

  return (
    <>
      <Typography variant="h4" component="h1" gutterBottom>
        {product?.name ?? code}
      </Typography>
      {product && (
        <Paper variant="outlined" sx={{ p: 2, mb: 3 }}>
          <Box sx={{ display: "flex", gap: 3, flexWrap: "wrap" }}>
            <Field label="Code" value={product.code} />
            <Field label="Loan type" value={product.loanType} />
            <Field label="Amortization" value={product.amortization} />
            <Field label="Term" value={`${product.termMonths} mo`} />
            <Field
              label="Status"
              value={
                <Chip
                  label={product.active ? "active" : "inactive"}
                  size="small"
                  color={product.active ? "success" : "default"}
                  variant="outlined"
                />
              }
            />
          </Box>
          {product.description && (
            <Typography variant="body2" color="text.secondary" sx={{ mt: 2 }}>
              {product.description}
            </Typography>
          )}
        </Paper>
      )}

      <Typography variant="h6" gutterBottom>
        Rules for this product
      </Typography>
      {productDecisions.length === 0 && (
        <Typography variant="body2" color="text.secondary">
          No decisions linked to {code}.
        </Typography>
      )}
      {productDecisions.map((d) => (
        <Paper key={d.key} variant="outlined" sx={{ p: 2, mb: 1 }}>
          <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
            <Typography
              component={Link}
              href={`/decisions/${encodeURIComponent(d.key)}`}
              sx={{ color: "primary.main", fontWeight: 600 }}
            >
              {d.key}
            </Typography>
            <Chip label={d.kind} size="small" variant="outlined" />
            <StatusChip status="draft" hasPublished={d.hasPublished} />
          </Box>
        </Paper>
      ))}
    </>
  );
}

function Field({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <Box>
      <Typography variant="caption" color="text.secondary">
        {label}
      </Typography>
      <Typography variant="body1" component="div">
        {value}
      </Typography>
    </Box>
  );
}
