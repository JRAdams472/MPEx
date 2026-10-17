"use client";

import Typography from "@mui/material/Typography";
import Paper from "@mui/material/Paper";
import Alert from "@mui/material/Alert";
import Chip from "@mui/material/Chip";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import TextField from "@mui/material/TextField";
import Divider from "@mui/material/Divider";
import type { DecisionGraphType } from "@gorules/jdm-editor";
import { useParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  endpoints,
  mutationError,
  type EligibilityOutcome,
  type EligibilityReport,
} from "@/lib/api";
import DecisionEditor from "@/components/DecisionEditor";
import StatusChip from "@/components/StatusChip";

const SAMPLE_LOAN = `{
  "loan": { "ltv": 78.0, "amount": 320000, "termMonths": 360, "purpose": "purchase", "type": "conventional" },
  "borrower": { "fico": 760 },
  "property": { "type": "sfr", "occupancy": "primary", "state": "TX" },
  "scenario": { "lockTermDays": 30 }
}`;

export default function DecisionDetailPage() {
  const { key: rawKey } = useParams<{ key: string }>();
  const key = decodeURIComponent(rawKey);
  const queryClient = useQueryClient();

  const { data, error, isLoading } = useQuery({
    queryKey: ["decision", key],
    queryFn: () => endpoints.decision(key),
  });

  // `edited` holds unsaved changes; absent edits, the graph derives from the
  // fetched draft (or latest published when no draft exists).
  const [edited, setEdited] = useState<DecisionGraphType | null>(null);
  const doc = data?.draft?.jdm ?? data?.published?.jdm;
  const graph =
    edited ?? (doc ? ({ nodes: doc.nodes, edges: doc.edges } as DecisionGraphType) : null);
  const dirty = edited !== null;

  const save = useMutation({
    mutationFn: () =>
      endpoints.saveDecision(key, {
        kind: data?.draft?.kind ?? data?.published?.kind ?? "shared",
        productCode: data?.draft?.productCode ?? data?.published?.productCode ?? null,
        jdm: graph,
      }),
    onSuccess: async (res) => {
      if (res.status >= 400) return;
      setEdited(null);
      await queryClient.invalidateQueries({ queryKey: ["decision", key] });
      await queryClient.invalidateQueries({ queryKey: ["decisions"] });
    },
  });
  const saveError = mutationError(save);

  const remove = useMutation({
    mutationFn: () => endpoints.deleteDecision(key),
    onSuccess: async () => {
      setEdited(null);
      await queryClient.invalidateQueries();
    },
  });

  const [loanJson, setLoanJson] = useState(SAMPLE_LOAN);
  const [evalResult, setEvalResult] = useState<EligibilityReport | null>(null);
  const evaluate = useMutation({
    mutationFn: async () => {
      const loan = JSON.parse(loanJson);
      // "draft" revision — the saved draft is what the user is editing.
      const res = await endpoints.evaluateEligibility(loan, "draft");
      if (res.status >= 400) throw new Error(`evaluate failed: HTTP ${res.status}`);
      return res.data;
    },
    onSuccess: setEvalResult,
  });

  if (isLoading) return <Typography>Loading…</Typography>;
  if (error) return <Alert severity="error">{`${error}`}</Alert>;
  if (!data) return null;

  const status = data.draft ? "draft" : "published";
  const productCode = key.startsWith("eligibility.")
    ? key.slice("eligibility.".length)
    : null;
  const outcome: EligibilityOutcome | undefined = evalResult
    ? [...evalResult.eligible, ...evalResult.ineligible].find(
        (o) => !productCode || o.product.code === productCode,
      )
    : undefined;

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", gap: 2, mb: 2 }}>
        <Typography variant="h5" component="h1" sx={{ flexGrow: 1 }}>
          {key}
        </Typography>
        <StatusChip status={status} hasPublished={Boolean(data.published)} />
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

      {graph ? (
        <DecisionEditor
          value={graph}
          onChange={setEdited}
          disabled={!data.draft && !data.published}
        />
      ) : (
        <Alert severity="info">No draft or published content for this key.</Alert>
      )}
      {!data.draft && data.published && (
        <Typography variant="caption" color="text.secondary" sx={{ mt: 1, display: "block" }}>
          Viewing the latest published revision — saving creates a new draft.
        </Typography>
      )}

      <Divider sx={{ my: 3 }} />
      <Typography variant="h6" gutterBottom>
        Evaluate (draft revision)
      </Typography>
      <Box sx={{ display: "flex", gap: 2, alignItems: "flex-start" }}>
        <TextField
          label="Loan scenario (JSON)"
          multiline
          minRows={8}
          value={loanJson}
          onChange={(e) => setLoanJson(e.target.value)}
          sx={{ flex: 1, fontFamily: "monospace" }}
          slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
        />
        <Box sx={{ width: 380 }}>
          <Button
            variant="outlined"
            onClick={() => evaluate.mutate()}
            disabled={evaluate.isPending}
          >
            Run eligibility
          </Button>
          {evaluate.isError && (
            <Alert severity="error" sx={{ mt: 1 }}>{`${evaluate.error}`}</Alert>
          )}
          {outcome && (
            <Paper variant="outlined" sx={{ p: 2, mt: 1 }}>
              <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
                <Typography variant="subtitle2">{outcome.product.code}</Typography>
                <Chip
                  label={outcome.eligible ? "eligible" : "ineligible"}
                  size="small"
                  color={outcome.eligible ? "success" : "error"}
                />
                {outcome.performance && (
                  <Typography variant="caption" color="text.secondary">
                    {outcome.performance}
                  </Typography>
                )}
              </Box>
              {outcome.reasons.length > 0 && (
                <Box component="ul" sx={{ m: 0, mt: 1, pl: 2 }}>
                  {outcome.reasons.map((r) => (
                    <li key={JSON.stringify(r)}>
                      <Typography variant="body2">{String(r)}</Typography>
                    </li>
                  ))}
                </Box>
              )}
            </Paper>
          )}
        </Box>
      </Box>

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
    </>
  );
}
