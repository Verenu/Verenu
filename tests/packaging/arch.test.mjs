import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

test('Arch packaging normalizes nightly versions and makes extracted runtime readable', async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'verenu-arch-package-'));
  const srcdir = path.join(directory, 'source');
  const pkgdir = path.join(directory, 'package');
  try {
    await fs.mkdir(path.join(srcdir, 'squashfs-root', 'usr', 'bin'), { recursive: true, mode: 0o700 });
    await fs.mkdir(pkgdir);
    await fs.writeFile(path.join(srcdir, 'LICENSE'), 'Synthetic public license fixture');
    const app = path.join(srcdir, 'squashfs-root');
    await fs.writeFile(path.join(app, 'AppRun'), '#!/bin/sh\nexit 0\n', { mode: 0o700 });
    await fs.writeFile(path.join(app, 'usr', 'bin', 'Verenu'), 'Synthetic executable fixture', { mode: 0o700 });
    await fs.writeFile(path.join(app, 'Verenu.desktop'), '[Desktop Entry]\nExec=private-build-path\nIcon=private-build-icon\n', { mode: 0o600 });
    await fs.writeFile(path.join(app, 'Verenu.png'), 'Synthetic public icon fixture', { mode: 0o600 });
    const result = spawnSync('bash', ['-euo', 'pipefail', '-c', 'source "$VERENU_PKGBUILD"; test "$pkgver" = 0.20.0_nightly.20261005; test "${source[0]}" = Verenu_0.20.0-nightly.20261005_x86_64.AppImage; package'], {
      env: { ...process.env, VERENU_PKGBUILD: path.resolve('packaging/arch/PKGBUILD'), VERENU_PKGVER: '0.20.0-nightly.20261005', srcdir, pkgdir }, encoding: 'utf8',
    });
    assert.equal(result.status, 0, result.stderr);
    for (const name of ['opt/verenu', 'opt/verenu/usr', 'opt/verenu/usr/bin']) {
      assert.equal((await fs.stat(path.join(pkgdir, name))).mode & 0o005, 0o005, `Other users can read/traverse ${name}`);
    }
    assert.equal((await fs.stat(path.join(pkgdir, 'opt/verenu/usr/bin/Verenu'))).mode & 0o005, 0o005);
    assert.equal(await fs.readlink(path.join(pkgdir, 'usr/bin/verenu')), '/opt/verenu/AppRun');
    assert.match(await fs.readFile(path.join(pkgdir, 'usr/share/applications/verenu.desktop'), 'utf8'), /Exec=verenu\nIcon=verenu/);
  } finally { await fs.rm(directory, { recursive: true, force: true }); }
});
