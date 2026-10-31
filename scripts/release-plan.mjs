import { appendFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkVersion } from './check-version.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const CHANNELS = { develop: 'beta', master: 'stable' };

/**
 * What a push to `branch` publishes. Only `develop` (beta) and `master` (stable) publish, and the
 * committed version must match the branch's channel. A stable release also advances the beta feed,
 * so beta players graduate to it.
 */
export async function releasePlan({ branch, base = root }) {
  const channel = CHANNELS[branch];
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
  const branch = process.argv[2];
  releasePlan({ branch })
    .then(plan => {
      const lines = Object.entries({ ...plan, feeds: plan.feeds.join(' ') }).map(([key, value]) => `${key}=${value}`).join('\n');
      console.log(lines);
      if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${lines}\n`);
    })
    .catch(error => { console.error(error.message); process.exit(1); });
}
