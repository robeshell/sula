import { create } from 'zustand';
type Request = { title: string; description: string; resolve: (confirmed: boolean) => void };
export const useConfirmation = create<{ request: Request | null }>(() => ({ request: null }));
export function confirmAction(options: { title: string; description: string }): Promise<boolean> {
  useConfirmation.getState().request?.resolve(false);
  return new Promise(resolve => useConfirmation.setState({ request: { ...options, resolve } }));
}
export function resolveConfirmation(confirmed: boolean) {
  const request = useConfirmation.getState().request;
  useConfirmation.setState({ request: null });
  request?.resolve(confirmed);
}
