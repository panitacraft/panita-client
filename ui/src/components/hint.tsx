import type { ReactElement, ReactNode } from 'react';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

/** Wrap a disabled control in a focusable span when its explanation must be reachable. */
export function Hint({ children, text, side = 'top', sideOffset = 4 }: { children: ReactElement; text: ReactNode; side?: 'top' | 'right' | 'bottom' | 'left'; sideOffset?: number }) {
  return <Tooltip><TooltipTrigger asChild>{children}</TooltipTrigger><TooltipContent side={side} sideOffset={sideOffset} className="hint-content border border-border bg-secondary text-foreground">{text}</TooltipContent></Tooltip>;
}
