//! Value overrides (MPE-126): append-only artifact rows that surgically
//! patch a target artifact's resolved content at snapshot-build time, so an
//! operational constant can change without republishing the whole graph or
//! spec that contains it.
//!
//! An override row says: "in the resolved document for
//! `(target_kind, target_key)`, replace the scalar leaf at `value_path`
//! (RFC 6901 JSON Pointer) with `value`." The patchable document differs by
//! target kind — the document the pipeline actually consumes:
//!
//! - `decision` — the JDM graph (`decisions.jdm`)
//! - `matrix` — the authoring spec (`matrices.spec`); the loader JDM is
//!   re-derived after patching
//! - `rate_sheet` — the sheet spec (`rate_sheets.spec`)
//! - `expression` — `{"expression": <text>}`, the stored source text
//!
//! Overrides resolve through the same revision/lane pipeline as their
//! targets: drafts apply to the draft working set, published rows apply to
//! published/asOf snapshots, and `LatestPublished` fills unpublished keys
//! from dev drafts. Application is deterministic — overrides sort by key and
//! apply in order, so two rows hitting the same pointer resolve last-wins.
//! A row whose target or pointer fails to resolve at build time is skipped
//! with a warn (never fails the whole snapshot); PUT-time validation against
//! the current working set keeps that from happening in the first place.

use serde_json::Value;

/// One resolved override row, in application order (sorted by key).
#[derive(Debug, Clone)]
pub struct OverrideRow {
    pub key: String,
    pub target_kind: String,
    pub target_key: String,
    pub value_path: String,
    pub value: Value,
}

/// Valid `target_kind` values — mirrors the CHECK in migration 0013.
pub const TARGET_KINDS: &[&str] = &["decision", "expression", "matrix", "rate_sheet"];

/// Scalars only: an override replaces a leaf, never a subtree. `null` is a
/// legitimate leaf value.
pub fn is_scalar(v: &Value) -> bool {
    !matches!(v, Value::Object(_) | Value::Array(_))
}

/// Basic pointer hygiene shared by PUT validation and build-time apply —
/// `""` (whole document) and non-`/` strings aren't patchable leaves.
pub fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || !path.starts_with('/') {
        return Err(format!(
            "valuePath {path:?} must be a JSON Pointer like \"/nodes/0/name\""
        ));
    }
    Ok(())
}

/// Replace the scalar leaf `path` addresses inside `doc`. Errors when the
/// pointer doesn't resolve or addresses a structural node — overrides never
/// create nodes or replace subtrees, so a bad patch is always a bad row,
/// not silent structural surgery.
pub fn apply(doc: &mut Value, path: &str, value: &Value) -> Result<(), String> {
    validate_path(path)?;
    if !is_scalar(value) {
        return Err("override value must be a scalar leaf (string, number, bool, null)".into());
    }
    let slot = doc
        .pointer_mut(path)
        .ok_or_else(|| format!("valuePath {path:?} does not resolve in the target document"))?;
    if !is_scalar(slot) {
        return Err(format!(
            "valuePath {path:?} addresses a structural node; overrides replace scalar leaves only"
        ));
    }
    *slot = value.clone();
    Ok(())
}

/// The document an expression override patches: just its source text under
/// `/expression`.
pub fn expression_doc(source: &str) -> Value {
    serde_json::json!({ "expression": source })
}

/// Extract the patched expression text — must remain a string.
pub fn expression_from_doc(doc: &Value) -> Result<&str, String> {
    doc.get("expression")
        .and_then(Value::as_str)
        .ok_or_else(|| "patched /expression must be a string".to_string())
}
