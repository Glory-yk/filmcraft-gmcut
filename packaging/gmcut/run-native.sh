#!/bin/sh
# Native PCM analysis; requires an existing Python environment and cached Silero model.
set -eu
GMCUT_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
FILMCRAFT_GMCUT_PYTHON=${FILMCRAFT_GMCUT_PYTHON:-"$GMCUT_ROOT/.venv/bin/python"}
if [ ! -x "$FILMCRAFT_GMCUT_PYTHON" ]; then
    echo 'Set FILMCRAFT_GMCUT_PYTHON to an existing Python with numpy and torch, or create .venv.' >&2
    exit 1
fi
if [ -z "${FILMCRAFT_GMCUT_MODEL:-}" ] || [ ! -f "$FILMCRAFT_GMCUT_MODEL" ]; then
    echo 'Set FILMCRAFT_GMCUT_MODEL to an existing Silero silero_vad.jit file.' >&2
    exit 1
fi
FILMCRAFT_GMCUT_PYTHON=$("$FILMCRAFT_GMCUT_PYTHON" -c 'import os, sys; print(os.path.abspath(sys.executable))')
FILMCRAFT_GMCUT_MODEL=$("$FILMCRAFT_GMCUT_PYTHON" -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$FILMCRAFT_GMCUT_MODEL")
FILMCRAFT_GMCUT_SCRIPT="$GMCUT_ROOT/apps/filmcraft/gmcut_pcm.py"
FILMCRAFT_GMCUT_ANALYSIS_DIR="$GMCUT_ROOT/apps/filmcraft/gmcut_analysis"
export FILMCRAFT_GMCUT_PYTHON FILMCRAFT_GMCUT_MODEL FILMCRAFT_GMCUT_SCRIPT FILMCRAFT_GMCUT_ANALYSIS_DIR
if [ ! -f "$FILMCRAFT_GMCUT_SCRIPT" ] || [ ! -f "$FILMCRAFT_GMCUT_ANALYSIS_DIR/analysis.py" ]; then
    echo 'GMcut shared source files are missing.' >&2
    exit 1
fi
if [ "${1:-}" = '--dry-run' ]; then
    echo 'Runtime and model paths validated. Would run: cargo run -p filmcraft -- ...'
    exit 0
fi
"$FILMCRAFT_GMCUT_PYTHON" -c 'import numpy, torch'
cd "$GMCUT_ROOT"
exec cargo run -p filmcraft -- "$@"
