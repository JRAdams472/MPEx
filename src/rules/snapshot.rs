use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Context;
use sqlx::PgPool;
use zen_engine::loader::{CachedLoader, DecisionLoader, DynamicLoader, MemoryLoader};
use zen_engine::model::DecisionContent;

use crate::rules::loader::Revision;

/// Matrix identity needed by pricing: which matrices apply to a product.
#[derive(Debug, Clone)]
pub struct MatrixMeta {
    pub key: String,
    pub kind: String,
    /// None = shared matrix applying to every product.
    pub product_code: Option<String>,
}

/// A revision's artifacts resolved to plain data, so evaluation threads never
/// touch the database: compiled decision JDM in the loader, rate-sheet specs
/// keyed by product code, and matrix metadata for product scoping. Published
/// snapshots are cached behind a cheap fingerprint probe (max(updated_at),
/// count across all artifact tables) — publishing bumps the fingerprint and
/// naturally invalidates the cached snapshot.
pub struct Snapshot {
    pub fingerprint: String,
    pub loader: Arc<CachedLoader>,
    /// product code -> rate sheet spec.
    pub rate_sheets: HashMap<String, serde_json::Value>,
    /// All loaded matrices (decisions aren't scoped to products here).
    pub matrices: Vec<MatrixMeta>,
}

pub struct SnapshotStore {
    pool: PgPool,
    cache: RwLock<HashMap<String, Arc<Snapshot>>>,
    /// Memoized pricing responses keyed (fingerprint, loan, products, rate,
    /// lockTerm) — safe because evaluation is a pure function of those.
    memo: RwLock<HashMap<String, Arc<serde_json::Value>>>,
}

/// Memoized pricing responses live as long as their snapshot fingerprint
/// stays current; cap the map so drafts churn can't grow it unboundedly.
const MEMO_CAP: usize = 512;

