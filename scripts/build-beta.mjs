import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { copyFile, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const platform = { win32: ['windows', 'nsis', '.exe'], linux: ['linux', 'deb', '.deb'] }[process.platform];
if (!platform || process.arch !== 'x64') throw new Error('Beta packaging currently requires Windows x64 or Linux x64.');
const [os, bundle, extension] = platform;
const config = JSON.parse(await readFile(join(root, 'desktop/tauri.beta.conf.json'), 'utf8'));
if (!/^\d+\.\d+\.\d+-beta\.\d+$/.test(config.version)) throw new Error('Expected an explicit beta version.');
const args = process.argv.slice(2);
if (args.some(arg => arg !== '--check')) throw new Error('Supported option: --check');

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
if (dirty) throw new Error('Commit task changes before producing a beta candidate.');
const name = `panita-client_${config.version}_${os}_x64_unsigned${extension}`;
const destination = join(root, 'target/beta', config.version, os);
console.log(`Unsigned beta: ${name}\nSource: ${revision}\nOutput: ${destination}`);
if (args.includes('--check')) process.exit(0);

// No signing or publication is performed. Tauri retains the existing application identity.
run(process.execPath, [join(root, 'scripts/tauri.mjs'), 'build', '--ci', '--no-sign', '--config', join(root, 'desktop/tauri.beta.conf.json'), '--bundles', bundle, '--', '--locked']);
if (run('git', ['rev-parse', 'HEAD'], true) !== revision
    || run('git', ['status', '--porcelain', '--untracked-files=normal'], true).length > 0) {
  throw new Error('Source changed during compilation. Commit changes and rebuild before preparing a candidate.');
}
const bundleDirectory = join(root, 'target/phase-7-build/release/bundle', bundle);
const expected = bundle === 'nsis'
  ? `Panita Client_${config.version}_x64-setup.exe`
  : `panita-client_${config.version}_amd64.deb`;
const files = await readdir(bundleDirectory);
// Match the exact version to avoid copying an older candidate left by an interrupted build.
const candidates = files.filter(file => file === expected);
if (candidates.length !== 1) throw new Error(`Expected exactly one ${expected} installer; found ${candidates.length}.`);
const source = join(bundleDirectory, candidates[0]);
const hash = createHash('sha256');
for await (const chunk of createReadStream(source)) hash.update(chunk);
const sha256 = hash.digest('hex');
await mkdir(destination, { recursive: true });
await copyFile(source, join(destination, name));
await writeFile(join(destination, 'SHA256SUMS'), `${sha256}  ${name}\n`);
await writeFile(join(destination, 'candidate.json'), `${JSON.stringify({
  schema: 1,
  type: 'unsigned_beta_candidate',
  version: config.version,
  source_revision: revision,
  platform: `${os}_x64`,
  artifact: name,
  sha256,
  signed: false,
  qualification: 'pending',
}, null, 2)}\n`);
console.log(`Candidate prepared: ${join(destination, name)}\nSHA-256: ${sha256}\nInstalled-product qualification remains pending.`);
