import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { settingsApi } from './settings-api';
import { globalSettingsPayload, instanceSettingsPayload, settingsDraft, updateSettingsDraft, type SettingsDraft, type SettingsSnapshot } from './settings-state';
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
    try { await settingsApi.select(next); setId(next); }
    catch (cause) { setError(messageForError(cause)); }
    finally { selecting.current = false; setBusy(false); }
  }
  return <SelectionContext.Provider value={{ id, busy, error, warning, select, retry: () => setAttempt(previous => previous + 1) }}>{children}</SelectionContext.Provider>;
}

export function usePreferences() { return useContext(SelectionContext); }

export function SettingsPanel({ instanceId, name, local }: { instanceId: string | null; name: string | null; local: boolean }) {
  const [mode, setMode] = useState<'global' | 'instance'>('global');
  const preferences = usePreferences();
  return <section className="settings-section" aria-labelledby="settings-heading">
    <h2 id="settings-heading">Ajustes</h2>
    <p>Los cambios se aplican al próximo inicio de Minecraft. La cuenta y la instancia seleccionadas se recuerdan. El tamaño de Panita se guarda al cerrar su ventana.</p>
    {preferences?.error && <div role="alert"><p>No pudimos cargar o guardar tu selección. {preferences.error}</p><button type="button" disabled={preferences.busy} onClick={preferences.retry}>Reintentar preferencias</button></div>}
    {preferences?.warning && <p role="alert">No pudimos restaurar o guardar el tamaño de Panita. {preferences.warning}</p>}
    <div className="account-buttons" role="group" aria-label="Ámbito de los ajustes">
      <button type="button" aria-pressed={mode === 'global'} onClick={() => setMode('global')}>Ajustes globales</button>
      <button type="button" aria-pressed={mode === 'instance'} disabled={!instanceId} onClick={() => setMode('instance')}>Ajustes de la instancia seleccionada</button>
    </div>
    {mode === 'instance' && <p>{name ? `Instancia: ${name}` : 'La instancia seleccionada no está disponible en esta lista.'}</p>}
    {mode === 'instance' && !local ? <p>Instala o reintenta instalar esta instancia para poder guardar sus ajustes propios. Puedes configurar los valores globales mientras tanto.</p>
      : <SettingsEditor key={mode === 'global' ? 'global' : instanceId} instanceId={mode === 'global' ? null : instanceId} />}
  </section>;
}

