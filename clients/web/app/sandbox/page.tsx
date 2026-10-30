"use client";

import { useState } from "react";
import Alert from "@mui/material/Alert";
import Box from "@mui/material/Box";
import Button from "@mui/material/Button";
import Chip from "@mui/material/Chip";
import Divider from "@mui/material/Divider";
import FormControlLabel from "@mui/material/FormControlLabel";
import Checkbox from "@mui/material/Checkbox";
import MenuItem from "@mui/material/MenuItem";
import Paper from "@mui/material/Paper";
import Stack from "@mui/material/Stack";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import TextField from "@mui/material/TextField";
import Typography from "@mui/material/Typography";
import Accordion from "@mui/material/Accordion";
import AccordionSummary from "@mui/material/AccordionSummary";
import AccordionDetails from "@mui/material/AccordionDetails";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import { useMutation, useQuery } from "@tanstack/react-query";

import {
  endpoints,
  EligibilityReport,
  Env,
  ENVS,
  PricingReport,
} from "../../lib/api";
import { useMe } from "../../lib/me";

const SAMPLE = {
  loan: {
    purpose: "purchase",
    type: "conventional",
    amount: 320000,
    termMonths: 360,
    ltv: 78,
  },
  property: { type: "sfr", occupancy: "primary", state: "TX" },
  borrower: { creditScore: 760, creditModel: "FICO" },
  scenario: { lockTermDays: 30 },
};

const LOAN_TYPES = ["conventional", "fha", "va", "usda", "jumbo"];
const PURPOSES = ["purchase", "rateTermRefi", "cashOutRefi"];
const CREDIT_MODELS = ["FICO", "VantageScore", "F10T"];
const PROPERTY_TYPES = [
  "sfr",
  "condo",
  "townhouse",
  "multiUnit2_4",
  "manufactured",
  "pud",
];
const OCCUPANCIES = ["primary", "secondHome", "investment"];

type Fields = {
  purpose: string;
  type: string;
  amount: string;
  termMonths: string;
  ltv: string;
  dti: string;
  units: string;
  propertyType: string;
  occupancy: string;
  state: string;
  creditScore: string;
  creditModel: string;
  lockTermDays: string;
};

function toFields(loan: typeof SAMPLE): Fields {
  return {
    purpose: loan.loan.purpose,
    type: loan.loan.type,
    amount: String(loan.loan.amount),
    termMonths: String(loan.loan.termMonths),
    ltv: String(loan.loan.ltv),
    dti: "",
    units: "",
    propertyType: loan.property.type,
    occupancy: loan.property.occupancy,
    state: loan.property.state,
    creditScore: String(loan.borrower.creditScore),
    creditModel: String(loan.borrower.creditModel ?? "FICO"),
    lockTermDays: String(loan.scenario.lockTermDays),
  };
}

function num(v: string): number | undefined {
  const n = Number(v);
  return v.trim() === "" || Number.isNaN(n) ? undefined : n;
}

function toLoan(f: Fields, extras: Record<string, unknown>) {
  const loan: Record<string, unknown> = { ...((extras.loan as object) ?? {}) };
  const property: Record<string, unknown> = {
    ...((extras.property as object) ?? {}),
  };
  const borrower: Record<string, unknown> = {
    ...((extras.borrower as object) ?? {}),
  };
  const scenario: Record<string, unknown> = {
    ...((extras.scenario as object) ?? {}),
  };
  if (f.purpose) loan.purpose = f.purpose;
  if (f.type) loan.type = f.type;
  if (num(f.amount) !== undefined) loan.amount = num(f.amount);
  if (num(f.termMonths) !== undefined) loan.termMonths = num(f.termMonths);
  if (num(f.ltv) !== undefined) loan.ltv = num(f.ltv);
  if (num(f.dti) !== undefined) loan.dti = num(f.dti);
  if (num(f.units) !== undefined) loan.units = num(f.units);
  if (f.propertyType) property.type = f.propertyType;
  if (f.occupancy) property.occupancy = f.occupancy;
  if (f.state) property.state = f.state;
  if (num(f.creditScore) !== undefined)
    borrower.creditScore = num(f.creditScore);
  if (f.creditModel) borrower.creditModel = f.creditModel;
  if (num(f.lockTermDays) !== undefined)
    scenario.lockTermDays = num(f.lockTermDays);
  return { ...extras, loan, property, borrower, scenario };
}

