// Read-only public-service checks. Run explicitly; not an offline unit test.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';

const base = 'https://api.panitacraft.com';
const checks = [];
async function get(path, status = 200, extra = {}) {
  const response = await fetch(base + path, {
    headers: { 'User-Agent': 'PanitaClient/0.0.0 (+https://www.panitacraft.com/)', ...extra },
    signal: AbortSignal.timeout(25000),
  });
  const body = await response.text();
  assert.equal(response.status, status, path);
  checks.push({ path, status, etag: response.headers.get('etag'),
    cache_control: response.headers.get('cache-control'), body_empty: body.length === 0 });
  return { json: body ? JSON.parse(body) : null, response, body };
}
const catalog = await get('/v1/instances');
assert.equal(catalog.json.meta.count, catalog.json.data.length);
const sorted = [...catalog.json.data].sort((a, b) => a.sort_order - b.sort_order || a.id.localeCompare(b.id));
assert.deepEqual(catalog.json.data, sorted);
const etag = catalog.response.headers.get('etag');
assert.ok(etag);
assert.equal((await get('/v1/instances', 304, { 'If-None-Match': etag })).body, '');
for (const version of ['0.0.0', '999.0.0', '0.0.0-beta.1', 'invalid']) {
  const filtered = await get('/v1/instances?launcher_version=' + version);
  // Current catalog has no minimum versions, so all records must remain visible.
  assert.ok(catalog.json.data.every(item => item.min_launcher_version === null));
  assert.deepEqual(filtered.json.data, catalog.json.data);
}
const invalid = await get('/v1/instances/not-a-uuid', 400);
assert.equal(invalid.json.error.code, 'invalid_request');
const missing = await get('/v1/instances/00000000-0000-4000-8000-000000000000', 404);
assert.equal(missing.json.error.code, 'not_found');
const manifestUrl = 'https://piston-meta.mojang.com/mc/game/version_manifest_v2.json';
const manifestResponse = await fetch(manifestUrl, { signal: AbortSignal.timeout(25000) });
assert.ok(manifestResponse.ok);
const manifest = await manifestResponse.json();
const targets = [];
for (const instance of catalog.json.data.filter(item => item.source_type === 'vanilla')) {
  const detailPath = '/v1/instances/' + instance.id;
  const detail = await get(detailPath);
  assert.deepEqual(detail.json.data, instance);
  assert.equal((await get(detailPath, 304, { 'If-None-Match': detail.response.headers.get('etag') })).body, '');
  const entry = manifest.versions.find(version => version.id === instance.minecraft_version);
  assert.ok(entry, instance.minecraft_version);
  assert.equal(entry.type, 'release');
  const metadataResponse = await fetch(entry.url, { signal: AbortSignal.timeout(25000) });
  assert.ok(metadataResponse.ok);
  const bytes = Buffer.from(await metadataResponse.arrayBuffer());
  assert.equal(createHash('sha1').update(bytes).digest('hex'), entry.sha1);
  const metadata = JSON.parse(bytes);
  assert.equal(metadata.id, instance.minecraft_version);
  targets.push({ catalog_id: instance.id, slug: instance.slug, minecraft_version: metadata.id,
    metadata_url: entry.url, metadata_sha1: entry.sha1, java: metadata.javaVersion,
    client: metadata.downloads.client, asset_index: metadata.assetIndex,
    native_libraries: metadata.libraries.filter(lib => lib.natives || lib.name.includes('natives-'))
      .map(lib => ({ name: lib.name, rules: lib.rules, natives: lib.natives })),
    runtime_download_and_game_launch_verified: false });
}
assert.ok(targets.length >= 2);
const result = { checked_at: new Date().toISOString(), manifest_url: manifestUrl,
  latest_release: manifest.latest.release, catalog: catalog.json, checks, targets,
  pending: ['Known disabled UUID unavailable; cannot independently verify disabled detail behavior.',
    'All minimum launcher versions are null; exclusion threshold behavior is untested.',
    'No deliberate server failure was induced; 500 handling remains untested.',
    'Java runtime downloads, native compatibility and game launch remain unverified.'] };
await writeFile(new URL('./fixtures/readiness.json', import.meta.url), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify({ requests: checks.length, targets: targets.map(({ minecraft_version, java }) => ({ minecraft_version, java })) }));
