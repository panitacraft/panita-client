import type { ReactElement, ReactNode } from 'react';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

/** Wrap a disabled control in a focusable span when its explanation must be reachable. */
export function Hint({ children, text, side = 'top' }: { children: ReactElement; text: ReactNode; side?: 'top' | 'right' | 'bottom' | 'left' }) {
  return <Tooltip><TooltipTrigger asChild>{children}</TooltipTrigger><TooltipContent side={side} className="hint-content border border-border bg-secondary text-foreground">{text}</TooltipContent></Tooltip>;
}
