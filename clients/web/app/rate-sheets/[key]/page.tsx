"use client";

import Typography from "@mui/material/Typography";
import Paper from "@mui/material/Paper";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import TextField from "@mui/material/TextField";
import Divider from "@mui/material/Divider";
import Chip from "@mui/material/Chip";
import IconButton from "@mui/material/IconButton";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import AddIcon from "@mui/icons-material/Add";
import DeleteIcon from "@mui/icons-material/Delete";
import { useParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { endpoints, type RateSheetSpec } from "@/lib/api";
import {
  nextUid,
  rateSheetDraftToSpec,
  specToRateSheetDraft,
  type GridValue,
} from "@/lib/specs";
import StatusChip from "@/components/StatusChip";

export default function RateSheetDetailPage() {
  const { key: rawKey } = useParams<{ key: string }>();
  const key = decodeURIComponent(rawKey);
  const queryClient = useQueryClient();

  const { data, error, isLoading } = useQuery({
    queryKey: ["rateSheet", key],
    queryFn: () => endpoints.rateSheet(key),
  });

  const [rates, setRates] = useState<GridValue[]>([]);
  const [lockTerms, setLockTerms] = useState<GridValue[]>([]);
  const [prices, setPrices] = useState<Record<string, string>>({});
  const [dirty, setDirty] = useState(false);
  const [specErrors, setSpecErrors] = useState<string[]>([]);
  const loadedRef = useRef(false);

  useEffect(() => {
    if (data && !loadedRef.current) {
      loadedRef.current = true;
      const row = data.draft ?? data.published;
      const spec = (row?.spec ?? { rates: [], lockTerms: [], prices: {} }) as RateSheetSpec;
      const draft = specToRateSheetDraft(spec);
      setRates(draft.rates);
      setLockTerms(draft.lockTerms);
      setPrices(draft.prices);
    }
  }, [data]);

  const markDirty = () => setDirty(true);
  const priceKey = (rate: string, term: string) => `${rate}:${term}`;

  // Editing a header value re-keys that row/column's price cells so values
  // follow the header rather than the index.
  const setRate = (i: number, v: string) => {
    const old = rates[i].text;
    setRates(rates.map((r, j) => (j === i ? { ...r, text: v } : r)));
    setPrices((prev) => {
      const out = { ...prev };
      for (const t of lockTerms) {
        const oldK = priceKey(old, t.text);
        if (oldK in out) {
          out[priceKey(v, t.text)] = out[oldK];
          delete out[oldK];
        }
      }
      return out;
    });
    markDirty();
  };

  const setTerm = (i: number, v: string) => {
    const old = lockTerms[i].text;
    setLockTerms(lockTerms.map((t, j) => (j === i ? { ...t, text: v } : t)));
    setPrices((prev) => {
      const out = { ...prev };
      for (const r of rates) {
        const oldK = priceKey(r.text, old);
        if (oldK in out) {
          out[priceKey(r.text, v)] = out[oldK];
          delete out[oldK];
        }
      }
      return out;
    });
    markDirty();
  };

  const addRate = () => {
    setRates([...rates, { uid: nextUid(), text: "" }]);
    markDirty();
  };
  const addTerm = () => {
    setLockTerms([...lockTerms, { uid: nextUid(), text: "" }]);
    markDirty();
  };
  const removeRate = (i: number) => {
    const gone = rates[i].text;
    setRates(rates.filter((_, j) => j !== i));
    setPrices((prev) => {
      const out = { ...prev };
      for (const t of lockTerms) delete out[priceKey(gone, t.text)];
      return out;
    });
    markDirty();
  };
  const removeTerm = (i: number) => {
    const gone = lockTerms[i].text;
    setLockTerms(lockTerms.filter((_, j) => j !== i));
    setPrices((prev) => {
      const out = { ...prev };
      for (const r of rates) delete out[priceKey(r.text, gone)];
      return out;
    });
    markDirty();
  };

  const save = useMutation({
    mutationFn: async () => {
      const { spec, errors } = rateSheetDraftToSpec({ rates, lockTerms, prices });
      if (!spec) {
        setSpecErrors(errors);
        throw new Error("rate sheet spec invalid");
      }
      setSpecErrors([]);
      return endpoints.saveRateSheet(key, { spec });
    },
    onSuccess: async (res) => {
      if (!res || res.status >= 400) return;
      setDirty(false);
      await queryClient.invalidateQueries({ queryKey: ["rateSheet", key] });
      await queryClient.invalidateQueries({ queryKey: ["rateSheets"] });
    },
  });
  const saveError =
    specErrors.length > 0
      ? specErrors.join("; ")
      : save.data && save.data.status >= 400
        ? ((save.data.data as { error?: string }).error ?? `HTTP ${save.data.status}`)
        : save.isError
          ? `${save.error}`
          : null;

  const remove = useMutation({
    mutationFn: () => endpoints.deleteRateSheet(key),
    onSuccess: () => queryClient.invalidateQueries(),
  });

  if (isLoading) return <Typography>Loading…</Typography>;
  if (error) return <Alert severity="error">{`${error}`}</Alert>;
  if (!data) return null;

  const pricedCells = lockTerms.reduce(
    (n, t) =>
      n +
      rates.filter((r) => (prices[priceKey(r.text, t.text)] ?? "").trim() !== "")
        .length,
    0,
  );

  return (
    <>
      <Box sx={{ display: "flex", alignItems: "center", gap: 2, mb: 2 }}>
        <Typography variant="h5" component="h1" sx={{ flexGrow: 1 }}>
          Rate sheet — {key}
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
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Rows are offered note rates; columns are lock terms in days. Prices are
        par-adjusted (100 = par); blank cells are not offered. {pricedCells} of{" "}
        {rates.length * lockTerms.length} cells priced.
      </Typography>

      <TableContainer component={Paper} variant="outlined" sx={{ mb: 2 }}>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell sx={{ fontWeight: 600 }}>rate \ term</TableCell>
              {lockTerms.map((t, i) => (
                <TableCell key={t.uid} align="center">
                  <Box sx={{ display: "flex", alignItems: "center", gap: 0.5 }}>
                    <TextField
                      size="small"
                      value={t.text}
                      onChange={(e) => setTerm(i, e.target.value)}
                      slotProps={{
                        input: {
                          sx: { fontFamily: "monospace", fontSize: 13 },
                          endAdornment: (
                            <Typography variant="caption" color="text.secondary">
                              d
                            </Typography>
                          ),
                        },
                      }}
                      sx={{ width: 76 }}
                    />
                    <IconButton
                      size="small"
                      color="error"
                      onClick={() => removeTerm(i)}
                    >
                      <DeleteIcon fontSize="small" />
                    </IconButton>
                  </Box>
                </TableCell>
              ))}
              <TableCell>
                <IconButton size="small" onClick={addTerm} title="Add lock term">
                  <AddIcon fontSize="small" />
                </IconButton>
              </TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {rates.map((r, i) => (
              <TableRow key={r.uid}>
                <TableCell>
                  <Box sx={{ display: "flex", alignItems: "center", gap: 0.5 }}>
                    <TextField
                      size="small"
                      value={r.text}
                      onChange={(e) => setRate(i, e.target.value)}
                      placeholder="6.50"
                      slotProps={{
                        input: {
                          sx: { fontFamily: "monospace", fontSize: 13 },
                          endAdornment: (
                            <Typography variant="caption" color="text.secondary">
                              %
                            </Typography>
                          ),
                        },
                      }}
                      sx={{ width: 92 }}
                    />
                    <IconButton
                      size="small"
                      color="error"
                      onClick={() => removeRate(i)}
                    >
                      <DeleteIcon fontSize="small" />
                    </IconButton>
                  </Box>
                </TableCell>
                {lockTerms.map((t) => {
                  const ck = priceKey(r.text, t.text);
                  return (
                    <TableCell key={t.uid} align="center">
                      <TextField
                        size="small"
                        value={prices[ck] ?? ""}
                        onChange={(e) => {
                          setPrices({ ...prices, [ck]: e.target.value });
                          markDirty();
                        }}
                        placeholder="—"
                        slotProps={{
                          input: {
                            sx: { fontFamily: "monospace", fontSize: 13, textAlign: "center" },
                          },
                        }}
                        sx={{ width: 96 }}
                      />
                    </TableCell>
                  );
                })}
                <TableCell />
              </TableRow>
            ))}
            {rates.length === 0 && (
              <TableRow>
                <TableCell colSpan={lockTerms.length + 2}>
                  Add a rate row to start the grid.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </TableContainer>
      <Button size="small" startIcon={<AddIcon />} onClick={addRate}>
        Add rate
      </Button>

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
