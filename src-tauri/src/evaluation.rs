//! Network-free, versioned safety-scene evaluation.
//! This fixture evaluates routing/tool selection and deterministic validators;
//! it never calls a model or AutoCAD.
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

const DATASET: &str = include_str!("../../data/evals/safety_scene_eval_v1.json");

#[derive(Debug, Deserialize)]
struct Dataset {
    dataset_id: String,
    version: String,
    cases: Vec<EvalCase>,
}
#[derive(Debug, Deserialize)]
struct EvalCase {
    id: String,
    split: String,
    input: String,
    expected_scene: Option<String>,
    expected_tools: Vec<String>,
    #[allow(dead_code)]
    required_params: Vec<String>,
    #[allow(dead_code)]
    expected_rules: Vec<String>,
    #[allow(dead_code)]
    citations: Vec<String>,
    #[allow(dead_code)]
    human_label: String,
    validator_kind: Option<String>,
    validator_args: Option<Value>,
    expected_validator_ok: Option<bool>,
}

#[derive(Debug, serde::Serialize)]
pub struct EvaluationReport {
    pub dataset_id: String,
    pub version: String,
    pub cases: usize,
    pub scene_recall: f64,
    pub scene_precision: f64,
    pub route_accuracy: f64,
    pub tool_call_validity: f64,
    pub validator_correctness: Option<f64>,
    pub false_negative_rate: f64,
    pub false_positive_rate: f64,
    pub confusion_matrix: BTreeMap<String, BTreeMap<String, usize>>,
}
fn metric(n: usize, d: usize) -> f64 {
    if d == 0 {
        0.0
    } else {
        n as f64 / d as f64
    }
}
fn num(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or_default()
}
fn boolean(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn validator_result(case: &EvalCase) -> Option<bool> {
    let kind = case.validator_kind.as_deref()?;
    let args = case.validator_args.as_ref()?;
    match kind {
        "opening_cover" => Some(
            crate::safety::validate_opening_cover(
                num(args, "short"),
                num(args, "long"),
                args.get("method").and_then(Value::as_str).unwrap_or(""),
                boolean(args, "fixed"),
                num(args, "height"),
                boolean(args, "net"),
            )
            .ok,
        ),
        "edge_guardrail" => Some(
            crate::safety::validate_edge_guardrail(
                num(args, "length"),
                num(args, "top"),
                num(args, "spacing"),
                num(args, "toe"),
                boolean(args, "mesh"),
            )
            .ok,
        ),
        "safety_net" => Some(
            crate::safety::validate_elevator_shaft_safety_net(
                num(args, "width"),
                num(args, "depth"),
                num(args, "floor"),
                num(args, "gap"),
                boolean(args, "isolation"),
            )
            .ok,
        ),
        _ => None,
    }
}

pub fn run_offline_evaluation() -> Result<EvaluationReport, String> {
    let dataset: Dataset =
        serde_json::from_str(DATASET).map_err(|e| format!("eval dataset: {e}"))?;
    let mut matrix = BTreeMap::new();
    let mut scene_tp = 0;
    let mut scene_expected = 0;
    let mut predicted_scene = 0;
    let mut route_ok = 0;
    let mut tool_ok = 0;
    let mut validator_ok = 0;
    let mut validator_total = 0;
    let mut false_negative = 0;
    let mut false_positive = 0;
    for case in &dataset.cases {
        let predicted = crate::tools::safety_context_scene(&case.input).map(str::to_string);
        let actual_key = case.expected_scene.clone().unwrap_or_else(|| "none".into());
        let predicted_key = predicted.clone().unwrap_or_else(|| "none".into());
        *matrix
            .entry(actual_key)
            .or_insert_with(BTreeMap::new)
            .entry(predicted_key)
            .or_insert(0) += 1;
        if case.expected_scene.is_some() {
            scene_expected += 1;
        }
        if predicted.is_some() {
            predicted_scene += 1;
        }
        if predicted == case.expected_scene {
            route_ok += 1;
        }
        if predicted == case.expected_scene && case.expected_scene.is_some() {
            scene_tp += 1;
        }
        if case.expected_scene.is_some() && predicted.is_none() {
            false_negative += 1;
        }
        if case.expected_scene.is_none() && predicted.is_some() {
            false_positive += 1;
        }
        let tooling = crate::tools::select_tooling_context(
            &case.input,
            &[],
            crate::settings::WorkMode::SafetyDemoMode,
        );
        if case
            .expected_tools
            .iter()
            .all(|tool| tooling.tool_names.contains(tool))
            && (case.expected_scene.is_some() || !tooling.safety_scoped)
        {
            tool_ok += 1;
        }
        if let (Some(expected), Some(actual)) = (case.expected_validator_ok, validator_result(case))
        {
            validator_total += 1;
            if expected == actual {
                validator_ok += 1;
            }
        }
    }
    let total = dataset.cases.len();
    Ok(EvaluationReport {
        dataset_id: dataset.dataset_id,
        version: dataset.version,
        cases: total,
        scene_recall: metric(scene_tp, scene_expected),
        scene_precision: metric(scene_tp, predicted_scene),
        route_accuracy: metric(route_ok, total),
        tool_call_validity: metric(tool_ok, total),
        validator_correctness: (validator_total > 0).then(|| metric(validator_ok, validator_total)),
        false_negative_rate: metric(false_negative, scene_expected),
        false_positive_rate: metric(false_positive, total - scene_expected),
        confusion_matrix: matrix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_eval_report_is_versioned_and_balanced() {
        let dataset: Dataset = serde_json::from_str(DATASET).unwrap();
        assert_eq!(dataset.cases.len(), 60);
        for (split, expected) in [
            ("normal", 24),
            ("boundary", 18),
            ("adversarial", 12),
            ("noise", 6),
        ] {
            assert_eq!(
                dataset.cases.iter().filter(|c| c.split == split).count(),
                expected,
                "split {split}"
            );
        }
        let report = run_offline_evaluation().unwrap();
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert!(report.scene_recall >= 0.95);
        assert!(report.tool_call_validity >= 0.95);
        assert!(report.validator_correctness.unwrap_or(0.0) >= 0.95);
        assert_eq!(
            report
                .confusion_matrix
                .values()
                .flat_map(|row| row.values())
                .sum::<usize>(),
            60
        );
    }
}
