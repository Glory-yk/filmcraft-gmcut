# Local GMcut native packaging

GMcut analyzes the editor's native 16 kHz mono PCM. Neither script requires FFmpeg or a web server. Normal FilmCraft transcription keeps its existing backend.

Use an existing Python 3.11+ environment with `numpy` and `torch`. Set `FILMCRAFT_GMCUT_PYTHON` to that environment's executable, or put the environment in this repository's `.venv`. Set `FILMCRAFT_GMCUT_MODEL` to your existing `silero_vad.jit` cache. No script downloads packages or weights.

Run the source checkout:

```sh
export FILMCRAFT_GMCUT_PYTHON="$VIRTUAL_ENV/bin/python"
export FILMCRAFT_GMCUT_MODEL="$SILERO_CACHE/silero_vad.jit"
packaging/gmcut/run-native.sh --dry-run
packaging/gmcut/run-native.sh
```

Build the regular FilmCraft binary with the repository's documented Cargo workflow first. Then assemble a separate local bundle:

```sh
python3 packaging/gmcut/build-macos.py --binary target/release/filmcraft --dry-run
python3 packaging/gmcut/build-macos.py --binary target/release/filmcraft
python3 packaging/gmcut/build-macos.py --help
```

The only app destination is a new `dist/FilmCraft GMcut.app`. Existing destinations are refused. The scripts never install to or modify `/Applications`. The bundle contains the shared analyzer, worker, cached Silero model and licenses, and an installation-local Python path in `Resources/gmcut-python.txt`. Finder can launch it on the same Mac without shell environment variables. The external Python environment must remain at its recorded path; another Mac must create its own environment and assemble its own bundle. This is a local ad-hoc signed bundle, without notarization or a bundled Python runtime.

Keep the same `CRAFT_FONTS_DIR` environment setting used to build the binary when assembling the bundle, so licenses for its embedded craft-fonts are included alongside the bundled Latin font licenses and source attribution.

GMcut uses the [Silero VAD MIT license](https://github.com/snakers4/silero-vad/blob/master/LICENSE). The model is supplied explicitly from the user's cache and never committed by this packaging workflow.

Smoke tests, using temporary files and no installed app:

```sh
python3 -m unittest discover -s packaging/gmcut -p 'test_*.py'
```
