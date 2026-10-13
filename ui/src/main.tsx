import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { loadCatalog } from './catalog-api';
import type { CatalogInstance } from './preview-catalog';
import { messageForError } from './errors';
import logoWhite from './assets/logo-white.svg';
import './style.css';
import { Accounts } from './accounts';

type CatalogState =
  | { kind: 'loading' }
  | { kind: 'error'; message: string }
  | { kind: 'blocked' }
  | { kind: 'ready'; instances: CatalogInstance[] };

function App() {
  const [catalog, setCatalog] = useState<CatalogState>({ kind: 'loading' });

  useEffect(() => {
    let active = true;
    loadCatalog()
      .then(instances => {
        if (active) setCatalog({ kind: 'ready', instances });
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
                .then(instances => setCatalog({ kind: 'ready', instances }))
                .catch(error => setCatalog({ kind: 'error', message: messageForError(error) }));
            }}>Reintentar</button>
          </div>
        )}
        {catalog.kind === 'ready' && catalog.instances.length === 0 && (
          <p role="status">No hay instancias disponibles por ahora.</p>
        )}
        {catalog.kind === 'ready' && catalog.instances.length > 0 && (
          <div className="instance-grid">
            {catalog.instances.map(instance => (
              <article className={`instance-card${instance.enabled ? '' : ' disabled'}`} key={instance.id}>
                <div className="card-heading">
                  <h2>{instance.name}</h2>
                  {!instance.enabled && <span aria-label="Instancia deshabilitada">🔒 Instancia deshabilitada</span>}
                </div>
                <p>{instance.description}</p>
                <p className="version">Minecraft {instance.minecraftVersion}</p>
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
