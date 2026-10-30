import { useCallback, useEffect, useRef, useState } from 'react';
import { CircleAlert, CircleCheck, LoaderCircle } from 'lucide-react';

type Notice = { message: string; tone: 'success' | 'error' | 'progress' };

/** Keep confirmations outside the layout; repeat actions restart the expiry timer. */
export function useTransientNotice() {
  const [notice, setNotice] = useState<Notice | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const clear = useCallback(() => { clearTimeout(timer.current); setNotice(null); }, []);
  const show = useCallback((message: string, tone: Notice['tone'] = 'success') => {
    clearTimeout(timer.current);
    setNotice({ message, tone });
    if (tone !== 'progress') timer.current = setTimeout(() => setNotice(null), 4000);
  }, []);
  useEffect(() => () => clearTimeout(timer.current), []);
  return { notice, show, clear };
}

export function TransientNotice({ notice }: { notice: Notice | null }) {
  return <div className="transient-notice-region" role="status" aria-live="polite" aria-atomic="true">
    {notice && <div className={`transient-notice notice-${notice.tone}`}>
      {notice.tone === 'progress' ? <LoaderCircle className="spin" aria-hidden="true" /> : notice.tone === 'success' ? <CircleCheck aria-hidden="true" /> : <CircleAlert aria-hidden="true" />}<span>{notice.message}</span>
    </div>}
  </div>;
}
