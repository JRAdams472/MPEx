-- MPE-126: value overrides — append-only artifact rows that surgically
-- patch a target artifact's resolved content at snapshot-build time. A row
-- says "in the resolved document for (target_kind, target_key), replace the
-- scalar leaf at value_path (RFC 6901 JSON Pointer) with value." Resolved
-- through the same fingerprint / DISTINCT ON (key) pipeline as every other
-- kind: drafts edit the dev working set, publish stamps immutable rows,
-- promote carries them lane-forward. Published overrides can be superseded
-- (new draft for the same key, republish) but never deleted — like every
-- published artifact.
CREATE TABLE overrides (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key TEXT NOT NULL,
    target_kind TEXT NOT NULL
        CHECK (target_kind IN ('decision', 'expression', 'matrix', 'rate_sheet')),
    target_key TEXT NOT NULL,
    -- RFC 6901 JSON Pointer into the target's patchable document
    -- (decision → jdm, matrix → spec, rate_sheet → spec,
    -- expression → {"expression": <text>}).
    value_path TEXT NOT NULL,
    -- Scalar leaf only: string/number/bool/null. Strings may carry ZEN
    -- expression text where the target expects it (e.g. a table cell).
    value JSONB NOT NULL
        CHECK (jsonb_typeof(value) IS NULL
               OR jsonb_typeof(value) NOT IN ('object', 'array')),
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'draft', -- draft | published
    env TEXT NOT NULL DEFAULT 'dev'
        CHECK (env IN ('dev', 'qa', 'uat', 'prod')),
    published_at TIMESTAMPTZ,
    publication_id UUID REFERENCES publications (id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT overrides_published_at
        CHECK (status <> 'published' OR published_at IS NOT NULL)
);

CREATE INDEX idx_overrides_env_key_status
    ON overrides (env, key, status, published_at DESC);
CREATE INDEX idx_overrides_target
    ON overrides (target_kind, target_key);

CREATE UNIQUE INDEX uniq_overrides_draft
    ON overrides (key) WHERE status = 'draft';
CREATE UNIQUE INDEX uniq_overrides_pub
    ON overrides (key, publication_id) WHERE status = 'published';

-- Widen the audit vocabulary for override mutations.
ALTER TABLE audit_events DROP CONSTRAINT audit_events_action_check;
ALTER TABLE audit_events ADD CONSTRAINT audit_events_action_check
    CHECK (action IN (
        'artifact.put', 'artifact.delete', 'publication.create',
        'auth.denied', 'grant.put', 'grant.delete',
        'rate_index.import', 'product.rate_config.put', 'promotion.create',
        'override.put', 'override.delete'
    ));
