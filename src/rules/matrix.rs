use serde::Deserialize;
use serde_json::{json, Map, Value};

/// Row×column adjustment matrix spec — the stored authoring format. Each row
/// and column is a standard ZEN rule evaluated against the full loan context;
/// a populated cell's value applies when its row AND column rules are true.
/// Cells may overlap — all matches collect into `adjustments[]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatrixSpec {
    pub rows: Vec<MatrixAxis>,
    pub cols: Vec<MatrixAxis>,
    /// Sparse map `"<rowId>:<colId>"` -> scalar adjustment (number/bool/string).
    #[serde(default)]
    pub cells: Map<String, Value>,
    /// How downstream combines matches: sum (default) | list | min | max.
    #[serde(default)]
    pub op: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatrixAxis {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Standard ZEN expression evaluated in the full context, e.g.
    /// `loan.ltv > 80 && loan.ltv <= 90`.
    pub expr: String,
}

/// One spec cell exploded for evaluation/debug output.
pub struct MatrixCell<'a> {
    pub row_id: &'a str,
    pub col_id: &'a str,
    pub row_expr: &'a str,
    pub col_expr: &'a str,
    pub value: &'a Value,
}

const OPS: [&str; 4] = ["sum", "list", "min", "max"];

impl MatrixSpec {
    /// Parses and validates a spec: ids are unique and colon-free, row/col
    /// expressions compile as standard ZEN, every cell references a known
    /// row:col pair and carries a scalar value, `op` is a known aggregator.
    pub fn parse(spec: &Value) -> Result<Self, String> {
        let parsed: Self = serde_json::from_value(spec.clone())
            .map_err(|e| format!("invalid matrix spec: {e}"))?;
        parsed.validate()?;
        Ok(parsed)
    }

    fn validate(&self) -> Result<(), String> {
        let op = self.op();
        if !OPS.contains(&op.as_str()) {
            return Err(format!("op must be one of {OPS:?}, got {op:?}"));
        }
        for axis in self.rows.iter().chain(self.cols.iter()) {
            if axis.id.is_empty() || axis.id.contains(':') {
                return Err(format!(
                    "axis id must be non-empty and colon-free: {:?}",
                    axis.id
                ));
            }
            zen_expression::compile_expression(&axis.expr)
                .map_err(|e| format!("axis {:?} expression invalid: {e}", axis.id))?;
        }
        for (dup, kind) in [
            (has_dup(self.rows.iter().map(|r| r.id.as_str())), "row"),
            (has_dup(self.cols.iter().map(|c| c.id.as_str())), "col"),
        ] {
            if let Some(id) = dup {
                return Err(format!("duplicate {kind} id {id:?}"));
            }
        }
        for (cell_key, value) in &self.cells {
            let (row_id, col_id) = split_cell_key(cell_key)?;
            if !self.rows.iter().any(|r| r.id == row_id) {
                return Err(format!(
                    "cell {cell_key:?} references unknown row {row_id:?}"
                ));
            }
            if !self.cols.iter().any(|c| c.id == col_id) {
                return Err(format!(
                    "cell {cell_key:?} references unknown col {col_id:?}"
                ));
            }
            match value {
                Value::Number(_) | Value::Bool(_) | Value::String(_) => {}
                _ => {
                    return Err(format!(
                        "cell {cell_key:?} value must be a scalar (number/bool/string)"
                    ))
                }
            }
        }
        Ok(())
    }

    pub fn op(&self) -> String {
        self.op.clone().unwrap_or_else(|| "sum".into())
    }

