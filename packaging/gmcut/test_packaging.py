import importlib.util
import os
import plistlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('gmcut_build', Path(__file__).with_name('build-macos.py'))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / 'checkout'
        for path in ('target/release/filmcraft', '.venv/bin/python', 'cache/silero_vad.jit',
                     'apps/filmcraft/gmcut_pcm.py', 'apps/filmcraft/gmcut_analysis/analysis.py',
                     'apps/filmcraft/gmcut_analysis/vendor/silence_analysis.py',
                     'apps/filmcraft/gmcut_analysis/LICENSE', 'apps/filmcraft/gmcut_analysis/ATTRIBUTION.md',
                     'apps/filmcraft/gmcut_analysis/THIRD_PARTY.md',
                     'assets/fonts/OFL-Inter.txt', 'assets/fonts/OFL-JetBrainsMono.txt', 'assets/fonts/OFL-NotoSerif.txt',
                     'NOTICE', 'ATTRIBUTION.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'packaging/gmcut/LICENSE-SILERO'):
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(b'fixture')
        self.binary = self.root / 'target/release/filmcraft'
        self.binary.write_bytes(b'\xcf\xfa\xed\xfe' + b'fixture')
        self.binary.chmod(0o755)
        self.python = self.root / '.venv/bin/python'
        self.python.chmod(0o755)
        self.model = self.root / 'cache/silero_vad.jit'
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion="0.10.0"\n')
        template = self.root / 'packaging/macos/Info.plist.in'
        template.parent.mkdir(parents=True)
        template.write_bytes(plistlib.dumps({'CFBundleExecutable': 'FilmCraft', 'CFBundleShortVersionString': '@SHORT_VERSION@'}))
        self.installed = Path(self.directory.name) / 'Applications/FilmCraft.app/original'
        self.installed.parent.mkdir(parents=True)
        self.installed.write_bytes(b'installed app must survive')
        self.environment = patch.dict(os.environ, {'CRAFT_FONTS_DIR': ''})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def test_dry_run_writes_nothing_and_never_signs(self):
        before = set(self.root.rglob('*'))
        with patch.object(builder.subprocess, 'run') as run:
            result = builder.assemble(self.root, self.binary, self.python, self.model, dry_run=True)
            run.assert_not_called()
        self.assertFalse(result['writesFiles'])
        self.assertEqual(before, set(self.root.rglob('*')))
        self.assertEqual(self.installed.read_bytes(), b'installed app must survive')

    def test_only_new_bundle_receives_shared_analyzer_and_runtime_pointer(self):
        with patch.object(builder.subprocess, 'run') as run:
            result = builder.assemble(self.root, self.binary, self.python, self.model)
        destination = Path(result['destination'])
        self.assertEqual(destination, self.root.resolve() / 'dist/FilmCraft GMcut.app')
        with (destination / 'Contents/Info.plist').open('rb') as stream:
            self.assertEqual(plistlib.load(stream)['CFBundleIdentifier'], 'io.github.glory-yk.filmcraft-gmcut')
        self.assertEqual((destination / 'Contents/Resources/gmcut-python.txt').read_text().strip(), str(self.python))
        self.assertEqual((destination / 'Contents/Resources/gmcut_analysis/silero_vad.jit').read_bytes(), self.model.read_bytes())
        self.assertTrue((destination / 'Contents/Resources/LICENSE-SILERO').is_file())
        self.assertIn('license: MIT', (destination / 'Contents/Resources/gmcut_analysis/silero_vad.jit.attribution').read_text())
        self.assertEqual(run.call_args.args[0][-1], str(destination))
        self.assertEqual(self.installed.read_bytes(), b'installed app must survive')
        with self.assertRaisesRegex(ValueError, 'never overwritten'):
            builder.assemble(self.root, self.binary, self.python, self.model)

    def test_symlink_dist_cannot_target_applications(self):
        (self.root / 'dist').symlink_to(self.installed.parent.parent, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'never overwritten'):
            builder.plan(self.root, self.binary, self.python, self.model)
        self.assertEqual(self.installed.read_bytes(), b'installed app must survive')

    def test_bad_binary_or_missing_model_is_rejected_before_writes(self):
        self.binary.write_bytes(b'not Mach-O')
        with self.assertRaisesRegex(ValueError, 'Mach-O'):
            builder.plan(self.root, self.binary, self.python, self.model)
        self.binary.write_bytes(b'\xcf\xfa\xed\xfe' + b'fixture')
        with self.assertRaisesRegex(ValueError, 'cached Silero'):
            builder.plan(self.root, self.binary, self.python, self.model.with_name('missing'))
        self.assertFalse((self.root / 'dist').exists())

    def test_python_virtualenv_symlink_is_not_resolved_to_system_python(self):
        actual = self.python.with_name('base-python')
        self.python.rename(actual)
        self.python.symlink_to(actual.name)
        _, _, python, _ = builder.plan(self.root, self.binary, self.python, self.model)
        self.assertEqual(python, self.python)
        self.assertNotEqual(python, self.python.resolve())

    def test_custom_font_licenses_are_included_when_configured(self):
        fonts = Path(self.directory.name) / 'craft-fonts'
        license = fonts / 'fonts/Test Family/OFL.txt'
        license.parent.mkdir(parents=True)
        license.write_text('open font license fixture')
        with patch.dict(os.environ, {'CRAFT_FONTS_DIR': str(fonts)}):
            _, _, _, files = builder.plan(self.root, self.binary, self.python, self.model)
        self.assertEqual(files['font-licenses/craft-fonts/Test Family/OFL.txt'], license)

    def test_signing_failure_removes_only_the_new_bundle(self):
        with patch.object(builder.subprocess, 'run', side_effect=builder.subprocess.CalledProcessError(1, 'codesign')):
            with self.assertRaises(builder.subprocess.CalledProcessError):
                builder.assemble(self.root, self.binary, self.python, self.model)
        self.assertFalse((self.root / 'dist/FilmCraft GMcut.app').exists())
        self.assertEqual(self.installed.read_bytes(), b'installed app must survive')


if __name__ == '__main__':
    unittest.main()
