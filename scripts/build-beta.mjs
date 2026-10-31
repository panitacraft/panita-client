import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { copyFile, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkVersion } from './check-version.mjs';
import { releaseVersion } from './release-version.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const platform = { win32: ['windows', [['nsis', '.exe']]], linux: ['linux', [['deb', '.deb'], ['appimage', '.AppImage']]] }[process.platform];
if (!platform || process.arch !== 'x64') throw new Error('Packaging currently requires Windows x64 or Linux x64.');
const [os, bundles] = platform;
const config = JSON.parse(await readFile(join(root, 'desktop/tauri.beta.conf.json'), 'utf8'));
const baseConfig = JSON.parse(await readFile(join(root, 'desktop/tauri.conf.json'), 'utf8'));
const args = process.argv.slice(2);
const loaderValidation = args.includes('--loader-validation');
if (loaderValidation) args.splice(args.indexOf('--loader-validation'), 1);
const releaseIndex = args.indexOf('--release-tag');
const release = releaseIndex !== -1;
const releaseTag = release ? args[releaseIndex + 1] : undefined;
if (loaderValidation && release) throw new Error('Loader validation builds cannot be release candidates.');
if (release) {
  // The committed version is authoritative: every manifest must already carry the tagged version.
  await checkVersion({ tag: releaseTag });
  config.version = releaseVersion(releaseTag);
  args.splice(releaseIndex, 2);
} else if (!/^\d+\.\d+\.\d+-beta\.\d+$/.test(config.version)) throw new Error('Expected an explicit beta version.');
if (args.some(arg => arg !== '--check')) throw new Error('Supported options: --check, --release-tag <vVERSION>, --loader-validation');

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
if (release) {
  // The tag is created at the built commit when the release is published, so it may not exist
  // yet. If it does exist it must already identify this exact commit.
  const tagged = spawnSync('git', ['rev-parse', '--quiet', '--verify', `refs/tags/${releaseTag}^{commit}`], { cwd: root, encoding: 'utf8', windowsHide: true });
  if (tagged.status === 0 && tagged.stdout.trim() !== revision) throw new Error('Release tag must identify the current source commit.');
}
const destination = join(root, loaderValidation ? 'target/loader-validation' : release ? 'target/releases' : 'target/beta', config.version, os);
console.log(`Unsigned ${loaderValidation ? 'internal loader validation' : release ? 'release' : 'beta'}: ${config.version} (${bundles.map(([bundle]) => bundle).join(', ')})\nSource: ${revision}\nOutput: ${destination}`);
if (args.includes('--check')) process.exit(0);

// No OS code signing (the configuration disables it) and no publication is performed. Tauri
// retains the existing application identity. Updater signing needs TAURI_SIGNING_PRIVATE_KEY
// and is skipped only for internal loader-validation builds, which are never distributed.
run(process.execPath, [join(root, 'scripts/tauri.mjs'), 'build', '--ci', ...(loaderValidation ? ['--no-sign'] : []), '--config', release ? JSON.stringify(config) : join(root, 'desktop/tauri.beta.conf.json'), '--bundles', bundles.map(([bundle]) => bundle).join(','), ...(loaderValidation ? ['--features', 'loader-validation'] : []), '--', '--locked']);
if (run('git', ['rev-parse', 'HEAD'], true) !== revision
    || run('git', ['status', '--porcelain', '--untracked-files=normal'], true).length > 0) {
  throw new Error('Source changed during compilation. Commit changes and rebuild before preparing a candidate.');
}
const checksums = [];
await mkdir(destination, { recursive: true });
for (const [bundle, extension] of bundles) {
  const name = `panita-client_${config.version}_${os}_x64_unsigned${loaderValidation ? '_loader-validation' : ''}${extension}`;
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
  if (!loaderValidation) {
    // The signature covers the installer bytes, so it stays valid under the renamed artifact.
    if (!files.includes(`${expected}.sig`)) throw new Error(`Missing updater signature ${expected}.sig; set TAURI_SIGNING_PRIVATE_KEY.`);
    await copyFile(join(bundleDirectory, `${expected}.sig`), join(destination, `${name}.sig`));
  }
  // Preserve the original manifest name for the primary installer.
  const manifest = bundle === 'appimage' ? 'candidate-appimage.json' : 'candidate.json';
  await writeFile(join(destination, manifest), `${JSON.stringify({
    schema: 1,
    type: loaderValidation ? 'unsigned_loader_validation_candidate' : release ? 'unsigned_release_candidate' : 'unsigned_beta_candidate',
    ...(loaderValidation ? { features: ['loader-validation'], distribution: 'internal_validation_only' } : {}),
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
