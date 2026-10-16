-- A named publish event: each publish stamps one row here and links the
-- published artifact rows back to it, so daily pricing drops are named
-- snapshots. Status is always 'published' on commit — the transaction either
-- lands fully or not at all.
CREATE TABLE publications (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    label TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'published',
    published_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Published artifact rows point at the publication that stamped them; drafts
-- keep NULL. Published rows are append-only — never UPDATE them.
ALTER TABLE decisions
    ADD COLUMN publication_id UUID REFERENCES publications (id),
    ADD CONSTRAINT decisions_published_at
        CHECK (status <> 'published' OR published_at IS NOT NULL);

-- Named reusable ZEN expression fragments, versioned exactly like decisions.
CREATE TABLE expressions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT 'standard', -- standard | unary
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'draft',  -- draft | published
    published_at TIMESTAMPTZ,
    publication_id UUID REFERENCES publications (id),
    expression TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT expressions_published_at
        CHECK (status <> 'published' OR published_at IS NOT NULL)
);

CREATE INDEX idx_expressions_key_status
    ON expressions (key, status, published_at DESC);

-- At most one mutable draft per key; at most one published row per key per
-- publication. Published history stacks up keyed (key, status, published_at).
CREATE UNIQUE INDEX uniq_decisions_draft
    ON decisions (key) WHERE status = 'draft';
CREATE UNIQUE INDEX uniq_expressions_draft
    ON expressions (key) WHERE status = 'draft';
CREATE UNIQUE INDEX uniq_decisions_pub
    ON decisions (key, publication_id) WHERE status = 'published';
CREATE UNIQUE INDEX uniq_expressions_pub
    ON expressions (key, publication_id) WHERE status = 'published';
