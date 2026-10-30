"use client";

import Typography from "@mui/material/Typography";
import Paper from "@mui/material/Paper";
import Alert from "@mui/material/Alert";
import Chip from "@mui/material/Chip";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import TextField from "@mui/material/TextField";
import MenuItem from "@mui/material/MenuItem";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { endpoints, mutationError, type Product } from "@/lib/api";
import StatusChip from "@/components/StatusChip";
import { useMe } from "@/lib/me";

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
      {product && (
        <RateSourceCard
          key={`${product.mndLabel}-${product.baseProductCode}-${product.rateAdjustment}`}
          product={product}
          products={products.data?.products ?? []}
        />
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

type SourceMode = "manual" | "index" | "derived";

/// Admin-editable rate source: `index` = MND index base product,
/// `derived` = base product + adjustment, `manual` = hand-built sheet.
function RateSourceCard({
  product,
  products,
}: {
  product: Product;
  products: Product[];
}) {
  const queryClient = useQueryClient();
  const { canPublish } = useMe();
  const index = useQuery({ queryKey: ["rateIndex"], queryFn: endpoints.rateIndex });

  const initial: SourceMode = product.mndLabel
    ? "index"
    : product.baseProductCode
      ? "derived"
      : "manual";
  const [mode, setMode] = useState<SourceMode>(initial);
  const [label, setLabel] = useState(product.mndLabel ?? "");
  const [base, setBase] = useState(product.baseProductCode ?? "");
  const [adj, setAdj] = useState(String(product.rateAdjustment));
  const [dirty, setDirty] = useState(false);

  const save = useMutation({
    mutationFn: () =>
      endpoints.saveProductRateConfig(product.code, {
        mndLabel: mode === "index" ? label : null,
        baseProductCode: mode === "derived" ? base : null,
        rateAdjustment: mode === "derived" ? parseFloat(adj) || 0 : 0,
      }),
    onSuccess: async ({ status }) => {
      if (status >= 400) return;
      setDirty(false);
      await queryClient.invalidateQueries({ queryKey: ["products"] });
    },
  });
  const saveError = mutationError(save);
  const baseOptions = products.filter((p) => p.mndLabel && p.code !== product.code);
  const canSave =
    dirty &&
    (mode === "manual" ||
      (mode === "index" && label !== "") ||
      (mode === "derived" && base !== "" && adj.trim() !== "" && isFinite(parseFloat(adj))));

  return (
    <Paper variant="outlined" sx={{ p: 2, mb: 3 }}>
      <Typography variant="h6" gutterBottom>
        Rate source
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        How the daily index import prices this product&rsquo;s sheet — base
        products take the index rate directly; derived products take a base
        product&rsquo;s rate plus an adjustment.
      </Typography>
      <Box sx={{ display: "flex", gap: 2, flexWrap: "wrap", alignItems: "center" }}>
        <TextField
          select
          label="Source"
          size="small"
          value={mode}
          disabled={!canPublish}
          onChange={(e) => {
            setMode(e.target.value as SourceMode);
            setDirty(true);
          }}
          sx={{ minWidth: 140 }}
        >
          <MenuItem value="manual">manual</MenuItem>
          <MenuItem value="index">MND index</MenuItem>
          <MenuItem value="derived">derived</MenuItem>
        </TextField>
        {mode === "index" && (
          <TextField
            select
            label="Index product"
            size="small"
            value={label}
            disabled={!canPublish}
            onChange={(e) => {
              setLabel(e.target.value);
              setDirty(true);
            }}
            sx={{ minWidth: 180 }}
          >
            {(index.data?.indexLabels ?? []).map((l) => (
              <MenuItem key={l} value={l}>
                {l}
              </MenuItem>
            ))}
          </TextField>
        )}
        {mode === "derived" && (
          <>
            <TextField
              select
              label="Base product"
              size="small"
              value={base}
              disabled={!canPublish}
              onChange={(e) => {
                setBase(e.target.value);
                setDirty(true);
              }}
              sx={{ minWidth: 180 }}
            >
              {baseOptions.map((p) => (
                <MenuItem key={p.code} value={p.code}>
                  {p.code} — {p.name}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              label="Adjustment"
              size="small"
              value={adj}
              disabled={!canPublish}
              onChange={(e) => {
                setAdj(e.target.value);
                setDirty(true);
              }}
              slotProps={{
                input: {
                  sx: { fontFamily: "monospace", fontSize: 13 },
                  endAdornment: (
                    <Typography variant="caption" color="text.secondary">
                      pts
                    </Typography>
                  ),
                },
              }}
              sx={{ width: 120 }}
            />
          </>
        )}
        {canPublish && (
          <Button
            variant="contained"
            size="small"
            disabled={!canSave || save.isPending}
            onClick={() => save.mutate()}
          >
            Save
          </Button>
        )}
      </Box>
      {saveError && (
        <Alert severity="error" sx={{ mt: 2 }}>
          {saveError}
        </Alert>
      )}
    </Paper>
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
