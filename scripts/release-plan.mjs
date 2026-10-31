import { appendFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkVersion } from './check-version.mjs';
import { committedVersion } from './version-files.mjs';
import { parseVersion } from './version-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const CHANNELS = { develop: 'beta', master: 'stable' };

/**
 * What a push to `branch` publishes. Only `develop` (beta) and `master` (stable) publish, and the
 * committed version must match the branch's channel. A stable release also advances the beta feed,
 * so beta players graduate to it. A dry run may use any branch: it takes the channel from the
 * committed version, and the workflow publishes nothing public in that mode.
 */
export async function releasePlan({ branch, base = root, dryRun = false }) {
  let channel = CHANNELS[branch];
  if (!channel && dryRun) channel = parseVersion(await committedVersion(base)).beta === null ? 'stable' : 'beta';
  if (!channel) throw new Error(`Branch ${branch} does not publish releases.`);
  const version = await checkVersion({ channel, base });
  return {
    version,
    tag: `v${version}`,
    channel,
    prerelease: channel === 'beta',
    feeds: channel === 'stable' ? ['updater-stable', 'updater-beta'] : ['updater-beta'],
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const dryRun = args.includes('--dry-run');
  const branch = args.find(arg => arg !== '--dry-run');
  releasePlan({ branch, dryRun })
    .then(plan => {
      const lines = Object.entries({ ...plan, feeds: plan.feeds.join(' '), dry_run: dryRun }).map(([key, value]) => `${key}=${value}`).join('\n');
      console.log(lines);
      if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${lines}\n`);
    })
    .catch(error => { console.error(error.message); process.exit(1); });
}
