//! Active log masking (MPE-152, Zara invariant 10): a scrub layer in front
//! of every log sink. `tests/pii.rs` proves evaluation payloads are never
//! *recorded*; this module guarantees that even if a sensitive value slips
//! through — a Debug impl, a query string, a credential inside an error
//! message — the serialized output is masked before it reaches stdout or
//! the OTLP aggregation pipeline.
//!
//! Coverage:
//! - `mask()` — pure text rewriting for every serialization form: JSON
//!   `"key":"value"` (incl. nested objects), bare `key=value` / `key: value`
//!   debug and query-string output, DSN credentials
//!   (`postgres://user:pass@host`), and value shapes (SSN, 9-digit account
//!   numbers, compact JWTs) regardless of the field name carrying them.
//! - `MaskWriter` / `MaskingStdout` — wired into the stdout JSON fmt layer
//!   (`telemetry.rs`) so every emitted event is scrubbed line-wise.
//! - `MaskingProcessor` — wraps the OTLP log processor and masks the
//!   record *body* before export. SDK 0.33 exposes attributes read-only,
//!   so structured record attributes cannot be rewritten here — they are
//!   covered by the "never record sensitive fields" guarantee
//!   (`tests/pii.rs`) and the caveat is documented in docs/security.md.
//!   Sensitive *keys* inside the body's own Map/List values are masked.
//!
//! Replacement text is always `[MASKED]`. The bias is deliberate
//! over-masking: a benign field scrubbed is a nuisance, a leaked secret is
//! an incident.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::{self, Write};
use std::sync::LazyLock;
use std::time::Duration;

use opentelemetry::logs::{AnyValue, LogRecord as _, Severity};
use opentelemetry::InstrumentationScope;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::logs::{LogProcessor, SdkLogRecord};
use opentelemetry_sdk::Resource;
use regex::{Captures, Regex};

use crate::security::pii_guard::{normalize, tokenize};

/// Keys whose values are always sensitive — the `pii_guard` field-name
/// denylist plus credential material. Snake/camel/kebab/compound forms are
/// matched by tokenizing the candidate key, so `refreshToken`,
/// `borrower_ssn`, `x-api-key`, and `clientSecret` all hit while
/// `existingLiens`, `settings`, and `filename` pass.
const SENSITIVE_KEYS: &[&str] = &[
    // PII field names (mirrors pii_guard::DENYLIST).
    "ssn",
    "social_security",
    "tax_id",
    "tin",
    "ein",
    "client_name",
    "first_name",
    "last_name",
    "middle_name",
    "dob",
    "date_of_birth",
    "birth_date",
    "phone",
    "mobile",
    "address",
    "street_address",
    "zip",
    "zipcode",
    "postal",
    "account_number",
    "routing",
    "driver_license",
    "passport",
    "email",
    // Credential material.
    "password",
    "passwd",
    "secret",
    "token",
    "access_token",
    "refresh_token",
    "id_token",
    "api_key",
    "apikey",
    "authorization",
    "cookie",
    "credential",
];

const MASKED: &str = "[MASKED]";

/// Normalized (lowercase, alnum-only) concat forms — `date_of_birth` →
/// `dateofbirth` — for equality and substring checks on candidate keys.
static SENSITIVE_NORMS: LazyLock<BTreeSet<String>> =
    LazyLock::new(|| SENSITIVE_KEYS.iter().map(|k| normalize(k)).collect());

/// Same list as token sequences — `date_of_birth` → [date, of, birth] —
/// for contiguous token-window matching inside compound names.
static SENSITIVE_TOKENS: LazyLock<Vec<Vec<String>>> =
    LazyLock::new(|| SENSITIVE_KEYS.iter().map(|k| tokenize(k)).collect());

