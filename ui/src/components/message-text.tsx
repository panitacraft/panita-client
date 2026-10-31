import { Fragment } from 'react';

/** Player-facing text in which file names such as panita-session.json are shown as inline code. */
export function MessageText({ text }: { text: string }) {
  return <>{text.split(/([\w.-]+\.json)\b/).map((part, index) =>
    index % 2 === 1 ? <code key={index} className="inline-code">{part}</code> : <Fragment key={index}>{part}</Fragment>)}</>;
}
