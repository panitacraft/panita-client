import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { buildInstallerUi } from './build-installer-ui.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const executable = resolve(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js');
if (process.platform === 'win32' && ['build', 'bundle'].includes(process.argv[2])) buildInstallerUi();
const child = spawn(process.execPath, [executable, ...process.argv.slice(2)], {
  cwd: root,
  env: {
    ...process.env,
    TAURI_APP_PATH: resolve(root, 'desktop'),
    TAURI_FRONTEND_PATH: resolve(root, 'ui'),
  },
  stdio: 'inherit',
});

child.on('error', error => {
  process.stderr.write(`Tauri CLI failed to start: ${error.message}\n`);
  process.exitCode = 1;
});
child.on('exit', code => {
  process.exitCode = code ?? 1;
});
