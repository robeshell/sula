import { AnimatePresence, motion } from 'motion/react';
import { useAppStore } from '../store/appStore';
export function ToastHost() {
  const message = useAppStore(s => s.toastMessage);
  return <div className="pointer-events-none fixed inset-x-0 bottom-12 z-[100] flex justify-center px-4" role="status" aria-live="polite">
    <AnimatePresence mode="wait">{message && <motion.div key={message}
      initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 4 }}
      className="max-w-lg rounded-lg border border-border bg-popover px-4 py-3 text-sm text-popover-foreground shadow-lg">{message}</motion.div>}
    </AnimatePresence>
  </div>;
}
