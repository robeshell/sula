import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./ui/dropdown-menu";
import { useTranslation } from "react-i18next";

export type MediaContextMenuTarget = {
  x: number;
  y: number;
  itemId: string;
};

type MediaContextMenuProps = {
  menu: MediaContextMenuTarget;
  canScrapeAuto: boolean;
  canRescrape: boolean;
  canManualMatch: boolean;
  canRename: boolean;
  canOrganize: boolean;
  canMergeDuplicates: boolean;
  canCleanResiduals: boolean;
  canDelete: boolean;
  onClose: () => void;
  /** Close menu and keep the right-click selection (for the chosen action). */
  onAction: () => void;
  onScrapeConfirm: () => void;
  onScrapeAuto: () => void;
  onRescrape: () => void;
  onManualMatch: () => void;
  onRename: () => void;
  onOrganize: () => void;
  onMergeDuplicates: () => void;
  onCleanResiduals: () => void;
  onRefreshFromDisk: () => void;
  onReveal: () => void;
  onDelete: () => void;
};

export function MediaContextMenu({
  menu,
  canScrapeAuto,
  canRescrape,
  canManualMatch,
  canRename,
  canOrganize,
  canMergeDuplicates,
  canCleanResiduals,
  canDelete,
  onClose,
  onAction,
  onScrapeConfirm,
  onScrapeAuto,
  onRescrape,
  onManualMatch,
  onRename,
  onOrganize,
  onMergeDuplicates,
  onCleanResiduals,
  onRefreshFromDisk,
  onReveal,
  onDelete,
}: MediaContextMenuProps) {
  const { t } = useTranslation();
  const run = (action: () => void) => {
    onAction();
    action();
  };

  return (
    <DropdownMenu open modal={false} onOpenChange={open => { if (!open) onClose(); }}>
      <DropdownMenuTrigger asChild><span aria-hidden style={{ position: 'fixed', left: menu.x, top: menu.y, width: 1, height: 1 }} /></DropdownMenuTrigger>
      <DropdownMenuContent align="start" sideOffset={0} collisionPadding={8} onCloseAutoFocus={e => e.preventDefault()}>
      {canManualMatch ? (
        <DropdownMenuItem

          onSelect={() => run(onScrapeConfirm)}
        >
          {t("action.scrapeItem")}
        </DropdownMenuItem>
      ) : null}
      {canScrapeAuto ? (
        <DropdownMenuItem

          onSelect={() => run(onScrapeAuto)}
        >
          {t("action.scrapeAuto")}
        </DropdownMenuItem>
      ) : null}
      {canRescrape ? (
        <DropdownMenuItem

          onSelect={() => run(onRescrape)}
        >
          {t("action.rescrape")}
        </DropdownMenuItem>
      ) : null}
      {canManualMatch ? (
        <DropdownMenuItem

          onSelect={() => run(onManualMatch)}
        >
          {t("action.manualMatch")}
        </DropdownMenuItem>
      ) : null}
      {canRename ? (
        <DropdownMenuItem

          onSelect={() => run(onRename)}
        >
          {t("action.applyRename")}
        </DropdownMenuItem>
      ) : null}
      {canOrganize ? (
        <DropdownMenuItem

          onSelect={() => run(onOrganize)}
        >
          {t("action.organizeSeasons")}
        </DropdownMenuItem>
      ) : null}
      {canMergeDuplicates ? (
        <DropdownMenuItem

          onSelect={() => run(onMergeDuplicates)}
        >
          {t("action.mergeDuplicates")}
        </DropdownMenuItem>
      ) : null}
      {canCleanResiduals ? (
        <DropdownMenuItem

          onSelect={() => run(onCleanResiduals)}
        >
          {t("action.cleanResiduals")}
        </DropdownMenuItem>
      ) : null}
      <DropdownMenuItem

        onSelect={() => run(onRefreshFromDisk)}
      >
        {t("action.refreshFromDisk")}
      </DropdownMenuItem>
      <DropdownMenuItem

        onSelect={() => run(onReveal)}
      >
        {t("action.revealInFinder")}
      </DropdownMenuItem>
      {canDelete ? (
        <DropdownMenuItem

          variant="destructive"
          onSelect={() => run(onDelete)}
        >
          {t("action.deleteItem")}
        </DropdownMenuItem>
      ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
