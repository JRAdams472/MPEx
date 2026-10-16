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
import Box from "@mui/material/Box";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { endpoints } from "@/lib/api";
import StatusChip from "@/components/StatusChip";

export default function ExpressionsPage() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ["expressions"],
    queryFn: () => endpoints.expressions(),
  });

  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [expression, setExpression] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: () =>
      endpoints.saveExpression(key.trim(), { expression: expression.trim() }),
    onSuccess: async ({ status, data: saved }) => {
      if (status >= 400) {
        setFormError((saved as { error?: string }).error ?? `HTTP ${status}`);
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ["expressions"] });
      setOpen(false);
      router.push(`/expressions/${encodeURIComponent(key.trim())}`);
    },
    onError: (e) => setFormError(`${e}`),
  });

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", mb: 2 }}>
        <Typography variant="h4" component="h1" sx={{ flexGrow: 1 }}>
          Expressions
        </Typography>
        <Button variant="contained" size="small" onClick={() => setOpen(true)}>
          New expression
        </Button>
      </Box>
      {error && <Alert severity="error">{`${error}`}</Alert>}
      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Key</TableCell>
              <TableCell>Kind</TableCell>
              <TableCell>Description</TableCell>
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
            {data?.expressions.map((e) => (
              <TableRow key={e.key} hover>
                <TableCell>
                  <Link
                    href={`/expressions/${encodeURIComponent(e.key)}`}
                    style={{ color: "inherit", fontWeight: 600 }}
                  >
                    {e.key}
                  </Link>
                </TableCell>
                <TableCell>
                  <Chip label={e.kind} size="small" variant="outlined" />
                </TableCell>
                <TableCell>{e.description || "—"}</TableCell>
                <TableCell>
                  <StatusChip status="draft" hasPublished={e.hasPublished} />
                </TableCell>
                <TableCell>
                  {e.updatedAt ? new Date(e.updatedAt).toLocaleString() : "—"}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>

      <Dialog open={open} onClose={() => setOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>New expression</DialogTitle>
        <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
          <TextField
            label="Key"
            size="small"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="expr.ltv.gt80"
            autoFocus
            fullWidth
          />
          <TextField
            label="ZEN expression"
            size="small"
            value={expression}
            onChange={(e) => setExpression(e.target.value)}
            placeholder="loan.ltv > 80"
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
