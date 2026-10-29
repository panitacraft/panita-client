import { useCallback, useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { ArrowLeft, Blocks, Gamepad2, LockKeyhole, RefreshCw, Settings2, ShieldCheck, Users, WifiOff } from 'lucide-react';
import { loadCatalog } from './catalog-api';
import type { CatalogOverview } from './preview-catalog';
import { messageForError } from './errors';
import logoWhite from './assets/logo-white.svg';
import { Accounts } from './accounts';
import { invokeAccount } from './account-api';
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
import { InstanceBadges } from './components/instance-badges';

type CatalogState = { kind: 'loading' } | { kind: 'error'; message: string } | { kind: 'blocked' } | ({ kind: 'ready' } & CatalogOverview);

export function Launcher() {
  const [catalog, setCatalog] = useState<CatalogState>({ kind: 'loading' });
  const activation = useActivation();
  const unverified = activation?.view?.state === 'active_offline';
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const [catalogAttempt, setCatalogAttempt] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [page, setPage] = useState<LauncherPage>('library');
  const [settingsScope, setSettingsScope] = useState<'global' | 'instance'>('global');
  const [accountView, setAccountView] = useState<AccountsView | null>(null);
  const [accountBusy, setAccountBusy] = useState(false);
  const accountSelecting = useRef(false);
  const [accountError, setAccountError] = useState<string | null>(null);
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
    setSettingsScope(scope); setPage('settings');
  }

  async function selectAccount(accountId: string) {
    if (accountSelecting.current || accountView?.selected_account_id === accountId) return;
    accountSelecting.current = true;
    setAccountBusy(true); setAccountError(null);
    try {
      await invokeAccount('select_account', { accountId });
      acceptAccounts(await invokeAccount('account_snapshot', undefined));
    } catch (cause) { setAccountError(messageForError(cause)); }
    finally { accountSelecting.current = false; setAccountBusy(false); }
  }

  return <TooltipProvider delayDuration={250}><main className="launcher-shell">
    <a className="skip-link" href="#launcher-content">Saltar al contenido</a>
    <aside className="instance-rail" aria-label="Navegación de Panita">
      <Button variant="ghost" className="rail-logo size-14 p-0" aria-label="Inicio: tus instancias" onClick={() => setPage('library')}>
        <img src={logoWhite} alt="" aria-hidden="true" />
      </Button>
      <div className="rail-divider" />
      <nav className="rail-instances" aria-label="Instancias">
        {instances.map(instance => <Button key={instance.id} variant="ghost"
          className={`rail-instance size-13 p-0 ${instance.id === selectedId && page === 'library' ? 'is-selected' : ''} ${instance.availability === 'disabled' ? 'is-disabled' : ''}`}
          aria-label={`${instance.name}: ${instanceStatus(instance, liveEntry(instance.id))}`}
          aria-current={instance.id === selectedId && page === 'library' ? 'page' : undefined}
          title={instance.name} disabled={!preferences || preferences.busy}
          onClick={() => { setPage('library'); if (instance.id !== preferences?.id) void preferences?.select(instance.id); }}>
          <CatalogArtwork url={instance.iconUrl} />
          {liveEntry(instance.id)?.phase === 'running' && <Gamepad2 className="rail-lock" aria-hidden="true" />}
          {instance.availability === 'disabled' && <LockKeyhole className="rail-lock" aria-hidden="true" />}
        </Button>)}
        {catalog.kind === 'loading' && <span className="rail-placeholder"><Blocks aria-hidden="true" /></span>}
      </nav>
      <div className="rail-bottom">
        <Hint text="Gestionar cuentas" side="right"><Button variant="ghost" size="icon" className={page === 'accounts' ? 'is-selected' : ''} aria-label="Gestionar cuentas" aria-current={page === 'accounts' ? 'page' : undefined} onClick={() => setPage('accounts')}><Users aria-hidden="true" /></Button></Hint>
        <div className="rail-accounts" role="group" aria-label="Cuentas de Minecraft">
          {accountView?.accounts.map(account => <Hint key={account.metadata.account_id} side="right" text={`${account.metadata.username} · ${account.metadata.kind === 'offline' ? 'Sin conexión' : 'Microsoft'}${account.metadata.account_id === accountView.selected_account_id ? ' · Seleccionada' : ''}`}>
            <Button variant="ghost" className="rail-account size-10 p-0" aria-label={`Seleccionar cuenta: ${account.metadata.username}`} aria-pressed={account.metadata.account_id === accountView.selected_account_id} disabled={accountBusy} onClick={() => void selectAccount(account.metadata.account_id)}><AccountAvatar metadata={account.metadata} /></Button>
          </Hint>)}
          {!accountView?.accounts.length && <Hint text="Añadir una cuenta de Minecraft" side="right"><Button variant="ghost" className="rail-account size-10 p-0" aria-label="Añadir una cuenta de Minecraft" onClick={() => setPage('accounts')}><AccountAvatar metadata={null} /></Button></Hint>}
        </div>
        <span className="rail-version" title={`Panita Client ${version}`}>v{version}</span>
      </div>
    </aside>

    <div className="launcher-body">
      <header className="launcher-toolbar">
        <span className="toolbar-brand">PANITA <span>CLIENT</span></span>
        <div className="flex shrink-0 items-center gap-2">
          <Hint text={unverified || (catalog.kind === 'ready' && catalog.stale) ? 'Sin verificar: no pudimos conectar con Panitacraft.' : isTauri() ? 'Sesión de Panitacraft' : 'Vista previa del navegador'}><span className="connection-indicator" tabIndex={0} aria-label={unverified || (catalog.kind === 'ready' && catalog.stale) ? 'Sin verificar' : isTauri() ? 'Panitacraft' : 'Vista previa'}>{unverified || (catalog.kind === 'ready' && catalog.stale) ? <WifiOff aria-hidden="true" /> : <ShieldCheck aria-hidden="true" />}</span></Hint>
          <Hint text="Actualizar catálogo"><span className="icon-control"><Button variant="ghost" size="icon" disabled={refreshing} aria-label="Actualizar catálogo" onClick={() => setCatalogAttempt(previous => previous + 1)}><RefreshCw aria-hidden="true" /></Button></span></Hint>
          <Hint text="Ajustes del launcher"><Button variant="ghost" size="icon" aria-label="Ajustes del launcher" onClick={() => openSettings('global')} aria-current={page === 'settings' && settingsScope === 'global' ? 'page' : undefined}><Settings2 aria-hidden="true" /></Button></Hint>
        </div>
      </header>

      <div id="launcher-content" tabIndex={-1} className="launcher-content">
        <div className="launcher-notices">
          <PlayNotice />
          {accountError && <p role="alert">No pudimos cambiar de cuenta. {accountError}</p>}
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
                  <InstanceBadges instance={instance} />
                  {instance.availability === 'disabled' ? <Hint text="Esta instancia está deshabilitada. Tus mundos y archivos se conservan."><span className="instance-status status-warning" tabIndex={0}><LockKeyhole aria-hidden="true" />Deshabilitada</span></Hint> : instanceStatus(instance, liveEntry(instance.id)) !== 'Sin instalar' && <span className="instance-status">{instanceStatus(instance, liveEntry(instance.id))}</span>}
                </div>
              </div>
            </div>
            <div className="instance-workspace">
              {instance.availability === 'missing' && <p role="status">Esta instancia ya no está en el catálogo. Conservamos tus datos locales.</p>}
              {instance.availability === 'unverified' && <p role="status">No pudimos verificar el estado de esta instancia.</p>}
              {!instance.compatible && <p role="status">Necesitas Panita Client {instance.minLauncherVersion} o una versión posterior.</p>}
              {!instance.supported && <p role="status">Este tipo de instancia todavía no es compatible.</p>}
              {instance.installedVersion && <p className="installed-version">Versión instalada: Minecraft {instance.installedVersion}</p>}
              <PlayControls instanceId={instance.id} optionsAllowed={canOpenInstanceOptions(instance)} installAllowed={canInstall(instance, catalog.stale || unverified)} updateTarget={updateTarget(instance, catalog.stale || unverified)} repairAllowed={canRepair(instance, catalog.stale || unverified)} uninstallAllowed={canUninstall(instance)} pack={isPack(instance)} cleanupAllowed={instance.hasInstallationManifest && instance.localState !== 'running'} onSettings={() => openSettings('instance')} allowed={instance.compatible && instance.supported && instance.availability !== 'disabled' && instance.availability !== 'missing' && (instance.localState === 'installed' || instance.localState === 'update_available')} />
              <PackPanel instanceId={instance.id} awaiting={instance.localState === 'awaiting_files'} onChanged={() => setCatalogAttempt(previous => previous + 1)} />
            </div>
          </article>)}
        </div>

        <div hidden={page !== 'accounts'} className="utility-page">
          <div className="page-heading"><Button variant="ghost" onClick={() => setPage('library')}><ArrowLeft aria-hidden="true" />Volver a las instancias</Button><h1>Tus cuentas</h1><p>Tu acceso a Panitacraft y tu perfil de Minecraft.</p></div>
          <ActivationPanel /><Accounts onSnapshot={acceptAccounts} />
        </div>
        <div hidden={page !== 'settings'} className="utility-page">
          <div className="page-heading"><Button variant="ghost" onClick={() => setPage('library')}><ArrowLeft aria-hidden="true" />Volver a las instancias</Button><h1>{settingsScope === 'instance' ? selected?.name ?? 'Ajustes de instancia' : 'Ajustes del launcher'}</h1><p>{settingsScope === 'instance' ? 'Personaliza esta instancia sin cambiar las demás.' : 'Configura Minecraft y administra los recursos de Panita.'}</p></div>
          <SettingsPanel instanceId={selectedId} name={selected?.name ?? null} local={selected?.hasInstallationManifest ?? false} scope={settingsScope} />
          <div hidden={settingsScope !== 'global'}><CachePanel /><DiagnosticsPanel /></div>
        </div>
      </div>
    </div>
    <CrashNotifications names={Object.fromEntries(instances.map(instance => [instance.id, instance.name]))} />
  </main></TooltipProvider>;
}
