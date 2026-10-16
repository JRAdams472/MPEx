CREATE TABLE decisions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'shared', -- eligibility | llpa | pricing | shared
    product_code TEXT REFERENCES products(code) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'draft', -- draft | published
    published_at TIMESTAMPTZ,
    jdm JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_decisions_key_status ON decisions (key, status, published_at DESC);