impl SnapshotStore {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            cache: RwLock::new(HashMap::new()),
            memo: RwLock::new(HashMap::new()),
        }
    }

    /// Resolve + (possibly) cache the loader for a revision.
    pub async fn loader_for(
        &self,
        revision: Revision,
    ) -> Result<Arc<dyn DecisionLoader>, anyhow::Error> {
        Ok(self.snapshot_for(revision).await?.loader.clone())
    }

    /// Resolve the full snapshot (loader + rate sheets + matrix metadata).
    #[tracing::instrument(
        name = "snapshot.resolve",
        skip(self),
        fields(revision = ?revision, cache_hit = tracing::field::Empty)
    )]
    pub async fn snapshot_for(&self, revision: Revision) -> Result<Arc<Snapshot>, anyhow::Error> {
        let cache_key = self.fingerprint(revision).await?;
        let cache_hit = self
            .cache
            .read()
            .ok()
            .and_then(|c| c.get(&cache_key).cloned());
        tracing::Span::current().record("cache_hit", cache_hit.is_some());
        if let Some(snapshot) = cache_hit {
            return Ok(snapshot);
        }
        let snapshot = self.build(revision, cache_key.clone()).await?;
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(cache_key, snapshot.clone());
        }
        Ok(snapshot)
    }

    /// Pricing memo lookup — callers key on snapshot fingerprint plus the
    /// request tuple, so a publish invalidates entries without eviction.
    pub fn memo_get(&self, key: &str) -> Option<Arc<serde_json::Value>> {
        self.memo.read().ok()?.get(key).cloned()
    }

    pub fn memo_put(&self, key: String, value: serde_json::Value) {
        if let Ok(mut memo) = self.memo.write() {
            if memo.len() >= MEMO_CAP {
                memo.clear();
            }
            memo.insert(key, Arc::new(value));
        }
    }

    /// Latest-published and draft fingerprints can change; asOf snapshots are
    /// immutable by definition so their key is the timestamp itself. The
    /// fingerprint covers every loaded artifact kind.
    async fn fingerprint(&self, revision: Revision) -> anyhow::Result<String> {
        match revision {
            Revision::AsOf(ts) => Ok(format!("asof:{}", ts.to_rfc3339())),
            Revision::Draft | Revision::LatestPublished => {
                let published = self.status_fingerprint("published").await?;
                // Latest also falls back to drafts for keys lacking a
                // published revision, so its fingerprint covers both sets.
                let draft = if revision == Revision::LatestPublished {
                    self.status_fingerprint("draft").await?
                } else {
                    String::new()
                };
                Ok(format!("{revision:?}:{published}:{draft}"))
            }
        }
    }

    /// `max(updated_at):count` per artifact table at one status, in one query.
    #[tracing::instrument(name = "snapshot.fingerprint", skip(self))]
    async fn status_fingerprint(&self, status: &str) -> anyhow::Result<String> {
        let rows: Vec<(String, Option<chrono::DateTime<chrono::Utc>>, i64)> = sqlx::query_as(
            "SELECT t, max(updated_at), count(*) FROM (
               SELECT 'decisions' AS t, updated_at FROM decisions WHERE status = $1
               UNION ALL
               SELECT 'expressions', updated_at FROM expressions WHERE status = $1
               UNION ALL
               SELECT 'matrices', updated_at FROM matrices WHERE status = $1
               UNION ALL
               SELECT 'rate_sheets', updated_at FROM rate_sheets WHERE status = $1
             ) s GROUP BY t ORDER BY t",
        )
        .bind(status)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|(t, u, c)| format!("{t}={u:?}/{c}"))
            .collect::<Vec<_>>()
            .join(","))
    }

    #[tracing::instrument(
        name = "snapshot.build",
        skip(self, fingerprint),
        fields(
            revision = ?revision,
            decisions = tracing::field::Empty,
            expressions = tracing::field::Empty,
            rate_sheets = tracing::field::Empty,
            matrices = tracing::field::Empty
        )
    )]
    async fn build(
        &self,
        revision: Revision,
        fingerprint: String,
    ) -> anyhow::Result<Arc<Snapshot>> {
        // Each kind resolves DISTINCT ON (key) the same way; kinds merge with
        // decisions taking precedence over matrices on a key collision (keys
        // are namespaced by convention, e.g. eligibility.X / llpa.X).
        let rows: Vec<(String, serde_json::Value)> = match revision {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, jdm FROM (
                       (SELECT DISTINCT ON (key) key, jdm, 0 AS prio FROM decisions
                        WHERE status = 'draft' ORDER BY key, updated_at DESC)
                       UNION ALL
                       (SELECT DISTINCT ON (key) key, jdm, 1 AS prio FROM matrices
                        WHERE status = 'draft' ORDER BY key, updated_at DESC)
                     ) t ORDER BY key, prio",
                )
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, jdm FROM (
                       (SELECT DISTINCT ON (key) key, jdm, 0 AS prio FROM decisions
                        WHERE status = 'published' AND published_at <= $1
                        ORDER BY key, published_at DESC)
                       UNION ALL
                       (SELECT DISTINCT ON (key) key, jdm, 1 AS prio FROM matrices
                        WHERE status = 'published' AND published_at <= $1
                        ORDER BY key, published_at DESC)
                     ) t ORDER BY key, prio",
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
                       SELECT DISTINCT ON (key) key, jdm, 0 AS prio FROM (
                         SELECT key, jdm, published_at FROM decisions WHERE status = 'published'
                         UNION ALL
                         SELECT key, jdm, published_at FROM matrices WHERE status = 'published'
                       ) a ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, jdm, 1 AS prio FROM (
                         SELECT key, jdm, updated_at FROM decisions WHERE status = 'draft'
                         UNION ALL
                         SELECT key, jdm, updated_at FROM matrices WHERE status = 'draft'
                       ) a ORDER BY key, updated_at DESC
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

        // Rate sheet specs and matrix scoping metadata resolve with identical
        // revision semantics — data artifacts live beside the loader, not in it.
        let sheet_rows: Vec<(String, serde_json::Value)> = match revision {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, spec FROM rate_sheets
                     WHERE status = 'draft' ORDER BY key, updated_at DESC",
                )
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, spec FROM rate_sheets
                     WHERE status = 'published' AND published_at <= $1
                     ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, spec, 0 AS prio FROM rate_sheets
                       WHERE status = 'published' ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, spec, 1 AS prio FROM rate_sheets
                       WHERE status = 'draft' ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, spec FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        let rate_sheets: HashMap<String, serde_json::Value> = sheet_rows.into_iter().collect();

        let matrix_rows: Vec<(String, String, Option<String>)> = match revision {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, kind, product_code FROM matrices
                     WHERE status = 'draft' ORDER BY key, updated_at DESC",
                )
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, kind, product_code FROM matrices
                     WHERE status = 'published' AND published_at <= $1
                     ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, kind, product_code, 0 AS prio
                       FROM matrices WHERE status = 'published'
                       ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, kind, product_code, 1 AS prio
                       FROM matrices WHERE status = 'draft'
                       ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, kind, product_code FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        let matrices = matrix_rows
            .into_iter()
            .map(|(key, kind, product_code)| MatrixMeta {
                key,
                kind,
                product_code,
            })
            .collect();

        // Expressions resolve with the same revision semantics and enter the
        // loader as synthetic single-purpose decisions, so graphs call them
        // via decisionNode: `standard` -> `{result: <value>}`; `unary` ->
        // `{result: bool}` evaluated against the caller's `input` field.
        let expr_rows: Vec<(String, String, String)> = match revision {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, kind, expression FROM expressions
                     WHERE status = 'draft' ORDER BY key, updated_at DESC",
                )
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, kind, expression FROM expressions
                     WHERE status = 'published' AND published_at <= $1
                     ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, kind, expression, 0 AS prio
                       FROM expressions WHERE status = 'published'
                       ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, kind, expression, 1 AS prio
                       FROM expressions WHERE status = 'draft'
                       ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, kind, expression FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        let mut expressions = 0usize;
        for (key, kind, expression) in &expr_rows {
            if mem.get(key.as_str()).is_some() {
                // A decision/matrix already claimed the key — namespaced keys
                // (expr.*) make this unusual, so warn rather than silently win.
                tracing::warn!(
                    key,
                    "expression key collides with a decision/matrix; skipped"
                );
                continue;
            }
            let content: DecisionContent =
                serde_json::from_value(expression_jdm(key, kind, expression))
                    .with_context(|| format!("expression {key}: wrapper JDM invalid"))?;
            mem.add(key.clone(), content);
            expressions += 1;
        }

        let snapshot = Snapshot {
            fingerprint,
            loader: Arc::new(CachedLoader::from(Arc::new(mem) as DynamicLoader)),
            rate_sheets,
            matrices,
        };
        tracing::Span::current()
            .record("decisions", rows.len())
            .record("expressions", expressions)
            .record("rate_sheets", snapshot.rate_sheets.len())
            .record("matrices", snapshot.matrices.len());
        tracing::debug!(
            ?revision,
            decisions = rows.len(),
            expressions,
            "built decision snapshot"
        );
        Ok(Arc::new(snapshot))
    }
}

