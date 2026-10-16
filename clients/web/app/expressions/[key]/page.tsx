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
import CheckCircleIcon from "@mui/icons-material/CheckCircle";
import ErrorIcon from "@mui/icons-material/Error";
import { useParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { endpoints } from "@/lib/api";
import StatusChip from "@/components/StatusChip";

const KINDS = ["standard", "unary"];

type Validation = { state: "idle" } | { state: "ok" } | { state: "error"; error: string };

export default function ExpressionDetailPage() {
  const { key: rawKey } = useParams<{ key: string }>();
  const key = decodeURIComponent(rawKey);
  const queryClient = useQueryClient();

  const { data, error, isLoading } = useQuery({
    queryKey: ["expression", key],
    queryFn: () => endpoints.expression(key),
  });

  const [expression, setExpression] = useState("");
  const [kind, setKind] = useState("standard");
  const [description, setDescription] = useState("");
  const [dirty, setDirty] = useState(false);
  const [validation, setValidation] = useState<Validation>({ state: "idle" });
  const loadedRef = useRef(false);

  useEffect(() => {
    if (data && !loadedRef.current) {
      loadedRef.current = true;
      const row = data.draft ?? data.published;
      setExpression(row?.expression ?? "");
      setKind(row?.kind ?? "standard");
      setDescription(row?.description ?? "");
    }
  }, [data]);

  // Validate-on-save path, debounced on edit so the editor shows live feedback.
  const validateNow = async (expr: string, k: string) => {
    if (expr.trim() === "") {
      setValidation({ state: "idle" });
      return;
    }
    const res = await endpoints.validateExpression(expr, k);
    setValidation(
      res.data.ok
        ? { state: "ok" }
        : { state: "error", error: res.data.error ?? "invalid" },
    );
  };
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onExprChange = (v: string) => {
    setExpression(v);
    setDirty(true);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => void validateNow(v, kind), 400);
  };

  const save = useMutation({
    mutationFn: async () => {
      // Re-validate at save time regardless of debounce state.
      const v = await endpoints.validateExpression(expression, kind);
      if (!v.data.ok) {
        setValidation({ state: "error", error: v.data.error ?? "invalid" });
        throw new Error(`expression invalid: ${v.data.error}`);
      }
      return endpoints.saveExpression(key, { expression, kind, description });
    },
    onSuccess: async (res) => {
      setValidation({ state: "ok" });
      if (res.status >= 400) return;
      setDirty(false);
      await queryClient.invalidateQueries({ queryKey: ["expression", key] });
      await queryClient.invalidateQueries({ queryKey: ["expressions"] });
    },
  });
  const saveError =
    save.data && save.data.status >= 400
      ? ((save.data.data as { error?: string }).error ?? `HTTP ${save.data.status}`)
      : save.isError
        ? `${save.error}`
        : null;

  const remove = useMutation({
    mutationFn: () => endpoints.deleteExpression(key),
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
        <Typography variant="h5" component="h1" sx={{ flexGrow: 1 }}>
          {key}
        </Typography>
        <StatusChip
          status={data.draft ? "draft" : "published"}
          hasPublished={Boolean(data.published)}
        />
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
      </Box>
      {saveError && (
        <Alert severity="error" sx={{ mb: 2 }}>
          {saveError}
        </Alert>
      )}

      <Paper variant="outlined" sx={{ p: 2, display: "flex", flexDirection: "column", gap: 2 }}>
        <Box sx={{ display: "flex", gap: 2 }}>
          <TextField
            select
            label="Kind"
            size="small"
            value={kind}
            onChange={(e) => {
              setKind(e.target.value);
              setDirty(true);
            }}
            sx={{ width: 160 }}
          >
            {KINDS.map((k) => (
              <MenuItem key={k} value={k}>
                {k}
              </MenuItem>
            ))}
          </TextField>
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
        </Box>
        <TextField
          label="ZEN expression"
          value={expression}
          onChange={(e) => onExprChange(e.target.value)}
          multiline
          minRows={2}
          fullWidth
          slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 14 } } }}
        />
        <Box sx={{ display: "flex", alignItems: "center", gap: 1, minHeight: 24 }}>
          {validation.state === "ok" && (
            <>
              <CheckCircleIcon color="success" fontSize="small" />
              <Typography variant="body2" color="success.main">
                Compiles ({kind})
              </Typography>
            </>
          )}
          {validation.state === "error" && (
            <>
              <ErrorIcon color="error" fontSize="small" />
              <Typography variant="body2" color="error.main">
                {validation.error}
              </Typography>
            </>
          )}
        </Box>
      </Paper>

      {data.history.length > 0 && (
        <>
          <Divider sx={{ my: 3 }} />
          <Typography variant="h6" gutterBottom>
            History
          </Typography>
          {data.history.map((h) => (
            <Typography key={h.publicationId} variant="body2" color="text.secondary">
              {h.publicationLabel ?? h.publicationId} —{" "}
              {new Date(h.publishedAt).toLocaleString()}
            </Typography>
          ))}
        </>
      )}
      {data.history.length === 0 && (
        <Chip label="never published" size="small" variant="outlined" sx={{ mt: 2 }} />
      )}
    </>
  );
}
