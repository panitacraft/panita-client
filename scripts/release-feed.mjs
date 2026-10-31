import { appendFileSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { releaseVersion } from './release-version.mjs';
import { compareVersions } from './version-policy.mjs';

export const REPOSITORY = 'panitacraft/panita-client';

// The platform keys the updater plugin looks up: `{os}-{arch}-{package}` first, then `{os}-{arch}`.
const PACKAGES = [
  { key: 'windows-x86_64', file: version => `panita-client_${version}_windows_x64_unsigned.exe` },
  { key: 'linux-x86_64-appimage', file: version => `panita-client_${version}_linux_x64_unsigned.AppImage` },
  { key: 'linux-x86_64-deb', file: version => `panita-client_${version}_linux_x64_unsigned.deb` },
];

export const packageFiles = version => PACKAGES.map(item => ({ key: item.key, file: item.file(version) }));

/** The static update feed for one release. `signatures` maps each platform key to its .sig text. */
export function buildFeed({ version, signatures, pubDate = new Date().toISOString(), repository = REPOSITORY }) {
  releaseVersion(`v${version}`);
  const platforms = {};
  for (const { key, file } of packageFiles(version)) {
    const signature = signatures[key]?.trim();
    if (!signature) throw new Error(`Missing updater signature for ${key}.`);
    platforms[key] = { signature, url: `https://github.com/${repository}/releases/download/v${version}/${file}` };
  }
  return {
    version,
    notes: `Panita Client ${version}: https://github.com/${repository}/releases/tag/v${version}`,
    pub_date: pubDate,
    platforms,
  };
}

/** The version a feed currently offers, or null when it is absent or unreadable. */
export function feedVersion(feed) {
  try {
    const version = feed?.version;
    return typeof version === 'string' && releaseVersion(`v${version}`) === version ? version : null;
  } catch {
    return null;
  }
}

/** A feed only moves forward: an out-of-order or repeated run never regresses what players see. */
export function shouldAdvance(current, next) {
  const existing = feedVersion(current);
  return existing === null || compareVersions(next, existing) > 0;
}

async function main(args) {
  const option = name => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined);
  const version = option('--version');
  const directory = option('--dir');
  const out = option('--out');
  const currentPath = option('--current');
  if (!version || !directory || !out) throw new Error('Usage: node scripts/release-feed.mjs --version <V> --dir <assets> --out <latest.json> [--current <latest.json>]');
  let current = null;
  if (currentPath) current = await readFile(currentPath, 'utf8').then(JSON.parse).catch(() => null);
  const advance = shouldAdvance(current, version);
  if (advance) {
    const signatures = {};
    for (const { key, file } of packageFiles(version)) signatures[key] = await readFile(join(directory, `${file}.sig`), 'utf8');
    await writeFile(out, `${JSON.stringify(buildFeed({ version, signatures }), null, 2)}\n`);
  }
  const line = `advance=${advance}`;
  console.log(line);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${line}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).catch(error => { console.error(error.message); process.exit(1); });
}
