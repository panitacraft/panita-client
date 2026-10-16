import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { loadCatalog } from './catalog-api';
import type { CatalogInstance, CatalogOverview } from './preview-catalog';
import { messageForError } from './errors';
import logoWhite from './assets/logo-white.svg';
import './style.css';
import { Accounts } from './accounts';

type CatalogState =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'blocked' }
  | ({ kind: 'ready' } & CatalogOverview);

const localStateText: Record<CatalogInstance['localState'], string> = {
  not_installed: 'Sin instalar', installing: 'Instalando', installed: 'Instalada',
  update_available: 'Actualización disponible', needs_repair: 'Necesita reparación',
  running: 'En ejecución', failed: 'La operación falló', cancelled: 'Operación cancelada',
};

function App() {
  const [catalog, setCatalog] = useState<CatalogState>({ kind: 'loading' });

  useEffect(() => {
    let active = true;
    loadCatalog()
      .then(snapshot => {
        if (active) setCatalog({ kind: 'ready', ...snapshot });
      })
      .catch(error => {
        if (!active) return;
        // The account file problem is explained once, in the accounts section.
        setCatalog(error === 'accounts_file_invalid' || error === 'unsafe_permissions' ? { kind: 'blocked' } : { kind: 'error', message: messageForError(error) });
      });
    return () => { active = false; };
  }, []);

  return (
    <main>
      <header>
        <span className="wordmark"><img src={logoWhite} alt="" aria-hidden="true" />Panita Client</span>
        <span className="preview-label">Vista previa de desarrollo</span>
      </header>
      <Accounts />
      <section aria-labelledby="instances-heading">
        <h1 id="instances-heading">Tus instancias</h1>
        <p>Elige una versión de Minecraft para comenzar.</p>
        {catalog.kind === 'loading' && <p role="status">Cargando instancias…</p>}
        {catalog.kind === 'blocked' && <p role="status">Las instancias estarán disponibles cuando se resuelva el problema con las cuentas.</p>}
        {catalog.kind === 'error' && (
          <div role="alert">
            <p>{catalog.message}</p>
            <button type="button" onClick={() => {
              setCatalog({ kind: 'loading' });
              loadCatalog()
                .then(snapshot => setCatalog({ kind: 'ready', ...snapshot }))
                .catch(error => setCatalog({ kind: 'error', message: messageForError(error) }));
            }}>Reintentar</button>
          </div>
        )}
        {catalog.kind === 'ready' && catalog.stale && (
          <div className="catalog-notice" role="status">No pudimos conectar con el servidor. Puedes jugar las instancias que ya tienes instaladas.</div>
        )}
        {catalog.kind === 'ready' && catalog.cacheWarning && (
          <p role="alert">Cargamos el catálogo, pero no pudimos guardar una copia local. {messageForError(catalog.cacheWarning)}</p>
        )}
        {catalog.kind === 'ready' && catalog.registryWarning && (
          <p role="alert">No pudimos guardar el estado actualizado de tus instancias. {messageForError(catalog.registryWarning)}</p>
        )}
        {catalog.kind === 'ready' && catalog.instances.length === 0 && (
          <p role="status">No hay instancias disponibles por ahora.</p>
        )}
        {catalog.kind === 'ready' && catalog.instances.length > 0 && (
          <div className="instance-grid">
            {catalog.instances.map(instance => (
              <article className={`instance-card${instance.availability === 'disabled' || !instance.compatible || !instance.supported ? ' disabled' : ''}`} key={instance.id}>
                <div className="card-heading">
                  <h2>{instance.name}</h2>
                  {instance.availability === 'disabled' && <span><span aria-hidden="true">🔒 </span>Instancia deshabilitada</span>}
                </div>
                <p>{instance.description}</p>
                {instance.minecraftVersion && <p className="version">Minecraft {instance.minecraftVersion}</p>}
                <p>{localStateText[instance.localState]}</p>
                {instance.installedVersion && <p>Versión instalada: {instance.installedVersion}</p>}
                {instance.availability === 'missing' && <p>Esta instancia ya no está en el catálogo. Conservamos tus datos locales.</p>}
                {instance.availability === 'unverified' && <p>No pudimos verificar el estado de esta instancia.</p>}
                {!instance.compatible && <p>Necesitas Panita Client {instance.minLauncherVersion} o una versión posterior.</p>}
                {!instance.supported && <p>Este tipo de instancia o su acceso todavía no es compatible.</p>}
                <button type="button" disabled>Disponible próximamente</button>
              </article>
            ))}
          </div>
        )}
      </section>
    </main>
  );
}

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
