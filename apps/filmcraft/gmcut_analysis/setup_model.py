#!/usr/bin/env python3
"""Explicit, pinned Silero model setup; importing this module never downloads anything."""
import argparse
import hashlib
import os
import tempfile
from pathlib import Path
from urllib.request import urlopen

from analysis import MODEL

MODEL_COMMIT = '1e261b036686cd0017d500ee96acd1c4ba572a9d'
MODEL_URL = f'https://raw.githubusercontent.com/snakers4/silero-vad/{MODEL_COMMIT}/src/silero_vad/data/silero_vad.jit'
MODEL_SIZE = 2272526
MODEL_SHA256 = 'e1122837f4154c511485fe0b9c64455f7b929c96fbb8d79fbdb336383ebd3720'


def valid_model(path):
    path = Path(path)
    if not path.is_file() or path.stat().st_size != MODEL_SIZE:
        return False
    digest = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(65536), b''):
            digest.update(block)
    return digest.hexdigest() == MODEL_SHA256


def install_model(destination):
    """Keep any existing model intact until a bounded download passes both checks."""
    destination = Path(destination).expanduser()
    if valid_model(destination):
        return destination
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = None
    try:
        with urlopen(MODEL_URL, timeout=30) as response:
            length = response.headers.get('Content-Length')
            if length is not None and int(length) != MODEL_SIZE:
                raise ValueError('Model download has an unexpected Content-Length')
            digest, total = hashlib.sha256(), 0
            with tempfile.NamedTemporaryFile(dir=destination.parent, prefix=destination.name + '.', suffix='.tmp', delete=False) as target:
                temporary = Path(target.name)
                while True:
                    block = response.read(65536)
                    if not block:
                        break
                    total += len(block)
                    if total > MODEL_SIZE:
                        raise ValueError('Model download exceeds the pinned size')
                    digest.update(block)
                    target.write(block)
                target.flush()
                os.fsync(target.fileno())
            if total != MODEL_SIZE or digest.hexdigest() != MODEL_SHA256:
                raise ValueError('Model size or SHA-256 does not match the pinned release')
        os.replace(temporary, destination)
        temporary = None
        return destination
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=MODEL, help='destination (default: GMCUT_MODEL or user cache)')
    parser.add_argument('--check', action='store_true', help='verify an existing model without network access')
    args = parser.parse_args()
    try:
        if args.check:
            if not valid_model(args.output):
                raise ValueError('No verified model at the selected destination')
            path = args.output
        else:
            path = install_model(args.output)
        print(f'GM컷 모델 준비 완료: {path}')
    except (OSError, ValueError) as error:
        parser.exit(1, f'GM컷 모델 준비 실패: {error}\n')


if __name__ == '__main__':
    main()
