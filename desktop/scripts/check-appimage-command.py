#!/usr/bin/env python3
"""Exercise packaged setup, updates, conflicts, and an agent's bundled command.

Usage: check-appimage-command.py IMAGE BUILT_TERYON --build-newer
Alternatively supply --newer-image PATH. Run under xvfb-run on build servers.
All launches use isolated homes, fake agents, and completed diagnostic records.
The optional fixture build restores Cargo.toml and Cargo.lock even on failure.
"""

import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]


def wait_for(check, app, log):
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if app.poll() is not None:
            log.seek(0)
            raise RuntimeError(f'AppImage exited before completing setup: {log.read()}')
        result = check()
        if result:
            return result
        time.sleep(0.05)
    log.seek(0)
    raise RuntimeError(f'AppImage check did not finish: {log.read()}')


def json_file(path):
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError):
        return None


def prepare(home):
    project = Path(tempfile.mkdtemp(prefix='project-', dir=home))
    fake = home / 'fake-agent'
    fake.write_text(f'''#!{sys.executable}
import json, os, pathlib, shutil, subprocess
print(json.dumps({{"type": "system", "subtype": "init", "session_id": "11111111-1111-4111-8111-111111111111"}}), flush=True)
command = pathlib.Path(shutil.which("teryon")).resolve()
version = subprocess.check_output([str(command), "--version"], text=True).strip()
subprocess.run([str(command), "thread", "title", "Bundled command verified"], check=True)
proof = {{"bundled": command.is_relative_to(pathlib.Path(os.environ["APPDIR"]).resolve()), "version": version}}
pathlib.Path("agent-proof.json").write_text(json.dumps(proof))
print(json.dumps({{"type": "result", "subtype": "success", "result": "checked"}}), flush=True)
''')
    fake.chmod(0o755)
    config = home / 'service/config'
    config.mkdir(parents=True, exist_ok=True)
    (config / 'config.toml').write_text(f'''[service]
port = 0
[agents.claude]
program = {json.dumps(str(fake))}
[agents.codex]
program = "/nonexistent-codex"
models = []
[agents.grok]
program = "/nonexistent-grok"
models = []
''')
    (project / 'agent-proof.json').unlink(missing_ok=True)
    return project


def launch(image, stopper, home, conflict=False, expected=None):
    command = home / '.local/bin/teryon'
    command.parent.mkdir(parents=True, exist_ok=True)
    project = prepare(home)
    diagnostic = home / 'service/logs/terminal-command.json'
    diagnostic.unlink(missing_ok=True)
    service_file = home / 'service/data/service.json'
    service_file.unlink(missing_ok=True)
    environment = dict(os.environ)
    for name in ('TERYON_DATA', 'TERYON_THREAD', 'TERYON_TURN', 'APPDIR', 'APPIMAGE'):
        environment.pop(name, None)
    environment.update(
        HOME=str(home), TERYON_HOME=str(home / 'service'),
        XDG_DATA_HOME=str(home / 'data'), XDG_CONFIG_HOME=str(home / 'config'),
        XDG_STATE_HOME=str(home / 'state'), XDG_CACHE_HOME=str(home / 'cache'),
        PATH=f'{command.parent}:{os.environ["PATH"]}', APPIMAGE_EXTRACT_AND_RUN='1',
    )
    with (home / 'launch.log').open('w+') as log:
        app = subprocess.Popen([str(image)], env=environment, stdout=log, stderr=log, start_new_session=True)
        try:
            report = wait_for(lambda: json_file(diagnostic), app, log)
            assert report['pending'] is False, report
            if conflict:
                assert [notice['kind'] for notice in report['notices']] == ['conflict'], report
            else:
                assert report['notices'] == [], report
            wait_for(lambda: json_file(service_file), app, log)
            subprocess.run(['bun', str(ROOT / 'desktop/scripts/check-bundled-agent.mjs'), str(service_file), str(project)],
                           env=environment, check=True, timeout=20)
            proof = json_file(project / 'agent-proof.json')
            assert proof and proof['bundled'], proof
        finally:
            if app.poll() is None:
                os.killpg(app.pid, signal.SIGTERM)
                try:
                    app.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(app.pid, signal.SIGKILL)
                    app.wait()
            subprocess.run([str(stopper), 'stop'], env=environment, check=True, timeout=15)
    if conflict:
        assert not command.is_symlink() and command.read_text() == 'existing command\n'
    else:
        assert command.is_symlink()
        assert command.resolve().is_relative_to(home / 'data/teryon/commands')
        output = subprocess.check_output([str(command), '--version'], env=environment, text=True).strip()
        assert output == f'teryon {expected}', output
    return command


