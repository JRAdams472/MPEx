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
    pub fico: Option<u32>,
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
