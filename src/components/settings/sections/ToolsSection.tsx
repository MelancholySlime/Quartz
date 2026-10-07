import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Download, FolderOpen, Search, FolderTree, Terminal, Database, ImageUpscale, PackageOpen } from 'lucide-react';
import { desktopDir } from '@tauri-apps/api/path';
import { FormGroup, StatusBadge, Button, InputWithButton, cardSurface } from '../primitives';
import { useUiPrefsStore, useConfigStore, useNavigationStore } from '@/lib/stores';
import {
    getHashStatus, downloadHashes, getLeaguePath, checkLeaguePath, type HashStatus,
    upscaleCheckStatus, upscaleDownloadAll, type UpscaleStatus,
} from '@/lib/api';
import { log } from '@/lib/util/logger';
import { useFileExplorer } from '@/components/explorer';
import { useTranslation } from '@/i18n';

export function ToolsSection() {
    const { t } = useTranslation();
    const pick = useFileExplorer();
    const jadePath = useUiPrefsStore((s) => s.jadeExecutablePath);
    const set = useUiPrefsStore((s) => s.set);

    const leaguePath = useConfigStore((s) => s.settings.leaguePath) || '';
    const wadOutputPath = useConfigStore((s) => s.settings.wadOutputPath) || '';
    const update = useConfigStore((s) => s.update);

    // Deep-link highlight: when arriving from the Upscaler's "Install in Settings",
    // scroll the AI models card into view and briefly flash it.
    const highlightTarget = useNavigationStore((s) => s.settingsTarget?.highlight);
    const clearTarget = useNavigationStore((s) => s.clearSettingsTarget);
    const upscaleCardRef = useRef<HTMLDivElement>(null);
    const [flashUpscale, setFlashUpscale] = useState(false);

    useEffect(() => {
        if (highlightTarget !== 'upscale') return;
        upscaleCardRef.current?.scrollIntoView({ behavior: 'smooth', block: 'center' });
        setFlashUpscale(true);
        const off = setTimeout(() => setFlashUpscale(false), 2000);
        clearTarget();
        return () => clearTimeout(off);
    }, [highlightTarget, clearTarget]);

    const [detectStatus, setDetectStatus] = useState<null | 'loading' | 'success' | 'error'>(null);
    const [detectMessage, setDetectMessage] = useState('');

    /* Whether the configured League folder actually works.
       A wrong path used to stay silent here and only surface much later as
       "Could not locate a League of Legends install" when an extraction failed
       — which reads as "no path set" rather than "this path is wrong", and cost
       a user a support thread to work out. Checked as the field is edited so
       the answer is next to the input that caused it. */
    const [pathCheck, setPathCheck] = useState<{ valid: boolean; reason: string } | null>(null);

    useEffect(() => {
        const path = leaguePath.trim();
        if (!path) { setPathCheck(null); return; }
        let cancelled = false;
        // Debounced: this hits the filesystem and the field is typed into.
        const timer = setTimeout(() => {
            checkLeaguePath(path)
                .then((r) => { if (!cancelled) setPathCheck(r); })
                .catch((e) => { log.error('checkLeaguePath', e); if (!cancelled) setPathCheck(null); });
        }, 300);
        return () => { cancelled = true; clearTimeout(timer); };
    }, [leaguePath]);

    const [hashStatus, setHashStatus] = useState<HashStatus | null>(null);
    const [downloading, setDownloading] = useState(false);
    const [hashMessage, setHashMessage] = useState<string | null>(null);

    const refreshHashes = () => getHashStatus().then(setHashStatus).catch((e) => log.error('getHashStatus', e));
    useEffect(() => { refreshHashes(); }, []);

    const autoDetect = async () => {
        setDetectStatus('loading'); setDetectMessage(t('settings.tools.scanning'));
        try {
            const detected = await getLeaguePath();
            if (detected) {
                await update({ leaguePath: detected });
                setDetectStatus('success'); setDetectMessage(t('settings.tools.found'));
            } else {
                setDetectStatus('error'); setDetectMessage(t('settings.tools.notFound'));
            }
        } catch (e) {
            log.error('autoDetectLeaguePath', e);
            setDetectStatus('error'); setDetectMessage(t('settings.tools.detectionFailed'));
        }
        setTimeout(() => { setDetectStatus(null); setDetectMessage(''); }, 3000);
    };

    const browseLeague = async () => {
        const dir = await pick({ mode: 'directory' });
        if (typeof dir === 'string' && dir) await update({ leaguePath: dir });
    };

    const browseJade = async () => {
        const picked = await pick({ mode: 'file', filters: [{ name: 'Jade', extensions: ['exe'] }] });
        if (typeof picked === 'string') set('jadeExecutablePath', picked);
    };

    const browseWadOutput = async () => {
        const dir = await pick({ mode: 'directory' });
        if (typeof dir === 'string' && dir) await update({ wadOutputPath: dir });
    };

    // Default the extraction output to the user's Desktop when it's still unset,
    // so the Asset Extractor has a valid target out of the box.
    useEffect(() => {
        if (wadOutputPath) return;
        let cancelled = false;
        desktopDir()
            .then((dir) => { if (!cancelled && dir) void update({ wadOutputPath: dir }); })
            .catch((e) => log.error('desktopDir default', e));
        return () => { cancelled = true; };
    }, [wadOutputPath, update]);

    const doDownloadHashes = async () => {
        setDownloading(true); setHashMessage(null);
        try {
            const r = await downloadHashes(false);
            const errStr = r.errors ? `, ${r.errors} failed` : '';
            setHashMessage(t('settings.tools.downloadSummary', { downloaded: r.downloaded, skipped: r.skipped, errors: errStr }));
            await refreshHashes();
        } catch (e) { log.error('downloadHashes', e); setHashMessage(t('settings.tools.downloadFailed')); }
        finally { setDownloading(false); }
    };

    const statusColor = detectStatus === 'success'
        ? 'var(--color-success)'
        : detectStatus === 'error' ? 'var(--color-danger)' : 'var(--text-secondary)';

    const hashCountLabel = hashStatus
        ? hashStatus.present
            ? t('settings.tools.hashDatabasesPresent', { count: hashStatus.loadedCount.toLocaleString() })
            : t('settings.tools.hashNotDownloaded')
        : '';

    return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '16px' }}>
            <FormGroup label={t('settings.tools.leaguePath')} icon={<FolderTree size={15} />}>
                <div className="settings-card" style={{ ...cardSurface, display: 'flex', flexDirection: 'column', gap: '10px' }}>
                    <InputWithButton
                        value={leaguePath}
                        onChange={(e) => update({ leaguePath: e.target.value })}
                        placeholder={t('settings.tools.leaguePathPlaceholder')}
                        buttonIcon={<FolderOpen size={16} />}
                        buttonText={t('common.browse')}
                        onButtonClick={browseLeague}
                    />
                    <div style={{ fontSize: '11.5px', color: 'var(--text-secondary)', lineHeight: 1.5 }}>
                        Pick the <strong style={{ color: 'var(--text-primary)' }}>League of Legends</strong> folder itself, not the{' '}
                        <code style={{ fontSize: '11px', opacity: 0.85 }}>Game</code> folder inside it.
                        {' '}{t('settings.tools.leaguePathExample')}
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' }}>
                        <Button icon={<Search size={16} />} variant="secondary" onClick={autoDetect} disabled={detectStatus === 'loading'}>
                            {detectStatus === 'loading' ? t('settings.tools.scanning') : t('settings.tools.autoDetect')}
                        </Button>
                        {detectMessage && detectStatus !== 'loading' && (
                            <span style={{ fontSize: '12px', fontWeight: 600, color: statusColor }}>{detectMessage}</span>
                        )}
                        {pathCheck && (
                            pathCheck.valid
                                ? <StatusBadge status="success" text={t('settings.tools.validLeagueFolder')} />
                                : <StatusBadge status="warning" text={pathCheck.reason || t('settings.tools.notALeagueFolder')} />
                        )}
                    </div>
                </div>
            </FormGroup>

            <FormGroup label={t('settings.tools.wadOutputPath')} icon={<PackageOpen size={15} />}>
                <div className="settings-card" style={{ ...cardSurface, display: 'flex', flexDirection: 'column', gap: '8px' }}>
                    <InputWithButton
                        value={wadOutputPath}
                        onChange={(e) => update({ wadOutputPath: e.target.value })}
                        placeholder="C:\\Users\\<user>\\Desktop"
                        buttonIcon={<FolderOpen size={16} />}
                        buttonText={t('common.browse')}
                        onButtonClick={browseWadOutput}
                    />
                    <div style={{ fontSize: '11px', color: 'var(--text-muted)' }}>
                        {t('settings.tools.wadOutputPathHint')}
                    </div>
                </div>
            </FormGroup>

            <FormGroup label={t('settings.tools.jadePath')} icon={<Terminal size={15} />}>
                <div className="settings-card" style={{ ...cardSurface, display: 'flex', flexDirection: 'column', gap: '8px' }}>
                    <InputWithButton
                        value={jadePath}
                        onChange={(e) => set('jadeExecutablePath', e.target.value)}
                        placeholder="C:\\Users\\<user>\\AppData\\Local\\Jade\\Jade.exe"
                        buttonIcon={<FolderOpen size={16} />}
                        buttonText={t('common.browse')}
                        onButtonClick={browseJade}
                    />
                    <div style={{ fontSize: '11px', color: 'var(--text-muted)' }}>
                        {t('settings.tools.jadePathHint')}
                    </div>
                </div>
            </FormGroup>

            <FormGroup label={t('settings.tools.hashFiles')} icon={<Database size={15} />}>
                <div className="settings-card" style={{ ...cardSurface, display: 'flex', flexDirection: 'column', gap: '12px' }}>
                    {hashStatus && (
                        <div>
                            <StatusBadge
                                status={hashStatus.present ? 'success' : 'warning'}
                                text={hashCountLabel}
                            />
                            {hashStatus.lastUpdated && (
                                <div style={{ marginTop: '6px', fontSize: '11px', color: 'var(--text-muted)' }}>
                                    {t('settings.tools.updatedAt', { date: new Date(hashStatus.lastUpdated).toLocaleString() })}
                                </div>
                            )}
                        </div>
                    )}
                    <Button icon={<Download size={16} />} fullWidth variant="secondary" onClick={doDownloadHashes} disabled={downloading}>
                        {downloading ? t('settings.tools.downloading') : t('settings.tools.downloadUpdateHashes')}
                    </Button>
                    {hashMessage && <div style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>{hashMessage}</div>}
                </div>
            </FormGroup>

            <div ref={upscaleCardRef} className={flashUpscale ? 'settings-flash' : undefined}>
                <FormGroup label={t('settings.tools.aiUpscaleModels')} icon={<ImageUpscale size={15} />}>
                    <UpscaleCard />
                </FormGroup>
            </div>
        </div>
    );
}

