import { spawnSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { committedVersion, writeVersion } from './version-files.mjs';
import { nextVersion, releaseOf } from './version-policy.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const usage = 'Usage: node scripts/bump-version.mjs (--next <feat|fix|major> | --release | --set <version>) [--dry-run]';

function stableTags() {
  const result = spawnSync('git', ['tag', '--list', 'v*'], { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) throw new Error('Unable to list release tags.');
  return result.stdout.split(/\r?\n/).filter(Boolean);
}

async function main(args) {
  const dryRun = args.includes('--dry-run');
  const rest = args.filter(arg => arg !== '--dry-run');
  const current = await committedVersion(root);
  let target;
  if (rest[0] === '--next' && rest.length === 2) target = nextVersion(current, rest[1], stableTags());
  else if (rest[0] === '--release' && rest.length === 1) target = releaseOf(current);
  else if (rest[0] === '--set' && rest.length === 2) target = rest[1];
  else throw new Error(usage);
  console.log(`${current} -> ${target}${dryRun ? ' (dry run)' : ''}`);
  if (!dryRun) await writeVersion(root, target);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exit(1); });
}
