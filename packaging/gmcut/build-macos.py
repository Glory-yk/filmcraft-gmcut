#!/usr/bin/env python3
"""Assemble a NEW local GMcut bundle from a previously built FilmCraft binary."""
import argparse
import json
import os
import plistlib
import shutil
import subprocess
import sys
import tomllib
from pathlib import Path


def absolute(value):
    # Preserve .venv/bin/python symlinks: resolving them loses the virtual environment.
    return Path(os.path.abspath(os.path.expanduser(str(value))))


def plan(root, binary, python, model):
    root = root.resolve()
    binary, python, model = map(absolute, (binary, python, model))
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError('Build FilmCraft first, then supply its executable with --binary')
    with binary.open('rb') as stream:
        if stream.read(4) not in (b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca'):
            raise ValueError('--binary must be a macOS Mach-O executable')
    if not python.is_file() or not os.access(python, os.X_OK):
        raise ValueError('Set FILMCRAFT_GMCUT_PYTHON or --python to an existing executable')
    if not model.is_file() or not 0 < model.stat().st_size <= 64 * 1024 * 1024:
        raise ValueError('Supply an existing cached Silero model, at most 64 MiB')
    files = {
        'gmcut_pcm.py': root / 'apps/filmcraft/gmcut_pcm.py',
        'gmcut_analysis/analysis.py': root / 'apps/filmcraft/gmcut_analysis/analysis.py',
        'gmcut_analysis/vendor/silence_analysis.py': root / 'apps/filmcraft/gmcut_analysis/vendor/silence_analysis.py',
        'gmcut_analysis/LICENSE': root / 'apps/filmcraft/gmcut_analysis/LICENSE',
        'gmcut_analysis/ATTRIBUTION.md': root / 'apps/filmcraft/gmcut_analysis/ATTRIBUTION.md',
        'gmcut_analysis/THIRD_PARTY.md': root / 'apps/filmcraft/gmcut_analysis/THIRD_PARTY.md',
        'gmcut_analysis/silero_vad.jit': model,
        'LICENSE-MIT': root / 'LICENSE-MIT',
        'LICENSE-APACHE': root / 'LICENSE-APACHE',
        'LICENSE-SILERO': root / 'packaging/gmcut/LICENSE-SILERO',
        'NOTICE': root / 'NOTICE',
        'ATTRIBUTION.md': root / 'ATTRIBUTION.md',
    }
    analysis = root / 'apps/filmcraft/gmcut_analysis'
    for license in (analysis / 'LICENSES').glob('*'):
        if license.is_file():
            files[f'gmcut_analysis/LICENSES/{license.name}'] = license
    for license in (root / 'assets/fonts').glob('OFL-*.txt'):
        files[f'font-licenses/bundled/{license.name}'] = license
    if len(list((root / 'assets/fonts').glob('OFL-*.txt'))) < 3:
        raise ValueError('Bundled Latin font licenses are missing')
    if os.environ.get('CRAFT_FONTS_DIR'):
        font_licenses = list((absolute(os.environ['CRAFT_FONTS_DIR']) / 'fonts').glob('*/OFL.txt'))
        if not font_licenses:
            raise ValueError('CRAFT_FONTS_DIR has no fonts/*/OFL.txt license files')
        for license in font_licenses:
            files[f'font-licenses/craft-fonts/{license.parent.name}/OFL.txt'] = license
    for source in files.values():
        if not source.is_file():
            raise ValueError(f'Required source or license file is missing: {source.relative_to(root) if source.is_relative_to(root) else source.name}')
    destination = root / 'dist/FilmCraft GMcut.app'
    if (root / 'dist').is_symlink() or destination.exists() or destination.is_symlink():
        raise ValueError('Destination must be a new app under this repository dist directory; existing apps are never overwritten')
    return destination, binary, python, files


def assemble(root, binary, python, model, dry_run=False):
    destination, binary, python, files = plan(root, binary, python, model)
    if dry_run:
        return {'destination': str(destination), 'resourceFiles': list(files), 'writesFiles': False}
    # The destination and every child are newly created; never copy or mutate an installed app.
    destination.mkdir(parents=True, exist_ok=False)
    try:
        contents = destination / 'Contents'
        resources = contents / 'Resources'
        macos = contents / 'MacOS'
        resources.mkdir(parents=True)
        macos.mkdir()
        shutil.copy2(binary, macos / 'FilmCraft')
        for relative, source in files.items():
            target = resources / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        (resources / 'gmcut-python.txt').write_text(str(python) + '\n', encoding='utf-8')
        (resources / 'gmcut_analysis/silero_vad.jit.attribution').write_text(
            'asset: silero_vad.jit\n'
            'title: Silero VAD TorchScript model\n'
            'author: Silero Team\n'
            'source: https://github.com/snakers4/silero-vad/tree/1e261b036686cd0017d500ee96acd1c4ba572a9d\n'
            'license: MIT\n'
            'license-file: LICENSES/silero-vad-MIT.txt\n'
            'added: 2026-10-24 by Glory-yk\n'
            'notes: Optional local model, copied from the user-selected cache; not included in source.\n',
            encoding='utf-8',
        )
        version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version']
        template = (root / 'packaging/macos/Info.plist.in').read_text(encoding='utf-8')
        template = template.replace('@VERSION@', version).replace('@SHORT_VERSION@', version.split('-')[0]).replace('@BUILD_SHA@', 'local-gmcut')
        info = plistlib.loads(template.encode('utf-8'))
        info.update(CFBundleName='FilmCraft GMcut', CFBundleDisplayName='FilmCraft GMcut', CFBundleIdentifier='io.github.glory-yk.filmcraft-gmcut')
        info.pop('CFBundleIconFile', None)
        with (contents / 'Info.plist').open('wb') as stream:
            plistlib.dump(info, stream)
        (contents / 'PkgInfo').write_bytes(b'APPL????')
        # Local ad-hoc signature; this script never downloads certificates or notarizes.
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(destination)], check=True)
    except Exception:
        shutil.rmtree(destination)
        raise
    return {'destination': str(destination), 'resourceFiles': list(files), 'writesFiles': True}


def main():
    root = Path(__file__).resolve().parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=root / 'target/release/filmcraft')
    parser.add_argument('--python', default=os.environ.get('FILMCRAFT_GMCUT_PYTHON', str(root / '.venv/bin/python')))
    parser.add_argument('--model', default=os.environ.get('FILMCRAFT_GMCUT_MODEL'))
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    if not args.model:
        parser.error('Supply --model or FILMCRAFT_GMCUT_MODEL; models are never downloaded')
    try:
        print(json.dumps(assemble(root, args.binary, args.python, args.model, args.dry_run), indent=2))
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f'GMcut packaging failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
