import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

/** Grid/list geometry; keep in step with .sl-poster / .sl-row in styles/native.css. */
const POSTER = { minWidth: 146, gapX: 22, gapY: 26, caption: 40, padX: 26, padY: 22 };
const LIST = { rowHeight: 44, padX: 12, padY: 8, header: 26 };

/** Bound mounted rows; arrow keys can traverse rows outside the viewport. */
export function VirtualMediaList<T extends { id: string }>({ items, mode, resetKey, header, children }: {
  items: T[]; mode: 'poster' | 'list'; resetKey: string; header?: ReactNode; children: (item: T, index: number) => ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ width: 600, height: 600, top: 0 });
  useLayoutEffect(() => {
    const node = ref.current!;
    const update = () => setView({ width: node.clientWidth, height: node.clientHeight, top: node.scrollTop });
    const observer = new ResizeObserver(update);
    observer.observe(node); node.addEventListener('scroll', update, { passive: true }); update();
    return () => { observer.disconnect(); node.removeEventListener('scroll', update); };
  }, []);
  useLayoutEffect(() => { if (ref.current) { ref.current.scrollTop = 0; setView(v => ({ ...v, top: 0 })); } }, [resetKey, mode]);
  const poster = mode === 'poster';
  const padX = poster ? POSTER.padX : LIST.padX;
  const padY = poster ? POSTER.padY : LIST.padY;
  const headerHeight = !poster && header ? LIST.header : 0;
  const width = Math.max(1, view.width - padX * 2);
  const gapX = poster ? POSTER.gapX : 0;
  const columns = poster ? Math.max(1, Math.floor((width + gapX) / (POSTER.minWidth + gapX))) : 1;
  const itemWidth = (width - (columns - 1) * gapX) / columns;
  const rowHeight = poster ? itemWidth * 1.5 + POSTER.caption + POSTER.gapY : LIST.rowHeight;
  const rows = Math.ceil(items.length / columns);
  const offset = padY + headerHeight;
  const first = Math.max(0, Math.floor((view.top - offset) / rowHeight) - 3) * columns;
  const end = Math.min(items.length, (Math.ceil((view.top + view.height - offset) / rowHeight) + 3) * columns);
  return <div ref={ref} style={{ height: '100%', overflow: 'auto' }} onKeyDown={(event) => {
    const current = (event.target as HTMLElement).closest<HTMLElement>('[data-virtual-index]');
    if (!current) return;
    const index = Number(current.dataset.virtualIndex);
    const delta = event.key === 'ArrowDown' ? columns : event.key === 'ArrowUp' ? -columns : event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!delta && event.key !== 'Home' && event.key !== 'End') return;
    event.preventDefault();
    const target = Math.max(0, Math.min(items.length - 1, event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : index + delta));
    const top = offset + Math.floor(target / columns) * rowHeight;
    const node = ref.current!;
    if (top < node.scrollTop + offset || top + rowHeight > node.scrollTop + node.clientHeight) node.scrollTop = top - offset;
    setView(v => ({ ...v, top: node.scrollTop }));
    requestAnimationFrame(() => node.querySelector<HTMLElement>(`[data-virtual-index="${target}"] button`)?.focus());
  }}>
    {headerHeight ? (
      <div style={{ position: 'sticky', top: 0, zIndex: 2, padding: `0 ${padX}px`, background: 'var(--kg-canvas)' }}>{header}</div>
    ) : null}
    <div role="list" style={{ position: 'relative', height: rows * rowHeight + padY * 2 }}>
      {items.slice(first, end).map((item, i) => {
        const index = first + i;
        return <div key={item.id} role="listitem" aria-setsize={items.length} aria-posinset={index + 1} data-virtual-index={index}
          style={{ position: 'absolute', left: padX + (index % columns) * (itemWidth + gapX), top: padY + Math.floor(index / columns) * rowHeight, width: itemWidth }}>
          {children(item, index)}
        </div>;
      })}
    </div>
  </div>;
}
