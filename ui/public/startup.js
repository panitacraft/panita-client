// The Minecraft log viewer shows nothing while it starts.
try {
  const label = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  if (typeof label === 'string' && label.startsWith('game-log-')) document.documentElement.classList.add('log-startup');
} catch { /* not running inside the desktop shell */ }

// Keep startup failures readable even when the application module cannot load.
window.addEventListener('error', event => {
  if (event.target !== document.getElementById('application-entry')) return;
  const screen = document.getElementById('startup-screen');
  const message = document.getElementById('startup-message');
  if (screen && message) {
    screen.setAttribute('role', 'alert');
    message.textContent = 'No pudimos cargar la aplicación. Vuelve a cargarla para reintentar.';
  }
}, true);
