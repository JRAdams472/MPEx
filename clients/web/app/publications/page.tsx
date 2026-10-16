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
import { useQueries, useQuery } from "@tanstack/react-query";
import { useState } from "react";

import { endpoints, type Publication } from "@/lib/api";

const KIND_LABELS: [keyof Publication, string][] = [
  ["decisions", "Decisions"],
  ["expressions", "Expressions"],
  ["matrices", "Matrices"],
  ["rateSheets", "Rate sheets"],
];

export default function PublicationsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["publications"],
    queryFn: endpoints.publications,
  });
  const [openId, setOpenId] = useState<string | null>(null);
  // "" = working drafts; otherwise a publication's publishedAt timestamp.
  const [asOf, setAsOf] = useState("");

  const asOfParam = asOf || undefined;
  const [decisions, expressions, matrices, rateSheets] = useQueries({
    queries: [
      {
        queryKey: ["decisions", asOfParam ?? "draft"],
        queryFn: () => endpoints.decisions(asOfParam),
      },
      {
        queryKey: ["expressions", asOfParam ?? "draft"],
        queryFn: () => endpoints.expressions(asOfParam),
      },
      {
        queryKey: ["matrices", asOfParam ?? "draft"],
        queryFn: () => endpoints.matrices(asOfParam),
      },
      {
        queryKey: ["rateSheets", asOfParam ?? "draft"],
        queryFn: () => endpoints.rateSheets(asOfParam),
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
              <TableCell>Status</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={8}>Loading…</TableCell>
              </TableRow>
            )}
            {data?.publications.map((p) => (
              <PublicationRow
                key={p.id}
                publication={p}
                open={openId === p.id}
                onToggle={() => setOpenId(openId === p.id ? null : p.id)}
              />
            ))}
            {data && data.publications.length === 0 && (
              <TableRow>
                <TableCell colSpan={8}>
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
      <TextField
        select
        label="Revision"
        size="small"
        value={asOf}
        onChange={(e) => setAsOf(e.target.value)}
        sx={{ minWidth: 320, mb: 2 }}
      >
        <MenuItem value="">Working drafts</MenuItem>
        {data?.publications.map((p) => (
          <MenuItem key={p.id} value={p.publishedAt}>
            {p.label} — {new Date(p.publishedAt).toLocaleString()}
          </MenuItem>
        ))}
      </TextField>

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
    </>
  );
}

function PublicationRow({
  publication,
  open,
  onToggle,
}: {
  publication: Publication;
  open: boolean;
  onToggle: () => void;
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
          <Chip
            label={publication.status}
            size="small"
            color="success"
            variant="outlined"
          />
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell colSpan={8} sx={{ p: 0, borderBottom: open ? 1 : 0 }}>
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
