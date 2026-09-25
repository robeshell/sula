import { useTranslation } from "react-i18next";
import { ListChecks } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import { useAppStore, type TaskSnapshot } from "../../store/appStore";
import { localizeUserMessage } from "../../lib/localizeMessage";

const isActive = (task: TaskSnapshot) => task.status === "pending" || task.status === "running";

function percent(task: TaskSnapshot): number {
  const p = task.progress;
  return p && p.total > 0 ? Math.min(100, (p.completed / p.total) * 100) : 0;
}

/**
 * Background work lives in the sidebar footer instead of floating over posters:
 * the running task shows its progress in place; the history is one click away.
 */
export function TaskActivity() {
  const { t } = useTranslation();
  const tasks = useAppStore((s) => s.tasks);
  const active = tasks.filter(isActive);
  const current = active.find((task) => task.status === "running") ?? active[0];

  return (
    <Popover>
      <PopoverTrigger asChild>
        {current ? (
          <button type="button" className="sl-task" aria-label={t("tasks.open")}>
            <span className="h">
              <span>{current.title}</span>
              {current.progress && current.progress.total > 0 ? (
                <span>{current.progress.completed} / {current.progress.total}</span>
              ) : null}
            </span>
            <span className="sl-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent(current))}>
              <b style={{ width: `${percent(current)}%` }} />
            </span>
            <span className="s">
              {active.length > 1
                ? t("tasks.moreActive", { n: active.length - 1 })
                : current.progress?.current || t(`tasks.status.${current.status}`)}
            </span>
          </button>
        ) : (
          <button type="button" className="sl-nav plain">
            <ListChecks aria-hidden />
            <span className="nm">{t("tasks.title")}</span>
          </button>
        )}
      </PopoverTrigger>
      <PopoverContent side="right" align="end" className="w-80">
        <header className="flex items-center justify-between border-b border-hairline px-3.5 py-2.5">
          <h2 className="kg-type-title">{t("tasks.title")}</h2>
          {active.length > 0 ? (
            <span className="kg-type-caption text-fg-secondary">{t("tasks.activeCount", { n: active.length })}</span>
          ) : null}
        </header>
        <div className="max-h-[min(420px,60vh)] overflow-auto p-1.5">
          {tasks.length === 0 ? (
            <p className="px-2 py-6 text-center kg-type-body-secondary text-fg-secondary">{t("tasks.empty")}</p>
          ) : (
            <ul className="grid gap-0.5">
              {tasks.map((task) => (
                <li key={task.id} className="grid gap-1 rounded-md px-2.5 py-2">
                  <div className="flex items-baseline justify-between gap-3">
                    <span className="min-w-0 truncate kg-type-list-title">{task.title}</span>
                    <span className="shrink-0 kg-type-caption text-fg-secondary">
                      {t(`tasks.status.${task.status}`, { defaultValue: task.status })}
                    </span>
                  </div>
                  {isActive(task) && task.progress ? (
                    <>
                      <span className="sl-bar"><b style={{ width: `${percent(task)}%` }} /></span>
                      <p className="truncate kg-type-caption text-fg-secondary">{task.progress.current}</p>
                    </>
                  ) : null}
                  {task.result ? <p className="kg-type-caption text-fg-secondary">{t("tasks.result", task.result)}</p> : null}
                  {task.errorMessage ? (
                    <p className="kg-type-caption text-error">{localizeUserMessage(task.errorMessage)}</p>
                  ) : null}
                </li>
              ))}
            </ul>
          )}
        </div>
      </PopoverContent>
    </Popover>
  );
}
