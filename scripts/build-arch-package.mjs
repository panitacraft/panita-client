import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { archPackageVersion, releaseVersion } from './release-version.mjs';
import { verifyBeta } from './verify-beta.mjs';

if (process.platform !== 'linux' || process.arch !== 'x64') throw new Error('Arch packaging requires Linux x64.');
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const beta = args.length === 1 && args[0] === '--beta';
if (args.length !== 1) throw new Error('Expected --beta or a release tag');
const version = beta
  ? JSON.parse(await readFile(join(root, 'desktop/tauri.beta.conf.json'), 'utf8')).version
  : releaseVersion(args[0]);
const release = !beta;
const directory = join(root, release ? 'target/releases' : 'target/beta', version, 'linux');
const result = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' });
if (result.status !== 0) throw new Error('Cannot resolve source revision');
const revision = result.stdout.trim();
await verifyBeta({ directory, platform: 'linux', version, revision, release });
const pkgver = archPackageVersion(version);
const build = spawnSync('bash', [join(root, 'scripts/linux/build-arch-package.sh'), version, pkgver, beta ? 'beta' : 'release'], {
  cwd: root, stdio: 'inherit',
});
if (build.error || build.status !== 0) throw new Error('Arch package build or inspection failed.');
const artifact = `panita-client_${version}_archlinux_x64_unsigned.pkg.tar.zst`;
const hash = createHash('sha256');
for await (const chunk of createReadStream(join(directory, artifact))) hash.update(chunk);
const sha256 = hash.digest('hex');
await writeFile(join(directory, 'candidate-arch.json'), `${JSON.stringify({
  schema: 1, type: release ? 'unsigned_release_candidate' : 'unsigned_beta_candidate', version, source_revision: revision,
  platform: 'linux_x64', artifact, sha256, signed: false, qualification: 'pending',
  package_version: `${pkgver}-1`,
}, null, 2)}\n`);
const checksumPath = join(directory, 'SHA256SUMS');
const existing = await readFile(checksumPath, 'utf8');
await writeFile(checksumPath, `${existing}${sha256}  ${artifact}\n`);
await verifyBeta({ directory, platform: 'linux', version, revision, release, withArch: true });
