import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { releaseVersion } from './release-version.mjs';
import { verifyBeta } from './verify-beta.mjs';

test('release tags accept stable and prerelease versions and reject unsafe or invalid identities', () => {
  for (const version of ['0.1.0', '0.1.0-alpha', '0.1.0-beta.1', '1.0.0-rc.2']) {
    assert.equal(releaseVersion(`v${version}`), version);
  }
  for (const tag of [undefined, '0.1.0-alpha', 'v01.1.0', 'v1.0', 'v1.0.0-alpha.01', 'v1.0.0+build', 'v../../file', 'v1.0.0\n']) {
    assert.throws(() => releaseVersion(tag));
  }
});

for (const platform of ['windows', 'linux']) {
  for (const release of [true, false]) {
    test(`${platform} ${release ? 'release' : 'beta'} verification rejects corruption and incorrect provenance`, async () => {
      const directory = await mkdtemp(join(tmpdir(), 'panita-release-test-'));
      try {
        const version = release ? '0.1.0-alpha' : '0.1.1-beta.8';
        const revision = 'a'.repeat(40);
        const formats = platform === 'windows'
          ? [['.exe', 'candidate.json']]
          : [['.deb', 'candidate.json'], ['.AppImage', 'candidate-appimage.json']];
        const sums = [];
        for (const [extension, manifest] of formats) {
          const artifact = `panita-client_${version}_${platform}_x64_unsigned${extension}`;
          const bytes = `test installer ${extension}`;
          const sha256 = createHash('sha256').update(bytes).digest('hex');
          await writeFile(join(directory, artifact), bytes);
          await writeFile(join(directory, manifest), JSON.stringify({
            schema: 1, type: release ? 'unsigned_release_candidate' : 'unsigned_beta_candidate', version, source_revision: revision,
            platform: `${platform}_x64`, artifact, sha256, signed: false, qualification: 'pending',
          }));
          sums.push(`${sha256}  ${artifact}`);
        }
        await writeFile(join(directory, 'SHA256SUMS'), `${sums.join('\n')}\n`);
        const identity = { directory, platform, version, revision, release };
        assert.equal((await verifyBeta(identity)).length, formats.length);
        await assert.rejects(verifyBeta({ ...identity, revision: 'b'.repeat(40) }), /provenance mismatch/);
        await writeFile(join(directory, `panita-client_${version}_${platform}_x64_unsigned${formats[0][0]}`), 'corrupt');
        await assert.rejects(verifyBeta(identity), /checksum mismatch/);
      } finally {
        await rm(directory, { recursive: true, force: true });
      }
    });
  }
}
