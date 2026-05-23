# M1 Error Throw Map

Source of truth: `docs/impplan/02_m1_perception_mvp.md` lists 20 M1 error codes. This map excludes HUD and audio catalog codes because those are M3 scope in `docs/impplan/04_m3_reflex_mcp_surface.md`.

| Code | Implementing crate | Throw or mapping site |
|---|---|---|
| `OBSERVE_NO_PERCEPTION_AVAILABLE` | `synapse-perception`, `synapse-mcp` | `crates/synapse-perception/src/observe.rs:357`; `crates/synapse-mcp/src/m1.rs:267`; `crates/synapse-mcp/src/m1/sources.rs:118` |
| `OBSERVE_INTERNAL` | `synapse-perception`, `synapse-mcp` | `crates/synapse-perception/src/observe.rs:365`; `crates/synapse-mcp/src/m1.rs:261`; `crates/synapse-mcp/src/m1/sources.rs:131` |
| `CAPTURE_GRAPHICS_API_UNSUPPORTED` | `synapse-capture` | `crates/synapse-capture/src/lib.rs:207`; `crates/synapse-capture/src/lib.rs:522`; `crates/synapse-capture/src/lib.rs:1359` |
| `CAPTURE_TARGET_LOST` | `synapse-capture` | `crates/synapse-capture/src/lib.rs:209`; `crates/synapse-capture/src/lib.rs:912` |
| `CAPTURE_NO_DIRTY_REGIONS` | `synapse-capture` | `crates/synapse-capture/src/lib.rs:213`; `crates/synapse-capture/src/lib.rs:226` |
| `A11Y_NOT_AVAILABLE` | `synapse-a11y` | `crates/synapse-a11y/src/lib.rs:44`; `crates/synapse-a11y/src/lib.rs:1352` |
| `A11Y_ELEMENT_STALE` | `synapse-a11y` | `crates/synapse-a11y/src/lib.rs:46`; `crates/synapse-a11y/src/lib.rs:716` |
| `A11Y_NO_FOREGROUND` | `synapse-a11y` | `crates/synapse-a11y/src/lib.rs:45`; `crates/synapse-a11y/src/lib.rs:595`; `crates/synapse-a11y/src/lib.rs:622` |
| `A11Y_CDP_UNREACHABLE` | `synapse-a11y` | `crates/synapse-a11y/src/lib.rs:47`; `crates/synapse-a11y/src/lib.rs:440`; `crates/synapse-a11y/src/lib.rs:1569` |
| `DETECTION_MODEL_NOT_LOADED` | `synapse-models` | `crates/synapse-models/src/lib.rs:117`; `crates/synapse-models/src/lib.rs:152`; `crates/synapse-models/src/lib.rs:133` |
| `DETECTION_MODEL_INFER_FAILED` | `synapse-models` | `crates/synapse-models/src/lib.rs:121`; `crates/synapse-models/src/lib.rs:166`; `crates/synapse-models/src/lib.rs:135` |
| `DETECTION_NO_FRAME` | `synapse-models` | `crates/synapse-models/src/lib.rs:83`; `crates/synapse-models/src/lib.rs:119`; `crates/synapse-models/src/lib.rs:134` |
| `OCR_NO_TEXT` | `synapse-perception`, `synapse-mcp` | `crates/synapse-perception/src/ocr.rs:64`; `crates/synapse-perception/src/ocr.rs:83`; `crates/synapse-mcp/src/m1/ocr.rs:11` |
| `OCR_BACKEND_UNAVAILABLE` | `synapse-perception` | `crates/synapse-perception/src/ocr.rs:117`; `crates/synapse-perception/src/ocr.rs:145`; `crates/synapse-perception/src/error.rs:25` |
| `MODEL_DOWNLOAD_FAILED` | `synapse-models` | `crates/synapse-models/src/lib.rs:141`; `crates/synapse-models/src/lib.rs:148`; `crates/synapse-models/src/lib.rs:129` |
| `MODEL_HASH_MISMATCH` | `synapse-models` | `crates/synapse-models/src/lib.rs:292`; `crates/synapse-models/src/lib.rs:130` |
| `MODEL_LOAD_FAILED` | `synapse-models` | `crates/synapse-models/src/lib.rs:286`; `crates/synapse-models/src/lib.rs:113`; `crates/synapse-models/src/lib.rs:131` |
| `MODEL_BACKEND_UNAVAILABLE` | `synapse-models` | `crates/synapse-models/src/lib.rs:115`; `crates/synapse-models/src/lib.rs:132`; `crates/synapse-models/src/lib.rs:337` |
| `PERCEPTION_MODE_INVALID` | `synapse-perception`, `synapse-mcp` | `crates/synapse-perception/src/observe.rs:283`; `crates/synapse-mcp/src/m1.rs:359`; `crates/synapse-perception/src/error.rs:30` |
| `CAPTURE_TARGET_INVALID` | `synapse-capture`, `synapse-mcp` | `crates/synapse-capture/src/lib.rs:620`; `crates/synapse-capture/src/lib.rs:995`; `crates/synapse-mcp/src/m1.rs:386` |

Local audit command used for manual verification:

```bash
rg -n "OBSERVE_NO_PERCEPTION_AVAILABLE|OBSERVE_INTERNAL|CAPTURE_GRAPHICS_API_UNSUPPORTED|CAPTURE_TARGET_LOST|CAPTURE_NO_DIRTY_REGIONS|CAPTURE_TARGET_INVALID|A11Y_NOT_AVAILABLE|A11Y_ELEMENT_STALE|A11Y_NO_FOREGROUND|A11Y_CDP_UNREACHABLE|DETECTION_MODEL_NOT_LOADED|DETECTION_MODEL_INFER_FAILED|DETECTION_NO_FRAME|OCR_NO_TEXT|OCR_BACKEND_UNAVAILABLE|MODEL_DOWNLOAD_FAILED|MODEL_HASH_MISMATCH|MODEL_LOAD_FAILED|MODEL_BACKEND_UNAVAILABLE|PERCEPTION_MODE_INVALID" crates --glob '!target/**'
```
