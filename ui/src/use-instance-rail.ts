import { useLayoutEffect, useRef, useState } from 'react';
import { instanceRailCapacity } from './launcher-state';

export function useInstanceRail(count: number) {
  const rail = useRef<HTMLElement>(null);
  const [capacity, setCapacity] = useState(6);
  useLayoutEffect(() => {
    const element = rail.current;
    if (!element) return;
    const measure = () => {
      const styles = getComputedStyle(element);
      const height = element.clientHeight - parseFloat(styles.paddingTop || '0') - parseFloat(styles.paddingBottom || '0');
      if (height > 0) setCapacity(instanceRailCapacity(height, count));
    };
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(element);
    window.addEventListener('resize', measure);
    measure();
    return () => { observer?.disconnect(); window.removeEventListener('resize', measure); };
  }, [count]);
  return { rail, capacity };
}
