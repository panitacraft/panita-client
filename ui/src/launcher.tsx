import { useCallback, useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { LoaderCircle, ArrowLeft, Blocks, ChevronUp, ChevronDown, Gamepad2, LockKeyhole, RefreshCw, Settings2, Users, WifiOff } from 'lucide-react';
import { loadCatalog } from './catalog-api';
import type { CatalogOverview } from './preview-catalog';
import { messageForError } from './errors';
import logoBackground from './assets/logo_background.svg';
import { useInstanceRail } from './use-instance-rail';
import { Accounts } from './accounts';
import type { AccountsView } from './account-state';
import { ActivationPanel, useActivation } from './activation';
import { PlayControls, PlayNotice, usePlaySnapshot } from './play';
import { installationRefreshKey } from './play-state';
import { canInstall, canOpenInstanceOptions, canRepair, canUninstall, isPack, updateTarget } from './library-state';
import { SettingsPanel, usePreferences } from './settings';
import { DiagnosticsPanel } from './diagnostics';
import { CachePanel, PackPanel } from './pack';
import { CrashNotifications } from './game-output';
import { instanceStatus, visibleSelection, type LauncherPage } from './launcher-state';
import { CatalogArtwork } from './components/catalog-artwork';
import { Button } from './components/ui/button';
import { AccountAvatar } from './components/account-avatar';
import { Hint } from './components/hint';
import { TooltipProvider } from './components/ui/tooltip';
import { WindowDragRegion } from './components/window-drag-region';
import { WindowControls } from './components/window-controls';
import { InstanceBadges } from './components/instance-badges';

function UtilityFooter() {
  return <footer className="utility-footer"><p>© {new Date().getFullYear()} Panitacraft. Todos los derechos reservados.</p><p>Hecho por Javithor &amp; Aerø</p></footer>;
}

type CatalogState = { kind: 'loading' } | { kind: 'error'; message: string } | { kind: 'blocked' } | ({ kind: 'ready' } & CatalogOverview);

export function Launcher() {
  const [catalog, setCatalog] = useState<CatalogState>({ kind: 'loading' });
  const activation = useActivation();
  const unverified = activation?.view?.state === 'active_offline';
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const [catalogAttempt, setCatalogAttempt] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [page, setPage] = useState<LauncherPage>('library');
  const [cacheRequest, setCacheRequest] = useState(0);
  const [settingsScope, setSettingsScope] = useState<'global' | 'instance'>('global');
  const [settingsTab, setSettingsTab] = useState<'game' | 'storage' | 'diagnostics'>('game');
  const [accountView, setAccountView] = useState<AccountsView | null>(null);
  const [instancePage, setInstancePage] = useState(0);
  const [version, setVersion] = useState(__APP_VERSION__);
  const acceptAccounts = useCallback((next: AccountsView | null) => {
    setAccountView(previous => previous && next && previous.revision > next.revision ? previous : next);
  }, []);
  const playSnapshot = usePlaySnapshot();
  const installationChanges = installationRefreshKey(playSnapshot);
  const liveEntry = (id: string) => playSnapshot?.instances.find(entry => entry.instanceId === id);
  const preferences = usePreferences();
  const instances = catalog.kind === 'ready' ? catalog.instances : [];
  const selectedId = visibleSelection(instances, preferences?.id ?? null);
  const selected = instances.find(instance => instance.id === selectedId);
  const selectedAccount = accountView?.accounts.find(account => account.metadata.account_id === accountView.selected_account_id);
  const { rail, capacity } = useInstanceRail(instances.length);
  const pageCount = Math.ceil(instances.length / capacity);
  const visiblePage = Math.min(instancePage, Math.max(0, pageCount - 1));
  const visibleInstances = instances.slice(visiblePage * capacity, visiblePage * capacity + capacity);
  const instanceOrder = instances.map(instance => instance.id).join(',');
  useEffect(() => {
    const index = instances.findIndex(instance => instance.id === selectedId);
    setInstancePage(Math.max(0, Math.floor(index / capacity)));
  }, [selectedId, instanceOrder, capacity]);

  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    void getVersion().then(value => { if (active) setVersion(value); }).catch(() => {});
    return () => { active = false; };
  }, []);

  useEffect(() => {
    let active = true;
    setRefreshing(true); setRefreshError(null);
    loadCatalog().then(snapshot => { if (active) setCatalog({ kind: 'ready', ...snapshot }); })
      .catch(error => {
        if (!active) return;
        setRefreshError(messageForError(error));
        setCatalog(previous => previous.kind === 'ready' ? previous : error === 'accounts_file_invalid' || error === 'unsafe_permissions' ? { kind: 'blocked' } : { kind: 'error', message: messageForError(error) });
      }).finally(() => { if (active) setRefreshing(false); });
    return () => { active = false; };
  }, [installationChanges, catalogAttempt]);

  function openSettings(scope: 'global' | 'instance') {
    setSettingsScope(scope); setSettingsTab('game'); setPage('settings');
  }

  return <TooltipProvider delayDuration={250}><main className="launcher-shell">
    <a className="skip-link" href="#launcher-content">Saltar al contenido</a>
    <aside className="instance-rail" aria-label="Navegación de Panita">
      <div className="rail-header"><Button variant="ghost" className="rail-logo size-14 p-0 hover:bg-transparent hover:text-foreground" aria-label="Inicio: tus instancias" onClick={() => setPage('library')}>
        <img src={logoBackground} alt="" aria-hidden="true" />
      </Button></div>
      <nav ref={rail} className="rail-browser" aria-label="Instancias">
        {pageCount > 1 && <Hint text="Instancias anteriores" side="right"><Button variant="ghost" className="rail-nav-control size-6 bg-transparent p-0" aria-label="Instancias anteriores" disabled={visiblePage === 0} onClick={() => setInstancePage(visiblePage - 1)}><ChevronUp aria-hidden="true" /></Button></Hint>}
        <div className="rail-instances">
        {visibleInstances.map(instance => <Hint key={instance.id} side="right" sideOffset={20} text={<div className="instance-hint">
          <strong>{instance.name}</strong>
          <p>{instance.description}</p>
          <div className="instance-tags"><InstanceBadges instance={instance} updateAvailable={instance.localState === 'update_available'} />
            {!['Sin instalar', 'Lista para jugar', 'Actualización disponible'].includes(instanceStatus(instance, liveEntry(instance.id))) && <span className={`instance-status ${instance.availability === 'disabled' ? 'status-warning' : ''}`}>{instance.availability === 'disabled' && <LockKeyhole aria-hidden="true" />}{['Instalando', 'Reparando', 'Actualizando'].includes(instanceStatus(instance, liveEntry(instance.id))) && <LoaderCircle className="spin" aria-hidden="true" />}{instanceStatus(instance, liveEntry(instance.id))}</span>}
          </div>
        </div>}><Button variant="ghost"
          className={`rail-instance size-13 p-0 ${instance.id === selectedId && page === 'library' ? 'is-selected' : ''} ${instance.availability === 'disabled' ? 'is-disabled' : ''}`}
          aria-label={`${instance.name}: ${instanceStatus(instance, liveEntry(instance.id))}`}
          aria-current={instance.id === selectedId && page === 'library' ? 'page' : undefined}
          disabled={!preferences || preferences.busy}
          onClick={() => { setPage('library'); if (instance.id !== preferences?.id) void preferences?.select(instance.id); }}>
          <CatalogArtwork url={instance.iconUrl} />
          {['Instalando', 'Reparando', 'Actualizando'].includes(instanceStatus(instance, liveEntry(instance.id))) && <LoaderCircle className="rail-lock spin" aria-hidden="true" />}
          {liveEntry(instance.id)?.phase === 'running' && <Gamepad2 className="rail-lock" aria-hidden="true" />}
          {instance.availability === 'disabled' && <LockKeyhole className="rail-lock" aria-hidden="true" />}
        </Button></Hint>)}
        {catalog.kind === 'loading' && <span className="rail-placeholder"><Blocks aria-hidden="true" /></span>}
        </div>
        {pageCount > 1 && <Hint text="Más instancias" side="right"><Button variant="ghost" className="rail-nav-control size-6 bg-transparent p-0" aria-label="Más instancias" disabled={visiblePage + 1 >= pageCount} onClick={() => setInstancePage(visiblePage + 1)}><ChevronDown aria-hidden="true" /></Button></Hint>}
      </nav>
      <div className="rail-bottom">
        <Hint text="Gestionar cuentas" side="right"><Button variant="ghost" size="icon" className={`size-10 [&_svg]:size-5 ${page === 'accounts' ? 'is-selected' : ''}`} aria-label="Gestionar cuentas" aria-current={page === 'accounts' ? 'page' : undefined} onClick={() => setPage('accounts')}><Users aria-hidden="true" /></Button></Hint>
        <div className="rail-accounts" role="group" aria-label="Cuentas de Minecraft">
          {selectedAccount && <Hint side="right" text={selectedAccount.metadata.username + ' | ' + (selectedAccount.metadata.kind === 'offline' ? 'No premium' : 'Microsoft')}>
            <Button variant="ghost" className="rail-account current-account size-10 p-0" aria-label={'Gestionar cuenta: ' + selectedAccount.metadata.username} onClick={() => setPage('accounts')}><AccountAvatar metadata={selectedAccount.metadata} /></Button>
          </Hint>}
          {!selectedAccount && <Hint text={accountView?.accounts.length ? 'Elegir una cuenta de Minecraft' : 'Añadir una cuenta de Minecraft'} side="right"><Button variant="ghost" className="rail-account size-10 p-0" aria-label={accountView?.accounts.length ? 'Elegir una cuenta de Minecraft' : 'Añadir una cuenta de Minecraft'} onClick={() => setPage('accounts')}><AccountAvatar metadata={null} /></Button></Hint>}
        </div>
        <span className="rail-version" title={`Panita Client ${version}`}>v{version}</span>
      </div>
    </aside>

    <div className="launcher-body">
      <header className="launcher-toolbar">
        <WindowDragRegion><span className="toolbar-brand">PANITA <span>CLIENT</span></span></WindowDragRegion>
        <div className="flex shrink-0 items-center gap-2">
          <Hint text="Actualizar catálogo"><span className="icon-control"><Button variant="ghost" size="icon" disabled={refreshing} aria-label="Actualizar catálogo" onClick={() => setCatalogAttempt(previous => previous + 1)}><RefreshCw aria-hidden="true" /></Button></span></Hint>
          <Hint text="Ajustes Globales"><Button variant="ghost" size="icon" aria-label="Ajustes Globales" onClick={() => openSettings('global')} aria-current={page === 'settings' && settingsScope === 'global' ? 'page' : undefined}><Settings2 aria-hidden="true" /></Button></Hint>
          <WindowControls />
        </div>
      </header>

      <div id="launcher-content" tabIndex={-1} className="launcher-content">
        <div className="launcher-notices">
          <PlayNotice names={Object.fromEntries(instances.map(instance => [instance.id, instance.name]))} />
          {preferences?.error && <div role="alert" className="catalog-notice"><p>{preferences.error}</p><Button variant="secondary" disabled={preferences.busy} onClick={preferences.retry}>Reintentar preferencias</Button></div>}
          {preferences?.warning && <p role="alert">{preferences.warning}</p>}
          {refreshError && catalog.kind === 'ready' && <p role="alert">No pudimos actualizar las instancias. {refreshError}</p>}
          {catalog.kind === 'ready' && catalog.stale && <div className="catalog-notice" role="status"><WifiOff aria-hidden="true" /><span>No pudimos conectar con el servidor. Puedes jugar las instancias que ya tienes instaladas.</span></div>}
          {catalog.kind === 'ready' && catalog.cacheWarning && <p role="alert">No pudimos guardar el catálogo. {messageForError(catalog.cacheWarning)}</p>}
          {catalog.kind === 'ready' && catalog.registryWarning && <p role="alert">No pudimos preparar tus instancias. {messageForError(catalog.registryWarning)}</p>}
          {unverified && page !== 'accounts' && <div className="catalog-notice" role="status"><p>Tu sesión de Panitacraft no está verificada. Instalar, actualizar o reparar necesita conexión.</p><Button variant="secondary" disabled={activation?.busy === 'refresh_activation'} onClick={() => void activation?.refresh()}>Reintentar sesión</Button></div>}
          {activation?.view && !activation.view.persisted && page !== 'accounts' && <p role="alert">Tu sesión de Panitacraft no se ha guardado. <button type="button" onClick={() => setPage('accounts')}>Revisar cuenta</button></p>}
        </div>

        <div hidden={page !== 'library'} className="library-page">
          {catalog.kind === 'loading' && <div className="empty-state" role="status"><Blocks aria-hidden="true" /><h1>Preparando tus instancias</h1><p>Cargando el catálogo de Panitacraft…</p></div>}
          {catalog.kind === 'blocked' && <div className="empty-state" role="status"><LockKeyhole aria-hidden="true" /><h1>Revisa tus cuentas</h1><p>Las instancias estarán disponibles cuando se resuelva el problema con las cuentas.</p><Button onClick={() => setPage('accounts')}>Abrir cuentas</Button></div>}
          {catalog.kind === 'error' && <div className="empty-state" role="alert"><WifiOff aria-hidden="true" /><h1>No pudimos cargar tus instancias</h1><p>{catalog.message}</p><Button onClick={() => { setCatalog({ kind: 'loading' }); setCatalogAttempt(previous => previous + 1); }}>Reintentar</Button></div>}
          {catalog.kind === 'ready' && instances.length === 0 && <div className="empty-state"><Blocks aria-hidden="true" /><h1>No hay instancias disponibles</h1><p>Vuelve a consultar el catálogo más tarde.</p></div>}
          {/* Retain confirmations and progress while navigating between instances and settings. */}
          {catalog.kind === 'ready' && instances.map(instance => <article key={instance.id} hidden={instance.id !== selectedId} className="instance-detail" aria-label={instance.name}>
            <div className="instance-hero">
              <CatalogArtwork url={instance.bannerUrl} banner />
              <div className="hero-heading">
                <h1>{instance.name}</h1>
                <p>{instance.description}</p>
                <div className="instance-tags">
                  <InstanceBadges instance={instance} showLoaderHint />
                  {instance.availability === 'disabled' ? <Hint text="Esta instancia está deshabilitada. Tus mundos y archivos se conservan."><span className="instance-status status-warning" tabIndex={0}><LockKeyhole aria-hidden="true" />Deshabilitada</span></Hint> : !['Sin instalar', 'Lista para jugar', 'Actualización disponible', 'Instalando', 'Reparando', 'Actualizando'].includes(instanceStatus(instance, liveEntry(instance.id))) && <span className="instance-status">{instanceStatus(instance, liveEntry(instance.id))}</span>}
                </div>
              </div>
            </div>
            <div className="instance-workspace">
              {instance.removalPending && <p role="alert">La eliminación de esta instancia está incompleta. Abre sus opciones para reintentarla. No se borrará nada automáticamente.</p>}
              {instance.availability === 'missing' && <p role="status">Esta instancia ya no está en el catálogo. Conservamos tus datos locales.</p>}
              {instance.availability === 'unverified' && <p role="status">No pudimos verificar el estado de esta instancia.</p>}
              {!instance.compatible && <p role="status">Necesitas Panita Client {instance.minLauncherVersion} o una versión posterior.</p>}
              {!instance.supported && <p role="status">Este tipo de instancia todavía no es compatible.</p>}
              <PlayControls onPackCache={() => { openSettings('global'); setSettingsTab('storage'); setCacheRequest(value => value + 1); }} instanceId={instance.id} removalPending={instance.removalPending} optionsAllowed={canOpenInstanceOptions(instance)} installAllowed={canInstall(instance, catalog.stale || unverified)} updateTarget={updateTarget(instance, catalog.stale || unverified)} repairAllowed={canRepair(instance, catalog.stale || unverified)} uninstallAllowed={canUninstall(instance)} pack={isPack(instance)} cleanupAllowed={instance.hasInstallationManifest && instance.localState !== 'running'} onSettings={() => openSettings('instance')} allowed={instance.compatible && instance.supported && instance.availability !== 'disabled' && instance.availability !== 'missing' && (instance.localState === 'installed' || instance.localState === 'update_available' || liveEntry(instance.id)?.phase === 'installed')} />
              <PackPanel instanceId={instance.id} awaiting={instance.localState === 'awaiting_files'} onChanged={() => setCatalogAttempt(previous => previous + 1)} />
            </div>
          </article>)}
        </div>

        <div hidden={page !== 'accounts'} className="utility-page">
          <div className="page-heading"><Button variant="ghost" className="page-back h-auto p-0 hover:bg-transparent hover:text-foreground" onClick={() => setPage('library')}><ArrowLeft aria-hidden="true" />Instancias</Button><h1>Tus cuentas</h1></div>
          <ActivationPanel /><Accounts onSnapshot={acceptAccounts} />
          <UtilityFooter />
        </div>
        <div hidden={page !== 'settings'} className="utility-page settings-page">
          <div className="page-heading"><Button variant="ghost" className="page-back h-auto p-0 hover:bg-transparent hover:text-foreground" onClick={() => setPage('library')}><ArrowLeft aria-hidden="true" />Instancias</Button><h1>{settingsScope === 'instance' ? `Ajustes · ${selected?.name ?? 'Instancia'}` : 'Ajustes Globales'}</h1><p>{settingsScope === 'instance' ? 'Sobrescribe los ajustes globales solo para esta instancia.' : 'Estos valores se aplicarán por defecto a todas las instancias.'}</p></div>
          {settingsScope === 'global' && <div className="settings-tabs" role="tablist" aria-label="Secciones de ajustes">
            {([['game', 'Minecraft'], ['storage', 'Almacenamiento'], ['diagnostics', 'Diagnóstico']] as const).map(([value, label]) => <Button key={value} variant={settingsTab === value ? 'secondary' : 'ghost'} role="tab" aria-selected={settingsTab === value} onClick={() => setSettingsTab(value)}>{label}</Button>)}
          </div>}
          <div hidden={settingsTab !== 'game'}><SettingsPanel instanceId={selectedId} name={selected?.name ?? null} local={selected?.hasInstallationManifest ?? false} scope={settingsScope} /></div>
          <div hidden={settingsScope !== 'global' || settingsTab !== 'storage'}><CachePanel openRequest={cacheRequest} /></div>
          <div hidden={settingsScope !== 'global' || settingsTab !== 'diagnostics'}>{settingsScope === 'global' && settingsTab === 'diagnostics' && <DiagnosticsPanel />}</div>
          <UtilityFooter />
        </div>
      </div>
    </div>
    <CrashNotifications names={Object.fromEntries(instances.map(instance => [instance.id, instance.name]))} />
  </main></TooltipProvider>;
}
