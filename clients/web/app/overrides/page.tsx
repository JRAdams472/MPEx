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
import Autocomplete from "@mui/material/Autocomplete";
import Box from "@mui/material/Box";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  endpoints,
  type OverrideTargetKind,
  type ArtifactListItem,
} from "@/lib/api";
import StatusChip from "@/components/StatusChip";
import { useMe } from "@/lib/me";

const KINDS: OverrideTargetKind[] = [
  "decision",
  "expression",
  "matrix",
  "rate_sheet",
];

/** Parse a scalar override value: JSON when it parses (`0.125`, `true`,
 *  `null`, `"quoted"`), else the raw text as a string leaf. */
export function parseScalar(text: string): unknown {
  try {
    const v = JSON.parse(text);
    if (v !== null && typeof v === "object") return text;
    return v;
  } catch {
    return text;
  }
}

/** Draft keys of a target kind — hint options for the target picker; the
 *  field stays free-form for published-only targets. */
function useTargetKeys(kind: OverrideTargetKind) {
  return useQuery({
    queryKey: ["override-targets", kind],
    queryFn: async (): Promise<string[]> => {
      const items: ArtifactListItem[] =
        kind === "decision"
          ? (await endpoints.decisions()).decisions
          : kind === "expression"
            ? (await endpoints.expressions()).expressions
            : kind === "matrix"
              ? (await endpoints.matrices()).matrices
              : (await endpoints.rateSheets()).rateSheets;
      return items.map((i) => i.key);
    },
  });
}

export default function OverridesPage() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const { canEdit } = useMe();
  const { data, isLoading, error } = useQuery({
    queryKey: ["overrides"],
    queryFn: () => endpoints.overrides(),
  });

  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [targetKind, setTargetKind] = useState<OverrideTargetKind>("decision");
  const [targetKey, setTargetKey] = useState("");
  const [valuePath, setValuePath] = useState("");
  const [value, setValue] = useState("");
  const [description, setDescription] = useState("");
  const [formError, setFormError] = useState<string | null>(null);
  const targetKeys = useTargetKeys(targetKind);

  const create = useMutation({
    mutationFn: () =>
      endpoints.saveOverride(key.trim(), {
        targetKind,
        targetKey: targetKey.trim(),
        valuePath: valuePath.trim(),
        value: parseScalar(value.trim()),
        ...(description.trim() ? { description: description.trim() } : {}),
      }),
    onSuccess: async ({ status, data: saved }) => {
      if (status >= 400) {
        setFormError((saved as { error?: string }).error ?? `HTTP ${status}`);
        return;
      }
      await queryClient.invalidateQueries({ queryKey: ["overrides"] });
      setOpen(false);
      router.push(`/overrides/${encodeURIComponent(key.trim())}`);
    },
    onError: (e) => setFormError(`${e}`),
  });

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", mb: 2 }}>
        <Typography variant="h4" component="h1" sx={{ flexGrow: 1 }}>
          Overrides
        </Typography>
        {canEdit && (
          <Button variant="contained" size="small" onClick={() => setOpen(true)}>
            New override
          </Button>
        )}
      </Box>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Surgical value patches — a scalar leaf inside a target artifact,
        applied at snapshot build. Drafts edit the working set; publish stamps
        them into every snapshot like any other artifact.
      </Typography>
      {error && (
        <Alert severity="error" sx={{ mb: 2 }}>
          {`${error}`}
        </Alert>
      )}
      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Key</TableCell>
              <TableCell>Target</TableCell>
              <TableCell>Path</TableCell>
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
            {data?.overrides.length === 0 && (
              <TableRow>
                <TableCell colSpan={5} sx={{ color: "text.secondary" }}>
                  No draft overrides.{" "}
                  {canEdit ? "Create one to patch a value in place." : ""}
                </TableCell>
              </TableRow>
            )}
            {data?.overrides.map((o) => (
              <TableRow key={o.key} hover>
                <TableCell>
                  <Link
                    href={`/overrides/${encodeURIComponent(o.key)}`}
                    style={{ color: "inherit", fontWeight: 600 }}
                  >
                    {o.key}
                  </Link>
                </TableCell>
                <TableCell>
                  <Chip
                    label={o.targetKind}
                    size="small"
                    variant="outlined"
                    sx={{ mr: 1 }}
                  />
                  <Box component="span" sx={{ fontFamily: "monospace", fontSize: 12 }}>
                    {o.targetKey}
                  </Box>
                </TableCell>
                <TableCell sx={{ fontFamily: "monospace", fontSize: 12 }}>
                  {o.valuePath}
                </TableCell>
                <TableCell>
                  <StatusChip status="draft" hasPublished={o.hasPublished} />
                </TableCell>
                <TableCell>
                  {o.updatedAt ? new Date(o.updatedAt).toLocaleString() : "—"}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>

      <Dialog open={open} onClose={() => setOpen(false)} maxWidth="sm" fullWidth>
        <DialogTitle>New override</DialogTitle>
        <DialogContent
          sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}
        >
          <TextField
            label="Key"
            size="small"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="ovr.fico.floor"
            autoFocus
            fullWidth
          />
          <Box sx={{ display: "flex", gap: 2 }}>
            <TextField
              select
              label="Target kind"
              size="small"
              value={targetKind}
              onChange={(e) => {
                setTargetKind(e.target.value as OverrideTargetKind);
                setTargetKey("");
              }}
              sx={{ width: 180 }}
            >
              {KINDS.map((k) => (
                <MenuItem key={k} value={k}>
                  {k}
                </MenuItem>
              ))}
            </TextField>
            <Autocomplete
              freeSolo
              options={targetKeys.data ?? []}
              value={targetKey}
              onInputChange={(_e, v) => setTargetKey(v)}
              renderInput={(params) => (
                <TextField {...params} label="Target key" size="small" />
              )}
              sx={{ flexGrow: 1 }}
            />
          </Box>
          <TextField
            label="JSON Pointer"
            size="small"
            value={valuePath}
            onChange={(e) => setValuePath(e.target.value)}
            placeholder="/nodes/2/content/expressions/0/value"
            helperText="RFC 6901 pointer to a scalar leaf in the target document"
            fullWidth
            slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
          />
          <TextField
            label="Value"
            size="small"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            placeholder="0.125"
            helperText="Scalar leaf — 0.125, true, null, or text for a string"
            fullWidth
            slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
          />
          <TextField
            label="Description (optional)"
            size="small"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            fullWidth
          />
          {formError && <Alert severity="error">{formError}</Alert>}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            onClick={() => create.mutate()}
            disabled={
              key.trim() === "" ||
              targetKey.trim() === "" ||
              valuePath.trim() === "" ||
              value.trim() === "" ||
              create.isPending
            }
          >
            Create draft
          </Button>
        </DialogActions>
      </Dialog>
    </>
  );
}
