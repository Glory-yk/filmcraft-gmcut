#!/usr/bin/env python3
"""Offline GM컷 PCM bridge. Uses the user's existing shared analyzer; no decoder."""
import argparse
import json
import os
import sys
from pathlib import Path

MAX_SAMPLES = 16000 * 3600
MAX_OUTPUT = 2 * 1024 * 1024


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audio', required=True)
    parser.add_argument('--minimum', type=float, required=True)
    parser.add_argument('--sensitivity', choices=('quiet', 'normal', 'sensitive'), required=True)
    parser.add_argument('--fps', type=float, required=True)
    parser.add_argument('--progress', required=True)
    args = parser.parse_args()
    directory = Path(os.environ.get('FILMCRAFT_GMCUT_ANALYSIS_DIR', ''))
    if not directory.is_absolute() or not (directory / 'analysis.py').is_file():
        raise ValueError('FILMCRAFT_GMCUT_ANALYSIS_DIR must identify the existing shared analyzer')
    path = Path(args.audio)
    size = path.stat().st_size
    if size <= 0 or size > MAX_SAMPLES * 4 or size % 4:
        raise ValueError('PCM must contain 1 to 57600000 little-endian float32 samples')
    sys.path.insert(0, str(directory))
    import analysis
    model = os.environ.get('FILMCRAFT_GMCUT_MODEL')
    if model:
        model = Path(model)
        if not model.is_absolute() or not model.is_file():
            raise ValueError('Configured cached Silero model is missing')
        analysis.MODEL = model
    import numpy as np
    audio = np.fromfile(path, dtype='<f4')
    source = {'id': 'native-pcm', 'duration': len(audio) / 16000, 'fps': args.fps}
    def progress(value):
        Path(args.progress).write_text(json.dumps({'progress': value}), encoding='utf-8')
    result = analysis.analyze_pcm(source, audio, args.minimum, args.sensitivity, progress)
    candidates = result['candidates']
    if len(candidates) > 5000:
        raise ValueError('GM컷 response exceeds 5000 candidates')
    output = json.dumps({'candidates': candidates, 'diagnostics': result['diagnostics'],
                         'samples': len(audio), 'sampleRate': 16000}, allow_nan=False).encode('utf-8')
    if len(output) > MAX_OUTPUT:
        raise ValueError('GM컷 response exceeds 2 MiB')
    sys.stdout.buffer.write(output)


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'error': str(error)[:1000]}, ensure_ascii=False, allow_nan=False))
        raise SystemExit(2)
