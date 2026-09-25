import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { motion, AnimatePresence } from 'motion/react';
import { Button } from './ui/button';
import { Progress } from './ui/progress';
import { useAppStore } from '../store/appStore';
import { localizeUserMessage } from '../lib/localizeMessage';
export function TasksDock({
  open,
  onOpenChange,
  activeCount,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  activeCount: number;
}) {
  const { t } = useTranslation();
  const tasks = useAppStore((s) => s.tasks);
  const dockRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      const el = dockRef.current;
      if (el && !el.contains(e.target as Node)) {
        onOpenChange(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onOpenChange(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onOpenChange]);

  return (
    <div ref={dockRef} className="kg-tasks-dock">
      <AnimatePresence>
      {open ? (
        <motion.div initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0, y: 6 }}
          className="kg-tasks-panel border border-border bg-popover shadow-xl"
          role="dialog"
          aria-label={t("tasks.title")}
        >
          <header className="flex items-center justify-between gap-2 border-b border-hairline px-3.5 py-2.5">
            <h2 className="min-w-0 kg-type-body font-semibold text-fg">{t("tasks.title")}</h2>
            <Button variant="ghost" size="sm"
              type="button"
              onClick={() => onOpenChange(false)}
            >
              {t("tasks.close")}
            </Button>
          </header>
          <div className="min-h-0 flex-1 overflow-auto px-2 py-2">
            {tasks.length === 0 ? (
              <p className="px-2 py-6 text-center kg-type-caption text-fg-secondary">
                {t("tasks.empty")}
              </p>
            ) : (
              <ul className="space-y-0.5">
                {tasks.map((task) => (
                  <li key={task.id} className="rounded-control px-2.5 py-2 kg-type-caption">
                    <div className="flex justify-between gap-2">
                      <span className="min-w-0 flex-1 font-semibold text-fg">{task.title}</span>
                      <span className="shrink-0 text-fg-muted">
                        {t(`tasks.status.${task.status}`, { defaultValue: task.status })}
                      </span>
                    </div>
                    {task.progress ? (
                      <div className="mt-2 space-y-2"><Progress value={task.progress.total > 0 ? task.progress.completed / task.progress.total * 100 : 0} aria-label={task.title} /><p className="truncate text-fg-secondary">{task.progress.current}</p></div>
                    ) : null}
                    {task.result ? <p className="mt-1 text-fg-secondary">{t("tasks.result", task.result)}</p> : null}
                    {task.errorMessage ? (
                      <p className="mt-1 text-error">
                        {localizeUserMessage(task.errorMessage)}
                      </p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </motion.div>
      ) : null}
      </AnimatePresence>
      <Button variant="plain" size="none"
        type="button"
        className="kg-tasks-trigger border border-border bg-popover shadow-sm"
        data-active={activeCount > 0 || open}
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
      >
        <span>{t("tasks.title")}</span>
        {activeCount > 0 ? (
          <span className="kg-tasks-badge">{activeCount}</span>
        ) : null}
      </Button>
    </div>
  );
}
