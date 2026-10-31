use serde::Deserialize;
use serde_json::{Map, Value};

/// Rate sheet spec — a per-product rate x lock-term price grid. `prices` is a
/// sparse map `"<rate>:<lockTermDays>"` -> par-adjusted price (e.g. 99.875 =
/// 12.5bp cost). Missing cells mean that rate/term combo is not offered.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateSheetSpec {
    /// Offered note rates, e.g. [6.25, 6.5, 6.75]. Order is the display order.
    pub rates: Vec<f64>,
    /// Offered lock terms in days, e.g. [30, 45, 60].
    pub lock_terms: Vec<u32>,
    /// `"<rate>:<term>"` -> price. Rates/terms must appear in the lists above.
    #[serde(default)]
    pub prices: Map<String, Value>,
}

/// One resolved grid cell.
#[derive(Debug, Clone, Copy)]
pub struct GridPoint {
    pub rate: f64,
    pub lock_term_days: u32,
    pub price: f64,
}

impl RateSheetSpec {
    /// Parses and validates: non-empty rate/term lists with no duplicates,
    /// every price key is `"<rate>:<term>"` referencing listed values, every
    /// price is a finite number.
    pub fn parse(spec: &Value) -> Result<Self, String> {
        let parsed: Self = serde_json::from_value(spec.clone())
            .map_err(|e| format!("invalid rate sheet spec: {e}"))?;
        parsed.validate()?;
        Ok(parsed)
    }

    fn validate(&self) -> Result<(), String> {
        validate_lists(&self.rates, &self.lock_terms)?;
        validate_uniques(&self.rates, &self.lock_terms)?;
        self.validate_prices()
    }

    fn validate_prices(&self) -> Result<(), String> {
        for (k, v) in &self.prices {
            let (rate, term) = split_price_key(k)?;
            if !self.rates.iter().any(|r| rate_key(*r) == rate) {
                return Err(format!("price {k:?} references unlisted rate"));
            }
            if !self.lock_terms.contains(&term) {
                return Err(format!("price {k:?} references unlisted lock term"));
            }
            match v.as_f64() {
                Some(p) if p.is_finite() => {}
                _ => return Err(format!("price {k:?} must be a finite number")),
            }
        }
        Ok(())
    }

    /// Every priced grid point in (rates x lockTerms) order — rates outer,
    /// terms inner, so a rate's line reads across terms.
    pub fn grid(&self) -> Vec<GridPoint> {
        let mut out = Vec::new();
        for &rate in &self.rates {
            for &term in &self.lock_terms {
                if let Some(price) = self.price(rate, term) {
                    out.push(GridPoint {
                        rate,
                        lock_term_days: term,
                        price,
                    });
                }
            }
        }
        out
    }

    /// Price at one grid point; None when the combo isn't offered.
    pub fn price(&self, rate: f64, lock_term_days: u32) -> Option<f64> {
        self.prices
            .get(&format!("{}:{}", rate_key(rate), lock_term_days))
            .and_then(Value::as_f64)
    }
}

/// Rates serialize in keys like "6.5" — normalize via ryu-style shortest
/// round-trip so 6.50 and 6.5 collide as intended.
pub fn rate_key(rate: f64) -> String {
    format!("{rate}")
}

/// Non-empty checks plus per-element value constraints, in the original order:
/// rates empty, lockTerms empty, rate values, term values.
fn validate_lists(rates: &[f64], lock_terms: &[u32]) -> Result<(), String> {
    if rates.is_empty() {
        return Err("rates must be non-empty".into());
    }
    if lock_terms.is_empty() {
        return Err("lockTerms must be non-empty".into());
    }
    for r in rates {
        if !r.is_finite() || *r <= 0.0 {
            return Err(format!("rate must be positive and finite: {r}"));
        }
    }
    for t in lock_terms {
        if *t == 0 {
            return Err("lockTerms must be positive".into());
        }
    }
    Ok(())
}

fn validate_uniques(rates: &[f64], lock_terms: &[u32]) -> Result<(), String> {
    if has_dup(rates.iter().map(|r| rate_key(*r))) {
        return Err("duplicate rate in rates".into());
    }
    if has_dup(lock_terms.iter().copied()) {
        return Err("duplicate term in lockTerms".into());
    }
    Ok(())
}

fn split_price_key(key: &str) -> Result<(String, u32), String> {
    let (r, t) = key
        .split_once(':')
        .ok_or_else(|| format!("price key must be \"<rate>:<term>\", got {key:?}"))?;
    let rate: f64 = r
        .parse()
        .map_err(|_| format!("price key {key:?} has a non-numeric rate"))?;
    let term: u32 = t
        .parse()
        .map_err(|_| format!("price key {key:?} has a non-integer term"))?;
    Ok((rate_key(rate), term))
}

fn has_dup<T: std::hash::Hash + Eq>(mut items: impl Iterator<Item = T>) -> bool {
    let mut seen = std::collections::HashSet::new();
    !items.all(|i| seen.insert(i))
}
