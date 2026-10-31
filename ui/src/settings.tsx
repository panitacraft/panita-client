import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Save, MemoryStick, Monitor, Terminal, SlidersHorizontal, ChevronDown, AppWindow } from 'lucide-react';
import { Button } from './components/ui/button';
import { TransientNotice, useTransientNotice } from './components/transient-notice';
import { settingsApi } from './settings-api';
import { globalSettingsPayload, instanceSettingsPayload, settingsDraft, trayPreferences, updateSettingsDraft, type SettingsDraft, type SettingsSnapshot } from './settings-state';
import { messageForError } from './errors';

type Selection = { id: string | null; busy: boolean; error: string | null; warning: string | null; select: (id: string) => Promise<void>; retry: () => void };
const SelectionContext = createContext<Selection | null>(null);

export function PreferencesProvider({ children }: { children: ReactNode }) {
  const [id, setId] = useState<string | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [warning, setWarning] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const selecting = useRef(false);
  useEffect(() => {
    if (!isTauri()) { setBusy(false); return; }
    let active = true;
    let stop: (() => void) | undefined;
    setBusy(true); setError(null);
    void (async () => {
      const unlisten = await listen<string>('settings-warning', event => { if (active) setWarning(messageForError(event.payload)); });
      if (!active) { unlisten(); return; }
      stop = unlisten;
      await invoke<void>('initialize');
      const snapshot = await settingsApi.snapshot(null);
      if (active) { setId(snapshot.global.launcher.selected_instance_id); setWarning(snapshot.windowWarning ? messageForError(snapshot.windowWarning) : null); }
    })().catch(cause => { if (active) setError(messageForError(cause)); })
      .finally(() => { if (active) setBusy(false); });
    return () => { active = false; stop?.(); };
  }, [attempt]);

  async function select(next: string) {
    if (selecting.current || busy) return;
    selecting.current = true; setBusy(true); setError(null);
    try { if (isTauri()) await settingsApi.select(next); setId(next); }
    catch (cause) { setError(messageForError(cause)); }
    finally { selecting.current = false; setBusy(false); }
  }
  return <SelectionContext.Provider value={{ id, busy, error, warning, select, retry: () => setAttempt(previous => previous + 1) }}>{children}</SelectionContext.Provider>;
}

export function usePreferences() { return useContext(SelectionContext); }

export function SettingsPanel({ instanceId, local, scope }: { instanceId: string | null; name: string | null; local: boolean; scope?: 'global' | 'instance' }) {
  const [chosenMode, setMode] = useState<'global' | 'instance'>('global');
  const mode = scope ?? chosenMode;
  const preferences = usePreferences();
  return <section className="settings-section" aria-labelledby="settings-heading">
    <h2 id="settings-heading" className="sr-only">Ajustes</h2>
    {preferences?.error && <div role="alert"><p>No pudimos cargar o guardar tu selección. {preferences.error}</p><button type="button" disabled={preferences.busy} onClick={preferences.retry}>Reintentar preferencias</button></div>}
    {preferences?.warning && <p role="alert">No pudimos restaurar o guardar el tamaño de Panita. {preferences.warning}</p>}
    {!scope && <div className="account-buttons" role="group" aria-label="Ámbito de los ajustes">
      <button type="button" aria-pressed={mode === 'global'} onClick={() => setMode('global')}>Ajustes globales</button>
      <button type="button" aria-pressed={mode === 'instance'} disabled={!instanceId} onClick={() => setMode('instance')}>Ajustes de la instancia seleccionada</button>
    </div>}
    {mode === 'instance' && !local ? <p>Instala o reintenta instalar esta instancia para poder guardar sus ajustes propios. Puedes configurar los valores globales mientras tanto.</p>
      : <SettingsEditor key={mode === 'global' ? 'global' : instanceId} instanceId={mode === 'global' ? null : instanceId} />}
  </section>;
}

const RAM_SLIDER_STEP_MIB = 512;
const formatGiB = (mib: number) => Number((mib / 1024).toFixed(2));

function InheritToggle({ checked, onChange, name }: { checked: boolean; onChange: (value: boolean) => void; name: string }) {
  return <label className="inherit-toggle"><span>Heredar ajuste global</span><input role="switch" aria-label={`Heredar ajuste global: ${name}`} type="checkbox" checked={checked} onChange={event => onChange(event.target.checked)} /></label>;
}

