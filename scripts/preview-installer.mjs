import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const routes = new Map([
  ['/', ['../desktop/windows/installer-preview.html', 'text/html; charset=utf-8']],
  ['/logo.svg', ['../ui/src/assets/logo-white.svg', 'image/svg+xml']],
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
server.listen(1431, '127.0.0.1', () => console.log('Installer design preview: http://127.0.0.1:1431'));
server.on('error', error => { console.error(error.message); process.exitCode = 1; });
