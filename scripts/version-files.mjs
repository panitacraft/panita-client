import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { releaseVersion } from './release-version.mjs';

const VERSION = String.raw`(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)`;

// Each target replaces exactly one occurrence, so formatting and line endings are untouched.
export const versionTargets = [
  { file: 'package.json', pattern: new RegExp(String.raw`("version": ")${VERSION}(")`) },
  { file: 'ui/package.json', pattern: new RegExp(String.raw`("version": ")${VERSION}(")`) },
  { file: 'desktop/tauri.conf.json', pattern: new RegExp(String.raw`("version": ")${VERSION}(")`) },
  { file: 'desktop/tauri.beta.conf.json', pattern: new RegExp(String.raw`("version": ")${VERSION}(")`) },
  { file: 'desktop/Cargo.toml', pattern: new RegExp(String.raw`(^version = ")${VERSION}(")`, 'm') },
  { file: 'crates/launcher-core/Cargo.toml', pattern: new RegExp(String.raw`(^version = ")${VERSION}(")`, 'm') },
  { file: 'Cargo.lock', pattern: new RegExp(String.raw`(name = "launcher-core"\r?\nversion = ")${VERSION}(")`) },
  { file: 'Cargo.lock', pattern: new RegExp(String.raw`(name = "panita-desktop"\r?\nversion = ")${VERSION}(")`) },
  { file: 'desktop/windows/installer-preview.html', pattern: new RegExp(String.raw`(<span class="version">v)${VERSION}(</span>)`) },
];

/** Returns every target with the version it currently declares. */
export async function readVersions(root) {
  const found = [];
  for (const target of versionTargets) {
    const text = await readFile(join(root, target.file), 'utf8');
    const match = target.pattern.exec(text);
    if (!match) throw new Error(`No version found in ${target.file} (${target.pattern.source.slice(0, 40)}).`);
    found.push({ file: target.file, version: match[2] });
  }
  return found;
}

/** The single committed version, or an error naming every file that disagrees. */
export async function committedVersion(root) {
  const found = await readVersions(root);
  const versions = new Set(found.map(entry => entry.version));
  if (versions.size !== 1) {
    throw new Error(`Version files disagree:\n${found.map(entry => `  ${entry.file}: ${entry.version}`).join('\n')}`);
  }
  return found[0].version;
}

/** Writes one version into every target. Validates before touching any file. */
export async function writeVersion(root, version) {
  releaseVersion(`v${version}`);
  const edits = new Map();
  for (const target of versionTargets) {
    const text = edits.get(target.file) ?? await readFile(join(root, target.file), 'utf8');
    if (!target.pattern.test(text)) throw new Error(`No version found in ${target.file}.`);
    edits.set(target.file, text.replace(target.pattern, (_, before, _old, after) => `${before}${version}${after}`));
  }
  for (const [file, text] of edits) await writeFile(join(root, file), text);
}
