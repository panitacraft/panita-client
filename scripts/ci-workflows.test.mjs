import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const workflows = [
  '.github/workflows/checks.yml',
  '.github/workflows/release.yml',
];

test('Windows CI prepares a private temporary root before testing credentials', async () => {
  for (const path of workflows) {
    const workflow = await readFile(path, 'utf8');
    const prepare = workflow.indexOf('- name: Prepare private Windows test directory');
    const tests = workflow.indexOf('run: cargo test --workspace --locked', prepare);

    assert.notEqual(prepare, -1, path);
    assert.notEqual(tests, -1, path);
    assert.ok(prepare < tests, path);
    assert.match(workflow, /\$env:RUNNER_TEMP 'panita-tests'/);
    assert.match(workflow, /icacls\.exe \$testTemp '\/inheritance:r' '\/grant:r'/);
    assert.match(workflow, /"TEMP=\$testTemp" \| Out-File -FilePath \$env:GITHUB_ENV/);
    assert.match(workflow, /"TMP=\$testTemp" \| Out-File -FilePath \$env:GITHUB_ENV/);
  }
});

test('CI tests the patched filesystem dependency on both operating systems', async () => {
  for (const path of workflows) {
    const workflow = await readFile(path, 'utf8');
    const command = 'cargo test --manifest-path vendor/private-fs/Cargo.toml --lib --locked';

    assert.match(
      workflow,
      new RegExp(`if: runner\\.os == 'Windows'[\\s\\S]*?run: ${command.replaceAll('/', '\\/')}`),
      path,
    );
    assert.match(
      workflow,
      new RegExp(`if: runner\\.os == 'Linux'[\\s\\S]*?${command.replaceAll('/', '\\/')}`),
      path,
    );
  }
});

test('PR CI keeps every Rust build output under the ignored root target directory', async () => {
  const workflow = await readFile('.github/workflows/checks.yml', 'utf8');

  assert.match(
    workflow,
    /env:\r?\n\s+# Keep standalone manifest builds[\s\S]*?CARGO_TARGET_DIR: \$\{\{ github\.workspace \}\}\/target\/checks/,
  );
  assert.doesNotMatch(workflow, /vendor\/private-fs\/target/);
});

test('Linux release packaging runs the candidate regression inspection', async () => {
  const workflow = await readFile('.github/workflows/release.yml', 'utf8');
  assert.match(
    workflow,
    /if: runner\.os == 'Linux'\r?\n        run: bash scripts\/linux\/verify-candidate\.sh/,
  );
});

test('Ubuntu PR and develop CI build and inspect real unsigned Linux packages', async () => {
  const workflow = await readFile('.github/workflows/checks.yml', 'utf8');
  const builder = await readFile('scripts/build-beta.mjs', 'utf8');
  const inspection = await readFile('scripts/linux/verify-candidate.sh', 'utf8');
  const build = workflow.indexOf('- name: Build and inspect real Linux packages');

  assert.notEqual(build, -1);
  assert.match(
    workflow.slice(build),
    /if: runner\.os == 'Linux' && \(github\.event_name == 'pull_request' \|\| github\.ref == 'refs\/heads\/develop'\)/,
  );
  assert.match(workflow.slice(build), /node scripts\/build-beta\.mjs --ci-unsigned/);
  assert.match(workflow.slice(build), /CI_UNSIGNED=1 bash scripts\/linux\/verify-candidate\.sh/);
  assert.match(workflow, /patchelf desktop-file-utils xdg-utils/);
  assert.match(builder, /loaderValidation \|\| ciUnsigned \? \['--no-sign'\]/);
  assert.match(builder, /if \(!loaderValidation && !ciUnsigned\)/);
  assert.match(builder, /installer candidate:\\n\$\{dirty\}/);
  // A transient bundler download failure (a 403 from GitHub) must not fail the whole check.
  assert.match(builder, /runWithRetry\(process\.execPath, \[join\(root, 'scripts\/tauri\.mjs'\), 'build'/);
  assert.match(inspection, /artifact="\$root\/target\/ci\/\$version\/linux"/);
});