function SettingsEditor({ instanceId }: { instanceId: string | null }) {
  const [snapshot, setSnapshot] = useState<SettingsSnapshot | null>(null);
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
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

  function change(patch: Partial<SettingsDraft>) { setDraft(previous => previous && snapshot && updateSettingsDraft(previous, patch, snapshot)); setNotice(null); setError(null); }
  async function save() {
    if (!snapshot || !draft || saving.current) return;
    saving.current = true; setBusy(true); setError(null); setNotice(null);
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
      }
      if (mounted.current) { setSnapshot(saved); setDraft(settingsDraft(saved)); setNotice('Los ajustes se guardaron. Se aplicarán al próximo inicio de Minecraft.'); }
    } catch (cause) {
      const code = cause instanceof Error ? cause.message : cause;
      if (mounted.current) setError(code === 'invalid_data'
        ? 'Comprueba la RAM y el tamaño de la ventana. Usa un solo argumento -Xmx con M o G (por ejemplo, -Xmx4G) y opciones -D o -XX: compatibles. No añadas -Xms ni otros argumentos que sustituyan la memoria o las bibliotecas.'
        : messageForError(code));
    } finally { saving.current = false; if (mounted.current) setBusy(false); }
  }
  const prefix = instance ? 'instance-settings' : 'global-settings';
  const customJava = snapshot && (snapshot.instance?.java_path_override ?? snapshot.global.java_path_override);
  return <>
    {!desktop && <p>Los ajustes están disponibles en la aplicación de escritorio.</p>}
    {desktop && !draft && !error && <p role="status">Cargando ajustes…</p>}
    {error && <div role="alert"><p>No pudimos cargar o guardar los ajustes. {error}</p>{!draft && <button type="button" onClick={() => setAttempt(previous => previous + 1)}>Reintentar ajustes</button>}</div>}
    {notice && <p role="status">{notice}</p>}
    {snapshot && draft && <form className="settings-form" onSubmit={event => { event.preventDefault(); void save(); }}>
      <fieldset disabled={busy}>
        <legend>{instance ? 'Valores propios de esta instancia' : 'Valores predeterminados de Minecraft'}</legend>
        {instance && <label><input type="checkbox" checked={draft.inheritConsole} onChange={event => change({ inheritConsole: event.target.checked })} /> Usar la opción global de consola ({snapshot.global.show_console_on_start ? 'activada' : 'desactivada'})</label>}
        <label><input type="checkbox" checked={draft.showConsole} disabled={instance && draft.inheritConsole} onChange={event => change({ showConsole: event.target.checked })} /> Mostrar la consola cuando se inicie Minecraft</label>
        <p>Abre el registro del juego en una ventana separada. Los cambios se aplican al próximo inicio.</p>
        {instance && <label><input type="checkbox" checked={draft.inheritRam} onChange={event => change({ inheritRam: event.target.checked })} /> Usar RAM global ({snapshot.global.ram_mib} MiB)</label>}
        <label htmlFor={`${prefix}-ram`}>RAM para Minecraft (MiB)</label>
        <input aria-label="Control deslizante de RAM para Minecraft" type="range" min="512" max={Math.min(262144, Math.max(32768, Number(draft.ram) || 0))} step="1" disabled={instance && draft.inheritRam} value={/^\d+$/.test(draft.ram) ? draft.ram : '512'} onChange={event => change({ ram: event.target.value })} />
        <input id={`${prefix}-ram`} type="number" min="512" max="262144" step="1" required disabled={instance && draft.inheritRam} value={draft.ram} onChange={event => change({ ram: event.target.value })} />
        <output htmlFor={`${prefix}-ram`}>{Number(draft.ram) / 1024 || 0} GiB</output>
        {instance && <label><input type="checkbox" checked={draft.inheritWindow} onChange={event => change({ inheritWindow: event.target.checked })} /> Usar tamaño de Minecraft global ({snapshot.global.window_size.width} × {snapshot.global.window_size.height})</label>}
        <label htmlFor={`${prefix}-width`}>Ancho de Minecraft (píxeles)</label>
        <input id={`${prefix}-width`} type="number" min="320" max="16384" step="1" required disabled={instance && draft.inheritWindow} value={draft.width} onChange={event => change({ width: event.target.value })} />
        <label htmlFor={`${prefix}-height`}>Alto de Minecraft (píxeles)</label>
        <input id={`${prefix}-height`} type="number" min="240" max="16384" step="1" required disabled={instance && draft.inheritWindow} value={draft.height} onChange={event => change({ height: event.target.value })} />
        <details>
          <summary>Opciones avanzadas de Java</summary>
          <p>Panita descarga y verifica el Java necesario. Las rutas de Java personalizadas todavía no son compatibles.</p>
          {customJava && <div role="alert"><p>Hay una ruta personalizada guardada. Minecraft no puede iniciarse con ella.</p>
            {(!instance || snapshot.instance?.java_path_override) ? <label><input type="checkbox" checked={draft.clearJava} onChange={event => change({ clearJava: event.target.checked })} /> Quitar la ruta personalizada al guardar</label>
              : <p>Quita la ruta desde los ajustes globales.</p>}
          </div>}
          {instance && <label><input type="checkbox" checked={draft.inheritJvm} onChange={event => change({ inheritJvm: event.target.checked })} /> Usar argumentos JVM globales</label>}
          <label htmlFor={`${prefix}-jvm`}>Memoria y argumentos JVM adicionales (uno por línea)</label>
          <textarea id={`${prefix}-jvm`} rows={5} aria-describedby={`${prefix}-jvm-help`} value={draft.jvm} onChange={event => change({ jvm: event.target.value })} />
          <p id={`${prefix}-jvm-help`}>-Xmx se sincroniza con la RAM. Puedes usar M o G, por ejemplo -Xmx4G. Editar un valor heredado crea un ajuste propio. Para opciones adicionales, usa -D o -XX:. No se muestra el comando completo de inicio.</p>
        </details>
        <button type="submit">{busy ? 'Guardando ajustes…' : 'Guardar ajustes'}</button>
      </fieldset>
    </form>}
  </>;
}
