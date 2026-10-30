use chrono::{DateTime, Utc};

/// Environment lanes (MPE-153): `dev` is the working lane — the only one
/// holding drafts and the only one `publish` stamps. `qa`/`uat`/`prod` hold
/// published rows written exclusively by forward promotion
/// (dev→qa→uat→prod, adjacent only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Dev,
    Qa,
    Uat,
    Prod,
}

impl Env {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Qa => "qa",
            Self::Uat => "uat",
            Self::Prod => "prod",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "dev" => Self::Dev,
            "qa" => Self::Qa,
            "uat" => Self::Uat,
            "prod" => Self::Prod,
            _ => return None,
        })
    }

    /// The lane a promotion from `self` is allowed to stamp, in order.
    pub fn next(&self) -> Option<Self> {
        match self {
            Self::Dev => Some(Self::Qa),
            Self::Qa => Some(Self::Uat),
            Self::Uat => Some(Self::Prod),
            Self::Prod => None,
        }
    }
}

/// Which stored revision of the decision set an evaluation resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Revision {
    /// Latest `status='published'` row per key, drafts filling gaps.
    LatestPublished,
    /// Draft rows only (editor preview).
    Draft,
    /// Latest published row per key with `published_at <= ts` — historic replay.
    AsOf(DateTime<Utc>),
}
