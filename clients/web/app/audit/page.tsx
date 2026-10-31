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
import Button from "@mui/material/Button";
import IconButton from "@mui/material/IconButton";
import Tooltip from "@mui/material/Tooltip";
import TextField from "@mui/material/TextField";
import MenuItem from "@mui/material/MenuItem";
import Collapse from "@mui/material/Collapse";
import Skeleton from "@mui/material/Skeleton";
import KeyboardArrowDownIcon from "@mui/icons-material/KeyboardArrowDown";
import KeyboardArrowUpIcon from "@mui/icons-material/KeyboardArrowUp";
import NavigateBeforeIcon from "@mui/icons-material/NavigateBefore";
import NavigateNextIcon from "@mui/icons-material/NavigateNext";
import VerifiedUserIcon from "@mui/icons-material/VerifiedUser";
import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";

import {
  endpoints,
  AUDIT_ACTIONS,
  AUDIT_TABLES,
  type AuditEvent,
  type AuditVerify,
} from "@/lib/api";
import { useMe } from "@/lib/me";

const PAGE_SIZES = [25, 50, 100] as const;

interface Filters {
  actor: string;
  action: string;
  table: string;
  key: string;
  from: string;
  to: string;
}

const EMPTY_FILTERS: Filters = {
  actor: "",
  action: "",
  table: "",
  key: "",
  from: "",
  to: "",
};

/** datetime-local value -> RFC3339 for the API; `end` bumps to 23:59:59. */
function toRfc3339(local: string, end: boolean): string | undefined {
  if (!local) return undefined;
  const d = new Date(local);
  if (end) d.setSeconds(59, 999);
  return d.toISOString();
}

function EventRow({ event }: { event: AuditEvent }) {
  const [open, setOpen] = useState(false);
  const actor =
    (event.detail.actorEmail as string | undefined) ?? event.actorSub;
  return (
    <>
      <TableRow hover>
        <TableCell padding="checkbox">
          <IconButton
            size="small"
            aria-label="expand detail"
            onClick={() => setOpen((o) => !o)}
          >
            {open ? <KeyboardArrowUpIcon /> : <KeyboardArrowDownIcon />}
          </IconButton>
        </TableCell>
        <TableCell sx={{ whiteSpace: "nowrap" }}>
          {new Date(event.occurredAt).toLocaleString()}
        </TableCell>
        <TableCell>
          <Tooltip title={`sub: ${event.actorSub}`}>
            <span>{actor}</span>
          </Tooltip>
        </TableCell>
        <TableCell>
          <Chip label={event.action} size="small" variant="outlined" />
        </TableCell>
        <TableCell>
          {event.artifactTable ? (
            <span>
              {event.artifactTable}
              {event.artifactKey ? (
                <>
                  {" / "}
                  <code>{event.artifactKey}</code>
                </>
              ) : null}
            </span>
          ) : (
            "—"
          )}
        </TableCell>
      </TableRow>
      <TableRow>
        <TableCell sx={{ py: 0, borderBottom: open ? undefined : 0 }} colSpan={5}>
          <Collapse in={open} timeout="auto" unmountOnExit>
            <Box sx={{ py: 1.5 }}>
              <Typography variant="caption" color="text.secondary">
                detail
              </Typography>
              <Box
                component="pre"
                sx={{
                  m: 0,
                  p: 1,
                  fontSize: 12,
                  overflow: "auto",
                  bgcolor: "action.hover",
                  borderRadius: 1,
                }}
              >
                {JSON.stringify(event.detail, null, 2)}
              </Box>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ display: "block", mt: 0.5 }}
              >
                prev{" "}
                <Tooltip title={event.prevHash || "(genesis)"}>
                  <code>{event.prevHash ? event.prevHash.slice(0, 16) : "∅"}</code>
                </Tooltip>{" "}
                → hash{" "}
                <Tooltip title={event.hash}>
                  <code>{event.hash.slice(0, 16)}…</code>
                </Tooltip>
              </Typography>
            </Box>
          </Collapse>
        </TableCell>
      </TableRow>
    </>
  );
}

function VerifyResult({ result }: { result: AuditVerify }) {
  if (result.ok) {
    return (
      <Alert severity="success" icon={<VerifiedUserIcon />}>
        Chain intact — {result.checked} rows verified.
        <Typography component="span" variant="caption" sx={{ display: "block" }}>
          {result.scope === "incremental"
            ? `Incremental pass from checkpoint #${result.watermarkId}.`
            : "Full scan from genesis."}
        </Typography>
      </Alert>
    );
  }
  return (
    <Alert severity="error">
      Chain broken at row {result.firstBadId}: {result.reason} ({result.checked}{" "}
      rows checked before the break).
    </Alert>
  );
}

