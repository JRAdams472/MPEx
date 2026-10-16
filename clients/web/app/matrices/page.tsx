"use client";

import Typography from "@mui/material/Typography";
import Paper from "@mui/material/Paper";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import Chip from "@mui/material/Chip";
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

const KINDS = ["llpa", "eligibility", "pricing", "shared"];

const STARTER_SPEC = {
  rows: [{ id: "r1", label: "FICO 740+", expr: "borrower.fico >= 740" }],
  cols: [{ id: "c1", label: "LTV ≤ 80", expr: "loan.ltv <= 80" }],
  cells: { "r1:c1": 0 },
  op: "sum",
};

export default function MatricesPage() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ["matrices"],
    queryFn: () => endpoints.matrices(),
  });

  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [kind, setKind] = useState("llpa");
  const [productCode, setProductCode] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: () =>
      endpoints.saveMatrix(key.trim(), {
        kind,
        productCode: productCode.trim() || null,
        spec: STARTER_SPEC,
      }),
    onSuccess: async ({ status, data: saved }) => {
      if (status >= 400) {
        setFormError((saved as { error?: string }).error ?? `HTTP ${status}`);
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ["matrices"] });
      setOpen(false);
      router.push(`/matrices/${encodeURIComponent(key.trim())}`);
    },
    onError: (e) => setFormError(`${e}`),
  });

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", mb: 2 }}>
        <Typography variant="h4" component="h1" sx={{ flexGrow: 1 }}>
          Matrices
        </Typography>
        <Button variant="contained" size="small" onClick={() => setOpen(true)}>
          New matrix
        </Button>
      </Box>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Row×column adjustment grids — each populated cell applies when its row
        AND column rules match. Specs compile to JDM at save.
      </Typography>
      {error && <Alert severity="error">{`${error}`}</Alert>}
      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Key</TableCell>
              <TableCell>Kind</TableCell>
              <TableCell>Product</TableCell>
              <TableCell>State</TableCell>
              <TableCell>Updated</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={5}>Loading…</TableCell>
              </TableRow>
            )}
            {data?.matrices.map((m) => (
              <TableRow key={m.key} hover>
                <TableCell>
                  <Link
                    href={`/matrices/${encodeURIComponent(m.key)}`}
                    style={{ color: "inherit", fontWeight: 600 }}
                  >
                    {m.key}
                  </Link>
                </TableCell>
                <TableCell>
                  <Chip label={m.kind} size="small" variant="outlined" />
                </TableCell>
                <TableCell>{m.productCode ?? "—"}</TableCell>
                <TableCell>
                  <StatusChip status="draft" hasPublished={m.hasPublished} />
                </TableCell>
                <TableCell>
                  {m.updatedAt ? new Date(m.updatedAt).toLocaleString() : "—"}
                </TableCell>
              </TableRow>
            ))}
            {data && data.matrices.length === 0 && (
              <TableRow>
                <TableCell colSpan={5}>No matrices yet.</TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </TableContainer>

      <Dialog open={open} onClose={() => setOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>New matrix</DialogTitle>
        <DialogContent
          sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}
        >
          <TextField
            label="Key"
            size="small"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="llpa.fico-ltv"
            autoFocus
            fullWidth
          />
          <TextField
            select
            label="Kind"
            size="small"
            value={kind}
            onChange={(e) => setKind(e.target.value)}
          >
            {KINDS.map((k) => (
              <MenuItem key={k} value={k}>
                {k}
              </MenuItem>
            ))}
          </TextField>
          <TextField
            label="Product code (optional — shared when blank)"
            size="small"
            value={productCode}
            onChange={(e) => setProductCode(e.target.value)}
            placeholder="CONV30"
            fullWidth
          />
          {formError && <Alert severity="error">{formError}</Alert>}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => create.mutate()}
            disabled={key.trim() === "" || create.isPending}
          >
            Create draft
          </Button>
        </DialogActions>
      </Dialog>
    </>
  );
}
