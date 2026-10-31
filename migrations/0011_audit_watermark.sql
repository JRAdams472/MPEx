-- Verified-watermark for the audit chain (MPE-125). Every successful
-- verify_chain pass appends one row recording the tail it walked, so
-- subsequent verifies re-check only rows after the watermark — O(new
-- events) instead of O(chain length). Append-only like audit_events:
-- the history of verifications is itself forensic data.
CREATE TABLE audit_verify_watermarks (
    id BIGSERIAL PRIMARY KEY,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_event_id BIGINT NOT NULL,
    last_event_hash TEXT NOT NULL,
    checked BIGINT NOT NULL,
    scope TEXT NOT NULL CHECK (scope IN ('full', 'incremental'))
);

CREATE FUNCTION audit_watermark_block_mutation() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'audit_verify_watermarks is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_verify_watermarks_append_only
    BEFORE UPDATE OR DELETE ON audit_verify_watermarks
    FOR EACH ROW EXECUTE FUNCTION audit_watermark_block_mutation();
