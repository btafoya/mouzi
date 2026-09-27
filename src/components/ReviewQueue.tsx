import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { buttonClass, primaryClass, errorText, Results, OperationResult } from './BetaControls';
import { useAppStore } from '../store/useAppStore';

interface Entry { id: string; source: string; destination: string | null; rule: string; action: string; size: number; warning: boolean; error: string | null }
interface Preview { id: string; entries: Entry[] }
export default function ReviewQueue({ paths }: { paths: string[] | null }) {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<Preview | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [confirm, setConfirm] = useState(false);
  const [results, setResults] = useState<OperationResult[]>([]);
  const currentId = useRef<string | null>(null);
  const generation = useRef(0);
  async function refresh() {
    const request = ++generation.current;
    setBusy(true); setError(''); setConfirm(false); setResults([]);
    if (currentId.current) await invoke('discard_preview_cmd', { id: currentId.current }).catch(() => {});
    currentId.current = null; setPreview(null);
    try {
      const plan = await invoke<Preview>('preview_cmd', { paths: paths?.length ? paths : null });
      if (request !== generation.current) { await invoke('discard_preview_cmd', { id: plan.id }); return; }
      currentId.current = plan.id; setPreview(plan);
      setSelected(new Set(plan.entries.filter(e => !e.error && !e.warning && e.action !== 'delete').map(e => e.id)));
    } catch (error) { if (request === generation.current) setError(errorText(error)); }
    finally { if (request === generation.current) setBusy(false); }
  }
  useEffect(() => {
    void refresh();
    return () => { ++generation.current; if (currentId.current) void invoke('discard_preview_cmd', { id: currentId.current }); };
  }, [paths]);
  async function apply() {
    if (!preview || !selected.size) return;
    setBusy(true); setError('');
    try {
      setResults(await invoke<OperationResult[]>('apply_preview_cmd', { id: preview.id, selected: [...selected] }));
      currentId.current = null; setPreview(null); setSelected(new Set()); setConfirm(false);
      await Promise.all([useAppStore.getState().loadLogs(), useAppStore.getState().loadStats(), useAppStore.getState().getPendingFiles()]);
    } catch (error) { setError(errorText(error)); setConfirm(false); }
    finally { setBusy(false); }
  }
  const toggle = (id: string) => setSelected(old => { const next = new Set(old); if (next.has(id)) next.delete(id); else next.add(id); return next; });
  return <section className="space-y-4" aria-busy={busy}>
    <div className="flex flex-wrap justify-between items-center gap-3"><h2 className="text-lg font-semibold">{t('beta.review')}</h2><button className={buttonClass} onClick={refresh} disabled={busy}>{t('beta.refresh')}</button></div>
    <p className="text-sm text-text-muted">{t('beta.previewHelp')}</p>
    {busy && <p role="status">{t('beta.loading')}</p>}
    {error && <p role="alert" className="text-sm text-red-700 dark:text-red-300">{error}</p>}
    <Results results={results} />
    {preview && <>
      <div className="flex flex-wrap items-center gap-2">
        <button className={buttonClass} disabled={busy || confirm} onClick={() => setSelected(new Set(preview.entries.filter(e => !e.error && !e.warning && e.action !== 'delete').map(e => e.id)))}>{t('beta.selectAll')}</button>
        <button className={buttonClass} disabled={busy || confirm} onClick={() => setSelected(new Set())}>{t('beta.clearSelection')}</button>
        <span className="text-sm">{t('beta.selected', { count: selected.size })}</span>
      </div>
      {!preview.entries.length && <p className="py-8 text-text-muted">{t('beta.empty')}</p>}
      {confirm && <div className="rounded-lg border border-primary p-4 space-y-3" role="region" aria-label={t('beta.apply')}>
        <p className="text-sm">{t('beta.confirmHelp')}</p>
        <div className="flex gap-2"><button className={primaryClass} disabled={busy} onClick={apply}>{t('beta.confirm', { count: selected.size })}</button><button className={buttonClass} disabled={busy} onClick={() => setConfirm(false)}>{t('beta.cancel')}</button></div>
      </div>}
      <div className="space-y-2">{preview.entries.filter(e => !confirm || selected.has(e.id)).map(entry => <div key={entry.id} className="rounded-lg border border-border p-3">
        <label className="flex gap-3 items-start"><input type="checkbox" className="mt-1" checked={selected.has(entry.id)} disabled={busy || confirm || !!entry.error} onChange={() => toggle(entry.id)} />
          <span className="min-w-0 text-sm break-all"><span className="block font-medium">{entry.source}</span><span className="block mt-1">{entry.rule} · {t(`beta.${entry.action}`)} · {entry.size.toLocaleString()} B</span><span className="block text-text-muted">{entry.destination || t('settings.rules.recycleBin')}</span></span>
        </label>
        {entry.warning && <p className="mt-2 text-sm text-amber-800 dark:text-amber-200">{t('beta.warning')}</p>}
        {entry.error && <p className="mt-2 text-sm text-text-muted">{errorText(entry.error)}</p>}
      </div>)}</div>
      {!confirm && <button className={primaryClass} disabled={busy || !selected.size} onClick={() => setConfirm(true)}>{t('beta.apply')}</button>}
    </>}
  </section>;
}
