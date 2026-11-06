/** Field catalog for the simple (business) decision-table editor — powers
 *  the guided input-column picker so business users choose known loan
 *  scenario fields instead of typing ZEN paths. Mirrors the typed sections
 *  of `LoanScenario` (src/domain/loan.rs); extension fields remain usable
 *  via the advanced editor. */
export type LoanFieldSchema = { field: string; name?: string; items?: LoanFieldSchema[] };

export const LOAN_FIELD_SCHEMA: LoanFieldSchema[] = [
  { field: "loan.purpose", name: "Loan purpose" },
  { field: "loan.type", name: "Loan type" },
  { field: "loan.amount", name: "Loan amount" },
  { field: "loan.termMonths", name: "Term (months)" },
  { field: "loan.amortization", name: "Amortization" },
  { field: "loan.arm.initialFixedMonths", name: "ARM initial fixed months" },
  { field: "loan.arm.index", name: "ARM index" },
  { field: "loan.arm.margin", name: "ARM margin" },
  { field: "loan.interestOnly", name: "Interest only" },
  { field: "loan.ltv", name: "LTV" },
  { field: "loan.cltv", name: "CLTV" },
  { field: "loan.dti", name: "DTI" },
  { field: "loan.units", name: "Units" },
  { field: "property.type", name: "Property type" },
  { field: "property.occupancy", name: "Occupancy" },
  { field: "property.state", name: "Property state" },
  { field: "property.appraisedValue", name: "Appraised value" },
  { field: "property.purchasePrice", name: "Purchase price" },
  { field: "borrower.creditScore", name: "Credit score" },
  { field: "borrower.creditModel", name: "Credit model" },
  { field: "borrower.firstTimeBuyer", name: "First-time buyer" },
  { field: "borrower.selfEmployed", name: "Self-employed" },
  { field: "borrower.reservesMonths", name: "Reserves (months)" },
  { field: "borrower.escrowWaiver", name: "Escrow waiver" },
  { field: "scenario.lockTermDays", name: "Lock term (days)" },
];
