CREATE TABLE products (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    loan_type TEXT NOT NULL DEFAULT 'conventional',
    amortization TEXT NOT NULL DEFAULT 'fixed',
    term_months INTEGER NOT NULL DEFAULT 360,
    description TEXT NOT NULL DEFAULT '',
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
