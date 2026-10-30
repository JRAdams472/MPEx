-- MPE-153: environment lanes (dev/qa/uat/prod) on the append-only artifact
-- model. The dev lane IS today's single lane — every existing row defaults
-- to 'dev', so callers that never send an env selector get identical
-- resolution. qa/uat/prod stay empty until promoted (nothing reaches a
-- lane except an explicit promotion or a dev publish).
--
-- Drafts only ever live in 'dev': PUT handlers keep writing env='dev',
-- POST /api/publications stamps 'dev' publications, and qa/uat/prod rows
-- exist solely as status='published' copies written by
-- POST /api/admin/promotions.

ALTER TABLE decisions
    ADD COLUMN env TEXT NOT NULL DEFAULT 'dev'
        CHECK (env IN ('dev', 'qa', 'uat', 'prod'));
ALTER TABLE expressions
    ADD COLUMN env TEXT NOT NULL DEFAULT 'dev'
        CHECK (env IN ('dev', 'qa', 'uat', 'prod'));
ALTER TABLE matrices
    ADD COLUMN env TEXT NOT NULL DEFAULT 'dev'
        CHECK (env IN ('dev', 'qa', 'uat', 'prod'));
ALTER TABLE rate_sheets
    ADD COLUMN env TEXT NOT NULL DEFAULT 'dev'
        CHECK (env IN ('dev', 'qa', 'uat', 'prod'));
ALTER TABLE publications
    ADD COLUMN env TEXT NOT NULL DEFAULT 'dev'
        CHECK (env IN ('dev', 'qa', 'uat', 'prod'));

-- Env-scoped resolution indexes (DISTINCT ON (key) ... AND env = ...).
CREATE INDEX idx_decisions_env_key_status
    ON decisions (env, key, status, published_at DESC);
CREATE INDEX idx_expressions_env_key_status
    ON expressions (env, key, status, published_at DESC);
CREATE INDEX idx_matrices_env_key_status
    ON matrices (env, key, status, published_at DESC);
CREATE INDEX idx_rate_sheets_env_key_status
    ON rate_sheets (env, key, status, published_at DESC);

-- Permitted environment lanes per grant — Zara `permitted_environments`
-- analogue, enforced at the eval boundary. NULL = all lanes (every
-- existing grant stays unrestricted).
ALTER TABLE role_grants
    ADD COLUMN envs TEXT[] NULL;

-- New audited action for promotions.
ALTER TABLE audit_events DROP CONSTRAINT audit_events_action_check;
ALTER TABLE audit_events ADD CONSTRAINT audit_events_action_check
    CHECK (action IN (
        'artifact.put', 'artifact.delete', 'publication.create',
        'auth.denied', 'grant.put', 'grant.delete',
        'rate_index.import', 'product.rate_config.put', 'promotion.create'
    ));
