import { useEffect, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Search, LayoutGrid, List, X } from 'lucide-react';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';
import { Tabs, TabsList, TabsTrigger } from './ui/tabs';
import { Badge } from './ui/badge';
import { SORT_OPTIONS, STATUS_FILTERS } from '../lib/mediaList';

const SEARCH_DEBOUNCE_MS = 150;

type Status = typeof STATUS_FILTERS[number]['value'];
type Sort = typeof SORT_OPTIONS[number]['value'];
export function LibraryToolbar({ title, path, count, total, query, onQuery, status, onStatus, sort, onSort, view, onView, actions }: {
  title: string; path: string; count: number; total: number; query: string; onQuery: (v: string) => void;
  status: Status; onStatus: (v: Status) => void; sort: Sort; onSort: (v: Sort) => void;
  view: 'poster' | 'list'; onView: (v: 'poster' | 'list') => void; actions: ReactNode;
}) {
  const { t } = useTranslation();
  // Filtering + sorting a large library per keystroke is costly: type into a
  // local draft and publish it after a short pause.
  const [draft, setDraft] = useState(query);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const published = useRef(query);
  const onQueryRef = useRef(onQuery);
  onQueryRef.current = onQuery;
  const cancelPending = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
  };
  useEffect(() => {
    // Our own publish echoing back must not undo newer typing.
    if (query === published.current) return;
    // External changes (reset after a refresh) win over a pending draft.
    published.current = query;
    cancelPending();
    setDraft(query);
  }, [query]);
  useEffect(() => cancelPending, []);
  const changeDraft = (value: string) => {
    setDraft(value);
    cancelPending();
    timer.current = setTimeout(() => {
      timer.current = null;
      published.current = value;
      onQueryRef.current(value);
    }, SEARCH_DEBOUNCE_MS);
  };
  const clearSearch = () => {
    cancelPending();
    setDraft('');
    published.current = '';
    onQuery('');
  };
  return <header className="kg-library-header">
    <div className="flex min-w-0 flex-wrap items-center gap-3">
      <div className="min-w-[180px] flex-1">
        <div className="flex items-center gap-2.5"><h1 className="truncate kg-type-page-title">{title}</h1><Badge variant="secondary" className="tabular-nums">{count} / {total}</Badge></div>
        <p className="mt-1 truncate text-xs text-muted-foreground" title={path}>{path}</p>
      </div>
      {actions}
    </div>
    <div className="mt-5 flex flex-wrap items-center gap-2">
      <div className="relative min-w-40 flex-1 max-w-xl">
        <Search className="pointer-events-none absolute left-3 top-2.5 size-4 text-muted-foreground" />
        <Input aria-label={t('list.searchPlaceholder')} placeholder={t('list.searchPlaceholder')} value={draft} onChange={e => changeDraft(e.target.value)} className="pl-9 pr-9" />
        {draft && <Button variant="ghost" size="icon-sm" className="absolute right-0.5 top-0.5" aria-label={t('list.clearSearch')} onClick={clearSearch}><X /></Button>}
      </div>
      <Select value={status} onValueChange={value => onStatus(value as Status)}>
        <SelectTrigger aria-label={t('filter.status.all')}><SelectValue /></SelectTrigger>
        <SelectContent>{STATUS_FILTERS.map(option => <SelectItem key={option.value} value={option.value}>{t(`filter.status.${option.value}`)}</SelectItem>)}</SelectContent>
      </Select>
      <Select value={sort} onValueChange={value => onSort(value as Sort)}>
        <SelectTrigger aria-label={t('list.sort')}><SelectValue /></SelectTrigger>
        <SelectContent>{SORT_OPTIONS.map(option => <SelectItem key={option.value} value={option.value}>{t(`filter.sort.${option.value}`)}</SelectItem>)}</SelectContent>
      </Select>
      <Tabs value={view} onValueChange={value => onView(value as 'poster' | 'list')}>
        <TabsList aria-label={t('list.viewMode')}>
          <TabsTrigger value="poster" aria-label={t('list.viewPoster')}><LayoutGrid className="size-4" /></TabsTrigger>
          <TabsTrigger value="list" aria-label={t('list.viewList')}><List className="size-4" /></TabsTrigger>
        </TabsList>
      </Tabs>
    </div>
  </header>;
}
