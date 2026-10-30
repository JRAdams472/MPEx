"use client";

import Typography from "@mui/material/Typography";
import Paper from "@mui/material/Paper";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import Alert from "@mui/material/Alert";
import Chip from "@mui/material/Chip";
import Box from "@mui/material/Box";
import Collapse from "@mui/material/Collapse";
import IconButton from "@mui/material/IconButton";
import TextField from "@mui/material/TextField";
import MenuItem from "@mui/material/MenuItem";
import Divider from "@mui/material/Divider";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import Button from "@mui/material/Button";
import Dialog from "@mui/material/Dialog";
import DialogActions from "@mui/material/DialogActions";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import Stack from "@mui/material/Stack";
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  endpoints,
  mutationError,
  NEXT_ENV,
  ENVS,
  type Env,
  type Publication,
} from "@/lib/api";

const KIND_LABELS: [keyof Publication, string][] = [
  ["decisions", "Decisions"],
  ["expressions", "Expressions"],
  ["matrices", "Matrices"],
  ["rateSheets", "Rate sheets"],
];

const ENV_COLORS: Record<Env, "default" | "info" | "secondary" | "success"> = {
  dev: "default",
  qa: "info",
  uat: "secondary",
  prod: "success",
};

function EnvChip({ env }: { env: Env }) {
  return (
    <Chip label={env} size="small" color={ENV_COLORS[env]} variant="outlined" />
  );
}

export default function PublicationsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["publications"],
    queryFn: endpoints.publications,
  });
  const [openId, setOpenId] = useState<string | null>(null);
  // "" = working drafts; otherwise a publication's publishedAt timestamp.
  const [asOf, setAsOf] = useState("");
  // Lane the snapshot view resolves in (MPE-153).
  const [snapshotEnv, setSnapshotEnv] = useState<Env>("dev");
  const [promoteFrom, setPromoteFrom] = useState<Env | null>(null);

  const asOfParam = asOf || undefined;
  const [decisions, expressions, matrices, rateSheets] = useQueries({
    queries: [
      {
        queryKey: ["decisions", asOfParam ?? "draft", snapshotEnv],
        queryFn: () => endpoints.decisions(asOfParam, snapshotEnv),
      },
      {
        queryKey: ["expressions", asOfParam ?? "draft", snapshotEnv],
        queryFn: () => endpoints.expressions(asOfParam, snapshotEnv),
      },
      {
        queryKey: ["matrices", asOfParam ?? "draft", snapshotEnv],
        queryFn: () => endpoints.matrices(asOfParam, snapshotEnv),
      },
      {
        queryKey: ["rateSheets", asOfParam ?? "draft", snapshotEnv],
        queryFn: () => endpoints.rateSheets(asOfParam, snapshotEnv),
      },
    ],
  });

  const snapshotSets: [string, { key: string }[] | undefined][] = [
    ["Decisions", decisions.data?.decisions],
    ["Expressions", expressions.data?.expressions],
    ["Matrices", matrices.data?.matrices],
    ["Rate sheets", rateSheets.data?.rateSheets],
  ];

  return (
    <>
      <Typography variant="h4" component="h1" gutterBottom>
        Publications
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Each publication is a named point-in-time snapshot. Expand a row to see
        exactly which artifacts it stamped — the diff vs the prior publish —
        then evaluate historic revisions by passing its timestamp as `asOf`.
      </Typography>
      {error && <Alert severity="error">{`${error}`}</Alert>}
      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell sx={{ width: 32 }} />
              <TableCell>Label</TableCell>
              <TableCell>Published at</TableCell>
              <TableCell align="right">Decisions</TableCell>
              <TableCell align="right">Expressions</TableCell>
              <TableCell align="right">Matrices</TableCell>
              <TableCell align="right">Rate sheets</TableCell>
              <TableCell>Env</TableCell>
              <TableCell>Status</TableCell>
              <TableCell />
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={10}>Loading…</TableCell>
              </TableRow>
            )}
            {data?.publications.map((p) => (
              <PublicationRow
                key={p.id}
                publication={p}
                open={openId === p.id}
                onToggle={() => setOpenId(openId === p.id ? null : p.id)}
                onPromote={() => setPromoteFrom(p.env)}
              />
            ))}
            {data && data.publications.length === 0 && (
              <TableRow>
                <TableCell colSpan={10}>
                  No publications yet — use Publish in the header.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </TableContainer>

      <Divider sx={{ my: 3 }} />
      <Typography variant="h6" gutterBottom>
        Snapshot as of…
      </Typography>
      <Stack direction="row" spacing={2} sx={{ mb: 2 }}>
        <TextField
          select
          label="Environment"
          size="small"
          value={snapshotEnv}
          onChange={(e) => setSnapshotEnv(e.target.value as Env)}
          sx={{ minWidth: 160 }}
        >
          {ENVS.map((e) => (
            <MenuItem key={e} value={e}>
              {e}
            </MenuItem>
          ))}
        </TextField>
        <TextField
          select
          label="Revision"
          size="small"
          value={asOf}
          onChange={(e) => setAsOf(e.target.value)}
          sx={{ minWidth: 320 }}
        >
          <MenuItem value="">Working drafts</MenuItem>
          {data?.publications
            .filter((p) => p.env === snapshotEnv)
            .map((p) => (
              <MenuItem key={p.id} value={p.publishedAt}>
                {p.label} — {new Date(p.publishedAt).toLocaleString()}
              </MenuItem>
            ))}
        </TextField>
      </Stack>

      <Box sx={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 2 }}>
        {snapshotSets.map(([label, items]) => (
          <Paper key={label} variant="outlined" sx={{ p: 2 }}>
            <Typography variant="subtitle2" gutterBottom>
              {label}
              {items !== undefined && (
                <Typography component="span" color="text.secondary">
                  {" "}
                  ({items.length})
                </Typography>
              )}
            </Typography>
            {items === undefined && (
              <Typography variant="body2" color="text.secondary">
                Loading…
              </Typography>
            )}
            {items?.length === 0 && (
              <Typography variant="body2" color="text.secondary">
                None at this revision.
              </Typography>
            )}
            {items?.map((a) => (
              <Typography
                key={a.key}
                variant="body2"
                sx={{ fontFamily: "monospace" }}
              >
                {a.key}
              </Typography>
            ))}
          </Paper>
        ))}
      </Box>
      {promoteFrom && (
        <PromoteDialog from={promoteFrom} onClose={() => setPromoteFrom(null)} />
      )}
    </>
  );
}

