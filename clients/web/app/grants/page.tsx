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
import Dialog from "@mui/material/Dialog";
import DialogActions from "@mui/material/DialogActions";
import DialogContent from "@mui/material/DialogContent";
import DialogTitle from "@mui/material/DialogTitle";
import Switch from "@mui/material/Switch";
import FormControlLabel from "@mui/material/FormControlLabel";
import FormGroup from "@mui/material/FormGroup";
import Checkbox from "@mui/material/Checkbox";
import EditIcon from "@mui/icons-material/Edit";
import DeleteIcon from "@mui/icons-material/Delete";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import {
  endpoints,
  mutationError,
  ENVS,
  type Env,
  type Grant,
  type GrantUpsert,
  type Role,
  type SubjectType,
} from "@/lib/api";
import { useMe } from "@/lib/me";

const SUBJECT_TYPES: SubjectType[] = ["sub", "email", "aud"];
const ROLES: Role[] = ["viewer", "pricer", "admin", "service"];

function EnvsCell({ envs }: { envs: Env[] | null }) {
  if (envs === null) {
    return <Chip label="all lanes" size="small" color="success" variant="outlined" />;
  }
  if (envs.length === 0) {
    return <Chip label="none" size="small" color="warning" variant="outlined" />;
  }
  return (
    <Box sx={{ display: "flex", gap: 0.5 }}>
      {envs.map((e) => (
        <Chip key={e} label={e} size="small" variant="outlined" />
      ))}
    </Box>
  );
}

/** Env-lane restriction control. Maps the three backend states:
 *  switch ON → `envs` omitted (NULL, all lanes); OFF → checkbox list where
 *  zero checked submits [] (no lanes — warns loudly). */
function EnvRestriction({
  unrestricted,
  lanes,
  onChange,
}: {
  unrestricted: boolean;
  lanes: Set<Env>;
  onChange: (unrestricted: boolean, lanes: Set<Env>) => void;
}) {
  return (
    <Box>
      <FormControlLabel
        control={
          <Switch
            checked={unrestricted}
            onChange={(e) => onChange(e.target.checked, lanes)}
          />
        }
        label="Unrestricted — all lanes"
      />
      {!unrestricted && (
        <FormGroup row sx={{ pl: 2 }}>
          {ENVS.map((env) => (
            <FormControlLabel
              key={env}
              control={
                <Checkbox
                  size="small"
                  checked={lanes.has(env)}
                  onChange={(e) => {
                    const next = new Set(lanes);
                    if (e.target.checked) next.add(env);
                    else next.delete(env);
                    onChange(false, next);
                  }}
                />
              }
              label={env}
            />
          ))}
        </FormGroup>
      )}
      {!unrestricted && lanes.size === 0 && (
        <Alert severity="warning" sx={{ mt: 1 }}>
          No lanes selected — this principal cannot evaluate anywhere.
        </Alert>
      )}
    </Box>
  );
}

function GrantDialog({
  grant,
  onClose,
}: {
  grant: Grant | "new" | null;
  onClose: () => void;
}) {
  const isNew = grant === "new";
  const editing = grant !== "new" && grant !== null ? grant : null;

  const [subjectType, setSubjectType] = useState<SubjectType>(
    editing?.subjectType ?? "email",
  );
  const [subject, setSubject] = useState(editing?.subject ?? "");
  const [role, setRole] = useState<Role>(editing?.role ?? "viewer");
  const [unrestricted, setUnrestricted] = useState(editing == null || editing.envs === null);
  const [lanes, setLanes] = useState<Set<Env>>(() => new Set(editing?.envs ?? []));

  const qc = useQueryClient();
  const save = useMutation({
    mutationFn: () => {
      const body: GrantUpsert = {
        subjectType,
        subject,
        role,
        ...(unrestricted ? {} : { envs: ENVS.filter((e) => lanes.has(e)) }),
      };
      return endpoints.upsertGrant(body);
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["grants"] });
      onClose();
    },
  });
  const error = mutationError(save);

  if (!grant) return null;
  return (
    <Dialog open onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>{isNew ? "New grant" : `Edit grant — ${editing?.subject}`}</DialogTitle>
      <DialogContent sx={{ display: "flex", flexDirection: "column", gap: 2, pt: 2 }}>
        <TextField
          select
          label="Subject type"
          value={subjectType}
          onChange={(e) => setSubjectType(e.target.value as SubjectType)}
          disabled={!isNew}
          size="small"
        >
          {SUBJECT_TYPES.map((t) => (
            <MenuItem key={t} value={t}>
              {t}
            </MenuItem>
          ))}
        </TextField>
        <TextField
          label="Subject"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
          disabled={!isNew}
          size="small"
          helperText={
            subjectType === "email"
              ? "The principal's email claim"
              : subjectType === "sub"
                ? "The principal's sub claim"
                : "A token audience value"
          }
        />
        <TextField
          select
          label="Role"
          value={role}
          onChange={(e) => setRole(e.target.value as Role)}
          disabled={!isNew}
          size="small"
        >
          {ROLES.map((r) => (
            <MenuItem key={r} value={r}>
              {r}
            </MenuItem>
          ))}
        </TextField>
        <EnvRestriction
          unrestricted={unrestricted}
          lanes={lanes}
          onChange={(u, l) => {
            setUnrestricted(u);
            setLanes(l);
          }}
        />
        {error && <Alert severity="error">{error}</Alert>}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button
          variant="contained"
          disabled={save.isPending || !subject.trim()}
          onClick={() => save.mutate()}
        >
          {isNew ? "Create" : "Save"}
        </Button>
      </DialogActions>
    </Dialog>
  );
}

