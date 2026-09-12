# Real AutoCAD isolated safety smoke - 2026-09-12

## Host identity

- Process: `acad.exe /product ACAD`
- Product: `AutoCAD`
- AutoCAD version: `26.0s (LMS Tech)` / AutoCAD 2027
- Bridge version: `0.3.11.0`
- Document: `D:\CADEgg\test\Drawing1.dwg`

## Command

```powershell
cargo.exe test --manifest-path src-tauri\Cargo.toml isolated_safety_scenes_smoke_test_round_trip -- --ignored --nocapture --test-threads=1
```

The harness waited for six consecutive stable model-space handle snapshots before drawing, after drawing, and after handle-based cleanup.

## Results

| Case | Origin | Expected bounding box | Actual handles (inclusive) | Count | Validator | Cleanup |
| --- | --- | --- | --- | ---: | --- | --- |
| `opening_cover` | `(100000,100000)` | `X[99000,103500] Y[98500,102000]` | `127A..127C` | 3 | `ok=true` | verified |
| `elevator_shaft_protection` | `(120000,100000)` | `X[117000,126000] Y[96000,106000]` | `127D..12A3` | 39 | `ok=true` | verified |
| `elevator_shaft_safety_net` | `(140000,100000)` | `X[137000,146000] Y[96000,106000]` | `12A4..12CB` | 40 | `ok=true` | verified |
| `edge_guardrail` | `(160000,100000)` | `X[155000,167000] Y[96000,105000]` | `12CC..1316` | 75 | `ok=true` | verified |

Each hexadecimal range is contiguous and identifies every object created by that case. The harness inspected every handle before cleanup and fails if any inspection is unavailable.

Representative object snapshots:

```text
127A LWPOLYLINE closed: (99600,99800) -> (100400,99800) -> (100400,100200) -> (99600,100200)
127C TEXT: opening-cover fixed-cover note at (100000,100360)
127D LWPOLYLINE closed: (119000,99100) -> (121000,99100) -> (121000,100900) -> (119000,100900)
129B TEXT: elevator warning sign at (119246,101860)
12A4 LWPOLYLINE closed: (138900,99100) -> (141100,99100) -> (141100,100900) -> (138900,100900)
12CB TEXT: upper isolation status at (143651.2,100012.5)
12CC LWPOLYLINE closed: (157000,100000) -> (163000,100000) -> (163000,100180) -> (157000,100180)
12FD LINE: (164300,100800) -> (166470,100800)
1316 TEXT: dense-mesh status at (165486.6,99432.5)
```

## Cleanup audit

- Final model-space object count: `0`
- Final `CMDNAMES`: empty
- Cleanup scope: only handles created by each case
- Existing user geometry erased: none
- Legacy delayed smoke objects discovered during diagnosis: exact handles `FC2..1080`, 191 objects, all verified inside the old smoke region before removal in one AutoCAD undo group
- Before the pure-validation fix, a full test run also created exact handles `11BB..1279` (191 objects) at the old fixed origins; these were verified and removed in a separate AutoCAD undo group. Subsequent full tests left the model space at `0`.

This report records a real AutoCAD run. It is not based on mocks or the ignored-test declaration alone.