/// True when `key` names a sensitive field. Three tiers, checked cheaply:
/// normalized equality (`ssn`, `apikey`), token-window match inside
/// camel/snake compounds (`refreshToken`, `borrower_ssn`, `x-api-key`),
/// then substring on the normalized form for concatenated compounds
/// (`dbpassword`, `mysecretkey`) — restricted to patterns ≥5 chars so
/// `dob`/`tin`/`ein` can't substring-hit innocent words (`adobe`,
/// `settings`, `protein`).
fn is_sensitive_key(key: &str) -> bool {
    let norm = normalize(key);
    if SENSITIVE_NORMS.contains(&norm) {
        return true;
    }
    let tokens = tokenize(key);
    if SENSITIVE_TOKENS.iter().any(|deny| {
        deny.len() <= tokens.len() && tokens.windows(deny.len()).any(|w| w == deny.as_slice())
    }) {
        return true;
    }
    SENSITIVE_NORMS
        .iter()
        .any(|k| k.len() >= 5 && norm.contains(k.as_str()))
}

/// `"key" : <object-or-array>` — runs first. Under a sensitive key the
/// whole container is masked wholesale; under a benign key the captured
/// value is scrubbed *recursively* (`mask` on the inner text), so nested
/// sensitive pairs are found at any depth. One nested brace level is
/// matched per pass — deeper nests peel one level per recursive call.
static JSON_COMPLEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#""((?:[^"\\]|\\.)*)"(\s*:\s*)(\{(?:[^{}]|\{[^{}]*\})*\}|\[(?:[^\[\]]|\[[^\[\]]*\])*\])"#,
    )
    .unwrap()
});

/// `"key" : "value"` or `"key" : scalar`. `{`/`[` values are deliberately
/// excluded — JSON_COMPLEX owns those — so the match fails at that key and
/// the scanner moves inside the object.
static JSON_KV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""((?:[^"\\]|\\.)*)"(\s*:\s*)("(?:[^"\\]|\\.)*"|[^,\s\]\[{}]+)"#).unwrap()
});

