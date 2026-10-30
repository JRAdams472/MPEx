"use client";

import Typography from "@mui/material/Typography";
import Table from "@mui/material/Table";
import TableBody from "@mui/material/TableBody";
import TableCell from "@mui/material/TableCell";
import TableContainer from "@mui/material/TableContainer";
import TableHead from "@mui/material/TableHead";
import TableRow from "@mui/material/TableRow";
import Paper from "@mui/material/Paper";
import Chip from "@mui/material/Chip";
import Alert from "@mui/material/Alert";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";

import { endpoints, type Product } from "@/lib/api";

function rateSource(p: Product): string {
  if (p.mndLabel) return `index — ${p.mndLabel}`;
  if (p.baseProductCode) {
    const sign = p.rateAdjustment >= 0 ? "+" : "";
    return `${p.baseProductCode} ${sign}${p.rateAdjustment}`;
  }
  return "manual";
}

export default function ProductsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["products"],
    queryFn: endpoints.products,
  });

  if (error) return <Alert severity="error">{`${error}`}</Alert>;

  return (
    <>
      <Typography variant="h4" component="h1" gutterBottom>
        Products
      </Typography>
      <TableContainer component={Paper} variant="outlined">
        <Table size="small">
          <TableHead>
            <TableRow>
              <TableCell>Code</TableCell>
              <TableCell>Name</TableCell>
              <TableCell>Loan type</TableCell>
              <TableCell>Amortization</TableCell>
              <TableCell align="right">Term (mo)</TableCell>
              <TableCell>Rate source</TableCell>
              <TableCell>Status</TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {isLoading && (
              <TableRow>
                <TableCell colSpan={6}>Loading…</TableCell>
              </TableRow>
            )}
            {data?.products.map((p) => (
              <TableRow key={p.code} hover>
                <TableCell>
                  <Link
                    href={`/products/${p.code}`}
                    style={{ color: "inherit", fontWeight: 600 }}
                  >
                    {p.code}
                  </Link>
                </TableCell>
                <TableCell>{p.name}</TableCell>
                <TableCell>{p.loanType}</TableCell>
                <TableCell>{p.amortization}</TableCell>
                <TableCell align="right">{p.termMonths}</TableCell>
                <TableCell>
                  <Typography variant="body2" color="text.secondary">
                    {rateSource(p)}
                  </Typography>
                </TableCell>
                <TableCell>
                  <Chip
                    label={p.active ? "active" : "inactive"}
                    size="small"
                    color={p.active ? "success" : "default"}
                    variant="outlined"
                  />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </TableContainer>
    </>
  );
}
