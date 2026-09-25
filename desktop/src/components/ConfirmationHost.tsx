import { useTranslation } from 'react-i18next';
import { useConfirmation, resolveConfirmation } from '../lib/confirmation';
import { AlertDialog, AlertDialogContent, AlertDialogHeader, AlertDialogTitle, AlertDialogDescription, AlertDialogFooter, AlertDialogCancel, AlertDialogAction } from './ui/alert-dialog';
export function ConfirmationHost() {
  const { t } = useTranslation();
  const request = useConfirmation(s => s.request);
  return <AlertDialog open={Boolean(request)} onOpenChange={open => { if (!open) resolveConfirmation(false); }}>
    <AlertDialogContent>
      <AlertDialogHeader><AlertDialogTitle>{request?.title}</AlertDialogTitle><AlertDialogDescription>{request?.description}</AlertDialogDescription></AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel onClick={() => resolveConfirmation(false)}>{t('settings.close')}</AlertDialogCancel>
        <AlertDialogAction variant="destructive" onClick={() => resolveConfirmation(true)}>{t('delete.confirm')}</AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>;
}