/// Wraps a stored expression in a single-purpose decision so graphs reach it
/// through `decisionNode` on its key. `standard` emits `{result: <value>}`;
/// `unary` (e.g. `> 80`) can't run as a standard expression, so it's wrapped
/// in a one-rule table that predicates the caller's `input` field and emits
/// `{result: bool}` (wildcard rule defaults to false).
fn expression_jdm(key: &str, kind: &str, expression: &str) -> serde_json::Value {
    let middle = match kind {
        "unary" => serde_json::json!({
            "id": "tbl", "type": "decisionTableNode", "name": key,
            "content": {
                "hitPolicy": "first",
                "inputs": [{"id": "i1", "name": "input", "field": "input"}],
                "outputs": [{"id": "o1", "name": "result", "field": "result"}],
                "rules": [
                    {"_id": "r1", "i1": expression, "o1": "true"},
                    {"_id": "r2", "i1": "", "o1": "false"}
                ]
            }
        }),
        _ => serde_json::json!({
            "id": "expr", "type": "expressionNode", "name": key,
            "content": {"expressions": [
                {"id": "e1", "key": "result", "value": expression}
            ]}
        }),
    };
    serde_json::json!({
        "nodes": [
            {"id": "in", "type": "inputNode", "name": "Request"},
            middle,
            {"id": "out", "type": "outputNode", "name": "Response"}
        ],
        "edges": [
            {"id": "e1", "sourceId": "in", "targetId": middle["id"]},
            {"id": "e2", "sourceId": middle["id"], "targetId": "out"}
        ]
    })
}
