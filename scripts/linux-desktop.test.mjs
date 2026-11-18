import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { promisify } from 'node:util';
import test from 'node:test';

const readJson = async path => JSON.parse(await readFile(path, 'utf8'));
const execFileAsync = promisify(execFile);

function pngSize(bytes) {
  assert.deepEqual([...bytes.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10]);
  assert.equal(bytes.subarray(12, 16).toString('ascii'), 'IHDR');
  return [bytes.readUInt32BE(16), bytes.readUInt32BE(20)];
}

test('Linux desktop identity matches the packaged executable and icon theme', async () => {
  const base = await readJson('desktop/tauri.conf.json');
  const linux = await readJson('desktop/tauri.linux.conf.json');
  const entry = await readFile('desktop/resources/linux/panita.desktop', 'utf8');
  const canonicalEntry = await readFile('desktop/resources/linux/com.panitacraft.client.desktop', 'utf8');
  const postInstall = await readFile('desktop/resources/linux/deb-postinstall.sh', 'utf8');
  const identity = await readFile('desktop/src/linux_desktop.rs', 'utf8');
  const desktop = await readFile('desktop/src/lib.rs', 'utf8');
  const protocol = await readFile('desktop/src/linux_protocol.rs', 'utf8');
  const protocolBridge = await readFile('desktop/src/protocol.rs', 'utf8');

  assert.equal(base.mainBinaryName, 'panita-client');
  assert.equal(base.identifier, 'com.panitacraft.client');
  assert.equal(base.app.enableGTKAppId, true);
  assert.equal(linux.productName, 'panita-client');
  assert.equal(linux.bundle.icon[0], 'icons/icon.png');
  assert.deepEqual(new Set(linux.bundle.icon), new Set([
    'icons/icon.png',
    'icons/32x32.png',
    'icons/64x64.png',
    'icons/128x128.png',
    'icons/128x128@2x.png',
  ]));
  assert.match(entry, /^Name=Panita Client$/m);
  assert.match(entry, /^Exec=\{\{exec\}\} %u$/m);
  assert.match(entry, /^Icon=\{\{icon\}\}$/m);
  assert.match(entry, /^StartupWMClass=com\.panitacraft\.client$/m);
  assert.match(canonicalEntry, /^Exec=\/usr\/bin\/panita-client %u$/m);
  assert.match(canonicalEntry, /^StartupWMClass=com\.panitacraft\.client$/m);
  assert.equal(
    linux.bundle.linux.deb.files['/usr/share/applications/com.panitacraft.client.desktop'],
    'resources/linux/com.panitacraft.client.desktop',
  );
  assert.equal(
    linux.bundle.linux.appimage?.files?.['/usr/share/applications/com.panitacraft.client.desktop'],
    undefined,
  );
  assert.equal(linux.bundle.linux.deb.postInstallScript, 'resources/linux/deb-postinstall.sh');
  assert.match(postInstall, /X-PanitaClient-Owner=com\.panitacraft\.client/);
  assert.match(postInstall, /StartupWMClass=com\.panitacraft\.client/);
  assert.match(identity, /set_prgname\(Some\("com\.panitacraft\.client"\)\)/);
  assert.match(identity, /set_program_class\("com\.panitacraft\.client"\)/);
  assert.match(protocol, /const DESKTOP_ID: &str = "com\.panitacraft\.client\.desktop";/);
  assert.match(protocol, /Icon=panita-client/);
  assert.match(desktop, /protocol::prepare_desktop_integration\(app\.handle\(\)\)/);
  assert.match(desktop, /protocol::finish_desktop_integration\(pending\)/);
  assert.match(desktop, /tauri::async_runtime::spawn_blocking/);
  assert.match(protocolBridge, /pub fn prepare_desktop_integration/);
  assert.match(protocolBridge, /pub fn finish_desktop_integration/);
  assert.match(protocolBridge, /linux_protocol::prepare/);
  assert.match(protocolBridge, /linux_protocol::finish_migration/);
  assert.match(postInstall, /Panita Client\.desktop/);
  assert.match(postInstall, /StartupWMClass=panita-desktop/);
  assert.match(postInstall, /Exec=\(\/usr\/bin\/\)\?panita-client %u/);
});

