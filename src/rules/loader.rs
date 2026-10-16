use chrono::{DateTime, Utc};

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
