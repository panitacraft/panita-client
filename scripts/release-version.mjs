import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';

export function releaseVersion(tag) {
  const number = '(?:0|[1-9][0-9]*)';
  const identifier = '(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)';
  const pattern = new RegExp(`^v(${number}\\.${number}\\.${number}(?:-${identifier}(?:\\.${identifier})*)?)$`);
  const match = typeof tag === 'string' && pattern.exec(tag);
  if (!match) throw new Error('Expected a release tag such as v0.1.0, v0.1.0-alpha or v0.1.0-beta.1 (without build metadata).');
  return match[1];
}

export function archPackageVersion(version) {
  releaseVersion(`v${version}`);
  // Arch forbids hyphens. Attach 'pre' to the patch number so prereleases sort
  // before the matching stable version in pacman, including numeric identifiers.
  const separator = version.indexOf('-');
  return separator === -1 ? version
    : `${version.slice(0, separator)}pre.${version.slice(separator + 1).replaceAll('-', '_')}`;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(releaseVersion(process.argv[2]));
}
