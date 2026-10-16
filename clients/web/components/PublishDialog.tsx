"use client";

import Dialog from "@mui/material/Dialog";
import DialogTitle from "@mui/material/DialogTitle";
import DialogContent from "@mui/material/DialogContent";
import DialogActions from "@mui/material/DialogActions";
import Button from "@mui/material/Button";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import Alert from "@mui/material/Alert";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { endpoints, type Publication } from "@/lib/api";

/// Publishes every changed draft (decisions, expressions, matrices, rate
/// sheets) atomically under one named publication. 409 → nothing to publish.
export default function PublishDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const [label, setLabel] = useState("");
  const [result, setResult] = useState<Publication | null>(null);
  const [conflict, setConflict] = useState(false);

  const mutation = useMutation({
    mutationFn: () => endpoints.publish(label.trim()),
    onSuccess: async ({ status, data }) => {
      if (status === 409) {
        setConflict(true);
        setResult(null);
        return;
      }
      setConflict(false);
      setResult(data);
      await queryClient.invalidateQueries();
    },
  });

  const close = () => {
    setResult(null);
    setConflict(false);
    setLabel("");
    onClose();
  };

  const counts = result
    ? [
        ["decisions", result.decisions?.length ?? result.decisionCount ?? 0],
        ["expressions", result.expressions?.length ?? result.expressionCount ?? 0],
        ["matrices", result.matrices?.length ?? 0],
        ["rate sheets", result.rateSheets?.length ?? 0],
      ]
    : [];

  return (
    <Dialog open={open} onClose={close} maxWidth="sm" fullWidth>
      <DialogTitle>Publish rule artifacts</DialogTitle>
      <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 1 }}>
        <Typography variant="body2" color="text.secondary">
          Stamps every changed draft into an immutable published row under one
          named publication — the point-in-time snapshot that `asOf` resolution
          serves.
        </Typography>
        <TextField
          label="Publication label"
          size="small"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
          placeholder="e.g. 2026-10-16 pricing drop"
          autoFocus
          fullWidth
        />
        {conflict && (
          <Alert severity="warning">
            No draft changes to publish — every draft is already at its latest
            published content.
          </Alert>
        )}
        {mutation.isError && <Alert severity="error">{`${mutation.error}`}</Alert>}
        {result && (
          <Alert severity="success">
            Published <strong>{result.label}</strong> at{" "}
            {new Date(result.publishedAt).toLocaleString()} —{" "}
            {counts.map(([k, n]) => `${n} ${k}`).join(", ")}.
          </Alert>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={close}>{result ? "Done" : "Cancel"}</Button>
        {!result && (
          <Button
            variant="contained"
            onClick={() => mutation.mutate()}
            disabled={label.trim() === "" || mutation.isPending}
          >
            Publish
          </Button>
        )}
      </DialogActions>
    </Dialog>
  );
}
