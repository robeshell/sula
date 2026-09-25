import { Checkbox } from "./ui/checkbox";
import { ModalFrame } from "./ModalFrame";
import { DialogTitle } from "./ui/dialog";
import { Button } from "./ui/button";
import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import type { ResidualCandidate } from "../store/appStore";

export type { ResidualCandidate };

export function CleanupSheet({
  candidates,
  onConfirm,
  onClose,
}: {
  candidates: ResidualCandidate[];
  onConfirm: (paths: string[]) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(candidates.map((c) => c.path)),
  );

  const totalSize = useMemo(
    () =>
      candidates
        .filter((c) => selected.has(c.path))
        .reduce((sum, c) => sum + (c.size || 0), 0),
    [candidates, selected],
  );

  const toggle = (path: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const toggleAll = (on: boolean) => {
    setSelected(on ? new Set(candidates.map((c) => c.path)) : new Set());
  };

  const basename = (p: string) => {
    const parts = p.split(/[/\\]/);
    return parts[parts.length - 1] || p;
  };

  return (
    <ModalFrame onClose={onClose} labelledBy="cleanup-title" className="sm:max-w-xl">
        <header className="kg-dialog-header shrink-0">
          <DialogTitle
            id="cleanup-title"
            className="truncate kg-type-section-title font-extrabold tracking-[-0.25px] text-fg"
          >
            {t("cleanup.title")}
          </DialogTitle>
          <p className="mt-2 kg-type-body leading-[1.45] text-fg-secondary">
            {t("cleanup.subtitle", { count: candidates.length })}
          </p>
        </header>

        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-hairline px-6 py-2">
          <label className="flex cursor-pointer items-center gap-2 kg-type-body-secondary text-fg-secondary">
            <Checkbox

              checked={selected.size === candidates.length && candidates.length > 0}
              onCheckedChange={(e) => toggleAll(e === true)}
            />
            {t("cleanup.selectAll")}
          </label>
          <span className="kg-type-caption text-fg-muted">
            {t("cleanup.selectedBytes", {
              count: selected.size,
              size: formatBytes(totalSize),
            })}
          </span>
        </div>

        <ul className="min-h-0 flex-1 overflow-auto px-3 py-2">
          {candidates.map((c) => (
            <li key={c.path}>
              <label className="flex cursor-pointer items-start gap-2.5 rounded-control px-2.5 py-2 hover:bg-fill-secondary/40">
                <Checkbox

                  className="mt-1"
                  checked={selected.has(c.path)}
                  onCheckedChange={() => toggle(c.path)}
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate kg-type-body-secondary font-semibold text-fg">
                    {basename(c.path)}
                  </span>
                  <span className="mt-0.5 block truncate kg-type-caption text-fg-muted" title={c.path}>
                    {c.itemTitle} · {c.path}
                  </span>
                </span>
              </label>
            </li>
          ))}
        </ul>

        <div className="kg-dialog-footer shrink-0">
          <Button variant="ghost" size="sm" type="button" onClick={onClose}>
            {t("settings.close")}
          </Button>
          <Button variant="destructive" size="sm"
            type="button"
            disabled={selected.size === 0}
            onClick={() => onConfirm([...selected])}
          >
            {t("cleanup.confirm")}
          </Button>
        </div>
    </ModalFrame>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
