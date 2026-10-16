-- Per-product rate x lock-term price grids. Unlike decisions/matrices the
-- sheet is data, not rules, so `spec` IS the artifact (no JDM): validated on
-- write, resolved through the same snapshot mechanism at evaluation. `key`
-- is the product code — a rate sheet is meaningless without its product, and
-- one sheet per product keeps pricing lookup unambiguous.
CREATE TABLE rate_sheets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key TEXT NOT NULL REFERENCES products (code) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'draft', -- draft | published
    published_at TIMESTAMPTZ,
    publication_id UUID REFERENCES publications (id),
    spec JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT rate_sheets_published_at
        CHECK (status <> 'published' OR published_at IS NOT NULL)
);

CREATE INDEX idx_rate_sheets_key_status
    ON rate_sheets (key, status, published_at DESC);

CREATE UNIQUE INDEX uniq_rate_sheets_draft
    ON rate_sheets (key) WHERE status = 'draft';
CREATE UNIQUE INDEX uniq_rate_sheets_pub
    ON rate_sheets (key, publication_id) WHERE status = 'published';
