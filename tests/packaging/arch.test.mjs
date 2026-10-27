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

test('Arch upgrade repairs private directories from the 0.20 package', {
  skip: !process.env.VERENU_ARCH_UPDATE_FIXTURES,
  timeout: 300_000,
}, () => {
  const fixtures = path.resolve(process.env.VERENU_ARCH_UPDATE_FIXTURES || '.');
  const script = `
    pacman -Sy --noconfirm --needed squashfs-tools >/tmp/dependencies.log
    useradd -m builder
    mkdir /tmp/package
    cp /recipe/PKGBUILD /recipe/verenu.install /input/LICENSE /fixtures/Verenu_0.21.0-beta.1_x86_64.AppImage /tmp/package/
    chown -R builder:builder /tmp/package
    cd /tmp/package
    su builder -s /bin/bash -c 'VERENU_PKGVER=0.21.0-beta.1 makepkg --nodeps --noconfirm' >/tmp/build.log
    pacman -U --noconfirm /fixtures/previous/verenu-0.20.0-1-x86_64.pkg.tar.zst >/tmp/old.log
    test "$(stat -c %a /opt/verenu)" = 700
    pacman -U --noconfirm /tmp/package/verenu-0.21.0_beta.1-1-x86_64.pkg.tar.zst >/tmp/new.log
    setpriv --reuid=65534 --regid=65534 --clear-groups sh -c '
      test -x /usr/bin/verenu && test -x /opt/verenu/usr/bin/Verenu &&
      test -z "$(find /opt/verenu -type d ! -executable -print -quit)" &&
      test -z "$(find /opt/verenu -type f ! -readable -print -quit)"
    '
  `;
  const result = spawnSync('docker', [
    'run', '--rm',
    '-v', `${path.resolve('packaging/arch')}:/recipe:ro`,
    '-v', `${path.resolve('LICENSE')}:/input/LICENSE:ro`,
    '-v', `${fixtures}:/fixtures:ro`,
    'archlinux:base-devel', 'bash', '-euc', script,
  ], { encoding: 'utf8', timeout: 290_000 });
  assert.equal(result.status, 0, result.error?.message || result.stderr);
});
