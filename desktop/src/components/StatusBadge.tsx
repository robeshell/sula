import { useTranslation } from "react-i18next";
import { needsAttention } from "../lib/mediaList";

/**
 * One visual language for scrape state. On posters only items that need work get
 * a badge; lists and the inspector always show a dot + label.
 */
export function PosterBadge({ status }: { status: string }) {
  const { t } = useTranslation();
  if (!needsAttention(status)) return null;
  return (
    <span className="sl-badge" data-status={status}>
      <i aria-hidden />
      {t(`status.${status}`, { defaultValue: status })}
    </span>
  );
}

export function StatusText({ status }: { status: string }) {
  const { t } = useTranslation();
  return (
    <span className="sl-state" data-status={status}>
      <i aria-hidden />
      {t(`status.${status}`, { defaultValue: status })}
    </span>
  );
}