    /// Populated cells in deterministic (sorted) order.
    pub fn cells(&self) -> Vec<MatrixCell<'_>> {
        let mut keys: Vec<&String> = self.cells.keys().collect();
        keys.sort();
        keys.into_iter()
            .filter_map(|key| {
                let (row_id, col_id) = key.split_once(':')?;
                Some(MatrixCell {
                    row_id,
                    col_id,
                    row_expr: &self.rows.iter().find(|r| r.id == row_id)?.expr,
                    col_expr: &self.cols.iter().find(|c| c.id == col_id)?.expr,
                    value: self.cells.get(key)?,
                })
            })
            .collect()
    }

    /// Compiles the spec to a JDM decision graph.
    ///
    /// Each populated cell becomes one decision-table rule whose input cell is
    /// the standard expression `(<rowExpr>) and (<colExpr>)` — the input column
    /// carries no `field`, so cells evaluate against the whole context rather
    /// than as unary tests on one input. `hitPolicy: "first"` combined with the
    /// `adjustments[]` collect column gathers every matching cell (a plain
    /// `collect` policy emits an array that would replace the downstream
    /// context object). A final expression node applies `op` to `adjustments`.
    pub fn to_jdm(&self) -> Result<Value, String> {
        let op = self.op();
        let mut rules = Vec::new();
        for cell in self.cells() {
            rules.push(json!({
                "_id": format!("{}:{}", cell.row_id, cell.col_id),
                "i1": format!("({}) and ({})", cell.row_expr, cell.col_expr),
                // serde_json::to_string on a scalar Value cannot fail.
                "o1": serde_json::to_string(cell.value).unwrap_or_default(),
            }));
        }

        let aggregate = match op.as_str() {
            "list" => "adjustments ?? []".to_string(),
            // sum/min/max over the collected matches; empty -> null for
            // min/max, 0 for sum.
            other => format!("{other}(adjustments ?? [])"),
        };

        let jdm = json!({
            "nodes": [
                {"id": "in", "type": "inputNode", "name": "Request"},
                {"id": "mtx", "type": "decisionTableNode", "name": "Matrix",
                 "content": {
                    "hitPolicy": "first",
                    "inputs": [{"id": "i1", "name": "cell", "field": ""}],
                    "outputs": [{"id": "o1", "name": "adjustment", "field": "adjustments[]"}],
                    "rules": rules,
                 }},
                {"id": "agg", "type": "expressionNode", "name": "Aggregate",
                 "content": {"expressions": [
                    {"id": "e1", "key": "adjustments", "value": "adjustments ?? []"},
                    {"id": "e2", "key": "value", "value": aggregate},
                    {"id": "e3", "key": "op", "value": format!("'{op}'")},
                 ]}},
                {"id": "out", "type": "outputNode", "name": "Response"},
            ],
            "edges": [
                {"id": "e1", "sourceId": "in", "targetId": "mtx"},
                {"id": "e2", "sourceId": "mtx", "targetId": "agg"},
                {"id": "e3", "sourceId": "agg", "targetId": "out"},
            ],
        });
        // Prove the compiled graph parses as engine input before it ever
        // reaches a draft row.
        serde_json::from_value::<zen_engine::model::DecisionContent>(jdm.clone())
            .map_err(|e| format!("compiled JDM failed validation: {e}"))?;
        Ok(jdm)
    }
}

/// Per-cell debug detail from a simulation: which half of the rule matched.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellMatch {
    pub row: String,
    pub col: String,
    pub row_matched: bool,
    pub col_matched: bool,
    pub matched: bool,
    pub value: Value,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulateResult {
    /// The compiled decision's result: `{adjustments, value, op}`.
    pub result: Value,
    pub cells: Vec<CellMatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<String>,
}

/// Compiles `spec` and evaluates it against `context` directly (editor preview
/// — no publication needed). zen `Variable` is `Rc`-based/`!Send`, so the eval
/// runs on a blocking thread with its own current-thread runtime, same as the
/// eligibility path.
pub async fn simulate(spec_json: &Value, context: &Value) -> Result<SimulateResult, String> {
    let spec = MatrixSpec::parse(spec_json)?;
    let jdm = spec.to_jdm()?;
    let context = context.clone();
    let spec_cells: Vec<(String, String, String, String, Value)> = spec
        .cells()
        .into_iter()
        .map(|c| {
            (
                c.row_id.to_string(),
                c.col_id.to_string(),
                c.row_expr.to_string(),
                c.col_expr.to_string(),
                c.value.clone(),
            )
        })
        .collect();

    tokio::task::spawn_blocking(move || {
        use zen_engine::loader::MemoryLoader;
        use zen_engine::model::DecisionContent;
        use zen_engine::{DecisionEngine, Variable};

        let content: DecisionContent = serde_json::from_value(jdm)
            .map_err(|e| format!("compiled JDM failed validation: {e}"))?;
        let loader = MemoryLoader::default();
        loader.add("matrix.simulate", content);
        let engine = DecisionEngine::default().with_loader(std::sync::Arc::new(loader));

        let ctx_var: Variable = context.clone().into();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let eval =
            rt.block_on(engine.evaluate_serialized("matrix.simulate", ctx_var, Default::default()));
        let eval = eval.map_err(|e| format!("matrix evaluation failed: {e}"))?;

        // Per-cell detail: evaluate each axis rule separately so the editor can
        // show exactly which side of a cell failed.
        let cells = spec_cells
            .iter()
            .map(|(row_id, col_id, row_expr, col_expr, value)| {
                let eval_axis = |expr: &str| {
                    zen_expression::evaluate_expression(expr, context.clone().into())
                        .ok()
                        .and_then(|v| serde_json::to_value(v).ok())
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                };
                let (row_matched, col_matched) = (eval_axis(row_expr), eval_axis(col_expr));
                CellMatch {
                    row: row_id.clone(),
                    col: col_id.clone(),
                    row_matched,
                    col_matched,
                    matched: row_matched && col_matched,
                    value: value.clone(),
                }
            })
            .collect();

        Ok(SimulateResult {
            result: eval.get("result").cloned().unwrap_or(Value::Null),
            cells,
            performance: eval
                .get("performance")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    })
    .await
    .map_err(|e| format!("simulate task failed: {e}"))?
}

fn split_cell_key(key: &str) -> Result<(&str, &str), String> {
    key.split_once(':')
        .filter(|(r, c)| !r.is_empty() && !c.is_empty())
        .ok_or_else(|| format!("cell key must be \"<rowId>:<colId>\", got {key:?}"))
}

fn has_dup<'a>(mut ids: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    ids.find(|id| !seen.insert(*id)).map(str::to_string)
}
