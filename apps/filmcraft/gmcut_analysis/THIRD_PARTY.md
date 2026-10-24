# Shared analysis provenance

`analysis.py` and `vendor/silence_analysis.py` are the same GMcut analyzer as [Glory-yk/gmcut](https://github.com/Glory-yk/gmcut), Copyright (c) 2026 Glory-yk, MIT (LICENSE). Copied from standalone source commit `ba9e0b84ae9020ed1955c9dd1fba6db431677b5f`. The native worker calls `analyze_pcm()`; `analyze_source()` and `render_plan()` are standalone-only and are never called by FilmCraft.

Silero VAD is a separately downloaded MIT model by the Silero Team. Pinned source: [snakers4/silero-vad commit 1e261b036686cd0017d500ee96acd1c4ba572a9d](https://github.com/snakers4/silero-vad/tree/1e261b036686cd0017d500ee96acd1c4ba572a9d). SHA-256: `e1122837f4154c511485fe0b9c64455f7b929c96fbb8d79fbdb336383ebd3720`; size: 2,272,526 bytes. License: LICENSES/silero-vad-MIT.txt. Model weights are not committed.

NumPy and PyTorch are installed separately from requirements.txt. FFmpeg is used only by the independent tool; it is neither required nor shipped by the native PCM integration.
