-- Typed grant subjects + surrogate id (MPE-120). Each grant matches exactly
-- one identity claim class — 'sub' (opaque OIDC sub), 'email' (Dex static
-- users), 'aud' (client-credentials client_id) — so a subject string can
-- never accidentally match a different claim type.
--
-- Backfill classifies existing rows by convention: subjects containing '@'
-- become 'email', everything else 'aud' (the dev seeds mpex-svc/mpex-ci are
-- aud-keyed; the admin API is the first writer of real grant rows — any
-- hypothetical out-of-band sub-keyed row would classify as 'aud' and simply
-- stop matching, which fails closed).

ALTER TABLE role_grants ADD COLUMN subject_type TEXT NOT NULL DEFAULT '';
UPDATE role_grants
   SET subject_type = CASE WHEN position('@' IN subject) > 0 THEN 'email'
                           ELSE 'aud' END;
ALTER TABLE role_grants ALTER COLUMN subject_type DROP DEFAULT;
ALTER TABLE role_grants ADD CONSTRAINT role_grants_subject_type_check
    CHECK (subject_type IN ('sub', 'email', 'aud'));

-- Surrogate id for admin-API addressing; the natural key stays unique for
-- upsert semantics.
ALTER TABLE role_grants ADD COLUMN id BIGINT GENERATED ALWAYS AS IDENTITY;
ALTER TABLE role_grants DROP CONSTRAINT role_grants_pkey;
ALTER TABLE role_grants ADD PRIMARY KEY (id);
ALTER TABLE role_grants ADD CONSTRAINT role_grants_natural_key
    UNIQUE (subject_type, subject, role);

-- Widen the audit vocabulary for grant mutations.
ALTER TABLE audit_events DROP CONSTRAINT audit_events_action_check;
ALTER TABLE audit_events ADD CONSTRAINT audit_events_action_check
    CHECK (action IN (
        'artifact.put',
        'artifact.delete',
        'publication.create',
        'auth.denied',
        'grant.put',
        'grant.delete'
    ));
