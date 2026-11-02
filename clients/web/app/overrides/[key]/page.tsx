"use client";

import Typography from "@mui/material/Typography";
import Paper from "@mui/material/Paper";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import TextField from "@mui/material/TextField";
import MenuItem from "@mui/material/MenuItem";
import Divider from "@mui/material/Divider";
import Chip from "@mui/material/Chip";
import { useParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { endpoints, mutationError, type OverrideTargetKind } from "@/lib/api";
import StatusChip from "@/components/StatusChip";
import { useMe } from "@/lib/me";
import { parseScalar } from "../page";

const KINDS: OverrideTargetKind[] = [
  "decision",
  "expression",
  "matrix",
  "rate_sheet",
];

function formatValue(v: unknown): string {
  return typeof v === "string" ? v : JSON.stringify(v);
}

export default function OverrideDetailPage() {
  const { key: rawKey } = useParams<{ key: string }>();
  const key = decodeURIComponent(rawKey);
  const queryClient = useQueryClient();
  const { canEdit } = useMe();

  const { data, error, isLoading } = useQuery({
    queryKey: ["override", key],
    queryFn: () => endpoints.override(key),
  });

  const [targetKind, setTargetKind] = useState<OverrideTargetKind>("decision");
  const [targetKey, setTargetKey] = useState("");
  const [valuePath, setValuePath] = useState("");
  const [value, setValue] = useState("");
  const [description, setDescription] = useState("");
  const [dirty, setDirty] = useState(false);
  const loadedRef = useRef(false);

  useEffect(() => {
    if (data && !loadedRef.current) {
      loadedRef.current = true;
      // No draft? The form seeds from the published row — "Save draft" is
      // the new-draft-from-published verb, never an edit-in-place.
      const row = data.draft ?? data.published;
      setTargetKind(row?.targetKind ?? "decision");
      setTargetKey(row?.targetKey ?? "");
      setValuePath(row?.valuePath ?? "");
      setValue(row ? formatValue(row.value) : "");
      setDescription(row?.description ?? "");
    }
  }, [data]);

  const save = useMutation({
    mutationFn: () =>
      endpoints.saveOverride(key, {
        targetKind,
        targetKey: targetKey.trim(),
        valuePath: valuePath.trim(),
        value: parseScalar(value.trim()),
        description,
      }),
    onSuccess: async (res) => {
      if (res.status >= 400) return;
      setDirty(false);
      await queryClient.invalidateQueries({ queryKey: ["override", key] });
      await queryClient.invalidateQueries({ queryKey: ["overrides"] });
    },
  });
  const saveError = mutationError(save);

  const remove = useMutation({
    mutationFn: () => endpoints.deleteOverride(key),
    onSuccess: async () => {
      await queryClient.invalidateQueries();
    },
  });

  if (isLoading) return <Typography>Loading…</Typography>;
  if (error) return <Alert severity="error">{`${error}`}</Alert>;
  if (!data) return null;

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", gap: 2, mb: 2 }}>
        <Typography
          variant="h5"
          component="h1"
          sx={{ flexGrow: 1, fontFamily: "monospace", fontSize: 20 }}
        >
          {key}
        </Typography>
        <StatusChip
          status={data.draft ? "draft" : "published"}
          hasPublished={Boolean(data.published)}
        />
        {canEdit && (
          <>
            <Button
              variant="contained"
              size="small"
              disabled={!dirty || save.isPending}
              onClick={() => save.mutate()}
            >
              Save draft
            </Button>
            <Button
              size="small"
              color="error"
              disabled={!data.draft || remove.isPending}
              onClick={() => remove.mutate()}
            >
              Delete draft
            </Button>
          </>
        )}
      </Box>
      {saveError && (
        <Alert severity="error" sx={{ mb: 2 }}>
          {saveError}
        </Alert>
      )}

      <Paper
        variant="outlined"
        sx={{ p: 2, display: "flex", flexDirection: "column", gap: 2 }}
      >
        <Box sx={{ display: "flex", gap: 2 }}>
          <TextField
            select
            label="Target kind"
            size="small"
            value={targetKind}
            onChange={(e) => {
              setTargetKind(e.target.value as OverrideTargetKind);
              setDirty(true);
            }}
            sx={{ width: 180 }}
          >
            {KINDS.map((k) => (
              <MenuItem key={k} value={k}>
                {k}
              </MenuItem>
            ))}
          </TextField>
          <TextField
            label="Target key"
            size="small"
            value={targetKey}
            onChange={(e) => {
              setTargetKey(e.target.value);
              setDirty(true);
            }}
            fullWidth
            slotProps={{
              input: { sx: { fontFamily: "monospace", fontSize: 13 } },
            }}
          />
        </Box>
        <TextField
          label="JSON Pointer"
          size="small"
          value={valuePath}
          onChange={(e) => {
            setValuePath(e.target.value);
            setDirty(true);
          }}
          helperText="RFC 6901 pointer to a scalar leaf — structural nodes can't be replaced"
          fullWidth
          slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
        />
        <TextField
          label="Value"
          size="small"
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
            setDirty(true);
          }}
          helperText="Scalar leaf — 0.125, true, null, or text for a string"
          fullWidth
          slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
        />
        <TextField
          label="Description"
          size="small"
          value={description}
          onChange={(e) => {
            setDescription(e.target.value);
            setDirty(true);
          }}
          fullWidth
        />
      </Paper>

      {data.published && (
        <>
          <Divider sx={{ my: 3 }} />
          <Typography variant="h6" gutterBottom>
            Published
          </Typography>
          <Paper variant="outlined" sx={{ p: 2 }}>
            <Typography variant="body2" color="text.secondary">
              <Chip
                label={data.published.targetKind}
                size="small"
                variant="outlined"
                sx={{ mr: 1 }}
              />
              <Box
                component="span"
                sx={{ fontFamily: "monospace", fontSize: 12 }}
              >
                {data.published.targetKey}
              </Box>{" "}
              at{" "}
              <Box
                component="span"
                sx={{ fontFamily: "monospace", fontSize: 12 }}
              >
                {data.published.valuePath}
              </Box>{" "}
              ={" "}
              <Box
                component="span"
                sx={{ fontFamily: "monospace", fontSize: 12 }}
              >
                {formatValue(data.published.value)}
              </Box>
            </Typography>
            {data.published.publishedAt && (
              <Typography
                variant="caption"
                color="text.secondary"
                component="div"
                sx={{ mt: 1 }}
              >
                {new Date(data.published.publishedAt).toLocaleString()} ·{" "}
                <Chip
                  label={data.published.env}
                  size="small"
                  variant="outlined"
                />
              </Typography>
            )}
          </Paper>
        </>
      )}

      {data.history.length > 0 && (
        <>
          <Divider sx={{ my: 3 }} />
          <Typography variant="h6" gutterBottom>
            History
          </Typography>
          {data.history.map((h) => (
            <Typography
              key={h.publicationId}
              variant="body2"
              color="text.secondary"
            >
              <Chip label={h.env} size="small" variant="outlined" sx={{ mr: 1 }} />
              {h.publicationLabel ?? h.publicationId} —{" "}
              {new Date(h.publishedAt).toLocaleString()}
            </Typography>
          ))}
        </>
      )}
      {data.history.length === 0 && (
        <Chip
          label="never published"
          size="small"
          variant="outlined"
          sx={{ mt: 2 }}
        />
      )}
    </>
  );
}
