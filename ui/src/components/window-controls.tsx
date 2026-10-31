import { useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Copy, Minus, Square, X } from 'lucide-react';
import { Button } from './ui/button';
import { Hint } from './hint';

type WindowAction = 'state' | 'minimize' | 'toggle_maximize' | 'close';

export function WindowControls({ separator = true }: { separator?: boolean }) {
  const native = isTauri();
  const [maximized, setMaximized] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(false);
  const revision = useRef(0);

  useEffect(() => {
    if (!native) return;
    let active = true;
    let stop: (() => void) | undefined;
    const sync = async () => {
      const request = ++revision.current;
      try {
        const state = await invoke<boolean>('main_window_control', { action: 'state' });
        if (active && request === revision.current) setMaximized(state);
      } catch { if (active) setError('No pudimos consultar el estado de la ventana.'); }
    };
    void (async () => {
      const unlisten = await getCurrentWindow().onResized(() => void sync());
      if (!active) { unlisten(); return; }
      stop = unlisten;
      await sync();
    })().catch(() => { if (active) setError('No pudimos consultar el estado de la ventana.'); });
    return () => { active = false; ++revision.current; stop?.(); };
  }, [native]);

  async function control(action: WindowAction) {
    if (!native || pending.current) return;
    const request = ++revision.current;
    pending.current = true;
    setBusy(true); setError(null);
    try {
      const state = await invoke<boolean | null>('main_window_control', { action });
      if (state !== null && request === revision.current) setMaximized(state);
    } catch { setError('No pudimos controlar la ventana. Vuelve a intentarlo.'); }
    finally { pending.current = false; setBusy(false); }
  }

  return <>
    {separator && <span className="window-control-divider" role="separator" aria-orientation="vertical" />}
    <div className="window-controls" role="group" aria-label="Controles de la ventana">
      <Hint side="bottom" text={native ? 'Minimizar' : 'Minimizar · disponible en la aplicación de escritorio'}><span className="icon-control" tabIndex={native ? undefined : 0}><Button variant="ghost" size="icon" className="bg-transparent hover:bg-transparent hover:text-foreground" aria-label="Minimizar" disabled={!native || busy} onClick={() => void control('minimize')}><Minus aria-hidden="true" /></Button></span></Hint>
      <Hint side="bottom" text={native ? maximized ? 'Restaurar' : 'Maximizar' : 'Maximizar · disponible en la aplicación de escritorio'}><span className="icon-control" tabIndex={native ? undefined : 0}><Button variant="ghost" size="icon" className="bg-transparent hover:bg-transparent hover:text-foreground" aria-label={maximized ? 'Restaurar' : 'Maximizar'} disabled={!native || busy} onClick={() => void control('toggle_maximize')}>{maximized ? <Copy aria-hidden="true" /> : <Square aria-hidden="true" />}</Button></span></Hint>
      <Hint side="bottom" text={native ? 'Cerrar' : 'Cerrar · disponible en la aplicación de escritorio'}><span className="icon-control" tabIndex={native ? undefined : 0}><Button variant="ghost" size="icon" className="window-close bg-transparent hover:bg-transparent hover:text-destructive" aria-label="Cerrar" disabled={!native || busy} onClick={() => void control('close')}><X aria-hidden="true" /></Button></span></Hint>
    </div>
    {error && <span className="window-control-error" role="alert">{error}</span>}
  </>;
}
