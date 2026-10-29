"use client";

import Typography from "@mui/material/Typography";

import Paper from "@mui/material/Paper";

import Table from "@mui/material/Table";

import TableBody from "@mui/material/TableBody";

import TableCell from "@mui/material/TableCell";

import TableContainer from "@mui/material/TableContainer";

import TableHead from "@mui/material/TableHead";

import TableRow from "@mui/material/TableRow";

import Alert from "@mui/material/Alert";

import Button from "@mui/material/Button";

import Dialog from "@mui/material/Dialog";

import DialogTitle from "@mui/material/DialogTitle";

import DialogContent from "@mui/material/DialogContent";

import DialogActions from "@mui/material/DialogActions";

import TextField from "@mui/material/TextField";

import MenuItem from "@mui/material/MenuItem";

import Box from "@mui/material/Box";

import Link from "next/link";

import { useRouter } from "next/navigation";

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";

import { useState } from "react";


import { endpoints } from "@/lib/api";

import StatusChip from "@/components/StatusChip";
import { useMe } from "@/lib/me";


const STARTER_SPEC = {
  rates: [6.5],
  lockTerms: [30],
  prices: { "6.5:30": 100.0 },
};

export default function RateSheetsPage() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const { canEdit } = useMe();
  const { data, isLoading, error } = useQuery({
    queryKey: ["rateSheets"],
    queryFn: () => endpoints.rateSheets(),
  });
  const products = useQuery({ queryKey: ["products"], queryFn: endpoints.products });

  const [open, setOpen] = useState(false);
  const [productCode, setProductCode] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const existing = new Set(data?.rateSheets.map((s) => s.key) ?? []);
  const available = (products.data?.products ?? []).filter(
    (p) => !existing.has(p.code),
  );

  const create = useMutation({
    mutationFn: () =>
      endpoints.saveRateSheet(productCode, { spec: STARTER_SPEC }),
    onSuccess: async ({ status, data: saved }) => {
      if (status >= 400) {
        setFormError((saved as { error?: string }).error ?? `HTTP ${status}`);
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ["rateSheets"] });
      setOpen(false);
      router.push(`/rate-sheets/${encodeURIComponent(productCode)}`);
    },
    onError: (e) => setFormError(`${e}`),
  });

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", mb: 2 }}>
        <Typography variant="h4" component="h1" sx={{ flexGrow: 1 }}>
          Rate sheets
        </Typography>
        {canEdit && (
          <Button variant="contained" size="small" onClick={() => setOpen(true)}>
            New rate sheet
          </Button>
        )}
      </Box>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Per-product rate × lock-term price grids — the base price the LLPA
        waterfall adjusts. One sheet per product.
      </Typography>
      {error && <Alert severity="error">{`${error}`}</Alert>}
      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Product</TableCell>
              <TableCell>State</TableCell>
              <TableCell>Updated</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={3}>Loading…</TableCell>
              </TableRow>
            )}
            {data?.rateSheets.map((s) => (
              <TableRow key={s.key} hover>
                <TableCell>
                  <Link
                    href={`/rate-sheets/${encodeURIComponent(s.key)}`}
                    style={{ color: "inherit", fontWeight: 600 }}
                  >
                    {s.key}
                  </Link>
                </TableCell>
                <TableCell>
                  <StatusChip status="draft" hasPublished={s.hasPublished} />
                </TableCell>
                <TableCell>
                  {s.updatedAt ? new Date(s.updatedAt).toLocaleString() : "—"}
                </TableCell>
              </TableRow>
            ))}
            {data && data.rateSheets.length === 0 && (
              <TableRow>
                <TableCell colSpan={3}>
                  No rate sheets yet — create one for a product.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </TableContainer>

      <Dialog open={open} onClose={() => setOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>New rate sheet</DialogTitle>
        <DialogContent
          sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}
        >
          <TextField
            select
            label="Product"
            size="small"
            value={productCode}
            onChange={(e) => setProductCode(e.target.value)}
          >
            {available.map((p) => (
              <MenuItem key={p.code} value={p.code}>
                {p.code} — {p.name}
              </MenuItem>
            ))}
          </TextField>
          {available.length === 0 && (
            <Alert severity="info">
              Every product already has a rate sheet draft.
            </Alert>
          )}
          {formError && <Alert severity="error">{formError}</Alert>}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => create.mutate()}
            disabled={productCode === "" || create.isPending}
          >
            Create draft
          </Button>
        </DialogActions>
      </Dialog>
    </>
  );
}
