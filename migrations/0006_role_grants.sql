-- Role grants: OIDC subject (or email for dev users) -> MPEx role.
-- Roles are resolved by the API per request (briefly cached); the IdP never
-- needs to know MPEx roles. 'service' is for client-credentials callers
-- (e.g. LOLA) that may only evaluate + read products.
CREATE TABLE role_grants (
    subject TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('viewer', 'pricer', 'admin', 'service')),
    granted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    granted_by TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (subject, role)
);
