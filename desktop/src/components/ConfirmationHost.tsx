import { useTranslation } from 'react-i18next';
import { useConfirmation, resolveConfirmation } from '../lib/confirmation';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogCancel, AlertDialogAction } from './ui/alert-dialog';
export function ConfirmationHost() {
  const { t } = useTranslation();
  const request = useConfirmation(s => s.request);
  const details = request?.details ?? [];
  return <AlertDialog open={Boolean(request)} onOpenChange={open => { if (!open) resolveConfirmation(false); }}>
    <AlertDialogContent>
      <AlertDialogHeader><AlertDialogTitle>{request?.title}</AlertDialogTitle><AlertDialogDescription>{request?.description}</AlertDialogDescription></AlertDialogHeader>
      {details.length > 0 && <ul className="max-h-48 overflow-y-auto rounded-md border border-border px-3 py-2 text-sm text-muted-foreground">
        {details.map((line, index) => <li key={index} className="truncate py-0.5" title={line}>{line}</li>)}
      </ul>}
      <AlertDialogFooter>
        <AlertDialogCancel onClick={() => resolveConfirmation(false)}>{t('common.cancel')}</AlertDialogCancel>
        <AlertDialogAction variant="destructive" onClick={() => resolveConfirmation(true)}>{request?.confirmLabel ?? t('delete.confirm')}</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>;
}
