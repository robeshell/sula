import { Checkbox } from "./ui/checkbox";
import { ModalFrame } from "./ModalFrame";
import { DialogTitle } from "./ui/dialog";
import { Button } from "./ui/button";
import { useState } from "react";
import { useTranslation } from "react-i18next";

export function DeleteConfirmModal({
  title,
  onConfirm,
  onClose,
}: {
  title: string;
  onConfirm: (alsoTrash: boolean) => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [alsoTrash, setAlsoTrash] = useState(false);

  return (
    <ModalFrame onClose={onClose} labelledBy="delete-confirm-title" className="sm:max-w-md">
        <header className="kg-dialog-header">
          <DialogTitle
            id="delete-confirm-title"
            className="truncate kg-type-section-title font-extrabold tracking-[-0.25px] text-fg"
          >
            {t("delete.title")}
          </DialogTitle>
          <p className="mt-2 kg-type-body leading-[1.45] text-fg-secondary">
            {t("delete.message", { title })}
          </p>
        </header>
        <div className="px-6 pb-2">
          <label className="flex cursor-pointer items-start gap-2.5 rounded-control px-1 py-2">
            <Checkbox

              checked={alsoTrash}
              onCheckedChange={(e) => setAlsoTrash(e === true)}
              className="mt-0.5"
            />
            <span className="min-w-0">
              <span className="block kg-type-body font-semibold text-fg">
                {t("delete.alsoTrash")}
              </span>
              <span className="mt-0.5 block kg-type-caption leading-[1.45] text-fg-muted">
                {t("delete.alsoTrashHint")}
              </span>
            </span>
          </label>
        </div>
        <div className="kg-dialog-footer">
          <Button variant="ghost" size="sm" type="button" onClick={onClose}>
            {t("settings.close")}
          </Button>
          <Button variant="destructive" size="sm"
            type="button"
            onClick={() => onConfirm(alsoTrash)}
          >
            {t("delete.confirm")}
          </Button>
        </div>
    </ModalFrame>
  );
}
