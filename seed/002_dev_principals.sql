-- Dev role grants for the Dex static users + service client (dex/config.dev.yaml).
-- Grant keys are token identity claims: `email` for humans (Dex `sub` is an
-- opaque encoded id) and the client_id for services — client-credentials
-- tokens carry the client_id in `aud`.
-- Runs via RUN_SEED=1 — dev-only data, idempotent.

INSERT INTO role_grants (subject, role, granted_by)
SELECT * FROM (VALUES
  ('admin@mpex.dev',  'admin',   'dev-seed'),
  ('pricer@mpex.dev', 'pricer',  'dev-seed'),
  ('viewer@mpex.dev', 'viewer',  'dev-seed'),
  ('mpex-svc',        'service', 'dev-seed')
) AS v(subject, role, granted_by)
WHERE NOT EXISTS (SELECT 1 FROM role_grants);
