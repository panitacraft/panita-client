import { releaseVersion } from './release-version.mjs';

export function parseVersion(version) {
  releaseVersion(`v${version}`);
  const [base, ...rest] = version.split('-');
  const [major, minor, patch] = base.split('.').map(Number);
  const prerelease = rest.length ? rest.join('-') : null;
  const beta = prerelease === null ? null : /^beta\.(0|[1-9]\d*)$/.exec(prerelease);
  if (prerelease !== null && !beta) throw new Error(`Expected a beta or stable version, found ${version}.`);
  return { base: [major, minor, patch], beta: beta ? Number(beta[1]) : null };
}

export function compareBase(left, right) {
  for (let index = 0; index < 3; index += 1) if (left[index] !== right[index]) return left[index] - right[index];
  return 0;
}

const format = ({ base, beta }) => `${base.join('.')}${beta === null ? '' : `-beta.${beta}`}`;

function raise([major, minor, patch], magnitude) {
  if (magnitude === 'major') return [major + 1, 0, 0];
  if (magnitude === 'feat') return [major, minor + 1, 0];
  if (magnitude === 'fix') return [major, minor, patch + 1];
  throw new Error('Expected a magnitude of major, feat or fix.');
}

/** The highest stable (suffix-free) version among tags such as v1.2.3, or 0.0.0. */
export function lastStable(tags) {
  const stable = tags.map(tag => /^v(\d+\.\d+\.\d+)$/.exec(tag)).filter(Boolean).map(match => match[1].split('.').map(Number));
  return stable.sort(compareBase).at(-1) ?? [0, 0, 0];
}

/**
 * The next develop version for a change of the given magnitude. The minimum base is
 * the last stable release (or the committed stable version) raised by the magnitude.
 * A beta whose base already satisfies it only increments its number.
 */
export function nextVersion(current, magnitude, stableTags) {
  const state = parseVersion(current);
  const reference = state.beta === null && compareBase(state.base, lastStable(stableTags)) > 0 ? state.base : lastStable(stableTags);
  const minimum = raise(reference, magnitude);
  if (state.beta !== null && compareBase(state.base, minimum) >= 0) return format({ base: state.base, beta: state.beta + 1 });
  return format({ base: minimum, beta: 1 });
}

/** Promotion to stable removes the beta suffix. */
export function releaseOf(current) {
  const state = parseVersion(current);
  if (state.beta === null) throw new Error(`${current} is already a stable version.`);
  return format({ base: state.base, beta: null });
}