/* The Upscayl binary + models downloader. Lives here (External Tools) so it's
   available in release builds — the AI Image Upscaler links users here when the
   binary isn't installed. */
function UpscaleCard() {
    const { t } = useTranslation();
    const [status, setStatus] = useState<UpscaleStatus | null>(null);
    const [busy, setBusy] = useState(false);
    const [pct, setPct] = useState(0);
    const [step, setStep] = useState('');
    const [message, setMessage] = useState<string | null>(null);

    const refresh = () => upscaleCheckStatus().then(setStatus).catch((e) => log.error('upscaleCheckStatus', e));
    useEffect(() => { refresh(); }, []);

    const run = async () => {
        setBusy(true); setMessage(null); setPct(0); setStep('starting…');
        const unlisten = await listen<{ step: string; message: string; progress: number }>('upscale:progress', (e) => {
            setPct(Math.round(e.payload.progress));
            setStep(e.payload.message || e.payload.step);
        });
        try {
            await upscaleDownloadAll();
            setMessage(t('settings.tools.componentsInstalled'));
            await refresh();
        } catch (e) {
            setMessage(`Download failed: ${String((e as Error)?.message || e)}`);
            log.error('upscaleDownloadAll', e);
        } finally { unlisten(); setBusy(false); }
    };

    const binOk = !!status?.binary.installed;
    const allModels = binOk && (status?.models.installed.length ?? 0) === (status?.models.total ?? 0);
    const modelLabel = status
        ? binOk
            ? t('settings.tools.upscaylReady', { installed: status.models.installed.length, total: status.models.total })
            : t('settings.tools.upscaylNotDownloaded')
        : '';

    return (
        <div className="settings-card" style={{ ...cardSurface, display: 'flex', flexDirection: 'column', gap: '12px' }}>
            {status && <StatusBadge status={binOk ? 'success' : 'warning'} text={modelLabel} />}
            {busy && (
                <div>
                    <div className="dl-progress"><div className="dl-progress__fill" style={{ width: `${Math.max(4, Math.min(100, pct))}%` }} /></div>
                    {step && <div style={{ marginTop: '4px', fontSize: '11px', color: 'var(--text-muted)', fontFamily: 'var(--font-mono)' }}>{step}</div>}
                </div>
            )}
            <Button icon={<Download size={16} />} fullWidth variant="secondary" onClick={run} disabled={busy || allModels}>
                {busy ? t('settings.tools.downloadingProgress', { percent: pct }) : binOk ? t('settings.tools.updateComponents') : t('settings.tools.downloadComponents')}
            </Button>
            {message && <div style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>{message}</div>}
        </div>
    );
}
