import { create } from 'zustand';
type Options = {
  title: string;
  description: string;
  /** Concrete entries the action will touch, listed under the description. */
  details?: string[];
  /** Label of the accept button; defaults to the delete wording. */
  confirmLabel?: string;
};
type Request = Options & { resolve: (confirmed: boolean) => void };
export const useConfirmation = create<{ request: Request | null }>(() => ({ request: null }));
export function confirmAction(options: Options): Promise<boolean> {
  useConfirmation.getState().request?.resolve(false);
  return new Promise(resolve => useConfirmation.setState({ request: { ...options, resolve } }));
}
export function resolveConfirmation(confirmed: boolean) {
  const request = useConfirmation.getState().request;
  useConfirmation.setState({ request: null });
  request?.resolve(confirmed);
}
