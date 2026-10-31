import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildFeed, feedVersion, packageFiles, shouldAdvance } from './release-feed.mjs';
import { releasePlan } from './release-plan.mjs';
import { versionTargets, writeVersion } from './version-files.mjs';
import { compareVersions } from './version-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');

async function fixture(version) {
  const directory = await mkdtemp(join(tmpdir(), 'panita-release-plan-'));
  for (const { file } of versionTargets) {
    await mkdir(dirname(join(directory, file)), { recursive: true });
    await copyFile(join(root, file), join(directory, file));
  }
  await writeVersion(directory, version);
  return directory;
}

test('develop publishes betas to the beta feed and master publishes stable releases to both feeds', async () => {
  const beta = await fixture('0.3.0-beta.2');
  const stable = await fixture('0.3.0');
  try {
    assert.deepEqual(await releasePlan({ branch: 'develop', base: beta }), {
      version: '0.3.0-beta.2', tag: 'v0.3.0-beta.2', channel: 'beta', prerelease: true, feeds: ['updater-beta'],
    });
    assert.deepEqual(await releasePlan({ branch: 'master', base: stable }), {
      version: '0.3.0', tag: 'v0.3.0', channel: 'stable', prerelease: false, feeds: ['updater-stable', 'updater-beta'],
    });
  } finally {
    await rm(beta, { recursive: true, force: true });
    await rm(stable, { recursive: true, force: true });
  }
});

test('a branch cannot publish a version of the other channel, and other branches never publish', async () => {
  const beta = await fixture('0.3.0-beta.2');
  const stable = await fixture('0.3.0');
  try {
    await assert.rejects(releasePlan({ branch: 'master', base: beta }), /stable channel requires/);
    await assert.rejects(releasePlan({ branch: 'develop', base: stable }), /beta channel requires/);
    for (const branch of ['feat/updater', 'main', '', undefined]) {
      await assert.rejects(releasePlan({ branch, base: stable }), /does not publish releases/);
    }
  } finally {
    await rm(beta, { recursive: true, force: true });
    await rm(stable, { recursive: true, force: true });
  }
});

test('versions order like semver: betas by number and a stable release above its betas', () => {
  const order = ['0.2.0-beta.1', '0.2.0-beta.2', '0.2.0-beta.10', '0.2.0', '0.2.1-beta.1', '0.3.0'];
  for (let index = 1; index < order.length; index += 1) {
    assert.ok(compareVersions(order[index], order[index - 1]) > 0, `${order[index]} > ${order[index - 1]}`);
    assert.ok(compareVersions(order[index - 1], order[index]) < 0);
  }
  assert.equal(compareVersions('1.0.0', '1.0.0'), 0);
  assert.equal(compareVersions('1.0.0-beta.3', '1.0.0-beta.3'), 0);
});

const signatures = { 'windows-x86_64': 'sig-win\n', 'linux-x86_64-appimage': 'sig-appimage', 'linux-x86_64-deb': ' sig-deb ' };

test('the feed names every package of the exact release and carries the signatures as written', () => {
  const feed = buildFeed({ version: '0.3.0-beta.2', signatures, pubDate: '2026-11-01T00:00:00.000Z' });
  assert.equal(feed.version, '0.3.0-beta.2');
  assert.equal(feed.pub_date, '2026-11-01T00:00:00.000Z');
  assert.deepEqual(Object.keys(feed.platforms), ['windows-x86_64', 'linux-x86_64-appimage', 'linux-x86_64-deb']);
  assert.equal(feed.platforms['windows-x86_64'].signature, 'sig-win');
  assert.equal(feed.platforms['linux-x86_64-deb'].signature, 'sig-deb');
  for (const { key, file } of packageFiles('0.3.0-beta.2')) {
    // The launcher accepts only artifacts under this exact prefix.
    assert.equal(feed.platforms[key].url, `https://github.com/panitacraft/panita-client/releases/download/v0.3.0-beta.2/${file}`);
  }
  assert.match(feed.notes, /releases\/tag\/v0\.3\.0-beta\.2/);
});

test('a feed cannot be built without every signature or for an invalid version', () => {
  for (const key of Object.keys(signatures)) {
    assert.throws(() => buildFeed({ version: '0.3.0', signatures: { ...signatures, [key]: '  ' } }), new RegExp(`Missing updater signature for ${key}`));
  }
  assert.throws(() => buildFeed({ version: '0.3', signatures }));
  assert.throws(() => buildFeed({ version: '../0.3.0', signatures }));
});

test('a feed only moves forward and an unreadable one is replaced', () => {
  const at = version => ({ version });
  assert.equal(shouldAdvance(null, '0.3.0-beta.1'), true);
  assert.equal(shouldAdvance(at('0.3.0-beta.1'), '0.3.0-beta.2'), true);
  assert.equal(shouldAdvance(at('0.3.0-beta.2'), '0.3.0-beta.2'), false);
  assert.equal(shouldAdvance(at('0.3.0-beta.3'), '0.3.0-beta.2'), false);
  assert.equal(shouldAdvance(at('0.3.0-beta.5'), '0.3.0'), true);
  assert.equal(shouldAdvance(at('0.3.0'), '0.3.0-beta.6'), false);
  assert.equal(shouldAdvance(at('0.3.0'), '0.3.1-beta.1'), true);
  for (const broken of [{}, { version: 3 }, { version: 'x' }, 'text', []]) assert.equal(shouldAdvance(broken, '0.3.0'), true);
  assert.equal(feedVersion(at('0.3.0')), '0.3.0');
  assert.equal(feedVersion(at('nope')), null);
});

test('the feed command writes a feed only when it advances', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'panita-release-feed-'));
  try {
    for (const { key, file } of packageFiles('0.3.0')) await writeFile(join(directory, `${file}.sig`), `signature of ${key}\n`);
    const script = join(root, 'scripts/release-feed.mjs');
    const out = join(directory, 'latest.json');
    const run = (...extra) => spawnSync(process.execPath, [script, '--version', '0.3.0', '--dir', directory, '--out', out, ...extra], { encoding: 'utf8' });
    const first = run();
    assert.equal(first.status, 0, first.stderr);
    assert.match(first.stdout, /advance=true/);
    const written = JSON.parse(await readFile(out, 'utf8'));
    assert.equal(written.platforms['windows-x86_64'].signature, 'signature of windows-x86_64');
    const repeat = run('--current', out);
    assert.match(repeat.stdout, /advance=false/);
    await rm(out);
    const newer = spawnSync(process.execPath, [script, '--version', '0.3.0', '--dir', directory, '--out', out, '--current', join(directory, 'absent.json')], { encoding: 'utf8' });
    assert.match(newer.stdout, /advance=true/);
    await rm(join(directory, `${packageFiles('0.3.0')[1].file}.sig`));
    await rm(out);
    const missing = run();
    assert.notEqual(missing.status, 0);
    assert.match(missing.stderr, /ENOENT|no such file/i);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
