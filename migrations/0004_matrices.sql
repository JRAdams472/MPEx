-- Row×column adjustment matrices (LLPA etc.). `spec` is the authoring format
-- ({rows[], cols[], cells{"r:c": value}, op}); `jdm` is the compiled decision
-- table produced at write time so drafts evaluate identically to published
-- artifacts and snapshots stay a uniform key -> JDM load.
CREATE TABLE matrices (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'llpa', -- llpa | pricing | shared
    product_code TEXT REFERENCES products(code) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'draft', -- draft | published
    published_at TIMESTAMPTZ,
    publication_id UUID REFERENCES publications (id),
    spec JSONB NOT NULL,
    jdm JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT matrices_published_at
        CHECK (status <> 'published' OR published_at IS NOT NULL)
);

CREATE INDEX idx_matrices_key_status
    ON matrices (key, status, published_at DESC);

CREATE UNIQUE INDEX uniq_matrices_draft
    ON matrices (key) WHERE status = 'draft';
CREATE UNIQUE INDEX uniq_matrices_pub
    ON matrices (key, publication_id) WHERE status = 'published';
