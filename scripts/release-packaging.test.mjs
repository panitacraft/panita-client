import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { archPackageVersion, releaseVersion } from './release-version.mjs';
import { verifyBeta } from './verify-beta.mjs';

test('release tags accept stable and prerelease versions and reject unsafe or invalid identities', () => {
  for (const version of ['0.1.0', '0.1.0-alpha', '0.1.0-beta.1', '1.0.0-rc.2']) {
    assert.equal(releaseVersion(`v${version}`), version);
  }
  for (const tag of [undefined, '0.1.0-alpha', 'v01.1.0', 'v1.0', 'v1.0.0-alpha.01', 'v1.0.0+build', 'v../../file', 'v1.0.0\n']) {
    assert.throws(() => releaseVersion(tag));
  }
});

test('Arch package versions preserve stable versions and map prereleases without hyphens', () => {
  assert.equal(archPackageVersion('0.1.0'), '0.1.0');
  assert.equal(archPackageVersion('0.1.0-alpha'), '0.1.0pre.alpha');
  assert.equal(archPackageVersion('0.1.0-beta.1'), '0.1.0pre.beta.1');
  assert.equal(archPackageVersion('0.1.0-1'), '0.1.0pre.1');
  assert.equal(archPackageVersion('0.1.0-alpha-test.2'), '0.1.0pre.alpha_test.2');
  assert.throws(() => archPackageVersion('../escape'));
});

for (const platform of ['windows', 'linux']) {
  for (const release of [true, false]) {
    for (const withArch of platform === 'linux' && release ? [false, true] : [false]) {
      test(`${platform} ${release ? 'release' : 'beta'}${withArch ? ' with Arch package' : ''} verification rejects corruption and incorrect provenance`, async () => {
        const directory = await mkdtemp(join(tmpdir(), 'panita-release-test-'));
        try {
          const version = release ? '0.1.0-alpha' : '0.1.1-beta.8';
          const revision = 'a'.repeat(40);
          const formats = platform === 'windows'
            ? [['.exe', 'candidate.json']]
            : [['.deb', 'candidate.json'], ['.AppImage', 'candidate-appimage.json']];
          if (withArch) formats.push(['.pkg.tar.zst', 'candidate-arch.json', 'archlinux']);
          const sums = [];
          for (const [extension, manifest, distribution = platform] of formats) {
            const artifact = `panita-client_${version}_${distribution}_x64_unsigned${extension}`;
            const bytes = `test installer ${extension}`;
            const sha256 = createHash('sha256').update(bytes).digest('hex');
            await writeFile(join(directory, artifact), bytes);
            await writeFile(join(directory, manifest), JSON.stringify({
              schema: 1, type: release ? 'unsigned_release_candidate' : 'unsigned_beta_candidate', version, source_revision: revision,
              platform: `${platform}_x64`, artifact, sha256, signed: false, qualification: 'pending',
              ...(distribution === 'archlinux' ? { package_version: `${archPackageVersion(version)}-1` } : {}),
            }));
            sums.push(`${sha256}  ${artifact}`);
          }
          await writeFile(join(directory, 'SHA256SUMS'), `${sums.join('\n')}\n`);
          const identity = { directory, platform, version, revision, release, withArch };
          assert.equal((await verifyBeta(identity)).length, formats.length);
          await assert.rejects(verifyBeta({ ...identity, revision: 'b'.repeat(40) }), /provenance mismatch/);
          if (withArch) {
            await assert.rejects(verifyBeta({ ...identity, withArch: false }), /checksum list/);
            const manifestPath = join(directory, 'candidate-arch.json');
            const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
            await writeFile(manifestPath, JSON.stringify({ ...manifest, package_version: '0.1.0-1' }));
            await assert.rejects(verifyBeta(identity), /Arch package version mismatch/);
            await writeFile(manifestPath, JSON.stringify(manifest));
            const archivePath = join(directory, `panita-client_${version}_archlinux_x64_unsigned.pkg.tar.zst`);
            await writeFile(archivePath, 'corrupt');
            await assert.rejects(verifyBeta(identity), /checksum mismatch/);
            await rm(archivePath);
            await assert.rejects(verifyBeta(identity));
            return;
          }
          await writeFile(join(directory, `panita-client_${version}_${platform}_x64_unsigned${formats[0][0]}`), 'corrupt');
          await assert.rejects(verifyBeta(identity), /checksum mismatch/);
        } finally {
          await rm(directory, { recursive: true, force: true });
        }
      });
    }
  }
}
