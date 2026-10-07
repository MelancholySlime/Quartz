import { useState } from 'react';
import { Wand2, Play, X, File as FileIcon, Folder as FolderIcon } from 'lucide-react';
import { pickPath } from '@/components/explorer';
import { Button } from '@/components/settings/primitives';
import { Checkbox } from '@/components/ui/Checkbox';
import { toolsFixVfxShape } from '@/lib/api/vfxTools';
import { log } from '@/lib/util/logger';
import { useTranslation } from '@/i18n';
import './tools-cards.css';

interface NotifyArg { message: string; severity: 'info' | 'success' | 'error' | 'warning' }

const basename = (p: string) => p.replace(/\\/g, '/').split('/').pop() ?? p;

async function pickBinFile(title: string): Promise<string | null> {
    const r = await pickPath({ mode: 'file', title, filters: [{ name: 'BIN Files', extensions: ['bin'] }], recentsKey: 'bin' });
    return typeof r === 'string' ? r : null;
}
async function pickFolder(title: string): Promise<string | null> {
    const r = await pickPath({ mode: 'directory', title });
    return typeof r === 'string' ? r : null;
}

interface FixResult {
    shapesRewrittenRadius: number;
    shapesRewrittenVec3: number;
    shapesRewrittenEmpty: number;
    birthTranslationsLifted: number;
}

export function FixVfxShapeCard({ onNotify }: { onNotify?: (a: NotifyArg) => void }) {
    const { t } = useTranslation();
    const [targetPath, setTargetPath] = useState('');
    const [createBackup, setCreateBackup] = useState(true);
    const [busy, setBusy] = useState(false);
    const [lastResult, setLastResult] = useState<FixResult | null>(null);

    const notify = (message: string, severity: NotifyArg['severity'] = 'info') => onNotify?.({ message, severity });

    const handleRun = async () => {
        if (!targetPath) { notify(t('toolsPage.builtIn.fixVfxShape.pickBinOrFolderFirst') || 'Pick a .bin file or a folder first', 'warning'); return; }
        setBusy(true);
        setLastResult(null);
        try {
            // Backend auto-detects file-vs-folder via stat and routes to the
            // right code path, so we just hand it the target and let it decide.
            const res = await toolsFixVfxShape({ filePath: targetPath }, createBackup);
            const total = res.shapesRewrittenRadius + res.shapesRewrittenVec3 + res.shapesRewrittenEmpty;
            const where = res.filesProcessed > 1
                ? `${res.filesModified}/${res.filesProcessed} file(s) in ${basename(targetPath)}`
                : basename(targetPath);
            const failed = res.filesFailed > 0 ? ` — ${res.filesFailed} failed` : '';
            notify(
                t('toolsPage.builtIn.fixVfxShape.fixSuccess', { total, lifted: res.birthTranslationsLifted, where, failed }) ||
                `Fixed ${total} shape(s), lifted ${res.birthTranslationsLifted} BirthTranslation(s) across ${where}${failed}`,
                res.filesFailed > 0 ? 'warning' : total > 0 || res.birthTranslationsLifted > 0 ? 'success' : 'info',
            );
            setLastResult({
                shapesRewrittenRadius: res.shapesRewrittenRadius,
                shapesRewrittenVec3: res.shapesRewrittenVec3,
                shapesRewrittenEmpty: res.shapesRewrittenEmpty,
                birthTranslationsLifted: res.birthTranslationsLifted,
            });
        } catch (e) {
            log.error('bin:fixVfxShape', e);
            notify(t('toolsPage.builtIn.fixVfxShape.fixCrashed', { error: String((e as Error)?.message || e) }) || `Fix crashed: ${String((e as Error)?.message || e)}`, 'error');
        } finally {
            setBusy(false);
        }
    };

    const pickFile = async () => { const p = await pickBinFile(t('toolsPage.builtIn.fixVfxShape.selectBinToFix') || ''); if (p) setTargetPath(p); };
    const pickDir = async () => { const p = await pickFolder(t('toolsPage.builtIn.fixVfxShape.selectFolder') || ''); if (p) setTargetPath(p); };

    return (
        <div className="tc-card">
            <div className="tc-card__head">
                <div className="tc-card__icon"><Wand2 size={20} /></div>
                <div style={{ flex: 1, minWidth: 0 }}>
                    <h3 className="tc-card__title">{t('toolsPage.builtIn.fixVfxShape.title')}</h3>
                    <p className="tc-card__desc">{t('toolsPage.builtIn.fixVfxShape.desc')}</p>
                </div>
            </div>

            <div className="tc-card__body">
                <div className="tc-picker">
                    <input
                        className="dl-input"
                        placeholder={t('toolsPage.builtIn.fixVfxShape.pastePlaceholder') || ''}
                        value={targetPath}
                        onChange={(e) => setTargetPath(e.target.value)}
                    />
                    <button className="tc-iconbtn" title={t('toolsPage.builtIn.fixVfxShape.selectBinToFix') || ''} onClick={pickFile}><FileIcon size={16} /></button>
                    <button className="tc-iconbtn" title={t('toolsPage.builtIn.fixVfxShape.selectFolder') || ''} onClick={pickDir}><FolderIcon size={16} /></button>
                    {targetPath && <button className="tc-iconbtn tc-iconbtn--danger" title={t('common.reset') || 'Clear'} onClick={() => setTargetPath('')}><X size={16} /></button>}
                </div>

                <div className="tc-foot">
                    <div className="tc-foot__opts">
                        <Checkbox label={t('toolsPage.builtIn.fixVfxShape.createBackup') || 'Create .bak backup before write'} checked={createBackup} onChange={setCreateBackup} />
                    </div>
                    <div className="tc-foot__spacer" />
                    {lastResult && (
                        <span className="tc-foot__result">
                            Radius:{lastResult.shapesRewrittenRadius} · Vec3:{lastResult.shapesRewrittenVec3} · Empty:{lastResult.shapesRewrittenEmpty} · BT:{lastResult.birthTranslationsLifted}
                        </span>
                    )}
                    <div className="tc-foot__run">
                        <Button icon={<Play size={16} />} variant="primary" disabled={busy || !targetPath} onClick={handleRun}>
                            {busy ? t('toolsPage.builtIn.fixVfxShape.fixing') : t('toolsPage.builtIn.fixVfxShape.runFix')}
                        </Button>
                    </div>
                </div>
            </div>
        </div>
    );
}

export default FixVfxShapeCard;
