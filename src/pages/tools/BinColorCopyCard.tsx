import { useState } from 'react';
import { Palette, FolderOpen, Play, X } from 'lucide-react';
import { pickPath } from '@/components/explorer';
import { Button } from '@/components/settings/primitives';
import { Checkbox } from '@/components/ui/Checkbox';
import { toolsBinCopyColors } from '@/lib/api/vfxTools';
import { log } from '@/lib/util/logger';
import { useTranslation } from '@/i18n';
import './tools-cards.css';

interface NotifyArg { message: string; severity: 'info' | 'success' | 'error' | 'warning' }

const basename = (p: string) => p.replace(/\\/g, '/').split('/').pop() ?? p;
const dirname = (p: string) => {
    const norm = p.replace(/\\/g, '/');
    const idx = norm.lastIndexOf('/');
    return idx >= 0 ? norm.slice(0, idx) : '';
};

async function pickBinFile(title: string): Promise<string | null> {
    const r = await pickPath({ mode: 'file', title, filters: [{ name: 'BIN Files', extensions: ['bin'] }], recentsKey: 'bin' });
    return typeof r === 'string' ? r : null;
}

async function pickSaveBinFile(title: string, defaultPath: string): Promise<string | null> {
    const r = await pickPath({ mode: 'save', title, defaultPath, filters: [{ name: 'BIN Files', extensions: ['bin'] }], recentsKey: 'bin' });
    return typeof r === 'string' ? r : null;
}

function PathPicker({ placeholder, browseTitle, value, onChange, onPick }: {
    placeholder: string; browseTitle: string; value: string; onChange: (v: string) => void; onPick: () => void;
}) {
    const { t } = useTranslation();
    return (
        <div className="tc-picker">
            <input className="dl-input" placeholder={placeholder} value={value} onChange={(e) => onChange(e.target.value)} />
            <button className="tc-iconbtn" title={browseTitle} onClick={onPick}><FolderOpen size={16} /></button>
            {value && <button className="tc-iconbtn tc-iconbtn--danger" title={t('common.reset') || 'Clear'} onClick={() => onChange('')}><X size={16} /></button>}
        </div>
    );
}

interface CopyResult { fieldsCopied: number; entriesMatched: number; entriesSkipped: number }

export function BinColorCopyCard({ onNotify }: { onNotify?: (a: NotifyArg) => void }) {
    const { t } = useTranslation();
    const [sourcePath, setSourcePath] = useState('');
    const [targetPath, setTargetPath] = useState('');
    const [overwriteTarget, setOverwriteTarget] = useState(true);
    const [createBackup, setCreateBackup] = useState(true);
    const [busy, setBusy] = useState(false);
    const [lastResult, setLastResult] = useState<CopyResult | null>(null);

    const notify = (message: string, severity: NotifyArg['severity'] = 'info') => onNotify?.({ message, severity });

    const handleRun = async () => {
        if (!sourcePath || !targetPath) { notify(t('toolsPage.builtIn.binColorCopy.selectBothBins') || 'Select both source and target bins first', 'warning'); return; }
        let outputPath: string | null = null;
        if (!overwriteTarget) {
            const base = basename(targetPath).replace(/\.bin$/i, '');
            outputPath = await pickSaveBinFile(t('toolsPage.builtIn.binColorCopy.saveBinAs') || 'Save modified BIN as', `${dirname(targetPath)}/${base}_colored.bin`);
            if (!outputPath) return;
        }
        setBusy(true);
        setLastResult(null);
        try {
            const res = await toolsBinCopyColors(sourcePath, targetPath, outputPath, createBackup);
            notify(
                t('toolsPage.builtIn.binColorCopy.copySuccess', { fields: res.fieldsCopied, matched: res.entriesMatched, skipped: res.entriesSkipped, output: basename(res.outputPath) }) ||
                `Copied ${res.fieldsCopied} color field(s) — ${res.entriesMatched} entr(ies) matched, ${res.entriesSkipped} skipped → ${basename(res.outputPath)}`,
                res.fieldsCopied > 0 ? 'success' : 'info',
            );
            setLastResult({ fieldsCopied: res.fieldsCopied, entriesMatched: res.entriesMatched, entriesSkipped: res.entriesSkipped });
        } catch (e) {
            log.error('bin:copyColors', e);
            notify(t('toolsPage.builtIn.binColorCopy.copyCrashed', { error: String((e as Error)?.message || e) }) || `Copy crashed: ${String((e as Error)?.message || e)}`, 'error');
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="tc-card">
            <div className="tc-card__head">
                <div className="tc-card__icon"><Palette size={20} /></div>
                <div style={{ flex: 1, minWidth: 0 }}>
                    <h3 className="tc-card__title">{t('toolsPage.builtIn.binColorCopy.title')}</h3>
                    <p className="tc-card__desc">{t('toolsPage.builtIn.binColorCopy.desc')}</p>
                </div>
            </div>

            <div className="tc-card__body">
                <div className="tc-fields">
                    <div>
                        <label className="tc-field__label">{t('toolsPage.builtIn.binColorCopy.sourceLabel')}</label>
                        <PathPicker placeholder={`${t('toolsPage.builtIn.binColorCopy.sourceLabel')} .bin`} browseTitle={t('toolsPage.builtIn.binColorCopy.selectSourceBin') || 'Select'} value={sourcePath} onChange={setSourcePath} onPick={async () => { const p = await pickBinFile(t('toolsPage.builtIn.binColorCopy.selectSourceBin') || ''); if (p) setSourcePath(p); }} />
                    </div>
                    <div>
                        <label className="tc-field__label">{t('toolsPage.builtIn.binColorCopy.targetLabel')}</label>
                        <PathPicker placeholder={`${t('toolsPage.builtIn.binColorCopy.targetLabel')} .bin`} browseTitle={t('toolsPage.builtIn.binColorCopy.selectTargetBin') || 'Select'} value={targetPath} onChange={setTargetPath} onPick={async () => { const p = await pickBinFile(t('toolsPage.builtIn.binColorCopy.selectTargetBin') || ''); if (p) setTargetPath(p); }} />
                    </div>
                </div>

                <div className="tc-foot">
                    <div className="tc-foot__opts">
                        <Checkbox label={t('toolsPage.builtIn.binColorCopy.overwriteTarget') || 'Overwrite target in place'} checked={overwriteTarget} onChange={setOverwriteTarget} />
                        <Checkbox label={t('toolsPage.builtIn.binColorCopy.createBackup') || 'Create .bak backup'} checked={createBackup} disabled={!overwriteTarget} onChange={setCreateBackup} />
                    </div>
                    <div className="tc-foot__spacer" />
                    {lastResult && (
                        <span className="tc-foot__result">
                            {lastResult.fieldsCopied} {t('common.done') || 'field(s)'} · {lastResult.entriesMatched} {t('common.done') || 'matched'} · {lastResult.entriesSkipped} {t('common.done') || 'skipped'}
                        </span>
                    )}
                    <div className="tc-foot__run">
                        <Button icon={<Play size={16} />} variant="primary" disabled={busy || !sourcePath || !targetPath} onClick={handleRun}>
                            {busy ? t('toolsPage.builtIn.binColorCopy.copying') : t('toolsPage.builtIn.binColorCopy.copyColors')}
                        </Button>
                    </div>
                </div>
            </div>
        </div>
    );
}

export default BinColorCopyCard;
