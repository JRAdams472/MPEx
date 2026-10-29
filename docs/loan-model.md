# Loan model

The loan model is the single input contract for both eligibility and pricing
evaluation. It is **PII-free** — it carries loan/scenario attributes only, never
names, SSNs, addresses, account numbers, or dates of birth. Evaluation is
idempotent: the response depends only on this payload plus the resolved rule
revision (`asOf`).

Submitted as `loan` in `POST /api/evaluate/*` request bodies. All fields are
optional; unknown extension fields are preserved and visible to rules.

```jsonc
{
  "loan": {
    "purpose": "purchase",            // purchase | rateTermRefi | cashOutRefi
    "type": "conventional",           // conventional | fha | va | usda | jumbo
    "amount": 450000,
    "termMonths": 360,
    "amortization": "fixed",          // fixed | arm
    "arm": {                          // only when amortization = arm
      "initialFixedMonths": 84,
      "index": "SOFR",
      "margin": 2.75
    },
    "interestOnly": false,
    "ltv": 80.0,
    "cltv": 80.0,
    "dti": 36.0,
    "units": 1
  },
  "property": {
    "type": "sfr",                    // sfr | condo | townhouse | multiUnit2_4 | manufactured | pud
    "occupancy": "primary",           // primary | secondHome | investment
    "state": "TX",
    "appraisedValue": 562500,
    "purchasePrice": 562500
  },
  "borrower": {
    "creditScore": 740,
    "creditModel": "FICO",          // FICO | VantageScore | F10T | other string
    "firstTimeBuyer": false,
    "selfEmployed": false,
    "reservesMonths": 6,
    "escrowWaiver": false
  },
  "scenario": {
    "lockTermDays": 30
  }
}
```

Rules reference paths like `loan.ltv`, `property.occupancy`, `borrower.creditScore`
directly in ZEN expressions — e.g. `loan.ltv > 80`, `borrower.creditScore >= 700`,
`loan.purpose == 'cashOutRefi'`.

## Eligibility decision contract

Every product's eligibility decision is a JDM graph stored under key
`eligibility.<PRODUCT_CODE>` and must output:

```jsonc
{ "eligible": true|false, "reasons": ["why-not-1", "..."] }
```

The demo pattern: a decision table where each **rule is a fail condition**;
the `reasons[]` output column uses per-column collect so all matched fail
reasons gather into an array even under `first` hit policy; a final expression
node computes `eligible = len(reasons ?? []) == 0`.

## Revision keys

The engine resolves decisions as `<key>` (latest published, falling back to
draft when nothing is published), `<key>@draft`, or `<key>@<rfc3339>` (latest
publication at or before that time — historic pricing).
