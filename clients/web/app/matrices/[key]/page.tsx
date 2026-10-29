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
import IconButton from "@mui/material/IconButton";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import AddIcon from "@mui/icons-material/Add";
import DeleteIcon from "@mui/icons-material/Delete";
import WarningAmberIcon from "@mui/icons-material/WarningAmber";
import { useParams } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import {
  endpoints,
  mutationError,
  type CellMatch,
  type MatrixSpec,
  type SimulateResult,
} from "@/lib/api";
import {
  MATRIX_OPS,
  cellKey,
  matrixDraftToSpec,
  matrixOverlapWarnings,
  nextUid,
  specToMatrixDraft,
  type AxisDraft,
  type CellDrafts,
} from "@/lib/specs";
import StatusChip from "@/components/StatusChip";
import { useMe } from "@/lib/me";

const KINDS = ["llpa", "eligibility", "pricing", "shared"];

const SAMPLE_CONTEXT = `{
  "loan": { "ltv": 78.0, "amount": 320000, "termMonths": 360, "type": "conventional" },
  "borrower": { "fico": 760 },
  "property": { "state": "TX" },
  "scenario": { "lockTermDays": 30 }
}`;

export default function MatrixDetailPage() {
  const { key: rawKey } = useParams<{ key: string }>();
  const key = decodeURIComponent(rawKey);
  const queryClient = useQueryClient();
  const { canEdit } = useMe();

  const { data, error, isLoading } = useQuery({
    queryKey: ["matrix", key],
    queryFn: () => endpoints.matrix(key),
  });

  const [rows, setRows] = useState<AxisDraft[]>([]);
  const [cols, setCols] = useState<AxisDraft[]>([]);
  const [cells, setCells] = useState<CellDrafts>({});
  const [op, setOp] = useState("sum");
  const [kind, setKind] = useState("llpa");
  const [productCode, setProductCode] = useState("");
  const [dirty, setDirty] = useState(false);
  const [contextJson, setContextJson] = useState(SAMPLE_CONTEXT);
  const [simResult, setSimResult] = useState<SimulateResult | null>(null);
  const [simError, setSimError] = useState<string | null>(null);
  const loadedRef = useRef(false);

  useEffect(() => {
    if (data && !loadedRef.current) {
      loadedRef.current = true;
      const row = data.draft ?? data.published;
      const spec = (row?.spec ?? { rows: [], cols: [], cells: {} }) as MatrixSpec;
      const draft = specToMatrixDraft(spec);
      setRows(draft.rows);
      setCols(draft.cols);
      setCells(draft.cells);
      setOp(draft.op);
      setKind(row?.kind ?? "llpa");
      setProductCode(row?.productCode ?? "");
    }
  }, [data]);

  const markDirty = () => setDirty(true);
  const warnings = matrixOverlapWarnings(rows, cols);

  const save = useMutation({
    mutationFn: () =>
      endpoints.saveMatrix(key, {
        kind,
        productCode: productCode.trim() || null,
        spec: matrixDraftToSpec(rows, cols, cells, op),
      }),
    onSuccess: async ({ status }) => {
      if (status >= 400) return;
      setDirty(false);
      await queryClient.invalidateQueries({ queryKey: ["matrix", key] });
      await queryClient.invalidateQueries({ queryKey: ["matrices"] });
    },
  });
  const saveError = mutationError(save);

  const remove = useMutation({
    mutationFn: () => endpoints.deleteMatrix(key),
    onSuccess: () => queryClient.invalidateQueries(),
  });

  const simulate = useMutation({
    mutationFn: async () => {
      setSimError(null);
      let context: unknown;
      try {
        context = JSON.parse(contextJson);
      } catch {
        throw new Error("context is not valid JSON");
      }
      const res = await endpoints.simulateMatrix(
        matrixDraftToSpec(rows, cols, cells, op),
        context,
      );
      if (res.status >= 400) {
        throw new Error((res.data as { error?: string }).error ?? `HTTP ${res.status}`);
      }
      return res.data;
    },
    onSuccess: (r) => setSimResult(r),
    onError: (e) => {
      setSimResult(null);
      setSimError(`${e}`);
    },
  });

  const setAxis = (
    axes: AxisDraft[],
    setAxes: (v: AxisDraft[]) => void,
    i: number,
    patch: Partial<AxisDraft>,
  ) => {
    setAxes(axes.map((a, j) => (j === i ? { ...a, ...patch } : a)));
    markDirty();
  };

  const removeAxis = (
    axes: AxisDraft[],
    setAxes: (v: AxisDraft[]) => void,
    i: number,
    isRow: boolean,
  ) => {
    const gone = axes[i].id;
    setAxes(axes.filter((_, j) => j !== i));
    setCells((prev) => {
      const next = { ...prev };
      for (const k of Object.keys(next)) {
        const [r, c] = k.split(":");
        if ((isRow ? r : c) === gone) delete next[k];
      }
      return next;
    });
    markDirty();
  };

  const addAxis = (
    axes: AxisDraft[],
    setAxes: (v: AxisDraft[]) => void,
    prefix: string,
  ) => {
    let n = axes.length + 1;
    while (axes.some((a) => a.id === `${prefix}${n}`)) n += 1;
    setAxes([...axes, { uid: nextUid(), id: `${prefix}${n}`, label: "", expr: "" }]);
    markDirty();
  };

  if (isLoading) return <Typography>Loading…</Typography>;
  if (error) return <Alert severity="error">{`${error}`}</Alert>;
  if (!data) return null;

  const matchedCount = simResult?.cells.filter((c) => c.matched).length ?? 0;

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
      {warnings.map((w) => (
        <Alert
          key={w}
          severity="warning"
          icon={<WarningAmberIcon fontSize="small" />}
          sx={{ mb: 1 }}
        >
          {w}
        </Alert>
      ))}

      <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
        <Box sx={{ display: "flex", gap: 2, flexWrap: "wrap" }}>
          <TextField
            select
            label="Kind"
            size="small"
            value={kind}
            onChange={(e) => {
              setKind(e.target.value);
              markDirty();
            }}
            sx={{ width: 140 }}
          >
            {KINDS.map((k) => (
              <MenuItem key={k} value={k}>
                {k}
              </MenuItem>
            ))}
          </TextField>
          <TextField
            label="Product code (blank = shared)"
            size="small"
            value={productCode}
            onChange={(e) => {
              setProductCode(e.target.value);
              markDirty();
            }}
            sx={{ width: 240 }}
          />
          <TextField
            select
            label="Aggregation"
            size="small"
            value={op}
            onChange={(e) => {
              setOp(e.target.value);
              markDirty();
            }}
            sx={{ width: 160 }}
          >
            {MATRIX_OPS.map((o) => (
              <MenuItem key={o} value={o}>
                {o}
              </MenuItem>
            ))}
          </TextField>
        </Box>
      </Paper>

      <Typography variant="h6" gutterBottom>
        Row rules
      </Typography>
      <AxisTable axes={rows} setAxes={setRows} isRow remove={removeAxis} set={setAxis} />
      <Box sx={{ mt: 1, mb: 3 }}>
        <Button
          size="small"
          startIcon={<AddIcon />}
          onClick={() => addAxis(rows, setRows, "r")}
        >
          Add row
        </Button>
      </Box>

      <Typography variant="h6" gutterBottom>
        Column rules
      </Typography>
      <AxisTable axes={cols} setAxes={setCols} isRow={false} remove={removeAxis} set={setAxis} />
      <Box sx={{ mt: 1, mb: 3 }}>
        <Button
          size="small"
          startIcon={<AddIcon />}
          onClick={() => addAxis(cols, setCols, "c")}
        >
          Add column
        </Button>
      </Box>

      <Typography variant="h6" gutterBottom>
        Cell values
      </Typography>
      <TableContainer component={Paper} variant="outlined" sx={{ mb: 3 }}>
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell />
              {cols.map((c) => (
                <TableCell key={c.id} align="center">
                  <Typography variant="body2" sx={{ fontWeight: 600 }}>
                    {c.label || c.id}
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    {c.id}
                  </Typography>
                </TableCell>
              ))}
            </TableRow>
          </TableHead>
          <TableBody>
            {rows.map((r) => (
              <TableRow key={r.id}>
                <TableCell>
                  <Typography variant="body2" sx={{ fontWeight: 600 }}>
                    {r.label || r.id}
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    {r.id}
                  </Typography>
                </TableCell>
                {cols.map((c) => {
                  const ck = cellKey(r.id, c.id);
                  const matched = simResult?.cells.find(
                    (m) => m.row === r.id && m.col === c.id,
                  );
                  return (
                    <TableCell
                      key={c.id}
                      align="center"
                      sx={{
                        bgcolor: matchCellColor(matched),
                        opacity: matched && !matched.matched ? 0.5 : 1,
                      }}
                    >
                      <TextField
                        size="small"
                        value={cells[ck] ?? ""}
                        onChange={(e) => {
                          setCells({ ...cells, [ck]: e.target.value });
                          markDirty();
                        }}
                        placeholder="—"
                        slotProps={{
                          input: {
                            sx: { fontFamily: "monospace", fontSize: 13, textAlign: "center" },
                          },
                        }}
                        sx={{ width: 90 }}
                      />
                    </TableCell>
                  );
                })}
              </TableRow>
            ))}
            {rows.length === 0 && (
              <TableRow>
                <TableCell colSpan={cols.length + 1}>
                  Add a row to start building the grid.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </TableContainer>

      <Divider sx={{ my: 2 }} />
      <Typography variant="h6" gutterBottom>
        Simulate
      </Typography>
      <Box sx={{ display: "flex", flexDirection: "column", gap: 1.5 }}>
        <TextField
          label="Context (JSON)"
          value={contextJson}
          onChange={(e) => setContextJson(e.target.value)}
          multiline
          minRows={5}
          fullWidth
          slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
        />
        <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
          <Button
            variant="outlined"
            size="small"
            onClick={() => simulate.mutate()}
            disabled={simulate.isPending}
          >
            Run simulation
          </Button>
          {simResult && (
            <>
              <Chip
                label={`${matchedCount} cell${matchedCount === 1 ? "" : "s"} matched`}
                size="small"
                color={matchedCount > 1 ? "warning" : "success"}
                variant="outlined"
              />
              {matchedCount > 1 && (
                <Typography variant="caption" color="warning.main">
                  overlapping cells — all matched values collect
                </Typography>
              )}
              {simResult.performance && (
                <Typography variant="caption" color="text.secondary">
                  {simResult.performance}
                </Typography>
              )}
            </>
          )}
        </Box>
        {simError && <Alert severity="error">{simError}</Alert>}
        {simResult && (
          <Paper variant="outlined" sx={{ p: 1.5 }}>
            <Typography variant="caption" color="text.secondary">
              Result
            </Typography>
            <Box
              component="pre"
              sx={{ m: 0, fontFamily: "monospace", fontSize: 12, overflow: "auto" }}
            >
              {JSON.stringify(simResult.result, null, 2)}
            </Box>
          </Paper>
        )}
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
      {data.history.length === 0 && (
        <Chip label="never published" size="small" variant="outlined" sx={{ mt: 2 }} />
      )}
    </>
  );
}

function matchCellColor(cell?: CellMatch): string | undefined {
  if (!cell) return undefined;
  if (cell.matched) return "success.dark";
  if (cell.rowMatched || cell.colMatched) return "warning.dark";
  return undefined;
}

function AxisTable({
  axes,
  setAxes,
  isRow,
  remove,
  set,
}: Readonly<{
  axes: AxisDraft[];
  setAxes: (v: AxisDraft[]) => void;
  isRow: boolean;
  remove: (
    axes: AxisDraft[],
    setAxes: (v: AxisDraft[]) => void,
    i: number,
    isRow: boolean,
  ) => void;
  set: (
    axes: AxisDraft[],
    setAxes: (v: AxisDraft[]) => void,
    i: number,
    patch: Partial<AxisDraft>,
  ) => void;
}>) {
  return (
    <TableContainer component={Paper} variant="outlined">
      <Table size="small">
        <TableHead>
          <TableRow>
            <TableCell sx={{ width: 90 }}>Id</TableCell>
            <TableCell sx={{ width: 220 }}>Label</TableCell>
            <TableCell>ZEN rule</TableCell>
            <TableCell sx={{ width: 48 }} />
          </TableRow>
        </TableHead>
        <TableBody>
          {axes.map((a, i) => (
            <TableRow key={a.uid} hover>
              <TableCell>
                <TextField
                  size="small"
                  value={a.id}
                  onChange={(e) => set(axes, setAxes, i, { id: e.target.value })}
                  slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
                />
              </TableCell>
              <TableCell>
                <TextField
                  size="small"
                  value={a.label}
                  onChange={(e) => set(axes, setAxes, i, { label: e.target.value })}
                  fullWidth
                />
              </TableCell>
              <TableCell>
                <TextField
                  size="small"
                  value={a.expr}
                  onChange={(e) => set(axes, setAxes, i, { expr: e.target.value })}
                  placeholder="loan.ltv > 80 and loan.ltv <= 90"
                  fullWidth
                  slotProps={{ input: { sx: { fontFamily: "monospace", fontSize: 13 } } }}
                />
              </TableCell>
              <TableCell>
                <IconButton
                  size="small"
                  color="error"
                  onClick={() => remove(axes, setAxes, i, isRow)}
                >
                  <DeleteIcon fontSize="small" />
                </IconButton>
              </TableCell>
            </TableRow>
          ))}
          {axes.length === 0 && (
            <TableRow>
              <TableCell colSpan={4}>No {isRow ? "rows" : "columns"} yet.</TableCell>
            </TableRow>
          )}
        </TableBody>
      </Table>
    </TableContainer>
  );
}