function RevokeDialog({ grant, onClose }: { grant: Grant | null; onClose: () => void }) {
  const qc = useQueryClient();
  const revoke = useMutation({
    mutationFn: () => endpoints.revokeGrant(grant!.id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["grants"] });
      onClose();
    },
  });
  const error = mutationError(revoke);

  if (!grant) return null;
  return (
    <Dialog open onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle>Revoke grant</DialogTitle>
      <DialogContent>
        <Typography variant="body2" sx={{ mb: 1 }}>
          Revoke <strong>{grant.role}</strong> from{" "}
          <strong>
            {grant.subjectType}:{grant.subject}
          </strong>
          ?
        </Typography>
        {error && <Alert severity="error">{error}</Alert>}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button
          color="error"
          variant="contained"
          disabled={revoke.isPending}
          onClick={() => revoke.mutate()}
        >
          Revoke
        </Button>
      </DialogActions>
    </Dialog>
  );
}

export default function GrantsPage() {
  const { isAdmin } = useMe();
  const { data, isLoading, error } = useQuery({
    queryKey: ["grants"],
    queryFn: endpoints.grants,
    enabled: isAdmin,
  });
  const [dialog, setDialog] = useState<Grant | "new" | null>(null);
  const [revoking, setRevoking] = useState<Grant | null>(null);

  if (!isAdmin) {
    return <Alert severity="warning">Grants management requires the admin role.</Alert>;
  }
  if (error) return <Alert severity="error">{`${error}`}</Alert>;

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
            Grants
          </Typography>
          <Typography variant="body2" color="text.secondary">
            Role grants per principal. <code>envs</code> restricts which environment
            lanes a grant can evaluate — all lanes when unrestricted.
          </Typography>
        </Box>
        <Button variant="contained" onClick={() => setDialog("new")}>
          New grant
        </Button>
      </Box>

      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Subject</TableCell>
              <TableCell>Role</TableCell>
              <TableCell>Envs</TableCell>
              <TableCell>Granted</TableCell>
              <TableCell align="right">Actions</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={5}>Loading…</TableCell>
              </TableRow>
            )}
            {data?.grants.map((g) => (
              <TableRow key={g.id} hover>
                <TableCell>
                  <Chip label={g.subjectType} size="small" sx={{ mr: 1 }} />
                  {g.subject}
                </TableCell>
                <TableCell>
                  <Chip label={g.role} size="small" color="primary" variant="outlined" />
                </TableCell>
                <TableCell>
                  <EnvsCell envs={g.envs} />
                </TableCell>
                <TableCell>
                  <Typography variant="body2">{g.grantedBy}</Typography>
                  <Typography variant="caption" color="text.secondary">
                    {new Date(g.grantedAt).toLocaleString()}
                  </Typography>
                </TableCell>
                <TableCell align="right">
                  <Tooltip title="Edit">
                    <IconButton size="small" onClick={() => setDialog(g)}>
                      <EditIcon fontSize="small" />
                    </IconButton>
                  </Tooltip>
                  <Tooltip title="Revoke">
                    <IconButton size="small" onClick={() => setRevoking(g)}>
                      <DeleteIcon fontSize="small" />
                    </IconButton>
                  </Tooltip>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>

      {dialog && <GrantDialog grant={dialog} onClose={() => setDialog(null)} />}
      {revoking && <RevokeDialog grant={revoking} onClose={() => setRevoking(null)} />}
    </>
  );
}
