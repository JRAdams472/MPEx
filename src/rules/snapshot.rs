use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Context;
use sqlx::PgPool;
use zen_engine::loader::{CachedLoader, DecisionLoader, DynamicLoader, MemoryLoader};
use zen_engine::model::DecisionContent;

use crate::rules::loader::Revision;

/// A revision's full decision set resolved to plain key -> JDM, so evaluation
/// threads never touch the database. Published snapshots are cached behind a
/// cheap fingerprint probe (max(updated_at), count) — publishing bumps the
/// fingerprint and naturally invalidates the cached snapshot.
pub struct SnapshotStore {
    pool: PgPool,
    cache: RwLock<HashMap<String, Arc<CachedLoader>>>,
}

impl SnapshotStore {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Resolve + (possibly) cache the loader for a revision.
    pub async fn loader_for(
        &self,
        revision: Revision,
    ) -> Result<Arc<dyn DecisionLoader>, anyhow::Error> {
        let cache_key = self.fingerprint(revision).await?;
        if let Some(loader) = self
            .cache
            .read()
            .ok()
            .and_then(|c| c.get(&cache_key).cloned())
        {
            return Ok(loader);
        }
        let loader = self.build(revision).await?;
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(cache_key, loader.clone());
        }
        Ok(loader)
    }

    /// Latest-published and draft fingerprints can change; asOf snapshots are
    /// immutable by definition so their key is the timestamp itself.
    async fn fingerprint(&self, revision: Revision) -> anyhow::Result<String> {
        match revision {
            Revision::AsOf(ts) => Ok(format!("asof:{}", ts.to_rfc3339())),
            Revision::Draft | Revision::LatestPublished => {
                let (updated, count): (Option<chrono::DateTime<chrono::Utc>>, i64) =
                    sqlx::query_as(
                        "SELECT max(updated_at), count(*) FROM decisions WHERE status = $1",
                    )
                    .bind(match revision {
                        Revision::Draft => "draft",
                        _ => "published",
                    })
                    .fetch_one(&self.pool)
                    .await?;
                // Latest also falls back to drafts for keys lacking a
                // published revision, so its fingerprint must cover both sets.
                let draft_fp = if revision == Revision::LatestPublished {
                    let (du, dc): (Option<chrono::DateTime<chrono::Utc>>, i64) = sqlx::query_as(
                        "SELECT max(updated_at), count(*) FROM decisions WHERE status = 'draft'",
                    )
                    .fetch_one(&self.pool)
                    .await?;
                    format!("{du:?}:{dc}")
                } else {
                    String::new()
                };
                Ok(format!("{revision:?}:{updated:?}:{count}:{draft_fp}"))
            }
        }
    }

    async fn build(&self, revision: Revision) -> anyhow::Result<Arc<CachedLoader>> {
        let rows: Vec<(String, serde_json::Value)> = match revision {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, jdm FROM decisions \
                     WHERE status = 'draft' ORDER BY key, updated_at DESC",
                )
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, jdm FROM decisions \
                     WHERE status = 'published' AND published_at <= $1 \
                     ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                // Latest published per key; draft fills keys with no published
                // row (pre-publish dev ergonomics).
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, jdm, 0 AS prio FROM decisions
                       WHERE status = 'published' ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, jdm, 1 AS prio FROM decisions
                       WHERE status = 'draft' ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, jdm FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };

        let mem = MemoryLoader::default();
        for (key, jdm) in &rows {
            let content: DecisionContent = serde_json::from_value(jdm.clone())
                .with_context(|| format!("decision {key}: invalid JDM"))?;
            mem.add(key.clone(), content);
        }
        let cached = CachedLoader::from(Arc::new(mem) as DynamicLoader);
        tracing::debug!(?revision, decisions = rows.len(), "built decision snapshot");
        Ok(Arc::new(cached))
    }
}