export default function AuditPage() {
  const { isAdmin } = useMe();
  const [filters, setFilters] = useState<Filters>(EMPTY_FILTERS);
  const [pageSize, setPageSize] = useState<number>(50);
  const [offset, setOffset] = useState(0);
  const [verify, setVerify] = useState<AuditVerify | null>(null);
  const [verifyError, setVerifyError] = useState<string | null>(null);
  const [verifying, setVerifying] = useState(false);

  const query = useMemo(
    () => ({
      actor: filters.actor || undefined,
      action: filters.action || undefined,
      table: filters.table || undefined,
      key: filters.key || undefined,
      from: toRfc3339(filters.from, false),
      to: toRfc3339(filters.to, true),
      limit: pageSize + 1, // one extra row detects a next page
      offset,
    }),
    [filters, pageSize, offset],
  );

  const { data, isLoading, error } = useQuery({
    queryKey: ["audit-events", query],
    queryFn: () => endpoints.auditEvents(query),
    enabled: isAdmin,
    placeholderData: (prev) => prev,
  });

  const rows = data?.events.slice(0, pageSize) ?? [];
  const hasNext = (data?.events.length ?? 0) > pageSize;

  const set = (k: keyof Filters) => (e: { target: { value: string } }) => {
    setFilters((f) => ({ ...f, [k]: e.target.value }));
    setOffset(0);
  };

  const runVerify = async () => {
    setVerifying(true);
    setVerifyError(null);
    try {
      setVerify(await endpoints.auditVerify());
    } catch (e) {
      setVerifyError(`${e}`);
    } finally {
      setVerifying(false);
    }
  };

  if (!isAdmin) {
    return (
      <Alert severity="warning">
        The audit trail requires the admin role.
      </Alert>
    );
  }
  if (error) return <Alert severity="error">{`${error}`}</Alert>;

  const filterField = (
    label: string,
    k: keyof Filters,
    options?: readonly string[],
  ) => (
    <TextField
      key={k}
      size="small"
      select={!!options}
      label={label}
      value={filters[k]}
      onChange={set(k)}
      sx={{ minWidth: options ? 200 : 180 }}
    >
      {options && [
        <MenuItem key="" value="">
          all
        </MenuItem>,
        ...options.map((o) => (
          <MenuItem key={o} value={o}>
            {o}
          </MenuItem>
        )),
      ]}
    </TextField>
  );

  return (
    <>
      <Box
        sx={{
          display: "flex",
          alignItems: "baseline",
          justifyContent: "space-between",
          mb: 1,
        }}
      >
        <Box>
          <Typography variant="h4" component="h1">
            Audit trail
          </Typography>
          <Typography variant="body2" color="text.secondary">
            Append-only, hash-chained record of every sensitive mutation —
            newest first.
          </Typography>
        </Box>
        <Button
          variant="contained"
          disabled={verifying}
          onClick={runVerify}
        >
          {verifying ? "Verifying…" : "Verify chain"}
        </Button>
      </Box>

      {verify && <VerifyResult result={verify} />}
      {verifyError && <Alert severity="error">{verifyError}</Alert>}
      {(verify || verifyError) && <Box sx={{ mb: 1 }} />}

      <Box
        sx={{
          display: "flex",
          flexWrap: "wrap",
          gap: 1,
          mb: 1.5,
          alignItems: "center",
        }}
      >
        {filterField("Action", "action", AUDIT_ACTIONS)}
        {filterField("Artifact table", "table", AUDIT_TABLES)}
        {filterField("Actor", "actor")}
        {filterField("Artifact key", "key")}
        <TextField
          size="small"
          type="datetime-local"
          label="From"
          value={filters.from}
          onChange={set("from")}
          slotProps={{ inputLabel: { shrink: true } }}
        />
        <TextField
          size="small"
          type="datetime-local"
          label="To"
          value={filters.to}
          onChange={set("to")}
          slotProps={{ inputLabel: { shrink: true } }}
        />
        <Button
          size="small"
          onClick={() => {
            setFilters(EMPTY_FILTERS);
            setOffset(0);
          }}
        >
          Clear
        </Button>
      </Box>

      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell padding="checkbox" />
              <TableCell>Time</TableCell>
              <TableCell>Actor</TableCell>
              <TableCell>Action</TableCell>
              <TableCell>Artifact</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading &&
              Array.from({ length: 5 }, (_, i) => (
                <TableRow key={i}>
                  <TableCell colSpan={5}>
                    <Skeleton variant="text" />
                  </TableCell>
                </TableRow>
              ))}
            {!isLoading && rows.length === 0 && (
              <TableRow>
                <TableCell colSpan={5}>No audit events match.</TableCell>
              </TableRow>
            )}
            {rows.map((e) => (
              <EventRow key={e.id} event={e} />
            ))}
          </TableBody>
        </Table>
      </TableContainer>

      <Box
        sx={{
          display: "flex",
          alignItems: "center",
          justifyContent: "flex-end",
          gap: 1,
          mt: 0.5,
        }}
      >
        <TextField
          select
          size="small"
          label="Rows"
          value={pageSize}
          onChange={(e) => {
            setPageSize(Number(e.target.value));
            setOffset(0);
          }}
          sx={{ minWidth: 90 }}
        >
          {PAGE_SIZES.map((n) => (
            <MenuItem key={n} value={n}>
              {n}
            </MenuItem>
          ))}
        </TextField>
        <Typography variant="body2" color="text.secondary">
          {offset + 1}–{offset + rows.length}
        </Typography>
        <IconButton
          size="small"
          aria-label="previous page"
          disabled={offset === 0}
          onClick={() => setOffset((o) => Math.max(0, o - pageSize))}
        >
          <NavigateBeforeIcon />
        </IconButton>
        <IconButton
          size="small"
          aria-label="next page"
          disabled={!hasNext}
          onClick={() => setOffset((o) => o + pageSize)}
        >
          <NavigateNextIcon />
        </IconButton>
      </Box>
    </>
  );
}