function SettingsEditor({ instanceId }: { instanceId: string | null }) {
  const [advanced, setAdvanced] = useState(false);
  const [snapshot, setSnapshot] = useState<SettingsSnapshot | null>(null);
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const toast = useTransientNotice();
  const [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const mounted = useRef(false);
  const saving = useRef(false);
  const desktop = isTauri();
  const instance = instanceId !== null;
  useEffect(() => {
    mounted.current = true;
    let active = true;
    if (desktop) {
      setError(null);
      void (async () => {
        await invoke<void>('initialize');
        const next = await settingsApi.snapshot(instanceId);
        if (active) { setSnapshot(next); setDraft(settingsDraft(next)); }
      })().catch(cause => { if (active) setError(messageForError(cause)); });
    }
    return () => { active = false; mounted.current = false; };
  }, [instanceId, desktop, attempt]);

  function change(patch: Partial<SettingsDraft>) { setDraft(previous => previous && snapshot && updateSettingsDraft(previous, patch, snapshot)); toast.clear(); setError(null); }
  async function save() {
    if (!snapshot || !draft || saving.current) return;
    saving.current = true; setBusy(true); setError(null); toast.clear();
    try {
      let saved: SettingsSnapshot;
      if (instanceId) {
        const settings = instanceSettingsPayload(draft, snapshot);
        await settingsApi.saveInstance(instanceId, settings);
        saved = { ...snapshot, instance: settings };
      } else {
        const settings = globalSettingsPayload(draft, snapshot);
        await settingsApi.saveGlobal(settings);
        saved = { ...snapshot, global: { ...snapshot.global, ...settings } };
        if (snapshot.trayAvailable) {
          const tray = trayPreferences(draft);
          await settingsApi.saveTray(tray);
          saved = { ...saved, global: { ...saved.global, launcher: { ...saved.global.launcher, close_to_tray: tray.closeToTray, hide_on_game_start: tray.hideOnGameStart, reopen_on_game_exit: tray.reopenOnGameExit } } };
        }
      }
      if (mounted.current) { setSnapshot(saved); setDraft(settingsDraft(saved)); toast.show('Ajustes guardados.'); }
    } catch (cause) {
      const code = cause instanceof Error ? cause.message : cause;
      if (mounted.current) setError(code === 'invalid_data'
        ? 'Comprueba la RAM y el tamaño de la ventana. Usa un solo argumento -Xmx con M o G (por ejemplo, -Xmx4G) y opciones -D o -XX: compatibles. No añadas -Xms ni otros argumentos que sustituyan la memoria o las bibliotecas.'
        : messageForError(code));
    } finally { saving.current = false; if (mounted.current) setBusy(false); }
  }
  const prefix = instance ? 'instance-settings' : 'global-settings';
  const sliderMax = Math.ceil(Math.min(262144, Math.max(32768, Number(draft?.ram) || 0)) / RAM_SLIDER_STEP_MIB) * RAM_SLIDER_STEP_MIB;
  const customJava = snapshot && (snapshot.instance?.java_path_override ?? snapshot.global.java_path_override);
  return <>
    {!desktop && <p>Los ajustes están disponibles en la aplicación de escritorio.</p>}
    {desktop && !draft && !error && <p role="status">Cargando ajustes…</p>}
    {error && <div role="alert"><p>No pudimos cargar o guardar los ajustes. {error}</p>{!draft && <button type="button" onClick={() => setAttempt(previous => previous + 1)}>Reintentar ajustes</button>}</div>}
    <TransientNotice notice={toast.notice} />
    {snapshot && draft && <form className="settings-form compact-settings" onSubmit={event => { event.preventDefault(); void save(); }}>
      <fieldset disabled={busy}>
        <legend className="sr-only">{instance ? 'Ajustes de la instancia' : 'Valores predeterminados de Minecraft'}</legend>
        <div className="settings-grid">
          <div className="settings-card memory-card">
            <div className="settings-card-heading"><h3><MemoryStick aria-hidden="true" />Memoria</h3>{instance && <InheritToggle name="Memoria" checked={draft.inheritRam} onChange={value => change({ inheritRam: value })} />}</div>
            <div className={instance && draft.inheritRam ? 'inherited-content' : undefined}>
            <div className="memory-value"><label htmlFor={prefix + '-ram'}>RAM Asignada (MiB)</label><input id={prefix + '-ram'} type="number" min="512" max="262144" step="1" required disabled={instance && draft.inheritRam} value={draft.ram} onChange={event => change({ ram: event.target.value })} /><output>{formatGiB(Number(draft.ram) || 0)} GiB</output></div>
            <input className="memory-slider" aria-label="Control deslizante de RAM para Minecraft" type="range" min="512" max={sliderMax} step={RAM_SLIDER_STEP_MIB} disabled={instance && draft.inheritRam} value={/^\d+$/.test(draft.ram) ? draft.ram : '512'} onChange={event => change({ ram: event.target.value })} />
            <div className="memory-ticks" aria-hidden="true"><span>512 MiB</span><span>{formatGiB(sliderMax / 2)} GiB</span><span>{formatGiB(sliderMax)} GiB</span></div>
            </div>
          </div>
          <div className="settings-card">
            <div className="settings-card-heading"><h3><Monitor aria-hidden="true" />Ventana del juego</h3>{instance && <InheritToggle name="Ventana del juego" checked={draft.inheritWindow} onChange={value => change({ inheritWindow: value })} />}</div>
            <div className={instance && draft.inheritWindow ? 'inherited-content' : undefined}>
            <p className="resolution-label">Resolución (px)</p><div className="window-dimensions"><div><label htmlFor={prefix + '-width'} className="sr-only">Ancho (px)</label><input aria-label="Ancho de Minecraft (píxeles)" id={prefix + '-width'} type="number" min="320" max="16384" required disabled={instance && draft.inheritWindow} value={draft.width} onChange={event => change({ width: event.target.value })} /></div><span aria-hidden="true">×</span><div><label htmlFor={prefix + '-height'} className="sr-only">Alto (px)</label><input aria-label="Alto de Minecraft (píxeles)" id={prefix + '-height'} type="number" min="240" max="16384" required disabled={instance && draft.inheritWindow} value={draft.height} onChange={event => change({ height: event.target.value })} /></div></div>
            </div>
          </div>
          {!instance && snapshot.trayAvailable && <div className="settings-card tray-card">
            <div className="settings-card-heading"><h3><AppWindow aria-hidden="true" />Ventana del launcher</h3></div>
            <label><input type="checkbox" checked={draft.closeToTray} onChange={event => change({ closeToTray: event.target.checked })} />Al cerrar la ventana, mantener Panita Client en la bandeja del sistema</label>
            <label><input type="checkbox" checked={draft.hideOnGameStart} onChange={event => change({ hideOnGameStart: event.target.checked })} />Ocultar el launcher en la bandeja al iniciar Minecraft</label>
            <label><input type="checkbox" checked={draft.reopenOnGameExit} onChange={event => change({ reopenOnGameExit: event.target.checked })} />Volver a mostrar el launcher al cerrar el juego</label>
          </div>}
          <div className="settings-card console-card">
            <div className="settings-card-heading"><h3><Terminal aria-hidden="true" />Registro del juego</h3>{instance && <InheritToggle name="Registro del juego" checked={draft.inheritConsole} onChange={value => change({ inheritConsole: value })} />}</div>
            <div className={instance && draft.inheritConsole ? 'inherited-content' : undefined}>
            <label><input type="checkbox" checked={draft.showConsole} disabled={instance && draft.inheritConsole} onChange={event => change({ showConsole: event.target.checked })} />Abrir el registro al iniciar Minecraft</label>
            </div>
          </div>
        </div>
        <div className="settings-advanced">
          <div className="settings-card-heading"><button type="button" className="advanced-toggle" aria-expanded={advanced} aria-controls={prefix + '-advanced'} onClick={() => setAdvanced(value => !value)}><SlidersHorizontal aria-hidden="true" />Java y argumentos avanzados<ChevronDown className={advanced ? 'expanded' : ''} aria-hidden="true" /></button>
          {instance && <InheritToggle name="Java y argumentos avanzados" checked={draft.inheritJvm} onChange={value => change({ inheritJvm: value })} />}</div>
          <div id={prefix + '-advanced'} hidden={!advanced}>
          {customJava && <div role="alert"><p>Hay una ruta personalizada guardada. Minecraft no puede iniciarse con ella.</p>
            {(!instance || snapshot.instance?.java_path_override) ? <label><input type="checkbox" checked={draft.clearJava} onChange={event => change({ clearJava: event.target.checked })} /> Quitar la ruta personalizada al guardar</label>
              : <p>Quita la ruta desde los ajustes globales.</p>}
          </div>}
          <div className={instance && draft.inheritJvm ? 'inherited-content' : undefined}>
          <label htmlFor={`${prefix}-jvm`}>Memoria y argumentos JVM adicionales (uno por línea)</label>
          <textarea id={`${prefix}-jvm`} rows={4} disabled={instance && draft.inheritJvm} value={draft.jvm} onChange={event => change({ jvm: event.target.value })} />
          </div></div>
        </div>
        <div className="settings-save"><Button type="submit"><Save aria-hidden="true" />{busy ? 'Guardando…' : 'Guardar ajustes'}</Button></div>
      </fieldset>
    </form>}
  </>;
}