@contextmanager
def newer_fixture(image, version, supplied):
    if supplied:
        yield supplied.resolve(strict=True)
        return
    manifest = ROOT / 'Cargo.toml'
    lock = ROOT / 'Cargo.lock'
    original, locked = manifest.read_bytes(), lock.read_bytes()
    numbers = re.match(r'(\d+)\.(\d+)\.(\d+)', version)
    newer = f'{numbers[1]}.{numbers[2]}.{int(numbers[3]) + 1}'
    built = image.parent / image.name.replace(version, newer)
    assert built != image
    assert not built.exists(), f'Fixture output already exists: {built}'
    try:
        changed, count = re.subn(r'^version = "' + re.escape(version) + r'"$', f'version = "{newer}"', original.decode(), count=1, flags=re.MULTILINE)
        assert count == 1, 'Could not identify the workspace version'
        manifest.write_text(changed)
        subprocess.run(['cargo', 'tauri', 'build', '--bundles', 'appimage', '--ci'], cwd=ROOT / 'desktop/src-tauri', check=True)
        assert built.exists(), f'Test build did not produce {built}'
        yield built
    finally:
        manifest.write_bytes(original)
        lock.write_bytes(locked)
        built.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image', type=Path)
    parser.add_argument('command', type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--newer-image', type=Path)
    mode.add_argument('--build-newer', action='store_true')
    args = parser.parse_args()
    image = args.image.resolve(strict=True)
    version = subprocess.check_output([str(args.command.resolve()), '--version'], text=True).strip().removeprefix('teryon ')
    with tempfile.TemporaryDirectory(prefix='teryon-appimage-check-') as folder:
        scratch = Path(folder)
        stopper = scratch / 'stop-service'
        saved_image = scratch / image.name
        shutil.copy2(args.command, stopper)
        shutil.copy2(image, saved_image)
        # Tauri clears its AppImage output directory when building the fixture.
        # Preserve both original artifacts and restore them even if a check fails.
        try:
            with newer_fixture(image, version, args.newer_image) as newer:
                home = scratch / 'home'
                home.mkdir()
                command = launch(saved_image, stopper, home, expected=version)
                inode = command.stat().st_ino
                launch(saved_image, stopper, home, expected=version)
                assert command.stat().st_ino == inode, 'Repeated launch replaced the executable'
                match = re.search(r'Teryon_(.+)_amd64.AppImage$', newer.name)
                assert match, f'Cannot read the fixture version from {newer.name}'
                newest = match[1]
                launch(newer, stopper, home, expected=newest)
                launch(saved_image, stopper, home, expected=newest)
                conflict_home = scratch / 'conflict'
                existing = conflict_home / '.local/bin/teryon'
                existing.parent.mkdir(parents=True)
                existing.write_text('existing command\n')
                launch(saved_image, stopper, conflict_home, conflict=True)
        finally:
            image.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(saved_image, image)
            if args.build_newer:
                shutil.copy2(stopper, args.command)
    print('AppImage: first/repeat launch, upgrade/downgrade, completed conflict notice, command after closing, and bundled agent invocation passed.')


if __name__ == '__main__':
    main()
