import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const routes = new Map([
  ['/', ['../desktop/windows/installer-preview.html', 'text/html; charset=utf-8']],
  ['/installer.png', ['../desktop/icons/installer.png', 'image/png']],
  ['/uninstaller.png', ['../desktop/icons/uninstaller.png', 'image/png']],
  ['/installer.ico', ['../desktop/icons/installer.ico', 'image/x-icon']],
  ['/uninstaller.ico', ['../desktop/icons/uninstaller.ico', 'image/x-icon']],
  ['/logo.svg', ['../ui/src/assets/logo_background.svg', 'image/svg+xml']],
]);
const server = createServer(async (request, response) => {
  const route = routes.get(request.url);
  if (!route || request.method !== 'GET') { response.writeHead(404).end(); return; }
  try {
    const body = await readFile(fileURLToPath(new URL(route[0], import.meta.url)));
    response.writeHead(200, {
      'Content-Type': route[1],
      'Cache-Control': 'no-store',
      'Content-Security-Policy': "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; frame-ancestors 'none'",
    }).end(body);
  } catch {
    response.writeHead(500).end('Preview unavailable.');
  }
});
const port = Number(process.argv[2] ?? 1431);
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error('Choose a port between 1024 and 65535.');
server.listen(port, '127.0.0.1', () => console.log(`Installer design preview: http://127.0.0.1:${port}`));
server.on('error', error => { console.error(error.message); process.exitCode = 1; });
