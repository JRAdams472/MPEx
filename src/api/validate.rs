//! Input hardening (MPE-97 phase 5): bounded key space on artifact routes
//! and explicit size caps on text/spec payloads. Coarse body bytes are
//! capped by `DefaultBodyLimit` in `api::router`; these are the semantic
//! bounds applied before any DB work or spec compilation.

use serde_json::Value;

use crate::error::AppError;

/// `{key}` on artifact routes: `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`.
/// Bounds the characters that reach queries and path-derived identifiers —
/// no whitespace, slashes, control chars, or non-ASCII.
pub fn validate_key(key: &str) -> Result<(), AppError> {
    let ok = !key.is_empty()
        && key.len() <= 128
        && key.starts_with(|c: char| c.is_ascii_alphanumeric())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "invalid key {key:?}: must match ^[A-Za-z0-9][A-Za-z0-9._-]{{0,127}}$"
        )))
    }
}

/// Publication label cap.
pub const MAX_LABEL_LEN: usize = 200;

pub fn validate_label(label: &str) -> Result<(), AppError> {
    if label.chars().count() > MAX_LABEL_LEN {
        return Err(AppError::BadRequest(format!(
            "label too long: {} chars, max {MAX_LABEL_LEN}",
            label.chars().count()
        )));
    }
    Ok(())
}

/// Expression source cap — compiling a megabyte of ZEN is a CPU cost.
const MAX_EXPR_BYTES: usize = 64 * 1024;

pub fn validate_expression_source(src: &str) -> Result<(), AppError> {
    if src.len() > MAX_EXPR_BYTES {
        return Err(AppError::BadRequest(format!(
            "expression too large: {} bytes, max {MAX_EXPR_BYTES}",
            src.len()
        )));
    }
    Ok(())
}

/// Serialized cap for JSONB payloads (jdm / spec) — 1 MiB inside the 2 MiB
/// body cap keeps oversized specs from reaching the compilers.
const MAX_SPEC_BYTES: usize = 1024 * 1024;

pub fn validate_spec_size(spec: &Value, field: &str) -> Result<(), AppError> {
    let size = serde_json::to_vec(spec)
        .map(|b| b.len())
        .unwrap_or(usize::MAX);
    if size > MAX_SPEC_BYTES {
        return Err(AppError::BadRequest(format!(
            "{field} too large: {size} bytes, max {MAX_SPEC_BYTES}"
        )));
    }
    Ok(())
}