/// `key=value` or `key="v v"` in Debug/query-string output. The prefix
/// guard (`[^"'\w]`) keeps JSON keys out — JSON_KV owns those — and keeps
/// `=` inside base64/JWT blobs from being treated as an assignment.
static BARE_EQ: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(^|[^"'\w])([\w.-]+)(\s*=\s*)("(?:[^"\\]|\\.)*"|[^\s&,"'\]}]+)"#).unwrap()
});

/// `key: value` in Rust Debug / YAML-ish output. Same prefix guard; URLs
/// can't match (`https:` is preceded by a word char).
static BARE_COLON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(^|[^"'\w])([\w.-]+)(\s*:\s*)("(?:[^"\\]|\\.)*"|[^\s,"'\]}]+)"#).unwrap()
});

/// `scheme://user:password@host` — the password segment of any DSN/URL
/// authority, independent of key naming (error messages embed these). The
/// password runs to the LAST `@` before the next `/` or whitespace so
/// `p@ss` passwords mask fully rather than leaking a fragment as "host".
static DSN_CREDENTIAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(://[^:/@\s]+):[^/\s]+@").unwrap());

/// Compact JWS/JWT — the header always begins `eyJ` (base64 of `{"`).
static JWT_SHAPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\beyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{3,}\b").unwrap()
});

/// SSN `###-##-####` wherever it appears.
static SSN_SHAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{3}-\d{2}-\d{4}").unwrap());

/// Bare 9-digit run — account-number shape (mirrors the pii_guard value
/// guard; boundary chars are captured so they can be put back).
static ACCOUNT_SHAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|[^\d])\d{9}([^\d]|$)").unwrap());

/// Run `re` over `hay`; keep the borrowed line when nothing matched so the
/// common path stays allocation-free.
fn apply<'a>(mut hay: Cow<'a, str>, re: &Regex, rep: impl Fn(&Captures) -> String) -> Cow<'a, str> {
    if let Cow::Owned(s) = re.replace_all(&hay, rep) {
        hay = Cow::Owned(s);
    }
    hay
}

/// Scrub `line` of every sensitive key/value and value shape. Ordered:
/// complex JSON values first (wholesale masking under a sensitive key),
/// then string/scalar JSON pairs (which also reach inside nested objects),
/// DSN credentials, bare debug/query forms, then free-floating value
/// shapes.
pub fn mask(line: &str) -> Cow<'_, str> {
    let mut out: Cow<str> = Cow::Borrowed(line);

    out = apply(out, &JSON_COMPLEX, |c| {
        if is_sensitive_key(&c[1]) {
            format!("\"{}\"{}\"{MASKED}\"", &c[1], &c[2])
        } else {
            // Benign container: keep the key, recursively scrub the value
            // so nested sensitive pairs still mask.
            format!("\"{}\"{}{}", &c[1], &c[2], mask(&c[3]))
        }
    });
    out = apply(out, &JSON_KV, |c| {
        if is_sensitive_key(&c[1]) {
            format!("\"{}\"{}\"{MASKED}\"", &c[1], &c[2])
        } else {
            c[0].to_string()
        }
    });
    out = apply(out, &DSN_CREDENTIAL, |c| format!("{}:[MASKED]@", &c[1]));
    out = apply(out, &BARE_EQ, |c| {
        if is_sensitive_key(&c[2]) {
            format!("{}{}{}={MASKED}", &c[1], &c[2], &c[3])
        } else {
            c[0].to_string()
        }
    });
    out = apply(out, &BARE_COLON, |c| {
        if is_sensitive_key(&c[2]) {
            format!("{}{}{}: {MASKED}", &c[1], &c[2], &c[3])
        } else {
            c[0].to_string()
        }
    });
    out = apply(out, &JWT_SHAPE, |_| MASKED.to_string());
    out = apply(out, &SSN_SHAPE, |_| MASKED.to_string());
    apply(out, &ACCOUNT_SHAPE, |c| {
        format!("{}[MASKED]{}", &c[1], &c[2])
    })
}

/// `io::Write` that masks each complete line before handing it to `inner`.
/// Output is buffered to the newline so a sensitive `"k":"v"` split across
/// two `write` calls can't leak half-unmasked; `flush`/`Drop` scrub and
/// emit any tail.
pub struct MaskWriter<W: Write> {
    inner: W,
    pending: Vec<u8>,
}

impl<W: Write> MaskWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            pending: Vec::new(),
        }
    }

    fn emit_pending(&mut self, flush_all: bool) -> io::Result<()> {
        while let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            let masked = mask(&text);
            self.inner.write_all(masked.as_bytes())?;
        }
        if flush_all && !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            let text = String::from_utf8_lossy(&rest);
            let masked = mask(&text);
            self.inner.write_all(masked.as_bytes())?;
        }
        Ok(())
    }
}

impl<W: Write> Write for MaskWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        self.emit_pending(false)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.emit_pending(true)?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for MaskWriter<W> {
    fn drop(&mut self) {
        let _ = self.emit_pending(true);
    }
}

/// `MakeWriter` yielding stdout behind the mask — the fmt layer calls this
/// once per event and JSON events end in a newline, so each event scrubs
/// whole before hitting the process's real output.
#[derive(Clone, Copy, Debug, Default)]
pub struct MaskingStdout;

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for MaskingStdout {
    type Writer = MaskWriter<io::Stdout>;

    fn make_writer(&'a self) -> Self::Writer {
        MaskWriter::new(io::stdout())
    }
}

/// Wraps an OTLP `LogProcessor`, masking the record body before forwarding.
/// `emit` is the SDK's mutation point ("changes made to the log data in
/// this method will be reflected in the next log processor"), so the
/// downstream batch processor only ever sees scrubbed bodies.
#[derive(Debug)]
pub struct MaskingProcessor<P> {
    inner: P,
}

impl<P> MaskingProcessor<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

impl<P: LogProcessor> LogProcessor for MaskingProcessor<P> {
    fn emit(&self, data: &mut SdkLogRecord, instrumentation: &InstrumentationScope) {
        mask_record(data);
        self.inner.emit(data, instrumentation);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }

