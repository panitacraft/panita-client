import { useCallback, useEffect, useRef, useState } from 'react';
import { CircleAlert, CircleCheck, LoaderCircle, X } from 'lucide-react';

/** A button inside a toast, for the details a one-line message cannot hold. */
export type NoticeAction = { label: string; onClick: () => void };
export type TransientNoticeState = { message: string; tone: 'success' | 'error' | 'progress'; action?: NoticeAction; dismiss?: () => void };
type Notice = TransientNoticeState;

/**
 * Keep confirmations outside the layout; repeat actions restart the expiry timer. A toast
 * with an action stays until the player dismisses it or another toast replaces it: it may be
 * the only way back to something that needs a decision.
 */
export function useTransientNotice() {
  const [notice, setNotice] = useState<Notice | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const clear = useCallback(() => { clearTimeout(timer.current); setNotice(null); }, []);
  const show = useCallback((message: string, tone: Notice['tone'] = 'success', action?: NoticeAction) => {
    clearTimeout(timer.current);
    setNotice({ message, tone, action, dismiss: action ? clear : undefined });
    if (tone !== 'progress' && !action) timer.current = setTimeout(() => setNotice(null), 4000);
  }, [clear]);
  useEffect(() => () => clearTimeout(timer.current), []);
  return { notice, show, clear };
}

export function TransientNotice({ notice }: { notice: Notice | null }) {
  return <div className="transient-notice-region" role="status" aria-live="polite" aria-atomic="true">
    {notice && <div className={`transient-notice notice-${notice.tone}`}>
      {notice.tone === 'progress' ? <LoaderCircle className="spin" aria-hidden="true" /> : notice.tone === 'success' ? <CircleCheck aria-hidden="true" /> : <CircleAlert aria-hidden="true" />}<span>{notice.message}</span>
      {notice.action && <span className="notice-actions">
        <button type="button" className="notice-action" onClick={notice.action.onClick}>{notice.action.label}</button>
        {notice.dismiss && <button type="button" className="notice-dismiss" aria-label="Cerrar aviso" onClick={notice.dismiss}><X aria-hidden="true" /></button>}
      </span>}
    </div>}
  </div>;
}
