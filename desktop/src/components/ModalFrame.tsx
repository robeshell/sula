import { useRef, type ReactNode } from 'react';
import { motion } from 'motion/react';
import { Dialog, DialogOverlay, DialogPortal } from './ui/dialog';
import { Dialog as DialogPrimitive } from 'radix-ui';
import { cn } from '../lib/utils';

export function ModalFrame({ children, onClose, labelledBy, className }: {
  children: ReactNode; onClose: () => void; labelledBy: string; className?: string;
}) {
  const previousFocus = useRef(document.activeElement);
  return <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
    <DialogPortal><DialogOverlay />
    <DialogPrimitive.Content asChild data-slot="dialog-content" aria-labelledby={labelledBy} aria-describedby={undefined}
      onCloseAutoFocus={event => {
        event.preventDefault();
        const previous = previousFocus.current;
        if (previous instanceof HTMLElement && previous.isConnected) previous.focus();
        else document.querySelector<HTMLElement>('.kg-poster-card[data-selected="true"], .kg-list-row[data-selected="true"]')?.focus();
      }}
      className={cn('fixed top-1/2 left-1/2 z-50 flex w-[calc(100%-2rem)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border bg-background p-0 shadow-xl outline-none sm:max-w-xl', className)}>
      <motion.div initial={{ opacity: 0, scale: 0.98, y: 6 }} animate={{ opacity: 1, scale: 1, y: 0 }}>
        {children}
      </motion.div>
    </DialogPrimitive.Content></DialogPortal>
  </Dialog>;
}
