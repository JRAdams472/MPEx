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
import { useQuery } from "@tanstack/react-query";

import { endpoints } from "@/lib/api";

export default function PublicationsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["publications"],
    queryFn: endpoints.publications,
  });

  return (
    <>
      <Typography variant="h4" component="h1" gutterBottom>
        Publications
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Each publication is a named point-in-time snapshot — evaluate historic
        revisions by passing its timestamp as `asOf`.
      </Typography>
      {error && <Alert severity="error">{`${error}`}</Alert>}
      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Label</TableCell>
              <TableCell>Published at</TableCell>
              <TableCell align="right">Decisions</TableCell>
              <TableCell align="right">Expressions</TableCell>
              <TableCell>Status</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={5}>Loading…</TableCell>
              </TableRow>
            )}
            {data?.publications.map((p) => (
              <TableRow key={p.id} hover>
                <TableCell sx={{ fontWeight: 600 }}>{p.label}</TableCell>
                <TableCell>{new Date(p.publishedAt).toLocaleString()}</TableCell>
                <TableCell align="right">{p.decisionCount ?? "—"}</TableCell>
                <TableCell align="right">{p.expressionCount ?? "—"}</TableCell>
                <TableCell>
                  <Chip label={p.status} size="small" color="success" variant="outlined" />
                </TableCell>
              </TableRow>
            ))}
            {data && data.publications.length === 0 && (
              <TableRow>
                <TableCell colSpan={5}>
                  No publications yet — use Publish in the header.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </TableContainer>
    </>
  );
}
