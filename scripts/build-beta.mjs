import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { copyFile, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { releaseVersion } from './release-version.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const platform = { win32: ['windows', [['nsis', '.exe']]], linux: ['linux', [['deb', '.deb'], ['appimage', '.AppImage']]] }[process.platform];
if (!platform || process.arch !== 'x64') throw new Error('Packaging currently requires Windows x64 or Linux x64.');
const [os, bundles] = platform;
const config = JSON.parse(await readFile(join(root, 'desktop/tauri.beta.conf.json'), 'utf8'));
const baseConfig = JSON.parse(await readFile(join(root, 'desktop/tauri.conf.json'), 'utf8'));
const args = process.argv.slice(2);
const releaseIndex = args.indexOf('--release-tag');
const release = releaseIndex !== -1;
const releaseTag = release ? args[releaseIndex + 1] : undefined;
if (release) {
  config.version = releaseVersion(releaseTag);
  args.splice(releaseIndex, 2);
} else if (!/^\d+\.\d+\.\d+-beta\.\d+$/.test(config.version)) throw new Error('Expected an explicit beta version.');
if (args.some(arg => arg !== '--check')) throw new Error('Supported options: --check, --release-tag <vVERSION>');

function run(command, commandArgs, capture = false) {
  const result = spawnSync(command, commandArgs, {
    cwd: root,
    env: { ...process.env, CARGO_TARGET_DIR: join(root, 'target/phase-7-build') },
    encoding: 'utf8',
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status}).${capture ? ` ${result.stderr.trim()}` : ''}`);
  return result.stdout?.trim();
}

const revision = run('git', ['rev-parse', 'HEAD'], true);
const dirty = run('git', ['status', '--porcelain', '--untracked-files=normal'], true).length > 0;
if (dirty) throw new Error('Commit task changes before producing an installer candidate.');
if (release && run('git', ['rev-parse', `refs/tags/${releaseTag}^{commit}`], true) !== revision) {
  throw new Error('Release tag must identify the current source commit.');
}
const destination = join(root, release ? 'target/releases' : 'target/beta', config.version, os);
console.log(`Unsigned ${release ? 'release' : 'beta'}: ${config.version} (${bundles.map(([bundle]) => bundle).join(', ')})\nSource: ${revision}\nOutput: ${destination}`);
if (args.includes('--check')) process.exit(0);

// No signing or publication is performed. Tauri retains the existing application identity.
run(process.execPath, [join(root, 'scripts/tauri.mjs'), 'build', '--ci', '--no-sign', '--config', release ? JSON.stringify(config) : join(root, 'desktop/tauri.beta.conf.json'), '--bundles', bundles.map(([bundle]) => bundle).join(','), '--', '--locked']);
if (run('git', ['rev-parse', 'HEAD'], true) !== revision
    || run('git', ['status', '--porcelain', '--untracked-files=normal'], true).length > 0) {
  throw new Error('Source changed during compilation. Commit changes and rebuild before preparing a candidate.');
}
const checksums = [];
await mkdir(destination, { recursive: true });
for (const [bundle, extension] of bundles) {
  const name = `panita-client_${config.version}_${os}_x64_unsigned${extension}`;
  const bundleDirectory = join(root, 'target/phase-7-build/release/bundle', bundle);
  const expected = bundle === 'nsis'
    ? `Panita Client_${config.version}_x64-setup.exe`
    : `${config.productName ?? baseConfig.productName}_${config.version}_amd64${extension}`;
  const files = await readdir(bundleDirectory);
  // Match the exact version to avoid copying an older candidate left by an interrupted build.
  const candidates = files.filter(file => file === expected);
  if (candidates.length !== 1) throw new Error(`Expected exactly one ${expected} installer; found ${candidates.length}.`);
  const source = join(bundleDirectory, candidates[0]);
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(source)) hash.update(chunk);
  const sha256 = hash.digest('hex');
  await copyFile(source, join(destination, name));
  checksums.push(`${sha256}  ${name}`);
  // Preserve the original manifest name for the primary installer.
  const manifest = bundle === 'appimage' ? 'candidate-appimage.json' : 'candidate.json';
  await writeFile(join(destination, manifest), `${JSON.stringify({
    schema: 1,
    type: release ? 'unsigned_release_candidate' : 'unsigned_beta_candidate',
    version: config.version,
    source_revision: revision,
    platform: `${os}_x64`,
    artifact: name,
    sha256,
    signed: false,
    qualification: 'pending',
  }, null, 2)}\n`);
  console.log(`Candidate prepared: ${join(destination, name)}\nSHA-256: ${sha256}`);
}
await writeFile(join(destination, 'SHA256SUMS'), `${checksums.join('\n')}\n`);
console.log('Installed-product qualification remains pending.');
