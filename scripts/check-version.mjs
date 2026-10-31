import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { releaseVersion } from './release-version.mjs';
import { committedVersion } from './version-files.mjs';
import { parseVersion } from './version-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const usage = 'Usage: node scripts/check-version.mjs [--tag <vVERSION>] [--channel <beta|stable>]';

/** Throws unless every version file agrees and satisfies the optional tag and channel. */
export async function checkVersion({ tag, channel, base = root } = {}) {
  const version = await committedVersion(base);
  if (tag !== undefined && releaseVersion(tag) !== version) {
    throw new Error(`Release tag ${tag} does not match the committed version ${version}.`);
  }
  if (channel !== undefined) {
    const { beta } = parseVersion(version);
    if (channel === 'beta' && beta === null) throw new Error(`The beta channel requires an X.Y.Z-beta.N version, found ${version}.`);
    if (channel === 'stable' && beta !== null) throw new Error(`The stable channel requires an X.Y.Z version, found ${version}.`);
    if (channel !== 'beta' && channel !== 'stable') throw new Error(usage);
  }
  return version;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const option = name => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
  checkVersion({ tag: option('--tag'), channel: option('--channel') })
    .then(version => console.log(version))
    .catch(error => { console.error(error.message); process.exit(1); });
}
