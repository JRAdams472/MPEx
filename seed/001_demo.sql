-- Demo seed: products + per-product eligibility decisions.
-- Runs via `RUN_SEED=1` on the api service after sqlx migrations (idempotent).

-- Per-row idempotent: migration 0009 pre-inserts VA30 before seeding, so a
-- whole-table guard would skip everything on fresh databases. Rate-source
-- columns mirror the migration's mapping: `mnd_label` = MND index base
-- product, `base_product_code`+`rate_adjustment` = derived (FHA15 demos
-- this), neither = manual sheet.
INSERT INTO products (code, name, loan_type, amortization, term_months, description,
                      mnd_label, base_product_code, rate_adjustment)
VALUES
  ('CONV30',  'Conventional 30-Year Fixed', 'conventional', 'fixed', 360, 'Conforming conventional fixed-rate, 30y', '30 Yr. Fixed', NULL, 0),
  ('CONV15',  'Conventional 15-Year Fixed', 'conventional', 'fixed', 180, 'Conforming conventional fixed-rate, 15y', '15 Yr. Fixed', NULL, 0),
  ('CONV75ARM','Conventional 7/6 ARM',      'conventional', 'arm',   360, 'Conforming conventional ARM, 7y fixed then 6mo adj', '7/6 SOFR ARM', NULL, 0),
  ('FHA30',   'FHA 30-Year Fixed',          'fha',          'fixed', 360, 'FHA-insured fixed-rate, 30y', '30 Yr. FHA', NULL, 0),
  ('JUMBO30', 'Jumbo 30-Year Fixed',        'jumbo',        'fixed', 360, 'Non-conforming jumbo fixed-rate, 30y', '30 Yr. Jumbo', NULL, 0),
  ('VA30',    'VA 30-Year Fixed',           'va',           'fixed', 360, 'VA-guaranteed fixed-rate, 30y', '30 Yr. VA', NULL, 0),
  ('FHA15',   'FHA 15-Year Fixed',          'fha',          'fixed', 180, 'FHA-insured fixed-rate, 15y — derived off FHA30', NULL, 'FHA30', -0.25)
ON CONFLICT (code) DO NOTHING;

-- Eligibility decision shape:
--   input -> decision table of fail conditions (first-hit, reasons[] collects ALL
--   matched reasons) -> expression node computes eligible = len(reasons ?? []) == 0
-- Output contract: { "eligible": bool, "reasons": string[] }

