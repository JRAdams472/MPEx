use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::Context;
use sqlx::PgPool;
use zen_engine::loader::{CachedLoader, DecisionLoader, DynamicLoader, MemoryLoader};
use zen_engine::model::DecisionContent;

use crate::rules::compile_cache::CompileCache;
use crate::rules::loader::{Env, Revision};
use crate::rules::overrides;

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
    /// Compiled graphs shared across snapshots by (key, content hash) —
    /// a publish recompiles only what changed (MPE-127).
    compiles: CompileCache,
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
            compiles: CompileCache::default(),
        }
    }

    /// Compiled-graph cache misses since boot — a publish bumps this by the
    /// number of *changed* keys, not the catalog (MPE-127 instrumentation).
    pub fn compile_misses(&self) -> u64 {
        self.compiles.misses()
    }

    /// Resolve + (possibly) cache the loader for a revision within an
    /// environment lane.
    pub async fn loader_for(
        &self,
        revision: Revision,
        env: Env,
    ) -> Result<Arc<dyn DecisionLoader>, anyhow::Error> {
        Ok(self.snapshot_for(revision, env).await?.loader.clone())
    }

    /// Resolve the full snapshot (loader + rate sheets + matrix metadata).
    /// `revision` and `env` are independent selectors: `Draft` always means
    /// the dev working set (drafts exist only in dev), so its lane is
    /// forced to dev here regardless of `env`.
    #[tracing::instrument(
        name = "snapshot.resolve",
        skip(self),
        fields(revision = ?revision, env = env.as_str(), cache_hit = tracing::field::Empty)
    )]
    pub async fn snapshot_for(
        &self,
        revision: Revision,
        env: Env,
    ) -> Result<Arc<Snapshot>, anyhow::Error> {
        let env = if revision == Revision::Draft {
            Env::Dev
        } else {
            env
        };
        let cache_key = self.fingerprint(revision, env).await?;
        let cache_hit = self
            .cache
            .read()
            .ok()
            .and_then(|c| c.get(&cache_key).cloned());
        tracing::Span::current().record("cache_hit", cache_hit.is_some());
        if let Some(snapshot) = cache_hit {
            return Ok(snapshot);
        }
        let snapshot = self.build(revision, env, cache_key.clone()).await?;
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
    async fn fingerprint(&self, revision: Revision, env: Env) -> anyhow::Result<String> {
        match revision {
            Revision::AsOf(ts) => Ok(format!("asof:{}:{}", env.as_str(), ts.to_rfc3339())),
            Revision::Draft | Revision::LatestPublished => {
                let published = self.status_fingerprint("published", env).await?;
                // Drafts affect a snapshot when it resolves them: always for
                // Draft (also fixes draft-edit cache invalidation), and for
                // Latest's draft-fill — which only runs in the dev lane.
                let draft = if revision == Revision::Draft
                    || (revision == Revision::LatestPublished && env == Env::Dev)
                {
                    self.status_fingerprint("draft", env).await?
                } else {
                    String::new()
                };
                Ok(format!("{revision:?}:{}:{published}:{draft}", env.as_str()))
            }
        }
    }

    /// `max(updated_at):count` per artifact table at one status and env lane,
    /// in one query.
    #[tracing::instrument(name = "snapshot.fingerprint", skip(self))]
    async fn status_fingerprint(&self, status: &str, env: Env) -> anyhow::Result<String> {
        let rows: Vec<(String, Option<chrono::DateTime<chrono::Utc>>, i64)> = sqlx::query_as(
            "SELECT t, max(updated_at), count(*) FROM (
               SELECT 'decisions' AS t, updated_at FROM decisions WHERE status = $1 AND env = $2
               UNION ALL
               SELECT 'expressions', updated_at FROM expressions WHERE status = $1 AND env = $2
               UNION ALL
               SELECT 'matrices', updated_at FROM matrices WHERE status = $1 AND env = $2
               UNION ALL
               SELECT 'rate_sheets', updated_at FROM rate_sheets WHERE status = $1 AND env = $2
               UNION ALL
               SELECT 'overrides', updated_at FROM overrides WHERE status = $1 AND env = $2
             ) s GROUP BY t ORDER BY t",
        )
        .bind(status)
        .bind(env.as_str())
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
            matrices = tracing::field::Empty,
            overrides = tracing::field::Empty
        )
    )]
    async fn build(
        &self,
        revision: Revision,
        env: Env,
        fingerprint: String,
    ) -> anyhow::Result<Arc<Snapshot>> {
        // Each kind resolves DISTINCT ON (key) within one env lane; kinds
        // merge with decisions taking precedence over matrices on a key
        // collision (keys are namespaced, e.g. eligibility.X / llpa.X).
        // `$1 = 'dev'` in every draft branch makes draft-fill a no-op outside
        // the dev lane — drafts can never resolve in qa/uat/prod.
        // `spec` (matrices only) and `kind` ride along so value overrides
        // (MPE-126) can patch the authoring doc and re-derive loader JDM.
        let rows: Vec<(String, serde_json::Value, Option<serde_json::Value>, String)> =
            match revision {
                Revision::Draft => {
                    sqlx::query_as(
                        "SELECT DISTINCT ON (key) key, jdm, spec, kind FROM (
                       (SELECT DISTINCT ON (key) key, jdm, CAST(NULL AS jsonb) AS spec,
                               'decision' AS kind, 0 AS prio FROM decisions
                        WHERE status = 'draft' AND env = $1 ORDER BY key, updated_at DESC)
                       UNION ALL
                       (SELECT DISTINCT ON (key) key, jdm, spec, 'matrix' AS kind, 1 AS prio
                        FROM matrices
                        WHERE status = 'draft' AND env = $1 ORDER BY key, updated_at DESC)
                     ) t ORDER BY key, prio",
                    )
                    .bind(env.as_str())
                    .fetch_all(&self.pool)
                    .await?
                }
                Revision::AsOf(ts) => {
                    sqlx::query_as(
                        "SELECT DISTINCT ON (key) key, jdm, spec, kind FROM (
                       (SELECT DISTINCT ON (key) key, jdm, CAST(NULL AS jsonb) AS spec,
                               'decision' AS kind, 0 AS prio FROM decisions
                        WHERE status = 'published' AND env = $2 AND published_at <= $1
                        ORDER BY key, published_at DESC)
                       UNION ALL
                       (SELECT DISTINCT ON (key) key, jdm, spec, 'matrix' AS kind, 1 AS prio
                        FROM matrices
                        WHERE status = 'published' AND env = $2 AND published_at <= $1
                        ORDER BY key, published_at DESC)
                     ) t ORDER BY key, prio",
                    )
                    .bind(ts)
                    .bind(env.as_str())
                    .fetch_all(&self.pool)
                    .await?
                }
                Revision::LatestPublished => {
                    // Latest published per key in this lane; dev drafts fill keys
                    // with no published row — gated to `$1 = 'dev'` so drafts
                    // never leak into qa/uat/prod.
                    sqlx::query_as(
                        "WITH pub AS (
                       SELECT DISTINCT ON (key) key, jdm, spec, kind, 0 AS prio FROM (
                         SELECT key, jdm, CAST(NULL AS jsonb) AS spec, 'decision' AS kind,
                                published_at FROM decisions
                         WHERE status = 'published' AND env = $1
                         UNION ALL
                         SELECT key, jdm, spec, 'matrix' AS kind, published_at FROM matrices
                         WHERE status = 'published' AND env = $1
                       ) a ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, jdm, spec, kind, 1 AS prio FROM (
                         SELECT key, jdm, CAST(NULL AS jsonb) AS spec, 'decision' AS kind,
                                updated_at FROM decisions
                         WHERE status = 'draft' AND env = $1 AND $1 = 'dev'
                         UNION ALL
                         SELECT key, jdm, spec, 'matrix' AS kind, updated_at FROM matrices
                         WHERE status = 'draft' AND env = $1 AND $1 = 'dev'
                       ) a ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, jdm, spec, kind FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                    )
                    .bind(env.as_str())
                    .fetch_all(&self.pool)
                    .await?
                }
            };

        // Overrides resolve with identical revision semantics, then patch
        // each kind's patchable document below — before the MPE-127 compile
        // pass, so the content hash covers the patched bytes and an override
        // recompiles only its target.
        let override_rows: Vec<(String, String, String, String, serde_json::Value)> = match revision
        {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, target_kind, target_key, value_path, value
                         FROM overrides
                         WHERE status = 'draft' AND env = $1 ORDER BY key, updated_at DESC",
                )
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, target_kind, target_key, value_path, value
                         FROM overrides
                         WHERE status = 'published' AND env = $2 AND published_at <= $1
                         ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, target_kind, target_key, value_path, value,
                              0 AS prio
                       FROM overrides WHERE status = 'published' AND env = $1
                       ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, target_kind, target_key, value_path, value,
                              1 AS prio
                       FROM overrides WHERE status = 'draft' AND env = $1 AND $1 = 'dev'
                       ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, target_kind, target_key, value_path, value
                     FROM (SELECT * FROM pub UNION ALL SELECT * FROM drf) t
                     ORDER BY key, prio",
                )
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
        };
        // Deterministic application: sort by override key so two rows hitting
        // the same pointer resolve last-wins instead of plan-order-dependent.
        let mut override_rows: Vec<overrides::OverrideRow> = override_rows
            .into_iter()
            .map(
                |(key, target_kind, target_key, value_path, value)| overrides::OverrideRow {
                    key,
                    target_kind,
                    target_key,
                    value_path,
                    value,
                },
            )
            .collect();
        override_rows.sort_by(|a, b| a.key.cmp(&b.key));
        let mut by_target: HashMap<(&str, &str), Vec<&overrides::OverrideRow>> = HashMap::new();
        for o in &override_rows {
            by_target
                .entry((o.target_kind.as_str(), o.target_key.as_str()))
                .or_default()
                .push(o);
        }
        let mut applied_overrides = 0usize;

        let mem = std::sync::Arc::new(MemoryLoader::default());
        // Keys+JDM that entered the loader — phase 2 swaps each for its
        // shared compiled form (MPE-127).
        let mut loader_entries: Vec<(String, serde_json::Value)> = Vec::with_capacity(rows.len());
        for (key, jdm, spec, kind) in rows {
            let mut jdm = jdm;
            if let Some(list) = by_target.get(&(kind.as_str(), key.as_str())) {
                for ovr in list {
                    let patched = if kind == "matrix" {
                        // Matrices patch the authoring spec, then re-derive
                        // loader JDM — pointers address the doc pricers wrote.
                        match spec.clone() {
                            Some(mut doc) => {
                                overrides::apply(&mut doc, &ovr.value_path, &ovr.value)
                                    .and_then(|()| crate::rules::matrix::MatrixSpec::parse(&doc))
                                    .and_then(|m| m.to_jdm())
                            }
                            None => Err("matrix row carried no spec".to_string()),
                        }
                    } else {
                        overrides::apply(&mut jdm, &ovr.value_path, &ovr.value)
                            .map(|()| jdm.clone())
                    };
                    match patched {
                        Ok(p) => {
                            jdm = p;
                            applied_overrides += 1;
                        }
                        Err(e) => tracing::warn!(
                            override_key = %ovr.key,
                            target = %format!("{kind}:{key}"),
                            "override skipped at snapshot build: {e}"
                        ),
                    }
                }
            }
            let content: DecisionContent = serde_json::from_value(jdm.clone())
                .with_context(|| format!("decision {key}: invalid JDM"))?;
            mem.add(key.clone(), content);
            loader_entries.push((key, jdm));
        }

        // Rate sheet specs and matrix scoping metadata resolve with identical
        // revision semantics — data artifacts live beside the loader, not in it.
        let sheet_rows: Vec<(String, serde_json::Value)> = match revision {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, spec FROM rate_sheets
                     WHERE status = 'draft' AND env = $1 ORDER BY key, updated_at DESC",
                )
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, spec FROM rate_sheets
                     WHERE status = 'published' AND env = $2 AND published_at <= $1
                     ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, spec, 0 AS prio FROM rate_sheets
                       WHERE status = 'published' AND env = $1 ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, spec, 1 AS prio FROM rate_sheets
                       WHERE status = 'draft' AND env = $1 AND $1 = 'dev'
                       ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, spec FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                )
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
        };
        let rate_sheets: HashMap<String, serde_json::Value> = sheet_rows
            .into_iter()
            .map(|(key, spec)| {
                let mut spec = spec;
                if let Some(list) = by_target.get(&("rate_sheet", key.as_str())) {
                    for ovr in list {
                        match overrides::apply(&mut spec, &ovr.value_path, &ovr.value) {
                            Ok(()) => applied_overrides += 1,
                            Err(e) => tracing::warn!(
                                override_key = %ovr.key,
                                target = %format!("rate_sheet:{key}"),
                                "override skipped at snapshot build: {e}"
                            ),
                        }
                    }
                }
                (key, spec)
            })
            .collect();

        let matrix_rows: Vec<(String, String, Option<String>)> = match revision {
            Revision::Draft => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, kind, product_code FROM matrices
                     WHERE status = 'draft' AND env = $1 ORDER BY key, updated_at DESC",
                )
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, kind, product_code FROM matrices
                     WHERE status = 'published' AND env = $2 AND published_at <= $1
                     ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, kind, product_code, 0 AS prio
                       FROM matrices WHERE status = 'published' AND env = $1
                       ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, kind, product_code, 1 AS prio
                       FROM matrices WHERE status = 'draft' AND env = $1 AND $1 = 'dev'
                       ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, kind, product_code FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                )
                .bind(env.as_str())
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
                     WHERE status = 'draft' AND env = $1 ORDER BY key, updated_at DESC",
                )
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::AsOf(ts) => {
                sqlx::query_as(
                    "SELECT DISTINCT ON (key) key, kind, expression FROM expressions
                     WHERE status = 'published' AND env = $2 AND published_at <= $1
                     ORDER BY key, published_at DESC",
                )
                .bind(ts)
                .bind(env.as_str())
                .fetch_all(&self.pool)
                .await?
            }
            Revision::LatestPublished => {
                sqlx::query_as(
                    "WITH pub AS (
                       SELECT DISTINCT ON (key) key, kind, expression, 0 AS prio
                       FROM expressions WHERE status = 'published' AND env = $1
                       ORDER BY key, published_at DESC
                     ), drf AS (
                       SELECT DISTINCT ON (key) key, kind, expression, 1 AS prio
                       FROM expressions WHERE status = 'draft' AND env = $1 AND $1 = 'dev'
                       ORDER BY key, updated_at DESC
                     )
                     SELECT DISTINCT ON (key) key, kind, expression FROM (
                       SELECT * FROM pub UNION ALL SELECT * FROM drf
                     ) t ORDER BY key, prio",
                )
                .bind(env.as_str())
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
            // Expression overrides patch {"expression": <text>} then re-wrap.
            let mut expr_doc = overrides::expression_doc(expression);
            if let Some(list) = by_target.get(&("expression", key.as_str())) {
                for ovr in list {
                    match overrides::apply(&mut expr_doc, &ovr.value_path, &ovr.value) {
                        Ok(()) => applied_overrides += 1,
                        Err(e) => tracing::warn!(
                            override_key = %ovr.key,
                            target = %format!("expression:{key}"),
                            "override skipped at snapshot build: {e}"
                        ),
                    }
                }
            }
            let source = match overrides::expression_from_doc(&expr_doc) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(key, "expression override produced invalid doc: {e}");
                    expression
                }
            };
            let wrapper = expression_jdm(key, kind, source);
            let content: DecisionContent = serde_json::from_value(wrapper.clone())
                .with_context(|| format!("expression {key}: wrapper JDM invalid"))?;
            mem.add(key.clone(), content);
            loader_entries.push((key.clone(), wrapper));
            expressions += 1;
        }

        // Phase 2 (MPE-127): replace each raw entry with the shared compiled
        // form. Unchanged content hashes hit the cross-snapshot cache; only
        // keys whose bytes changed recompile. Runs after every raw add so
        // schema resolution sees the same full-catalog view zen's
        // `prepared()` would.
        let decision_count = loader_entries.len() - expressions;
        let dyn_loader: DynamicLoader = mem.clone();
        for (key, jdm) in loader_entries {
            let prepared = self
                .compiles
                .get_or_prepare(&key, &jdm, &dyn_loader)
                .await?;
            mem.add(key, (*prepared).clone());
        }

        let snapshot = Snapshot {
            fingerprint,
            loader: Arc::new(CachedLoader::from(mem as DynamicLoader)),
            rate_sheets,
            matrices,
        };
        tracing::Span::current()
            .record("decisions", decision_count)
            .record("expressions", expressions)
            .record("rate_sheets", snapshot.rate_sheets.len())
            .record("matrices", snapshot.matrices.len())
            .record("overrides", applied_overrides);
        tracing::debug!(
            ?revision,
            decisions = decision_count,
            expressions,
            overrides = applied_overrides,
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
