-- Auditor role (MPE-172): read-only parity with viewer plus access to the
-- audit-trail read surface. Widen the role vocabulary — grant rows written
-- before this migration keep their existing roles.
ALTER TABLE role_grants DROP CONSTRAINT role_grants_role_check;
ALTER TABLE role_grants ADD CONSTRAINT role_grants_role_check
    CHECK (role IN ('viewer', 'pricer', 'admin', 'auditor', 'service'));
