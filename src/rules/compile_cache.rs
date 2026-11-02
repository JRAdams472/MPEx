//! Process-wide compiled-graph cache (MPE-127). `CachedLoader` compiled
//! entries die with their snapshot, so any publish forced a full-catalog
//! recompile on the next eval. Keying compiled content by
//! `(key, content_hash)` instead lets unchanged artifacts survive snapshot
//! invalidation — a publish recompiles only the keys whose bytes changed.
//!
//! Entries are the same `Arc<DecisionContent>` `CachedLoader` would have
//! produced (`compile()` + `resolve_schemas()` pre-run), so downstream
//! behavior is unchanged — the per-snapshot loader hits its
//! `compiled_cache.is_some()` fast path.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::Context;
use zen_engine::loader::DynamicLoader;
use zen_engine::model::{DecisionContent, GraphContent};

use crate::audit;

/// Bound on distinct `(key, hash)` entries — published versions are
/// immutable so churn is the only growth pressure; clear-all on full,
/// matching `MEMO_CAP`'s pattern.
const COMPILE_CAP: usize = 4096;

#[derive(Default)]
pub struct CompileCache {
    inner: RwLock<HashMap<(String, String), Arc<DecisionContent>>>,
    /// Real compiles performed — acceptance instrumentation: a publish
    /// should raise this by the number of *changed* keys, not the catalog.
    misses: AtomicU64,
}

impl CompileCache {
    /// Return compiled content for `(key, jdm)`, compiling once per distinct
    /// content hash across the process. `loader` is the snapshot's raw
    /// loader — `resolve_schemas` sees the same view zen's `prepared()`
    /// would (every key already added before the compile pass runs).
    pub async fn get_or_prepare(
        &self,
        key: &str,
        jdm: &serde_json::Value,
        loader: &DynamicLoader,
    ) -> anyhow::Result<Arc<DecisionContent>> {
        let content: DecisionContent = serde_json::from_value(jdm.clone())
            .with_context(|| format!("decision {key}: invalid JDM"))?;

        // Graphs importing schema dictionaries resolve against snapshot-local
        // content — sharing their compiled form across snapshots could go
        // stale. MPEx JDMs never set imports; bypass the cache when one does.
        let shareable = !matches!(&content, DecisionContent::Graph(g) if !g.imports.is_empty());
        let hash = shareable.then(|| audit::content_hash(jdm));
        if let Some(h) = &hash {
            if let Some(hit) = self
                .inner
                .read()
                .ok()
                .and_then(|m| m.get(&(key.to_string(), h.clone())).cloned())
            {
                return Ok(hit);
            }
        }

        let prepared = prepare(content, loader).await?;
        self.misses.fetch_add(1, Ordering::Relaxed);
        if let Some(h) = hash {
            if let Ok(mut m) = self.inner.write() {
                if m.len() >= COMPILE_CAP {
                    m.clear();
                }
                m.insert((key.to_string(), h), prepared.clone());
            }
        }
        Ok(prepared)
    }

    /// Compiled-entry misses since boot — test/observability surface.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

/// Mirror of zen's `compiled()`/`prepared()`: clone the graph, compile it,
/// then resolve schema references against `loader`.
async fn prepare(
    content: DecisionContent,
    loader: &DynamicLoader,
) -> anyhow::Result<Arc<DecisionContent>> {
    let DecisionContent::Graph(graph) = content else {
        return Ok(Arc::new(content));
    };
    if graph.compiled_cache.is_some() && graph.resolved_schemas.is_some() {
        return Ok(Arc::new(DecisionContent::Graph(graph)));
    }
    let mut owned: GraphContent = (*graph).clone();
    owned.compile();
    owned
        .resolve_schemas(loader)
        .await
        .map_err(|e| anyhow::anyhow!("schema resolution failed: {e}"))?;
    Ok(Arc::new(DecisionContent::Graph(Arc::new(owned))))
}
