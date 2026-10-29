use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// PII-free loan scenario submitted to eligibility/pricing evaluation.
/// All sections/fields are optional; unknown extension fields are preserved
/// verbatim so rules can reference them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoanScenario {
    #[serde(default)]
    pub loan: LoanSection,
    #[serde(default)]
    pub property: PropertySection,
    #[serde(default)]
    pub borrower: BorrowerSection,
    #[serde(default)]
    pub scenario: ScenarioSection,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl LoanScenario {
    /// Validates the payload shape, then returns the original JSON value so
    /// the engine context keeps every field (typed + extension).
    pub fn parse(value: &Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(value.clone())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoanSection {
    pub purpose: Option<LoanPurpose>,
    pub r#type: Option<LoanType>,
    pub amount: Option<f64>,
    pub term_months: Option<u32>,
    pub amortization: Option<Amortization>,
    pub arm: Option<ArmDetails>,
    pub interest_only: Option<bool>,
    pub ltv: Option<f64>,
    pub cltv: Option<f64>,
    pub dti: Option<f64>,
    pub units: Option<u32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArmDetails {
    pub initial_fixed_months: Option<u32>,
    pub index: Option<String>,
    pub margin: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertySection {
    pub r#type: Option<PropertyType>,
    pub occupancy: Option<Occupancy>,
    pub state: Option<String>,
    pub appraised_value: Option<f64>,
    pub purchase_price: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BorrowerSection {
    pub credit_score: Option<u32>,
    pub credit_model: Option<CreditModel>,
    pub first_time_buyer: Option<bool>,
    pub self_employed: Option<bool>,
    pub reserves_months: Option<f64>,
    pub escrow_waiver: Option<bool>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioSection {
    pub lock_term_days: Option<u32>,
    pub as_of: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Which credit-scoring model produced `credit_score`. Known models are
/// typed; `Other` passes any future/unknown model string through so new
/// models don't need a schema change (it re-serializes to the original
/// string for ZEN).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreditModel {
    #[serde(rename = "FICO")]
    Fico,
    VantageScore,
    #[serde(rename = "F10T")]
    Fico10T,
    #[serde(untagged)]
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoanPurpose {
    Purchase,
    RateTermRefi,
    CashOutRefi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoanType {
    Conventional,
    Fha,
    Va,
    Usda,
    Jumbo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Amortization {
    Fixed,
    Arm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyType {
    Sfr,
    Condo,
    Townhouse,
    #[serde(rename = "multiUnit2_4")]
    MultiUnit2To4,
    Manufactured,
    Pud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Occupancy {
    Primary,
    SecondHome,
    Investment,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_pair_parses_and_round_trips() {
        let s: BorrowerSection =
            serde_json::from_str(r#"{"creditScore": 740, "creditModel": "FICO"}"#).unwrap();
        assert_eq!(s.credit_score, Some(740));
        assert_eq!(s.credit_model, Some(CreditModel::Fico));
        // Re-serializes for ZEN as the same wire values.
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["creditScore"], 740);
        assert_eq!(v["creditModel"], "FICO");
    }

    #[test]
    fn known_models_deserialize() {
        for (s, want) in [
            ("FICO", CreditModel::Fico),
            ("VantageScore", CreditModel::VantageScore),
            ("F10T", CreditModel::Fico10T),
        ] {
            assert_eq!(
                serde_json::from_str::<CreditModel>(&format!("\"{s}\"")).unwrap(),
                want
            );
        }
    }

    #[test]
    fn future_model_passes_through() {
        let m: CreditModel = serde_json::from_str("\"ExperianNext\"").unwrap();
        assert_eq!(m, CreditModel::Other("ExperianNext".into()));
        assert_eq!(serde_json::to_value(&m).unwrap(), "ExperianNext");
    }

    #[test]
    fn non_string_model_rejected() {
        assert!(serde_json::from_str::<CreditModel>("123").is_err());
        assert!(serde_json::from_str::<CreditModel>("true").is_err());
    }
}
