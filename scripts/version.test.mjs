import assert from 'node:assert/strict';
import { test } from 'node:test';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkVersion } from './check-version.mjs';
import { committedVersion, versionTargets, writeVersion } from './version-files.mjs';
import { lastStable, nextVersion, releaseOf } from './version-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), 'panita-version-'));
  for (const { file } of versionTargets) {
    await mkdir(dirname(join(directory, file)), { recursive: true });
    await copyFile(join(root, file), join(directory, file));
  }
  return directory;
}

test('the committed version files agree in this repository', async () => {
  assert.match(await committedVersion(root), /^\d+\.\d+\.\d+(-beta\.\d+)?$/);
});

test('last stable ignores prereleases and picks the highest stable tag', () => {
  assert.deepEqual(lastStable(['v0.1.1-alpha', 'v0.2.0-beta.3']), [0, 0, 0]);
  assert.deepEqual(lastStable(['v0.9.0', 'v0.10.0', 'v0.10.1-beta.1']), [0, 10, 0]);
});

test('next version increments the beta number while its base satisfies the magnitude', () => {
  const tags = ['v0.1.1-alpha'];
  assert.equal(nextVersion('0.2.0-beta.2', 'feat', tags), '0.2.0-beta.3');
  assert.equal(nextVersion('0.2.0-beta.2', 'fix', tags), '0.2.0-beta.3');
  assert.equal(nextVersion('0.3.0-beta.1', 'fix', ['v0.2.0']), '0.3.0-beta.2');
});

test('next version starts a new base when the magnitude exceeds the current one', () => {
  assert.equal(nextVersion('0.2.0-beta.2', 'major', ['v0.1.1-alpha']), '1.0.0-beta.1');
  assert.equal(nextVersion('0.2.0-beta.3', 'feat', ['v0.2.0']), '0.3.0-beta.1');
  assert.equal(nextVersion('0.2.0-beta.3', 'fix', ['v0.2.0']), '0.2.1-beta.1');
  assert.equal(nextVersion('0.2.0', 'fix', ['v0.2.0']), '0.2.1-beta.1');
  assert.equal(nextVersion('0.2.0', 'feat', ['v0.1.1-alpha']), '0.3.0-beta.1');
  assert.throws(() => nextVersion('0.2.0-beta.1', 'docs', []));
  assert.throws(() => nextVersion('0.2.0-alpha', 'fix', []));
});

test('release removes the beta suffix and refuses a stable version', () => {
  assert.equal(releaseOf('1.2.3-beta.7'), '1.2.3');
  assert.throws(() => releaseOf('1.2.3'));
});

test('writing a version changes only the version text of every target', async () => {
  const directory = await fixture();
  try {
    const original = await committedVersion(directory);
    await writeVersion(directory, '9.8.7-beta.6');
    assert.equal(await committedVersion(directory), '9.8.7-beta.6');
    for (const { file } of versionTargets) {
      const restored = (await readFile(join(directory, file), 'utf8')).replaceAll('9.8.7-beta.6', original);
      assert.equal(restored, await readFile(join(root, file), 'utf8'), file);
    }
    await assert.rejects(writeVersion(directory, '1.0'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('CRLF checkouts are read and written like LF ones', async () => {
  const directory = await fixture();
  try {
    const original = await committedVersion(directory);
    for (const { file } of versionTargets) {
      const path = join(directory, file);
      await writeFile(path, (await readFile(path, 'utf8')).replace(/\r?\n/g, '\r\n'));
    }
    assert.equal(await committedVersion(directory), original);
    await writeVersion(directory, '9.8.7-beta.6');
    assert.equal(await committedVersion(directory), '9.8.7-beta.6');
    assert.ok((await readFile(join(directory, 'Cargo.lock'), 'utf8')).includes('\r\n'));
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test('check reports disagreeing files, tag mismatches and channel violations', async () => {
  const directory = await fixture();
  try {
    const version = await checkVersion({ base: directory });
    assert.equal(await checkVersion({ base: directory, tag: `v${version}` }), version);
    await assert.rejects(checkVersion({ base: directory, tag: 'v9.9.9' }), /does not match/);
    await assert.rejects(checkVersion({ base: directory, channel: version.includes('-') ? 'stable' : 'beta' }), /requires/);
    const file = join(directory, 'ui/package.json');
    await writeFile(file, (await readFile(file, 'utf8')).replace(version, '0.0.1'));
    await assert.rejects(checkVersion({ base: directory }), /ui\/package\.json: 0\.0\.1/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