INSERT INTO decisions (key, kind, product_code, status, jdm)
SELECT key, kind, product_code, status, jdm::jsonb FROM (VALUES
('eligibility.CONV30', 'eligibility', 'CONV30', 'draft', $$
{
  "nodes": [
    {"id": "in", "type": "inputNode", "name": "Request", "position": {"x": 0, "y": 0}},
    {"id": "tbl", "type": "decisionTableNode", "name": "Eligibility rules", "position": {"x": 280, "y": 0},
     "content": {
       "hitPolicy": "first",
       "inputs": [
         {"id": "i1", "name": "Loan type",    "field": "loan.type"},
         {"id": "i2", "name": "LTV",          "field": "loan.ltv"},
         {"id": "i3", "name": "FICO",         "field": "borrower.creditScore"},
         {"id": "i4", "name": "Occupancy",    "field": "property.occupancy"},
         {"id": "i5", "name": "Units",        "field": "loan.units"},
         {"id": "i6", "name": "Amortization", "field": "loan.amortization"},
         {"id": "i7", "name": "Term",         "field": "loan.termMonths"}
       ],
       "outputs": [
         {"id": "o1", "name": "reason", "field": "reasons[]"}
       ],
       "rules": [
         {"_id": "r1", "i1": "!= 'conventional'", "o1": "'loan type must be conventional'"},
         {"_id": "r2", "i2": "> 97",              "o1": "'ltv exceeds 97%'"},
         {"_id": "r3", "i3": "< 620",             "o1": "'creditScore below 620'"},
         {"_id": "r4", "i5": "> 4",               "o1": "'max 4 units'"},
         {"_id": "r5", "i6": "'arm'",             "o1": "'fixed rate only'"},
         {"_id": "r6", "i7": "!= 360",            "o1": "'30-year term only'"}
       ]
     }},
    {"id": "fin", "type": "expressionNode", "name": "Verdict", "position": {"x": 560, "y": 0},
     "content": {"expressions": [
       {"id": "e1", "key": "eligible", "value": "len(reasons ?? []) == 0"},
       {"id": "e2", "key": "reasons",  "value": "reasons ?? []"},
       {"id": "e3", "key": "product",  "value": "'CONV30'"}
     ]}},
    {"id": "out", "type": "outputNode", "name": "Response", "position": {"x": 840, "y": 0}}
  ],
  "edges": [
    {"id": "e1", "sourceId": "in",  "targetId": "tbl"},
    {"id": "e2", "sourceId": "tbl", "targetId": "fin"},
    {"id": "e3", "sourceId": "fin", "targetId": "out"}
  ]
}$$),

('eligibility.CONV15', 'eligibility', 'CONV15', 'draft', $$
{
  "nodes": [
    {"id": "in", "type": "inputNode", "name": "Request", "position": {"x": 0, "y": 0}},
    {"id": "tbl", "type": "decisionTableNode", "name": "Eligibility rules", "position": {"x": 280, "y": 0},
     "content": {
       "hitPolicy": "first",
       "inputs": [
         {"id": "i1", "name": "Loan type",    "field": "loan.type"},
         {"id": "i2", "name": "LTV",          "field": "loan.ltv"},
         {"id": "i3", "name": "FICO",         "field": "borrower.creditScore"},
         {"id": "i5", "name": "Units",        "field": "loan.units"},
         {"id": "i6", "name": "Amortization", "field": "loan.amortization"},
         {"id": "i7", "name": "Term",         "field": "loan.termMonths"}
       ],
       "outputs": [{"id": "o1", "name": "reason", "field": "reasons[]"}],
       "rules": [
         {"_id": "r1", "i1": "!= 'conventional'", "o1": "'loan type must be conventional'"},
         {"_id": "r2", "i2": "> 97",              "o1": "'ltv exceeds 97%'"},
         {"_id": "r3", "i3": "< 620",             "o1": "'creditScore below 620'"},
         {"_id": "r5", "i5": "> 4",               "o1": "'max 4 units'"},
         {"_id": "r6", "i6": "'arm'",             "o1": "'fixed rate only'"},
         {"_id": "r7", "i7": "!= 180",            "o1": "'15-year term only'"}
       ]
     }},
    {"id": "fin", "type": "expressionNode", "name": "Verdict", "position": {"x": 560, "y": 0},
     "content": {"expressions": [
       {"id": "e1", "key": "eligible", "value": "len(reasons ?? []) == 0"},
       {"id": "e2", "key": "reasons",  "value": "reasons ?? []"},
       {"id": "e3", "key": "product",  "value": "'CONV15'"}
     ]}},
    {"id": "out", "type": "outputNode", "name": "Response", "position": {"x": 840, "y": 0}}
  ],
  "edges": [
    {"id": "e1", "sourceId": "in",  "targetId": "tbl"},
    {"id": "e2", "sourceId": "tbl", "targetId": "fin"},
    {"id": "e3", "sourceId": "fin", "targetId": "out"}
  ]
}$$),

('eligibility.CONV75ARM', 'eligibility', 'CONV75ARM', 'draft', $$
{
  "nodes": [
    {"id": "in", "type": "inputNode", "name": "Request", "position": {"x": 0, "y": 0}},
    {"id": "tbl", "type": "decisionTableNode", "name": "Eligibility rules", "position": {"x": 280, "y": 0},
     "content": {
       "hitPolicy": "first",
       "inputs": [
         {"id": "i1", "name": "Loan type",    "field": "loan.type"},
         {"id": "i2", "name": "LTV",          "field": "loan.ltv"},
         {"id": "i3", "name": "FICO",         "field": "borrower.creditScore"},
         {"id": "i5", "name": "Units",        "field": "loan.units"},
         {"id": "i6", "name": "Amortization", "field": "loan.amortization"},
         {"id": "i8", "name": "ARM months",   "field": "loan.arm.initialFixedMonths"}
       ],
       "outputs": [{"id": "o1", "name": "reason", "field": "reasons[]"}],
       "rules": [
         {"_id": "r1", "i1": "!= 'conventional'", "o1": "'loan type must be conventional'"},
         {"_id": "r2", "i2": "> 95",              "o1": "'ltv exceeds 95% for ARM'"},
         {"_id": "r3", "i3": "< 620",             "o1": "'creditScore below 620'"},
         {"_id": "r5", "i5": "> 4",               "o1": "'max 4 units'"},
         {"_id": "r6", "i6": "'fixed'",           "o1": "'arm amortization only'"},
         {"_id": "r7", "i8": "!= 84",             "o1": "'7/6 ARM only (84mo fixed)'"}
       ]
     }},
    {"id": "fin", "type": "expressionNode", "name": "Verdict", "position": {"x": 560, "y": 0},
     "content": {"expressions": [
       {"id": "e1", "key": "eligible", "value": "len(reasons ?? []) == 0"},
       {"id": "e2", "key": "reasons",  "value": "reasons ?? []"},
       {"id": "e3", "key": "product",  "value": "'CONV75ARM'"}
     ]}},
    {"id": "out", "type": "outputNode", "name": "Response", "position": {"x": 840, "y": 0}}
  ],
  "edges": [
    {"id": "e1", "sourceId": "in",  "targetId": "tbl"},
    {"id": "e2", "sourceId": "tbl", "targetId": "fin"},
    {"id": "e3", "sourceId": "fin", "targetId": "out"}
  ]
}$$),

('eligibility.FHA30', 'eligibility', 'FHA30', 'draft', $$
{
  "nodes": [
    {"id": "in", "type": "inputNode", "name": "Request", "position": {"x": 0, "y": 0}},
    {"id": "tbl", "type": "decisionTableNode", "name": "Eligibility rules", "position": {"x": 280, "y": 0},
     "content": {
       "hitPolicy": "first",
       "inputs": [
         {"id": "i1", "name": "Loan type",    "field": "loan.type"},
         {"id": "i2", "name": "LTV",          "field": "loan.ltv"},
         {"id": "i3", "name": "FICO",         "field": "borrower.creditScore"},
         {"id": "i4", "name": "Occupancy",    "field": "property.occupancy"},
         {"id": "i5", "name": "Units",        "field": "loan.units"},
         {"id": "i7", "name": "Term",         "field": "loan.termMonths"}
       ],
       "outputs": [{"id": "o1", "name": "reason", "field": "reasons[]"}],
       "rules": [
         {"_id": "r1", "i1": "!= 'fha'",        "o1": "'loan type must be fha'"},
         {"_id": "r2", "i2": "> 96.5",          "o1": "'ltv exceeds 96.5%'"},
         {"_id": "r3", "i3": "< 580",           "o1": "'creditScore below 580'"},
         {"_id": "r4", "i4": "!= 'primary'",    "o1": "'fha requires primary occupancy'"},
         {"_id": "r5", "i5": "> 4",             "o1": "'max 4 units'"},
         {"_id": "r6", "i7": "!= 360",          "o1": "'30-year term only'"}
       ]
     }},
    {"id": "fin", "type": "expressionNode", "name": "Verdict", "position": {"x": 560, "y": 0},
     "content": {"expressions": [
       {"id": "e1", "key": "eligible", "value": "len(reasons ?? []) == 0"},
       {"id": "e2", "key": "reasons",  "value": "reasons ?? []"},
       {"id": "e3", "key": "product",  "value": "'FHA30'"}
     ]}},
    {"id": "out", "type": "outputNode", "name": "Response", "position": {"x": 840, "y": 0}}
  ],
  "edges": [
    {"id": "e1", "sourceId": "in",  "targetId": "tbl"},
    {"id": "e2", "sourceId": "tbl", "targetId": "fin"},
    {"id": "e3", "sourceId": "fin", "targetId": "out"}
  ]
}$$),

('eligibility.JUMBO30', 'eligibility', 'JUMBO30', 'draft', $$
{
  "nodes": [
    {"id": "in", "type": "inputNode", "name": "Request", "position": {"x": 0, "y": 0}},
    {"id": "tbl", "type": "decisionTableNode", "name": "Eligibility rules", "position": {"x": 280, "y": 0},
     "content": {
       "hitPolicy": "first",
       "inputs": [
         {"id": "i1", "name": "Loan type", "field": "loan.type"},
         {"id": "i2", "name": "LTV",       "field": "loan.ltv"},
         {"id": "i3", "name": "FICO",      "field": "borrower.creditScore"},
         {"id": "i4", "name": "Amount",    "field": "loan.amount"},
         {"id": "i5", "name": "Units",     "field": "loan.units"},
         {"id": "i7", "name": "Term",      "field": "loan.termMonths"}
       ],
       "outputs": [{"id": "o1", "name": "reason", "field": "reasons[]"}],
       "rules": [
         {"_id": "r1", "i1": "!= 'jumbo'",      "o1": "'loan type must be jumbo'"},
         {"_id": "r2", "i2": "> 80",            "o1": "'ltv exceeds 80%'"},
         {"_id": "r3", "i3": "< 700",           "o1": "'creditScore below 700'"},
         {"_id": "r4", "i4": "<= 832750",       "o1": "'amount not above conforming limit'"},
         {"_id": "r5", "i5": "> 1",             "o1": "'single unit only'"},
         {"_id": "r6", "i7": "!= 360",          "o1": "'30-year term only'"}
       ]
     }},
    {"id": "fin", "type": "expressionNode", "name": "Verdict", "position": {"x": 560, "y": 0},
     "content": {"expressions": [
       {"id": "e1", "key": "eligible", "value": "len(reasons ?? []) == 0"},
       {"id": "e2", "key": "reasons",  "value": "reasons ?? []"},
       {"id": "e3", "key": "product",  "value": "'JUMBO30'"}
     ]}},
    {"id": "out", "type": "outputNode", "name": "Response", "position": {"x": 840, "y": 0}}
  ],
  "edges": [
    {"id": "e1", "sourceId": "in",  "targetId": "tbl"},
    {"id": "e2", "sourceId": "tbl", "targetId": "fin"},
    {"id": "e3", "sourceId": "fin", "targetId": "out"}
  ]
}$$)) AS v(key, kind, product_code, status, jdm)
WHERE NOT EXISTS (SELECT 1 FROM decisions);
