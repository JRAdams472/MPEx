-- Dev role grants for the Dex static users + service client (dex/config.dev.yaml).
-- Grant keys are token identity claims: `email` for humans (Dex `sub` is an
-- opaque encoded id) and the client_id for services — client-credentials
-- tokens carry the client_id in `aud`.
-- Runs via RUN_SEED=1 — dev-only data, idempotent.

INSERT INTO role_grants (subject_type, subject, role, granted_by) VALUES
  ('email', 'admin@mpex.dev',  'admin',   'dev-seed'),
  ('email', 'pricer@mpex.dev', 'pricer',  'dev-seed'),
  ('email', 'viewer@mpex.dev', 'viewer',  'dev-seed'),
  ('email', 'auditor@mpex.dev', 'auditor', 'dev-seed'),
  ('aud',   'mpex-svc',        'service', 'dev-seed'),
  -- Automation identity (CI seeding, tooling) — aud-keyed like mpex-svc.
  ('aud',   'mpex-ci',         'admin',   'dev-seed')
ON CONFLICT (subject_type, subject, role) DO NOTHING;
