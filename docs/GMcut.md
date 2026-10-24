# GMcut integration notes

## Provenance

- Base: FilmCraft `v0.10.0`, `cdf43fd366e378da266cb6d3c94549a45244be1f`.
- GMcut integration: Glory-yk, 2026-10-24.
- Fonts: storytold/craft-fonts `abb83316d96aa59c1cf64784289e378fe9fa5695`.
- Model: snakers4/silero-vad `1e261b036686cd0017d500ee96acd1c4ba572a9d` (MIT), downloaded separately and SHA-256 checked.

The native app decodes to bounded 16 kHz mono PCM through FilmCraft's own media pipeline.
An optional external Python worker runs the same energy/Silero candidate detector as the standalone tool.
No FFmpeg code or binary is linked or bundled. FilmCraft's ordinary transcription backend remains separate.

## Changed areas

- `crates/engine/src/gmcut.rs` and `gmcut/tests.rs`: review state, protected frame-aligned cuts, preview, atomic application and undo.
- `crates/speech/src/lib.rs`: optional candidate interface, defaulting to unsupported for other backends.
- `apps/filmcraft/src/gmcut_analyzer.rs`, `gmcut_pcm.py`, `gmcut_analysis/`: validated PCM worker and shared detector.
- `crates/ui-egui/src/panels/gmcut.rs`, workspace/menu/keyboard/state integration: native import, review, presets, preview and undo routing.
- `packaging/gmcut/`: portable launch and local bundle creation, without modifying installed apps.
- Restricted upstream wordmark assets and their callers removed for independent distribution; original copyrights and open asset attributions retained.
- Upstream publishing workflows replaced with explicit GMcut checks. No inherited signing credentials are required.
- Added complete new GMcut labels to the upstream complete Spanish, Japanese and Chinese catalogs, plus the Portuguese menu labels.
- Updated the workspace-menu snapshot to include GM컷 while retaining every existing workspace and layout command.
- Corrected two stale font-test assertions: installed Chinese/Korean/RTL fallbacks may follow the Japanese craft-font run. Japanese-before-CJK priority and actual glyph coverage remain asserted; runtime font behavior is unchanged.
- Equivalent boolean/match expressions in existing file-dialog, timeline and audio code simplified to pass Rust 1.95 lint gates; no behavior change intended.

## Verification boundary

Before public packaging, 24 targeted Rust regressions passed: GMcut engine (10), UI (2), existing transcript behavior (8), captions (1), adapter validation (3).
The installed local 0.10.0 edition was also checked with an audio/video sample: candidate detection, cut application, exact undo and unchanged source hash. This was an internal check; no private sample or project is distributed here.

Public portable checks: 7 package safety tests passed; both native launch and packaging dry-runs passed without writing an app. The shared Python worker also analyzed 16,000 synthetic zero samples with the existing pinned model and returned one candidate on the correct sample clock. No private media was used and no video was exported.

The full upstream check runs with `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=4`, `CARGO_PROFILE_RELEASE_LTO=false`, and `CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16` to reduce test linking time. The deployment build profile in Cargo.toml is unchanged. Final outcome: `cargo xtask ci` passed. Rust tests and doc tests: **3,533 passed, 0 failed, 45 ignored** (the upstream optional/fixture/performance cases remain ignored by default). Format, workspace Clippy with warnings denied, dependency layers, attribution of all 61 repository assets, and WASM checks for 49 crates passed. A running process or a health endpoint alone is not evidence of successful editing.

## Known limits

Single untrimmed source at normal speed; no overlapping or multi-clip GMcut session. Explicit apply is required; exporting remains the user's action. New-file defaults are tight (0.05 s / normal / keep 0), which need audible review on quiet speech. Native preset changes persist only during the current process. Source build instructions target Apple Silicon macOS; other platforms and signed/notarized public binaries have not been verified.