function TraceView({ trace }: { trace: unknown }) {
  return (
    <Accordion disableGutters elevation={0} sx={{ bgcolor: "transparent" }}>
      <AccordionSummary expandIcon={<ExpandMoreIcon />} sx={{ minHeight: 32 }}>
        <Typography variant="caption" color="text.secondary">
          Decision trace
        </Typography>
      </AccordionSummary>
      <AccordionDetails sx={{ p: 0 }}>
        <Box
          component="pre"
          sx={{
            m: 0,
            p: 1,
            fontSize: 11,
            overflow: "auto",
            maxHeight: 320,
            bgcolor: "background.default",
            borderRadius: 1,
          }}
        >
          {JSON.stringify(trace, null, 2)}
        </Box>
      </AccordionDetails>
    </Accordion>
  );
}

export default function SandboxPage() {
  const [fields, setFields] = useState<Fields>(() => toFields(SAMPLE));
  const [extrasJson, setExtrasJson] = useState("{}");
  const [extrasError, setExtrasError] = useState<string | null>(null);
  const [asOf, setAsOf] = useState("");
  // dev = the working lane; qa/uat/prod only see promoted artifacts (MPE-153).
  const [env, setEnv] = useState<Env>("dev");
  const { me } = useMe();
  const [trace, setTrace] = useState(true);
  const [rate, setRate] = useState("");
  const [lockTerm, setLockTerm] = useState("");
  const [selected, setSelected] = useState<string[]>([]);
  const [elig, setElig] = useState<EligibilityReport | null>(null);
  const [pricing, setPricing] = useState<PricingReport | null>(null);

  const products = useQuery({
    queryKey: ["products"],
    queryFn: endpoints.products,
  });

  const buildLoan = () => {
    let extras: Record<string, unknown>;
    try {
      extras = JSON.parse(extrasJson || "{}");
    } catch (e) {
      setExtrasError((e as Error).message);
      throw e;
    }
    setExtrasError(null);
    return toLoan(fields, extras);
  };

  const runEligibility = useMutation({
    mutationFn: async () => {
      const loan = buildLoan();
      const res = await endpoints.evaluateEligibility(loan, {
        asOf: asOf || undefined,
        env,
        trace,
      });
      if (res.status >= 400)
        throw new Error(
          (res.data as { error?: string }).error ??
            `eligibility: HTTP ${res.status}`,
        );
      return res.data;
    },
    onSuccess: (data) => {
      setElig(data);
      setPricing(null);
      setSelected(data.eligible.map((o) => o.product.code));
    },
  });

  const runPricing = useMutation({
    mutationFn: async () => {
      const loan = buildLoan();
      const res = await endpoints.evaluatePricing(loan, selected, {
        rate: num(rate),
        lockTermDays: num(lockTerm),
        asOf: asOf || undefined,
        env,
        trace,
      });
      if (res.status >= 400)
        throw new Error(
          (res.data as { error?: string }).error ??
            `pricing: HTTP ${res.status}`,
        );
      return res.data;
    },
    onSuccess: setPricing,
  });

  const set = (k: keyof Fields) => (e: React.ChangeEvent<HTMLInputElement>) =>
    setFields((f) => ({ ...f, [k]: e.target.value }));

  const allProducts = products.data?.products ?? [];
  const toggleProduct = (code: string) =>
    setSelected((s) =>
      s.includes(code) ? s.filter((c) => c !== code) : [...s, code],
    );

  return (
    <Stack spacing={3}>
      <Typography variant="h5">Pricer sandbox</Typography>
      <Typography variant="body2" color="text.secondary">
        Edit a loan scenario, run eligibility across the catalog, then price
        selected products with the LLPA waterfall.
      </Typography>

      <Box sx={{ display: "flex", gap: 3, flexWrap: "wrap" }}>
        <Paper variant="outlined" sx={{ p: 2, flex: "1 1 420px" }}>
          <Typography variant="subtitle1" gutterBottom>
            Loan scenario
          </Typography>
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: "repeat(3, 1fr)",
              gap: 1.5,
            }}
          >
            <TextField
              select
              size="small"
              label="Loan type"
              value={fields.type}
              onChange={set("type")}
            >
              {LOAN_TYPES.map((t) => (
                <MenuItem key={t} value={t}>
                  {t}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              select
              size="small"
              label="Purpose"
              value={fields.purpose}
              onChange={set("purpose")}
            >
              {PURPOSES.map((t) => (
                <MenuItem key={t} value={t}>
                  {t}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              size="small"
              label="Amount"
              value={fields.amount}
              onChange={set("amount")}
            />
            <TextField
              size="small"
              label="Term (mo)"
              value={fields.termMonths}
              onChange={set("termMonths")}
            />
            <TextField
              size="small"
              label="LTV %"
              value={fields.ltv}
              onChange={set("ltv")}
            />
            <TextField
              size="small"
              label="DTI %"
              value={fields.dti}
              onChange={set("dti")}
            />
            <TextField
              size="small"
              label="Units"
              value={fields.units}
              onChange={set("units")}
            />
            <TextField
              select
              size="small"
              label="Property type"
              value={fields.propertyType}
              onChange={set("propertyType")}
            >
              {PROPERTY_TYPES.map((t) => (
                <MenuItem key={t} value={t}>
                  {t}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              select
              size="small"
              label="Occupancy"
              value={fields.occupancy}
              onChange={set("occupancy")}
            >
              {OCCUPANCIES.map((t) => (
                <MenuItem key={t} value={t}>
                  {t}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              size="small"
              label="State"
              value={fields.state}
              onChange={set("state")}
            />
            <TextField
              size="small"
              label="Credit score"
              value={fields.creditScore}
              onChange={set("creditScore")}
            />
            <TextField
              size="small"
              select
              label="Credit model"
              value={fields.creditModel}
              onChange={set("creditModel")}
            >
              {CREDIT_MODELS.map((m) => (
                <MenuItem key={m} value={m}>
                  {m}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              size="small"
              label="Lock term (days)"
              value={fields.lockTermDays}
              onChange={set("lockTermDays")}
            />
          </Box>

          <TextField
            size="small"
            label='Extra fields (JSON, merged over sections)'
            value={extrasJson}
            onChange={(e) => setExtrasJson(e.target.value)}
            error={!!extrasError}
            helperText={extrasError}
            multiline
            minRows={2}
            fullWidth
            sx={{
              mt: 2,
              "& textarea": { fontFamily: "monospace", fontSize: 12 },
            }}
          />

          <Divider sx={{ my: 2 }} />
          <Typography variant="subtitle2" gutterBottom>
            Evaluation options
          </Typography>
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: "repeat(3, 1fr)",
              gap: 1.5,
            }}
          >
            <TextField
              select
              size="small"
              label="Environment"
              value={env}
              onChange={(e) => setEnv(e.target.value as Env)}
              helperText={
                me?.permittedEnvs
                  ? `your grants permit: ${me.permittedEnvs.join(", ")}`
                  : undefined
              }
            >
              {ENVS.map((e) => (
                <MenuItem key={e} value={e}>
                  {e}
                </MenuItem>
              ))}
            </TextField>
            <TextField
              size="small"
              label="asOf (blank, draft, RFC3339)"
              value={asOf}
              onChange={(e) => setAsOf(e.target.value)}
            />
            <TextField
              size="small"
              label="Rate (pin one)"
              value={rate}
              onChange={(e) => setRate(e.target.value)}
            />
            <TextField
              size="small"
              label="Lock term override"
              value={lockTerm}
              onChange={(e) => setLockTerm(e.target.value)}
            />
          </Box>
          <FormControlLabel
            control={
              <Checkbox
                checked={trace}
                onChange={(e) => setTrace(e.target.checked)}
              />
            }
            label="Include decision traces"
          />
          <Stack direction="row" spacing={1} sx={{ mt: 1 }}>
            <Button
              variant="contained"
              startIcon={<PlayArrowIcon />}
              onClick={() => runEligibility.mutate()}
              disabled={runEligibility.isPending}
            >
              Run eligibility
            </Button>
            <Button
              variant="outlined"
              onClick={() => {
                setFields(toFields(SAMPLE));
                setExtrasJson("{}");
              }}
            >
              Load sample
            </Button>
          </Stack>
          {runEligibility.error && (
            <Alert severity="error" sx={{ mt: 1 }}>
              {runEligibility.error.message}
            </Alert>
          )}
        </Paper>

        <Box sx={{ flex: "2 1 560px", minWidth: 0 }}>
          {elig && (
            <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
              <Typography variant="subtitle1" gutterBottom>
                Eligibility — revision {elig.revision} ·{" "}
                {elig.elapsedMs.toFixed(1)} ms · env {env}
              </Typography>
              <Stack
                direction="row"
                spacing={1}
                useFlexGap
                sx={{ flexWrap: "wrap" }}
              >
                {elig.eligible.map((o) => (
                  <Chip
                    key={o.product.code}
                    label={o.product.code}
                    color="success"
                    variant={selected.includes(o.product.code) ? "filled" : "outlined"}
                    onClick={() => toggleProduct(o.product.code)}
                  />
                ))}
              </Stack>
              {elig.ineligible.length > 0 && (
                <Table size="small" sx={{ mt: 1 }}>
                  <TableHead>
                    <TableRow>
                      <TableCell>Product</TableCell>
                      <TableCell>Reasons</TableCell>
                      <TableCell />
                    </TableRow>
                  </TableHead>
                  <TableBody>
                    {elig.ineligible.map((o) => (
                      <TableRow key={o.product.code}>
                        <TableCell>{o.product.code}</TableCell>
                        <TableCell sx={{ fontSize: 12 }}>
                          {o.reasons.map((r) => String(r)).join("; ")}
                        </TableCell>
                        <TableCell>
                          {trace && o.trace !== undefined && (
                            <TraceView trace={o.trace} />
                          )}
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              )}
              <Stack direction="row" spacing={1} sx={{ mt: 2 }}>
                {allProducts
                  .filter(
                    (p) =>
                      !elig.eligible.some((o) => o.product.code === p.code) &&
                      !elig.ineligible.some((o) => o.product.code === p.code),
                  )
                  .map((p) => (
                    <Chip
                      key={p.code}
                      label={p.code}
                      size="small"
                      variant={
                        selected.includes(p.code) ? "filled" : "outlined"
                      }
                      onClick={() => toggleProduct(p.code)}
                    />
                  ))}
                <Button
                  variant="contained"
                  size="small"
                  startIcon={<PlayArrowIcon />}
                  onClick={() => runPricing.mutate()}
                  disabled={runPricing.isPending || selected.length === 0}
                  sx={{ ml: "auto" }}
                >
                  Price {selected.length} product{selected.length === 1 ? "" : "s"}
                </Button>
              </Stack>
              {runPricing.error && (
                <Alert severity="error" sx={{ mt: 1 }}>
                  {runPricing.error.message}
                </Alert>
              )}
            </Paper>
          )}

          {pricing && (
            <Paper variant="outlined" sx={{ p: 2 }}>
              <Typography variant="subtitle1" gutterBottom>
                Pricing — revision {pricing.revision} ·{" "}
                {pricing.elapsedMs.toFixed(1)} ms · env {env}
              </Typography>
              {pricing.priced.map((p) => (
                <Accordion
                  key={p.product.code}
                  defaultExpanded
                  disableGutters
                  elevation={0}
                  sx={{ border: 1, borderColor: "divider", mb: 1 }}
                >
                  <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                    <Typography variant="subtitle2">
                      {p.product.code} — {p.product.name}
                    </Typography>
                  </AccordionSummary>
                  <AccordionDetails>
                    {p.lines.map((line) => (
                      <Box key={`${line.rate}:${line.lockTermDays}`} sx={{ mb: 2 }}>
                        <Typography variant="body2" sx={{ fontWeight: 600 }}>
                          {line.rate}% × {line.lockTermDays}d — price{" "}
                          {line.price.toFixed(3)}
                        </Typography>
                        <Table size="small">
                          <TableHead>
                            <TableRow>
                              <TableCell>Step</TableCell>
                              <TableCell align="right">Value</TableCell>
                            </TableRow>
                          </TableHead>
                          <TableBody>
                            <TableRow>
                              <TableCell>Base price ({p.rateSheet})</TableCell>
                              <TableCell align="right">
                                {line.basePrice.toFixed(3)}
                              </TableCell>
                            </TableRow>
                            {line.adjustments.map((a) => (
                              <TableRow key={a.matrix}>
                                <TableCell>
                                  {a.matrix}
                                  {a.error && (
                                    <Typography
                                      variant="caption"
                                      color="error"
                                      sx={{ display: "block" }}
                                    >
                                      {a.error}
                                    </Typography>
                                  )}
                                  {trace && a.trace !== undefined && (
                                    <TraceView trace={a.trace} />
                                  )}
                                </TableCell>
                                <TableCell align="right">
                                  {a.value === undefined
                                    ? "—"
                                    : a.value.toFixed(3)}
                                </TableCell>
                              </TableRow>
                            ))}
                            <TableRow>
                              <TableCell sx={{ fontWeight: 600 }}>
                                Final price
                              </TableCell>
                              <TableCell align="right" sx={{ fontWeight: 600 }}>
                                {line.price.toFixed(3)}
                              </TableCell>
                            </TableRow>
                          </TableBody>
                        </Table>
                      </Box>
                    ))}
                  </AccordionDetails>
                </Accordion>
              ))}
              {pricing.unpriced.length > 0 && (
                <Alert severity="warning" sx={{ mt: 1 }}>
                  Unpriced:{" "}
                  {pricing.unpriced
                    .map((u) => `${u.product.code} (${u.reason})`)
                    .join(", ")}
                </Alert>
              )}
            </Paper>
          )}
        </Box>
      </Box>
    </Stack>
  );
}
