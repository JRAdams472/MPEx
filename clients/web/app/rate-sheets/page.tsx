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


import { endpoints, type RateIndexImport } from "@/lib/api";

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
  const { canEdit, canPublish } = useMe();
  const { data, isLoading, error } = useQuery({
    queryKey: ["rateSheets"],
    queryFn: () => endpoints.rateSheets(),
  });
  const products = useQuery({ queryKey: ["products"], queryFn: endpoints.products });

  const [open, setOpen] = useState(false);
  const [loadOpen, setLoadOpen] = useState(false);
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
        {canPublish && (
          <Button
            variant="outlined"
            size="small"
            onClick={() => setLoadOpen(true)}
            sx={{ mr: 1 }}
          >
            Load today&rsquo;s rates
          </Button>
        )}
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

      <LoadRatesDialog
        open={loadOpen}
        onClose={() => setLoadOpen(false)}
        onApplied={async () => {
          await queryClient.invalidateQueries({ queryKey: ["rateSheets"] });
          await queryClient.invalidateQueries({ queryKey: ["publications"] });
        }}
      />
    </>
  );
}

/// Preview → apply the daily MND rate-index import. Admin-only (the parent
/// mounts this only behind `canPublish`). Apply writes single-rate-at-par
/// drafts and stamps a rate-sheets-only publication.
function LoadRatesDialog({
  open,
  onClose,
  onApplied,
}: {
  open: boolean;
  onClose: () => void;
  onApplied: () => Promise<void>;
}) {
  const [applied, setApplied] = useState<RateIndexImport | null>(null);
  const preview = useQuery({
    queryKey: ["rateIndexPreview"],
    enabled: open && !applied,
    retry: false,
    queryFn: () =>
      endpoints.previewRateIndex().then(({ status, data }) => {
        if (status >= 400) {
          throw new Error(
            (data as { error?: string }).error ?? `HTTP ${status}`,
          );
        }
        return data;
      }),
  });
  const apply = useMutation({
    mutationFn: endpoints.importRateIndex,
    onSuccess: async ({ status, data }) => {
      if (status >= 400) return;
      setApplied(data);
      await onApplied();
    },
  });
  const applyError =
    apply.data && apply.data.status >= 400
      ? ((apply.data.data as { error?: string }).error ??
        `HTTP ${apply.data.status}`)
      : apply.isError
        ? `${apply.error}`
        : null;

  const close = () => {
    setApplied(null);
    onClose();
  };

  return (
    <Dialog open={open} onClose={close} maxWidth="md" fullWidth>
      <DialogTitle>Load today&rsquo;s rates — MND index</DialogTitle>
      <DialogContent>
        {preview.isLoading && <Typography>Fetching the index…</Typography>}
        {preview.error && (
          <Alert severity="error">{`${preview.error}`}</Alert>
        )}
        {applied && (
          <Alert severity="success" sx={{ mb: 2 }}>
            {applied.publication
              ? `Published “${applied.publication.label}” — ${applied.sheets.length} sheet(s) live.`
              : `Rates unchanged — ${applied.sheets.length} sheet(s) already matched the index.`}
          </Alert>
        )}
        {preview.data && !applied && (
          <>
            <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
              Fetched {new Date(preview.data.fetchedAt).toLocaleString()} ·
              sha256 {preview.data.contentHash.slice(0, 12)}…
            </Typography>
            <Table size="small" sx={{ mb: 2 }}>
              <TableHead>
                <TableRow>
                  <TableCell>Index product</TableCell>
                  <TableCell align="right">Rate</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {preview.data.indexRates.map((r) => (
                  <TableRow key={r.label}>
                    <TableCell>{r.label}</TableCell>
                    <TableCell align="right" sx={{ fontFamily: "monospace" }}>
                      {r.rate.toFixed(2)}%
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            <Typography variant="subtitle2" gutterBottom>
              Sheets this will write ({preview.data.sheets.length})
            </Typography>
            <Table size="small" sx={{ mb: 2 }}>
              <TableHead>
                <TableRow>
                  <TableCell>Product</TableCell>
                  <TableCell align="right">Rate</TableCell>
                  <TableCell>Source</TableCell>
                </TableRow>
              </TableHead>
              <TableBody>
                {preview.data.sheets.map((s) => (
                  <TableRow key={s.productCode}>
                    <TableCell>{s.productCode}</TableCell>
                    <TableCell align="right" sx={{ fontFamily: "monospace" }}>
                      {s.rate.toFixed(2)}%
                    </TableCell>
                    <TableCell>
                      {s.source.startsWith("base:")
                        ? `${s.source.slice(5)} ${s.adjustment >= 0 ? "+" : ""}${s.adjustment}`
                        : s.source.slice(6)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
            {preview.data.skipped.length > 0 && (
              <Typography variant="body2" color="text.secondary">
                Manual sheets untouched: {preview.data.skipped.join(", ")}
              </Typography>
            )}
            {preview.data.unmapped.length > 0 && (
              <Alert severity="warning" sx={{ mt: 1 }}>
                Index labels with no product: {preview.data.unmapped.join(", ")}
              </Alert>
            )}
          </>
        )}
        {applyError && (
          <Alert severity="error" sx={{ mt: 2 }}>
            {applyError}
          </Alert>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={close}>{applied ? "Close" : "Cancel"}</Button>
        {!applied && (
          <Button
            variant="contained"
            disabled={!preview.data || apply.isPending}
            onClick={() => apply.mutate()}
          >
            Apply &amp; publish
          </Button>
        )}
      </DialogActions>
    </Dialog>
  );
}
