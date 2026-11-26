import { useEffect, useId, useRef, type ReactNode } from 'react';

/** `aside` sits beside the title (a version chip, for example) without becoming part of the dialog's name. */
export function AccountDialog({ title, children, onCancel, className = '', titleClassName, aside }: { title: ReactNode; children: ReactNode; onCancel: () => void; className?: string; titleClassName?: string; aside?: ReactNode }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => { dialog.current?.showModal(); }, []);
  const heading = <h2 id={titleId} className={titleClassName}>{title}</h2>;
  return <dialog ref={dialog} className={`account-dialog ${className}`.trim()} aria-labelledby={titleId} onCancel={event => { event.preventDefault(); onCancel(); }}>
    {aside ? <div className="update-head">{heading}{aside}</div> : heading}{children}
  </dialog>;
}