function PublicationRow({
  publication,
  open,
  onToggle,
  onPromote,
}: {
  publication: Publication;
  open: boolean;
  onToggle: () => void;
  onPromote: () => void;
}) {
  const detail = useQuery({
    queryKey: ["publication", publication.id],
    queryFn: () => endpoints.publication(publication.id),
    enabled: open,
  });

  return (
    <>
      <TableRow hover onClick={onToggle} sx={{ cursor: "pointer" }}>
        <TableCell>
          <IconButton size="small">
            {open ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </IconButton>
        </TableCell>
        <TableCell sx={{ fontWeight: 600 }}>{publication.label}</TableCell>
        <TableCell>
          {new Date(publication.publishedAt).toLocaleString()}
        </TableCell>
        <TableCell align="right">{publication.decisionCount ?? "—"}</TableCell>
        <TableCell align="right">{publication.expressionCount ?? "—"}</TableCell>
        <TableCell align="right">{publication.matrixCount ?? "—"}</TableCell>
        <TableCell align="right">{publication.rateSheetCount ?? "—"}</TableCell>
        <TableCell>
          <EnvChip env={publication.env} />
        </TableCell>
        <TableCell>
          <Chip
            label={publication.status}
            size="small"
            color="success"
            variant="outlined"
          />
        </TableCell>
        <TableCell>
          {NEXT_ENV[publication.env] && (
            <Button
              size="small"
              variant="outlined"
              onClick={(e) => {
                e.stopPropagation();
                onPromote();
              }}
            >
              Promote → {NEXT_ENV[publication.env]}
            </Button>
          )}
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={10} sx={{ p: 0, borderBottom: open ? 1 : 0 }}>
          <Collapse in={open} unmountOnExit>
            <Box sx={{ px: 3, py: 2 }}>
              <Typography variant="subtitle2" gutterBottom>
                Stamped in this publication (the diff vs the prior publish)
              </Typography>
              {detail.isLoading && (
                <Typography variant="body2" color="text.secondary">
                  Loading…
                </Typography>
              )}
              {detail.error && (
                <Alert severity="error">{`${detail.error}`}</Alert>
              )}
              {detail.data && (
                <Box
                  sx={{
                    display: "grid",
                    gridTemplateColumns: "repeat(4, 1fr)",
                    gap: 2,
                  }}
                >
                  {KIND_LABELS.map(([k, label]) => {
                    const keys = detail.data?.[k] as string[] | undefined;
                    return (
                      <Box key={k}>
                        <Typography variant="caption" color="text.secondary">
                          {label} ({keys?.length ?? 0})
                        </Typography>
                        {(keys ?? []).map((key) => (
                          <Typography
                            key={key}
                            variant="body2"
                            sx={{ fontFamily: "monospace" }}
                          >
                            {key}
                          </Typography>
                        ))}
                        {keys?.length === 0 && (
                          <Typography variant="body2" color="text.secondary">
                            —
                          </Typography>
                        )}
                      </Box>
                    );
                  })}
                </Box>
              )}
            </Box>
          </Collapse>
        </TableCell>
      </TableRow>
    </>
  );
}

/** Promote one lane forward (MPE-153): `from` is fixed by the row's lane and
 *  `to` is the adjacent lane — the API enforces adjacency, so the dialog only
 *  collects a label and surfaces 400/409 cleanly. */
function PromoteDialog({
  from,
  onClose,
}: {
  from: Env;
  onClose: () => void;
}) {
  const to = NEXT_ENV[from];
  const queryClient = useQueryClient();
  const [label, setLabel] = useState(`${from}-to-${to}`);
  const [result, setResult] = useState<Publication | null>(null);

  const promote = useMutation({
    mutationFn: () => endpoints.promote(from, to!, label.trim()),
    onSuccess: async (res) => {
      if (res.status < 400) {
        setResult(res.data);
        await queryClient.invalidateQueries({ queryKey: ["publications"] });
      }
    },
  });
  const error = mutationError(promote);

  if (!to) return null;
  const counts: [keyof Publication, string][] = [
    ["decisions", "decisions"],
    ["expressions", "expressions"],
    ["matrices", "matrices"],
    ["rateSheets", "rate sheets"],
  ];

  return (
    <Dialog open onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle>
        Promote <EnvChip env={from} /> → <EnvChip env={to} />
      </DialogTitle>
      <DialogContent>
        {result ? (
          <Alert severity="success" sx={{ mt: 1 }}>
            Promoted to {result.env} under {result.label ?? "publication"} —{" "}
            {counts
              .map(([k, l]) => `${(result[k] as string[])?.length ?? 0} ${l}`)
              .join(", ")}
            .
          </Alert>
        ) : (
          <>
            <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>
              Copies the latest published artifacts in {from} into {to} — only
              changed artifacts are stamped, in one transaction.
            </Typography>
            <TextField
              autoFocus
              fullWidth
              size="small"
              label="Promotion label"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              sx={{ mt: 2 }}
            />
            {error && (
              <Alert severity="error" sx={{ mt: 1 }}>
                {error}
              </Alert>
            )}
          </>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>{result ? "Close" : "Cancel"}</Button>
        {!result && (
          <Button
            variant="contained"
            disabled={promote.isPending || !label.trim()}
            onClick={() => promote.mutate()}
          >
            Promote
          </Button>
        )}
      </DialogActions>
    </Dialog>
  );
}
