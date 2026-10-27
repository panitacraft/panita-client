import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { archPackageVersion, releaseVersion } from './release-version.mjs';

// Artifact paths come from the release contract, never from a candidate manifest.
export async function verifyBeta({ directory, platform, version, revision, release = false, withArch = false, loaderValidation = false }) {
  if (loaderValidation && (release || withArch)) throw new Error('Loader validation cannot be a release');
  if (!['windows', 'linux'].includes(platform)
      || !(release ? releaseVersion(`v${version}`) === version : /^\d+\.\d+\.\d+-beta\.\d+$/.test(version))
      || !/^[a-f0-9]{40}$/.test(revision)) throw new Error('Invalid candidate identity');
  if (withArch && (!release || platform !== 'linux')) throw new Error('Arch packages require a Linux release');
  const formats = platform === 'windows'
    ? [['.exe', 'candidate.json']]
    : [['.deb', 'candidate.json'], ['.AppImage', 'candidate-appimage.json']];
  if (withArch) formats.push(['.pkg.tar.zst', 'candidate-arch.json', 'archlinux']);
  const checksums = new Map();
  const checksumPath = join(directory, 'SHA256SUMS');
  if (!(await lstat(checksumPath)).isFile()) throw new Error('Expected a regular checksum file');
  const sums = await readFile(checksumPath, 'utf8');
  for (const line of sums.trimEnd().split('\n')) {
    const match = /^([a-f0-9]{64})  ([^/\\\r\n]+)$/.exec(line);
    if (!match || checksums.has(match[2])) throw new Error('Invalid or duplicate checksum entry');
    checksums.set(match[2], match[1]);
  }
  if (checksums.size !== formats.length) throw new Error('Incomplete or unexpected checksum list');
  const verified = [];
  for (const [extension, manifestName, distribution = platform] of formats) {
    const name = `panita-client_${version}_${distribution}_x64_unsigned${loaderValidation ? '_loader-validation' : ''}${extension}`;
    const artifact = join(directory, name);
    const manifestPath = join(directory, manifestName);
    if (!(await lstat(artifact)).isFile() || !(await lstat(manifestPath)).isFile()) {
      throw new Error(`Expected regular candidate files: ${name}`);
    }
    const candidate = JSON.parse(await readFile(manifestPath, 'utf8'));
    if (distribution === 'archlinux' && candidate.package_version !== `${archPackageVersion(version)}-1`) {
      throw new Error('Arch package version mismatch');
    }
    if (candidate.schema !== 1 || candidate.type !== (loaderValidation ? 'unsigned_loader_validation_candidate' : release ? 'unsigned_release_candidate' : 'unsigned_beta_candidate')
        || candidate.version !== version || candidate.source_revision !== revision
        || candidate.platform !== `${platform}_x64` || candidate.artifact !== name
        || candidate.signed !== false || candidate.qualification !== 'pending') {
      throw new Error(`Candidate provenance mismatch: ${manifestName}`);
    }
    if (loaderValidation && (JSON.stringify(candidate.features) !== '["loader-validation"]'
        || candidate.distribution !== 'internal_validation_only')) throw new Error('Invalid loader validation provenance');
    const hash = createHash('sha256');
    for await (const chunk of createReadStream(artifact)) hash.update(chunk);
    const digest = hash.digest('hex');
    if (digest !== candidate.sha256 || digest !== checksums.get(name)) {
      throw new Error(`Candidate checksum mismatch: ${name}`);
    }
    verified.push({ artifact: name, sha256: digest });
  }
  return verified;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const args = process.argv.slice(2);
  let platform = { win32: 'windows', linux: 'linux' }[process.platform];
  let revision;
  let releaseTag;
  let withArch = false;
  let loaderValidation = false;
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === '--with-arch') { withArch = true; continue; }
    if (args[index] === '--loader-validation') { loaderValidation = true; continue; }
    if (args[index] === '--platform') platform = args[index + 1];
    else if (args[index] === '--revision') revision = args[index + 1];
    else if (args[index] === '--release-tag') releaseTag = args[index + 1];
    else throw new Error('Supported options: --platform windows|linux, --revision <full commit SHA>, --release-tag <vVERSION>, --with-arch');
    if (!args[index + 1]) throw new Error('Missing option value');
    index += 1;
  }
  if (revision === undefined) {
    const result = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8', windowsHide: true });
    if (result.status !== 0) throw new Error('Cannot resolve source revision');
    revision = result.stdout.trim();
  }
  const { version } = releaseTag
    ? { version: releaseVersion(releaseTag) }
    : JSON.parse(await readFile(join(root, 'desktop/tauri.beta.conf.json'), 'utf8'));
  // Validate before using platform/version to construct a directory.
  if (!['windows', 'linux'].includes(platform) || (!releaseTag && !/^\d+\.\d+\.\d+-beta\.\d+$/.test(version))) {
    throw new Error('Unsupported platform or beta version');
  }
  const verified = await verifyBeta({ directory: join(root, loaderValidation ? 'target/loader-validation' : releaseTag ? 'target/releases' : 'target/beta', version, platform), platform, version, revision, release: Boolean(releaseTag), withArch, loaderValidation });
  for (const item of verified) console.log(`${item.artifact}: OK (${item.sha256})`);
  console.log('Integrity and provenance verified; installed-product qualification remains pending.');
}
