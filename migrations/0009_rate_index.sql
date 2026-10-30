-- MND daily rate-index import (MPE-137). Products gain a "rate source":
-- `mnd_label` marks a base/index product whose sheet is written straight
-- from the scraped index; `base_product_code` + `rate_adjustment` derive a
-- product's rate off a base product's index rate; neither = manual sheet.
ALTER TABLE products
    ADD COLUMN mnd_label TEXT,
    ADD COLUMN base_product_code TEXT REFERENCES products (code),
    ADD COLUMN rate_adjustment DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD CONSTRAINT products_rate_source_chk
        CHECK (mnd_label IS NULL OR base_product_code IS NULL);

-- Base-product → index-label mapping (the six MND index products). The
-- products table is seed-managed, but the WHERE NOT EXISTS guard in the
-- seed means existing databases never re-seed — the mapping and the new
-- VA30 product must therefore live here so every environment gets them.
UPDATE products SET mnd_label = '30 Yr. Fixed' WHERE code = 'CONV30';
UPDATE products SET mnd_label = '15 Yr. Fixed' WHERE code = 'CONV15';
UPDATE products SET mnd_label = '7/6 SOFR ARM' WHERE code = 'CONV75ARM';
UPDATE products SET mnd_label = '30 Yr. FHA' WHERE code = 'FHA30';
UPDATE products SET mnd_label = '30 Yr. Jumbo' WHERE code = 'JUMBO30';

INSERT INTO products (code, name, loan_type, amortization, term_months, description)
VALUES ('VA30', 'VA 30-Year Fixed', 'va', 'fixed', 360, 'VA-guaranteed fixed-rate, 30y')
ON CONFLICT (code) DO NOTHING;
UPDATE products SET mnd_label = '30 Yr. VA' WHERE code = 'VA30';

-- Provenance for every import: what was fetched, when, and which
-- publication it produced (NULL when re-importing unchanged rates).
CREATE TABLE rate_index_imports (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    fetched_at TIMESTAMPTZ NOT NULL,
    source_url TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    rates JSONB NOT NULL,          -- {"30 Yr. Fixed": 7.29, ...}
    imported_by TEXT NOT NULL,     -- principal sub, or 'rate-index-scheduler'
    publication_id UUID REFERENCES publications (id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX rate_index_imports_created_at ON rate_index_imports (created_at DESC);

-- New audited actions for the importer and the product rate-config endpoint.
ALTER TABLE audit_events DROP CONSTRAINT audit_events_action_check;
ALTER TABLE audit_events ADD CONSTRAINT audit_events_action_check
    CHECK (action IN (
        'artifact.put',
        'artifact.delete',
        'publication.create',
        'auth.denied',
        'grant.put',
        'grant.delete',
        'rate_index.import',
        'product.rate_config.put'
    ));
