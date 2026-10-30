import { useState, type MouseEvent, type ReactNode } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';

/** Keep interactive controls outside this region; only the brand and empty header drag. */
export function WindowDragRegion({ children }: { children: ReactNode }) {
  const [failed, setFailed] = useState(false);
  function begin(event: MouseEvent<HTMLDivElement>) {
    if (!isTauri() || event.button !== 0) return;
    event.preventDefault();
    setFailed(false);
    void invoke('main_window_control', { action: event.detail === 2 ? 'toggle_maximize' : 'drag' }).catch(() => setFailed(true));
  }
  return <div className="window-drag-region" onMouseDown={begin}>
    {children}
    {failed && <span className="window-control-error" role="alert">No pudimos mover la ventana. Vuelve a intentarlo.</span>}
  </div>;
}
