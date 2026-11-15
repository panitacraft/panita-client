import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const readJson = async path => JSON.parse(await readFile(path, 'utf8'));

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
  assert.match(inspection, /file "\$appimage_icon" \| grep -Eq ': PNG image data,/);
  assert.doesNotMatch(
    inspection,
    /cmp squashfs-root\/panita-client\.png squashfs-root\/usr\/share\/icons\/hicolor\/512x512/,
  );
});
