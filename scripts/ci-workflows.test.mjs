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

test('Linux release packaging runs the candidate regression inspection', async () => {
  const workflow = await readFile('.github/workflows/release.yml', 'utf8');
  assert.match(
    workflow,
    /if: runner\.os == 'Linux'\r?\n        run: bash scripts\/linux\/verify-candidate\.sh/,
  );
});
