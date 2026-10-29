//! Inbound PII guard (MPE-97 phase 6): recursive JSON walk of `/api/evaluate/*`
//! loan payloads. The AGENTS invariant says the loan model carries loan and
//! scenario attributes only — never names, SSNs, addresses, or account
//! numbers. This enforces it at the boundary instead of trusting callers.
//!
//! Two checks per object field:
//! 1. **Name denylist** — matched on word boundaries after splitting camel /
//!    snake / kebab case, so `homePhone`, `borrowerDateOfBirth`, and
//!    `SSN_Number` all hit while `existingLiens` (contains "tin") and
//!    `property.state` pass. Concatenated forms (`firstname`, `dateofbirth`)
//!    match by normalized equality.
//! 2. **Value-shape regexes** on string leaves — SSN `###-##-####`, a bare
//!    9-digit run, and an email shape — independent of the field name.
//!
//! Rejections return `AppError::BadRequest` carrying the *field path* only —
//! the offending value never appears in the error (and so never in logs).

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::error::AppError;

/// Denied field names, as written in the ticket. Multi-word entries become
/// token sequences after splitting.
const DENYLIST: &[&str] = &[
    "ssn",
    "social_security",
    "first_name",
    "last_name",
    "middle_name",
    "dob",
    "date_of_birth",
    "birth_date",
    "email",
    "phone",
    "mobile",
    "address",
    "street_address",
    "zip",
    "postal",
    "account_number",
    "routing",
    "driver_license",
    "passport",
    "tin",
    "ein",
];

/// Normalized (lowercase, alnum-only) concat forms — `dateOfBirth`,
/// `date_of_birth`, and `dateofbirth` all collapse to `dateofbirth`.
static DENY_NORMS: LazyLock<BTreeSet<String>> =
    LazyLock::new(|| DENYLIST.iter().map(|s| normalize(s)).collect());

/// Same denylist as token sequences — `date_of_birth` → [date, of, birth] —
/// for contiguous token-sequence matching inside compound names.
static DENY_TOKENS: LazyLock<Vec<Vec<String>>> =
    LazyLock::new(|| DENYLIST.iter().map(|s| tokenize(s)).collect());

/// SSN, bare 9-digit run (account-number shape), email shape — checked on
/// every string leaf regardless of its field name.
static VALUE_SHAPES: LazyLock<[Regex; 3]> = LazyLock::new(|| {
    [
        Regex::new(r"\d{3}-\d{2}-\d{4}").unwrap(),
        Regex::new(r"(?:^|[^\d])\d{9}(?:[^\d]|$)").unwrap(),
        Regex::new(r"[^@\s]+@[^@\s]+\.[^@\s]+").unwrap(),
    ]
});

/// Reject the first PII-looking field or value in `loan`; `Ok` if clean.
/// `path` roots paths for error reporting ("" = the scenario itself).
pub fn check_loan(loan: &Value) -> Result<(), AppError> {
    if let Some(path) = first_violation(loan, "") {
        // Path only — never the value. The caller logs this message.
        return Err(AppError::BadRequest(format!("PII rejected: {path}")));
    }
    Ok(())
}

/// Depth-first walk returning the dotted path of the first violation.
/// Array elements appear as `field[i]`.
fn first_violation(v: &Value, path: &str) -> Option<String> {
    match v {
        Value::Object(map) => map.iter().find_map(|(k, val)| {
            let child = join(path, k);
            if name_denied(k) {
                return Some(child);
            }
            first_violation(val, &child)
        }),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(i, val)| first_violation(val, &format!("{path}[{i}]"))),
        Value::String(s) if value_denied(s) => Some(if path.is_empty() {
            "<root>".to_string()
        } else {
            path.to_string()
        }),
        _ => None,
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// Lowercase + alnum only: `dateOfBirth` ≡ `date_of_birth` ≡ `dateofbirth`.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Split a name on non-alphanumeric chars and camelCase boundaries:
/// `homePhone` → [home, phone], `SSN_Number` → [ssn, number].
fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            // Camel boundary: lowercase/digit → uppercase splits the token.
            if c.is_uppercase()
                && cur
                    .chars()
                    .last()
                    .is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
            {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.iter().map(|t| normalize(t)).collect()
}

fn name_denied(key: &str) -> bool {
    if DENY_NORMS.contains(&normalize(key)) {
        return true;
    }
    let tokens = tokenize(key);
    DENY_TOKENS.iter().any(|deny| {
        deny.len() <= tokens.len() && tokens.windows(deny.len()).any(|w| w == deny.as_slice())
    })
}

fn value_denied(s: &str) -> bool {
    VALUE_SHAPES.iter().any(|re| re.is_match(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn denied(loan: &Value) -> Option<String> {
        first_violation(loan, "")
    }

    #[test]
    fn denylist_hits() {
        for key in [
            "ssn",
            "firstName",
            "last_name",
            "dateOfBirth",
            "homePhone",
            "streetAddress",
            "accountNumber",
            "driverLicense",
            "passport",
            "borrowerTIN",
            "routing_number",
        ] {
            let loan = json!({ "borrower": { key: "x" } });
            assert!(
                denied(&loan).is_some(),
                "{key} must be denied (value is irrelevant)"
            );
        }
    }

    #[test]
    fn value_shapes_hit() {
        for v in [
            "123-45-6789",    // ssn shape
            "acct 123456789", // bare 9-digit run
            "me@example.com", // email shape
            "call me at x@y.io",
        ] {
            let loan = json!({ "borrower": { "notes": v } });
            assert!(denied(&loan).is_some(), "value {v:?} must be denied");
        }
    }

    #[test]
    fn legit_fields_do_not_trip() {
        let loan = json!({
            "loan": {"purpose": "purchase", "type": "conventional", "amount": 450000,
                     "termMonths": 360, "ltv": 80.0, "dti": 36.5, "units": 1},
            "property": {"type": "sfr", "occupancy": "primary", "state": "TX"},
            "borrower": {"fico": 740, "existingLiens": 0},
            "scenario": {"lockTermDays": 30, "estimatedClose": "2026-12-01"}
        });
        assert_eq!(denied(&loan), None);
    }

    #[test]
    fn deep_nesting_caught() {
        let loan = json!({
            "borrower": { "coborrowers": [ { "fico": 700 }, { "contact": { "ssn": "x" } } ] }
        });
        assert_eq!(
            denied(&loan).as_deref(),
            Some("borrower.coborrowers[1].contact.ssn")
        );
    }

    #[test]
    fn check_loan_returns_path_only() {
        let secret = "123-45-6789";
        let err = check_loan(&json!({ "borrower": { "ssn": secret } })).unwrap_err();
        assert!(
            !err.to_string().contains(secret),
            "error must not leak the value: {err}"
        );
        assert!(err.to_string().contains("borrower.ssn"));
    }
}
