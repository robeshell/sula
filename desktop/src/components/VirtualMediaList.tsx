import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

/** Bound mounted rows; arrow keys can traverse rows outside the viewport. */
export function VirtualMediaList<T extends { id: string }>({ items, mode, resetKey, children }: {
  items: T[]; mode: 'poster' | 'list'; resetKey: string; children: (item: T) => ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ width: 600, height: 600, top: 0, gutter: 16 });
  useLayoutEffect(() => {
    const node = ref.current!;
    const update = () => setView({ width: node.clientWidth, height: node.clientHeight, top: node.scrollTop,
      gutter: Number.parseFloat(getComputedStyle(node).getPropertyValue('--kg-page-gutter')) || 16 });
    const observer = new ResizeObserver(update);
    observer.observe(node); node.addEventListener('scroll', update, { passive: true }); update();
    return () => { observer.disconnect(); node.removeEventListener('scroll', update); };
  }, []);
  useLayoutEffect(() => { if (ref.current) { ref.current.scrollTop = 0; setView(v => ({ ...v, top: 0 })); } }, [resetKey, mode]);
  const gutter = mode === 'poster' ? view.gutter : 0;
  const width = Math.max(1, view.width - gutter * 2);
  const gap = mode === 'poster' ? 16 : 0;
  const columns = mode === 'poster' ? Math.max(1, Math.floor((width + gap) / (128 + gap))) : 1;
  const itemWidth = (width - (columns - 1) * gap) / columns;
  const rowHeight = mode === 'poster' ? itemWidth * 1.5 + 68 : 54;
  const rows = Math.ceil(items.length / columns);
  const first = Math.max(0, Math.floor(view.top / rowHeight) - 3) * columns;
  const end = Math.min(items.length, (Math.ceil((view.top + view.height) / rowHeight) + 3) * columns);
  return <div ref={ref} style={{ height: '100%', overflow: 'auto' }} onKeyDown={(event) => {
    const current = (event.target as HTMLElement).closest<HTMLElement>('[data-virtual-index]');
    if (!current) return;
    const index = Number(current.dataset.virtualIndex);
    const delta = event.key === 'ArrowDown' ? columns : event.key === 'ArrowUp' ? -columns : event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!delta && event.key !== 'Home' && event.key !== 'End') return;
    event.preventDefault();
    const target = Math.max(0, Math.min(items.length - 1, event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : index + delta));
    const top = Math.floor(target / columns) * rowHeight;
    const node = ref.current!;
    if (top < node.scrollTop || top + rowHeight > node.scrollTop + node.clientHeight) node.scrollTop = top;
    setView(v => ({ ...v, top: node.scrollTop }));
    requestAnimationFrame(() => node.querySelector<HTMLElement>(`[data-virtual-index="${target}"] button`)?.focus());
  }}>
    <div role="list" style={{ position: 'relative', height: rows * rowHeight + 20 }}>
      {items.slice(first, end).map((item, offset) => {
        const index = first + offset;
        return <div key={item.id} role="listitem" aria-setsize={items.length} aria-posinset={index + 1} data-virtual-index={index}
          style={{ position: 'absolute', left: gutter + (index % columns) * (itemWidth + gap), top: Math.floor(index / columns) * rowHeight + 4, width: itemWidth }}>
          {children(item)}
        </div>;
      })}
    </div>
  </div>;
}
