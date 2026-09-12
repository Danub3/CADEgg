# Offline safety-scene evaluation

`data/evals/safety_scene_eval_v1.json` is a versioned, AutoCAD-free fixture
with 60 audited cases: 24 normal, 18 boundary, 12 adversarial, and 6 noise.
Each case records input, expected scene, required parameters, expected tools,
rules, citations, and a human label. Validator fixtures call deterministic Rust
validators directly.

Run the report without a model API key or AutoCAD:

```powershell
cargo.exe test --manifest-path src-tauri\Cargo.toml evaluation::tests::offline_eval_report_is_versioned_and_balanced -- --exact --nocapture
```

The test prints scene recall/precision, route accuracy, tool-call validity,
validator correctness, false-negative/false-positive rates, and a scene
confusion matrix. It must not be presented as online model accuracy.
