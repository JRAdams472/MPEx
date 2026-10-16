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

const KINDS = ["eligibility", "llpa", "pricing", "shared"];

const STARTER_JDM = {
  nodes: [
    { id: "in", type: "inputNode", name: "Request", position: { x: 60, y: 120 } },
    {
      id: "out",
      type: "outputNode",
      name: "Response",
      position: { x: 460, y: 120 },
    },
  ],
  edges: [{ id: "e1", sourceId: "in", targetId: "out" }],
};

export default function DecisionsPage() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ["decisions"],
    queryFn: () => endpoints.decisions(),
  });

  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [kind, setKind] = useState("eligibility");
  const [productCode, setProductCode] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: () =>
      endpoints.saveDecision(key.trim(), {
        kind,
        productCode: productCode.trim() || null,
        jdm: STARTER_JDM,
      }),
    onSuccess: async ({ status, data: saved }) => {
      if (status >= 400) {
        setFormError((saved as { error?: string }).error ?? `HTTP ${status}`);
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ["decisions"] });
      setOpen(false);
      router.push(`/decisions/${encodeURIComponent(key.trim())}`);
    },
    onError: (e) => setFormError(`${e}`),
  });

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", mb: 2 }}>
        <Typography variant="h4" component="h1" sx={{ flexGrow: 1 }}>
          Decisions
        </Typography>
        <Button variant="contained" size="small" onClick={() => setOpen(true)}>
          New decision
        </Button>
      </Box>
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
            {data?.decisions.map((d) => (
              <TableRow key={d.key} hover>
                <TableCell>
                  <Link
                    href={`/decisions/${encodeURIComponent(d.key)}`}
                    style={{ color: "inherit", fontWeight: 600 }}
                  >
                    {d.key}
                  </Link>
                </TableCell>
                <TableCell>
                  <Chip label={d.kind} size="small" variant="outlined" />
                </TableCell>
                <TableCell>{d.productCode ?? "—"}</TableCell>
                <TableCell>
                  <StatusChip status="draft" hasPublished={d.hasPublished} />
                </TableCell>
                <TableCell>
                  {d.updatedAt ? new Date(d.updatedAt).toLocaleString() : "—"}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>

      <Dialog open={open} onClose={() => setOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>New decision</DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            label="Key"
            size="small"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="eligibility.PRODUCT"
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
            label="Product code (optional)"
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