    fn shutdown(&self) -> OTelSdkResult {
        self.inner.shutdown()
    }

    fn event_enabled(&self, level: Severity, target: &str, name: Option<&str>) -> bool {
        self.inner.event_enabled(level, target, name)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource)
    }
}

/// Mask the record body in place. Structured attributes are read-only in
/// SDK 0.33 — see the module docs / docs/security.md for that caveat.
fn mask_record(record: &mut SdkLogRecord) {
    let masked = record.body().map(mask_any_value);
    if let Some(body) = masked {
        record.set_body(body);
    }
}

/// Deep-scrub an `AnyValue`: strings go through `mask()`, map entries
/// under sensitive keys are replaced outright (a `"password"` map field is
/// the same leak as a JSON pair), and lists/maps recurse.
fn mask_any_value(v: &AnyValue) -> AnyValue {
    match v {
        AnyValue::String(s) => AnyValue::from(mask(s.as_str()).into_owned()),
        AnyValue::ListAny(items) => items.iter().map(mask_any_value).collect(),
        AnyValue::Map(map) => AnyValue::Map(Box::new(
            map.iter()
                .map(|(k, val)| {
                    let val = if is_sensitive_key(k.as_str()) {
                        AnyValue::from(MASKED)
                    } else {
                        mask_any_value(val)
                    };
                    (k.clone(), val)
                })
                .collect(),
        )),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key category the ticket requires, as a JSON `"k":"v"` pair.
    #[test]
    fn json_pairs_mask_all_required_keys() {
        for key in [
            "ssn",
            "tax_id",
            "client_name",
            "password",
            "secret",
            "token",
            "apikey",
            "authorization",
            "cookie",
        ] {
            let line = format!(r#"{{"msg":"x","{key}":"sensitive-value-1"}}"#);
            let out = mask(&line);
            assert!(
                !out.contains("sensitive-value-1"),
                "{key} value must be masked: {out}"
            );
            assert!(
                out.contains(&format!("\"{key}\":\"{MASKED}\"")),
                "{key} must show the marker: {out}"
            );
        }
    }

    /// camelCase / snake_case / kebab / concatenated compounds all hit.
    #[test]
    fn compound_key_forms_mask() {
        for key in [
            "refreshToken",
            "borrower_ssn",
            "clientSecret",
            "x-api-key",
            "apiKey",
            "access_token",
            "dbpassword",
            "borrowerSsn",
        ] {
            let line = format!(r#"{{"{key}":"s3cret"}}"#);
            let out = mask(&line);
            assert!(!out.contains("s3cret"), "{key} compound must mask: {out}");
        }
    }

    /// Scalar, null, and nested JSON values.
    #[test]
    fn json_scalar_and_nested_values() {
        let out = mask(
            r#"{"ssn":123,"ok":1,"session":{"token":"abc123"},"auth":{"inner":{"password":"pw"}}}"#,
        );
        assert!(!out.contains("123"), "{out}");
        assert!(!out.contains("abc123"), "nested ssn/token pair: {out}");
        assert!(!out.contains("\"pw\""), "deep nested password: {out}");
        assert!(out.contains(r#""ok":1"#), "benign sibling kept: {out}");

        // Whole object/array under a sensitive key masks wholesale.
        let out = mask(r#"{"token":{"jwt":"x"},"cookie":["a","b"]}"#);
        assert!(!out.contains("jwt"), "{out}");
        assert_eq!(out.matches(MASKED).count(), 2, "{out}");
    }

    /// `k=v`, `k: v`, and quoted-value variants.
    #[test]
    fn bare_forms_mask() {
        let out = mask("password=hunter2 token=abc.def ssn: 123-45-6789 ok=1");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("abc.def"), "{out}");
        assert!(!out.contains("123-45-6789"), "{out}");
        assert!(out.contains("ok=1"), "{out}");

        let out = mask(r#"password="hunter2 more" refresh_token="r.e.f""#);
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("r.e.f"), "{out}");

        // URL query strings.
        let out = mask("GET /cb?session_token=abc&code=xyz HTTP/1.1");
        assert!(!out.contains("session_token=abc"), "{out}");
        assert!(out.contains("code=xyz"), "{out}");
    }

    /// DSN credentials, SSN/account shapes, and compact JWTs anywhere in
    /// the line — independent of key names.
    #[test]
    fn value_shapes_mask() {
        let out = mask("connect failed: postgres://mpex:s3cretP@ss@db:5432/mpex");
        assert!(!out.contains("s3cretP@ss"), "{out}");
        assert!(out.contains("postgres://mpex:[MASKED]@db"), "{out}");

        let out = mask("applicant 123-45-6789 verified");
        assert_eq!(out, "applicant [MASKED] verified");

        let out = mask("acct=123456789 end");
        assert!(!out.contains("123456789"), "{out}");

        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjMiLCJhdWQiOiJtcGV4In0.c2lnbmF0dXJlYnl0ZXM";
        let line = format!("authorization header: Bearer {jwt}");
        let out = mask(&line);
        assert!(!out.contains("c2lnbmF0dXJlYnl0ZXM"), "{out}");
        assert!(out.contains("[MASKED]"), "{out}");
    }

    /// Benign lines pass through untouched — including the tricky cases:
    /// `existingLiens` (contains "tin"), `settings` ("tin"), `filename`
    /// ("name"), `destination` ("tin"), and actor emails (attribution, not
    /// payload PII — `email` masks by key only).
    #[test]
    fn benign_text_untouched() {
        let line = r#"{"msg":"pricing evaluated","elapsedMs":3.2,"actor":"admin@mpex.dev","loan":{"amount":450000,"termMonths":360},"settings":{"theme":"dark"},"existingLiens":0,"filename":"rates.csv","destination":"/tmp"}"#;
        assert_eq!(mask(line), line);
    }

    /// A sensitive value split across two writes must not leak — output is
    /// buffered to the newline.
    #[test]
    fn mask_writer_buffers_partial_lines() {
        let mut buf = Vec::new();
        {
            let mut w = MaskWriter::new(&mut buf);
            w.write_all(b"{\"ssn\":\"123-45").unwrap();
            w.write_all(b"-6789\"}\nrest").unwrap();
            w.flush().unwrap();
        }
        let out = String::from_utf8(buf).unwrap();
        assert!(!out.contains("123-45"), "split value leaked: {out}");
        assert!(out.contains(MASKED), "{out}");
        assert!(out.ends_with("rest"), "{out}");
    }

    /// End-to-end through the fmt layer: a tracing field named `ssn`
    /// nested inside the event's `fields` object is masked on the wire.
    #[test]
    fn fmt_layer_masks_events() {
        use std::sync::{Arc, Mutex};
        use tracing_subscriber::fmt::MakeWriter;

        #[derive(Clone)]
        struct SharedBuf(Arc<Mutex<Vec<u8>>>);
        impl Write for SharedBuf {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        struct MakeBuf(SharedBuf);
        impl<'a> MakeWriter<'a> for MakeBuf {
            type Writer = MaskWriter<SharedBuf>;
            fn make_writer(&'a self) -> Self::Writer {
                MaskWriter::new(self.0.clone())
            }
        }

        let buf = SharedBuf(Arc::new(Mutex::new(Vec::new())));
        let sub = tracing_subscriber::fmt()
            .with_writer(MakeBuf(buf.clone()))
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(sub, || {
            tracing::info!(ssn = "123-45-6789", actor = "admin@mpex.dev", "eval done");
        });

        let out = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(out.contains(MASKED), "event not masked: {out}");
        assert!(
            !out.contains("123-45-6789"),
            "ssn field leaked in event: {out}"
        );
        assert!(
            out.contains("admin@mpex.dev"),
            "actor attribution must survive: {out}"
        );
    }
}