test('Every Linux package provides libdecor so native Wayland windows keep their title bar', async () => {
  const linux = await readJson('desktop/tauri.linux.conf.json');
  const pkgbuild = await readFile('scripts/linux/arch/PKGBUILD', 'utf8');
  const stage = await readFile('scripts/linux/stage-libdecor.sh', 'utf8');
  const build = await readFile('scripts/build-beta.mjs', 'utf8');
  const inspection = await readFile('scripts/linux/verify-candidate.sh', 'utf8');
  const arch = await readFile('scripts/linux/build-arch-package.sh', 'utf8');
  const depends = linux.bundle.linux.deb.depends;

  assert.ok(depends.includes('libdecor-0-0'));
  assert.ok(depends.includes('libdecor-0-plugin-1-cairo'));
  assert.ok(!depends.some(entry => entry.includes('libdecor-0-plugin-1-gtk')));
  assert.match(pkgbuild, /depends=\([^)]*'libdecor'/s);
  assert.match(arch, /grep -Fx 'depend = libdecor'/);
  assert.match(inspection, /libdecor-0-0/);
  // The AppImage carries the library and plugin from the build host's apt packages.
  const files = linux.bundle.linux.appimage.files;
  assert.deepEqual(Object.keys(files).sort(), [
    '/usr/lib/panita/libdecor/copyright',
    '/usr/lib/panita/libdecor/libdecor-0.so.0',
    '/usr/lib/panita/libdecor/plugins-1/libdecor-cairo.so',
  ]);
  for (const source of Object.values(files)) assert.match(source, /^\.\.\/target\/linux-libdecor\//);
  assert.match(stage, /libdecor-0-plugin-1-cairo/);
  assert.doesNotMatch(stage, /https?:\/\/|sha256sum/);
  assert.match(build, /stage-libdecor\.sh/);
  assert.match(inspection, /usr\/lib\/panita\/libdecor/);
});

test('Linux updater relaunch is sanitized and deferred until final exit', async () => {
  const updater = await readFile('desktop/src/update_commands.rs', 'utf8');
  const desktop = await readFile('desktop/src/lib.rs', 'utf8');

  for (const variable of [
    'APPIMAGE',
    'APPDIR',
    'GIO_LAUNCHED_DESKTOP_FILE',
    'BAMF_DESKTOP_FILE_HINT',
    'XDG_ACTIVATION_TOKEN',
    'DESKTOP_STARTUP_ID',
    'LD_LIBRARY_PATH',
  ]) {
    assert.match(updater, new RegExp(`"${variable}"`));
  }
  assert.match(updater, /starts_with\("panitacraft:"\)/);
  assert.match(updater, /app\.exit\(0\)/);
  assert.match(desktop, /RunEvent::Exit/);
  assert.match(desktop, /update_commands::relaunch_after_exit\(app\)/);
});

test('Linux icon inputs contain every configured resolution', async () => {
  for (const [path, size] of [
    ['desktop/icons/32x32.png', 32],
    ['desktop/icons/64x64.png', 64],
    ['desktop/icons/128x128.png', 128],
    ['desktop/icons/128x128@2x.png', 256],
    ['desktop/icons/icon.png', 512],
  ]) {
    assert.deepEqual(pngSize(await readFile(path)), [size, size], path);
  }
});

test('AppImage inspection accepts the bundler-selected root icon size', async () => {
  const inspection = await readFile('scripts/linux/verify-candidate.sh', 'utf8');

  assert.match(inspection, /test "\$\(readlink squashfs-root\/\.DirIcon\)" = panita-client\.png/);
  assert.match(inspection, /readlink -e -- "\$appimage_icon"/);
  assert.match(inspection, /file -L -b -- "\$appimage_icon" \| grep -Eq '\^PNG image data,/);
  assert.doesNotMatch(
    inspection,
    /cmp squashfs-root\/panita-client\.png squashfs-root\/usr\/share\/icons\/hicolor\/512x512/,
  );
});

test('Linux AppImage icon inspection follows a packaged icon symlink', {
  skip: process.platform !== 'linux',
}, async () => {
  const directory = await mkdtemp(join(tmpdir(), 'panita-appimage-icon-'));
  try {
    const icon = join(directory, 'panita-client.png');
    await symlink(resolve('desktop/icons/128x128.png'), icon);
    const { stdout } = await execFileAsync('file', ['-L', '-b', '--', icon], {
      env: { ...process.env, LC_ALL: 'C' },
    });
    assert.match(stdout, /^PNG image data, 128 x 128,/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('Minecraft windows share one hidden desktop entry that matches the game application ID', async () => {
  const window = await readFile('crates/launcher-core/src/minecraft/linux_window.rs', 'utf8');
  const identity = await readFile('desktop/src/linux_game_identity.rs', 'utf8');
  const bridge = await readFile('desktop/src/protocol.rs', 'utf8');
  const play = await readFile('crates/launcher-core/src/minecraft/play.rs', 'utf8');

  assert.match(window, /pub\(super\) const APP_ID: &str = "com\.panitacraft\.minecraft";/);
  assert.match(window, /"SDL_APP_ID"/);
  assert.match(identity, /const ENTRY_ID: &str = "com\.panitacraft\.minecraft\.desktop";/);
  assert.match(identity, /StartupWMClass=com\.panitacraft\.minecraft\\n/);
  assert.match(identity, /NoDisplay=true/);
  assert.match(identity, /Icon=com\.panitacraft\.minecraft\\n/);
  assert.match(window, /const ICON_FILE: &str = "com\.panitacraft\.minecraft\.png";/);
  assert.match(identity, /const ICON_FILE: &str = "com\.panitacraft\.minecraft\.png";/);
  assert.match(bridge, /linux_game_identity::publish/);
  assert.match(play, /publish_minecraft_icons/);
});
