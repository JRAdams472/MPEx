-- Append-only, hash-chained audit trail (MPE-97 phase 3). Every sensitive
-- mutation writes one attributed row inside the mutation's own transaction;
-- `hash` chains each event to its predecessor so out-of-band edits break
-- verify_chain. The genesis row carries prev_hash = ''.
CREATE TABLE audit_events (
    id BIGSERIAL PRIMARY KEY,
    occurred_at TIMESTAMPTZ NOT NULL,
    actor_sub TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN (
        'artifact.put',
        'artifact.delete',
        'publication.create',
        'auth.denied'
    )),
    artifact_table TEXT,
    artifact_key TEXT,
    detail JSONB NOT NULL DEFAULT '{}',
    prev_hash TEXT NOT NULL,
    hash TEXT NOT NULL UNIQUE
);

CREATE INDEX audit_events_occurred_at ON audit_events (occurred_at DESC);
CREATE INDEX audit_events_actor ON audit_events (actor_sub);
CREATE INDEX audit_events_action ON audit_events (action);
CREATE INDEX audit_events_artifact ON audit_events (artifact_table, artifact_key);

-- Append-only is enforced by the storage layer, not by convention: any
-- UPDATE or DELETE is rejected outright.
CREATE FUNCTION audit_events_block_mutation() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'audit_events is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER audit_events_append_only
    BEFORE UPDATE OR DELETE ON audit_events
    FOR EACH ROW EXECUTE FUNCTION audit_events_block_mutation();
