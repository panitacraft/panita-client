import { useEffect, useId, useRef, type ReactNode } from 'react';

export function AccountDialog({ title, children, onCancel, className = '', titleClassName }: { title: ReactNode; children: ReactNode; onCancel: () => void; className?: string; titleClassName?: string }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => { dialog.current?.showModal(); }, []);
  return <dialog ref={dialog} className={`account-dialog ${className}`.trim()} aria-labelledby={titleId} onCancel={event => { event.preventDefault(); onCancel(); }}>
    <h2 id={titleId} className={titleClassName}>{title}</h2>{children}
  </dialog>;
}
