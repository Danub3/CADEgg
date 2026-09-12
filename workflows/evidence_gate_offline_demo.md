# Evidence Gate Offline Demo

This demo exercises the local evidence gate only. It does not call a model, access the network,
or start AutoCAD.

## Preconditions

- Run from the CADEgg repository root.
- Keep `data/atlas/` and `data/sources/` available. Packaged builds also contain built-in copies.

## Run

```powershell
cargo.exe test --manifest-path src-tauri\Cargo.toml knowledge::tests::verified_evidence_resolves_to_source_excerpt_and_raw_text -- --exact --nocapture
cargo.exe test --manifest-path src-tauri\Cargo.toml knowledge::tests::opening_cover_card_returns_auditable_evidence -- --exact --nocapture
cargo.exe test --manifest-path src-tauri\Cargo.toml knowledge::tests::citation_page_mismatch_is_refused_instead_of_silently_rendered -- --exact --nocapture
cargo.exe test --manifest-path src-tauri\Cargo.toml knowledge::tests::source_version_drift_is_refused -- --exact --nocapture
```

## Expected Results

1. `elevator_shaft_protection` resolves to `status=verified`. The evidence contains the card
   version and source version plus the exact excerpt, section, page, and raw text.
2. `opening_cover` resolves to `status=verified` with both JGJ 80-2016 4.2.1 and 建办质函〔2019〕90号 2.7.1 citations.
   The agent may continue only through the deterministic opening-cover validator and draw tool.
3. A deliberately incorrect page number resolves to `status=refused` with `page_mismatch`.
4. A deliberately changed source version resolves to `status=refused` with
   `source_version_mismatch`.

The read-only Tauri command `get_scene_evidence(scene)` exposes the same object to future UI and
audit-log work. Its payload contract is `data/schema/evidence_bundle.schema.json`.

## Boundaries

- The incorrect-page case is an explicit offline test fixture, not a real source defect.
- Passing this demo proves local citation integrity and deterministic refusal behavior only.
- It does not prove model accuracy, drawing correctness, AutoCAD connectivity, or professional
  approval. Real engineering conclusions still require the current source documents, site
  measurements, and a qualified reviewer.
