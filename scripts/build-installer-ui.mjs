import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function buildInstallerUi() {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const output = join(root, 'target/installer-ui');
  mkdirSync(output, { recursive: true });
  const query = spawnSync(join(process.env['ProgramFiles(x86)'], 'Microsoft Visual Studio/Installer/vswhere.exe'),
    ['-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'],
    { encoding: 'utf8', windowsHide: true });
  if (query.status !== 0 || !query.stdout.trim()) throw new Error('MSVC x86 tools are required for the native installer UI.');
  // NSIS is a Unicode x86 process even when packaging the x64 launcher.
  const script = join(output, 'build.cmd');
  writeFileSync(script, `@echo off\r\ncall "${join(query.stdout.trim(), 'VC/Auxiliary/Build/vcvarsall.bat')}" x64_x86 >nul\r\nif errorlevel 1 exit /b 1\r\ncl /nologo /LD /MT /std:c++17 /EHsc /O2 /W4 /WX /guard:cf /D_WIN32_WINNT=0x0A00 "${join(root, 'desktop/windows/installer-ui.cpp')}" /Fo"${join(output, 'installer-ui.obj')}" /link /OUT:"${join(output, 'PanitaUi.dll')}" /IMPLIB:"${join(output, 'PanitaUi.lib')}" /DYNAMICBASE /NXCOMPAT user32.lib gdi32.lib comctl32.lib gdiplus.lib\r\n`);
  const result = spawnSync('cmd.exe', ['/d', '/c', script], { stdio: 'inherit', windowsHide: true });
  if (result.error || result.status !== 0) throw new Error('Native installer UI compilation failed.');
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) buildInstallerUi();
