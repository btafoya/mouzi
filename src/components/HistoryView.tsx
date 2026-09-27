import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { ActionLog, useAppStore } from '../store/useAppStore';
import { buttonClass, fieldClass, primaryClass, errorText, OperationResult, Results } from './BetaControls';

const emptyFilters = { query: '', rule: '', extension: '', from: '', to: '', run_id: '' };
export default function HistoryView() {
  const { t } = useTranslation();
  const [filters, setFilters] = useState(emptyFilters);
  const [logs, setLogs] = useState<ActionLog[]>([]);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [clear, setClear] = useState(false);
  const [limit, setLimit] = useState(100);
  const [results, setResults] = useState<OperationResult[]>([]);
  const eligible = (log: ActionLog) => !log.undone && !!log.fingerprint && ['move', 'rename'].includes(log.action);
  const load = async (value = filters) => {
    setBusy(true); setError('');
    try {
      if (value.from && value.to && value.from > value.to) throw new Error(t('beta.validation.date'));
      setLogs(await invoke<ActionLog[]>('history_cmd', { filter: { ...value, from: value.from ? new Date(value.from).toISOString() : '', to: value.to ? new Date(value.to).toISOString() : '' } }));
      setSelected(new Set()); setConfirm(false); setLimit(100);
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); }
  };
  useEffect(() => { void load(emptyFilters); }, []);
  const toggle = (id: number) => setSelected(previous => { const next = new Set(previous); next.has(id) ? next.delete(id) : next.add(id); return next; });
  const undo = async () => {
    setBusy(true); setError('');
    try {
      setResults(await invoke<OperationResult[]>('undo_selected_cmd', { ids: [...selected] }));
      await load(); await useAppStore.getState().loadLogs(); await useAppStore.getState().loadStats();
    } catch (e) { setError(errorText(e)); } finally { setBusy(false); setConfirm(false); }
  };
  const groups = new Map<string, ActionLog[]>();
  for (const log of logs.slice(0, limit)) { const key = log.run_id || 'legacy'; groups.set(key, [...(groups.get(key) || []), log]); }
  return <div className="space-y-4">
    <h2 className="text-lg font-semibold">{t('settings.history.title')}</h2>
    <form className="grid grid-cols-1 sm:grid-cols-2 gap-3" onSubmit={e => { e.preventDefault(); void load(); }}>
      {(['query', 'rule', 'extension', 'from', 'to'] as const).map(key => <label key={key} className="text-sm">{t(`beta.${key === 'query' ? 'search' : key}`)}
        <input className={fieldClass} type={key === 'from' || key === 'to' ? 'datetime-local' : 'text'} value={filters[key]} onChange={e => setFilters({ ...filters, [key]: e.target.value })} />
      </label>)}
      <div className="flex items-end gap-2"><button className={buttonClass} disabled={busy}>{t('beta.filter')}</button><button type="button" className={buttonClass} disabled={busy} onClick={() => { setFilters(emptyFilters); void load(emptyFilters); }}>{t('beta.reset')}</button></div>
    </form>
    {error && <p role="alert" className="text-red-700 dark:text-red-300">{error}</p>}
    <Results results={results} />
    <div className="flex flex-wrap gap-2 items-center"><button className={buttonClass} disabled={busy} onClick={() => setSelected(new Set(logs.filter(eligible).map(log => log.id!)))}>{t('beta.selectAll')}</button><button className={buttonClass} onClick={() => { setSelected(new Set()); setConfirm(false); }}>{t('beta.clearSelection')}</button><span>{t('beta.selected', { count: selected.size })}</span><button className={primaryClass} disabled={busy || !selected.size} onClick={() => setConfirm(true)}>{t('beta.undoSelection')}</button></div>
    {confirm && <section className="border border-primary rounded-md p-3 space-y-3" aria-label={t('beta.undoSelection')}><p>{t('beta.undoHelp')}</p><ul className="max-h-48 overflow-auto text-sm">{logs.filter(log => selected.has(log.id!)).map(log => <li key={log.id} className="break-all">{log.source_path}</li>)}</ul><button className={primaryClass} disabled={busy} onClick={undo}>{t('beta.confirmUndo', { count: selected.size })}</button><button className={`${buttonClass} ml-2`} onClick={() => setConfirm(false)} disabled={busy}>{t('beta.cancel')}</button></section>}
    {!logs.length && <p>{t('beta.noHistory')}</p>}
    {[...groups].map(([key, entries]) => <section key={key} className="border border-border rounded-lg p-3 space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2"><h3 className="text-sm font-medium">{key === 'legacy' ? t('beta.legacy') : `${t('beta.run')} · ${t(`beta.${entries[0].trigger}`, { defaultValue: entries[0].trigger })}`}<span className="block text-xs text-text-muted">{new Date(entries[0].timestamp).toLocaleString()}</span></h3><button className={buttonClass} disabled={busy} onClick={() => setSelected(previous => new Set([...previous, ...logs.filter(log => (log.run_id || 'legacy') === key && eligible(log)).map(log => log.id!)]))}>{t('beta.selectRun')}</button></div>
      {entries.map(log => <label key={log.id} className="flex gap-3 border-t border-border pt-2 text-sm"><input type="checkbox" className="mt-1 shrink-0" checked={selected.has(log.id!)} disabled={busy || !eligible(log)} onChange={() => toggle(log.id!)} /><span className="min-w-0 break-all"><strong>{log.file_name}</strong> · {log.file_type} · {t(`beta.${log.action}`, { defaultValue: log.action })}<span className="block">{log.source_path}</span>{log.destination_path && <span className="block">→ {log.destination_path}</span>}<span className="block text-xs text-text-muted">{new Date(log.timestamp).toLocaleString()} · {log.file_size} B{log.undone ? ` · ${t('beta.undone')}` : log.action === 'delete' ? ` · ${t('beta.trashHelp')}` : !log.fingerprint ? ` · ${t('beta.errors.history.unverified')}` : ''}</span></span></label>)}
    </section>)}
    {logs.length > limit && <button className={buttonClass} onClick={() => setLimit(limit + 100)}>{t('beta.more')}</button>}
    <button className={buttonClass} onClick={() => setClear(true)}>{t('beta.clearHistory')}</button>
    {clear && <div className="space-y-2"><p>{t('beta.confirmClear')}</p><button className={buttonClass} disabled={busy} onClick={async () => { try { await invoke('clear_logs_cmd'); setClear(false); await load(); } catch (e) { setError(errorText(e)); } }}>{t('beta.clearHistory')}</button><button className={`${buttonClass} ml-2`} onClick={() => setClear(false)}>{t('beta.cancel')}</button></div>}
  </div>;
}
