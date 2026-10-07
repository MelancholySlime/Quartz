import { useEffect, useState } from 'react';
import type { Update } from '@tauri-apps/plugin-updater';
import { Download, RefreshCw, Plug, PanelLeftClose, FolderOpen, ListTree, SlidersHorizontal, FileText, Languages } from 'lucide-react';
import { FormGroup, Button, CardRow, Switch, CustomSelect, cardSurface } from '../primitives';
import { useConfigStore, useUiPrefsStore } from '@/lib/stores';
import { getAppInfo } from '@/lib/api';
import { checkForUpdate, installUpdate } from '@/lib/api/updater';
import { showUpdateNotes } from '@/components/update/updateShowcaseState';
import { log } from '@/lib/util/logger';
import { useTranslation, type SupportedLocale } from '@/i18n';

export function GeneralSection() {
    const { t, locale, setLocale, supportedLocales } = useTranslation();
    const autoUpdateEnabled = useConfigStore((s) => s.settings.autoUpdateEnabled);
    const updateSettings = useConfigStore((s) => s.update);
    const communicateWithJade = useUiPrefsStore((s) => s.communicateWithJade);
    const useNative = useUiPrefsStore((s) => s.useNativeFileBrowser);
    const sidebarCollapsed = useUiPrefsStore((s) => s.sidebarCollapsed);
    const expand = useUiPrefsStore((s) => s.expandSystemsOnLoad);
    const set = useUiPrefsStore((s) => s.set);

    const [version, setVersion] = useState('');
    const [checking, setChecking] = useState(false);
    const [pending, setPending] = useState<Update | null>(null);
    const [updateMessage, setUpdateMessage] = useState<string | null>(null);

    useEffect(() => {
        getAppInfo().then((i) => setVersion(i.version)).catch(() => {});
    }, []);

    const checkUpdate = async () => {
        setChecking(true); setUpdateMessage(t('settings.general.checking'));
        try {
            const { info, update } = await checkForUpdate();
            setPending(update);
            setUpdateMessage(info.available ? t('settings.general.updateAvailable', { version: info.version ?? '' }) : t('settings.general.upToDate'));
        } catch (e) { log.error('checkForUpdate', e); setUpdateMessage(t('settings.general.updateCheckFailed')); }
        finally { setChecking(false); }
    };

    return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '20px' }}>
            <FormGroup label={t('settings.general.preferences')} icon={<SlidersHorizontal size={15} />}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: '10px' }}>
                    <CardRow
                        icon={<Languages size={18} />}
                        label={t('settings.general.language')}
                        description={t('settings.general.languageDesc')}
                        control={
                            <div style={{ width: '140px' }}>
                                <CustomSelect
                                    value={locale}
                                    onChange={(v) => setLocale(v as SupportedLocale)}
                                    options={supportedLocales}
                                />
                            </div>
                        }
                    />
                    <CardRow
                        icon={<Plug size={18} />}
                        label={t('settings.general.communicateWithJade')}
                        description={t('settings.general.communicateWithJadeDesc')}
                        onActivate={() => set('communicateWithJade', !communicateWithJade)}
                        control={<Switch checked={communicateWithJade} onChange={(c) => set('communicateWithJade', c)} />}
                    />
                    <CardRow
                        icon={<PanelLeftClose size={18} />}
                        label={t('settings.general.collapseSidebar')}
                        description={t('settings.general.collapseSidebarDesc')}
                        onActivate={() => set('sidebarCollapsed', !sidebarCollapsed)}
                        control={<Switch checked={sidebarCollapsed} onChange={(c) => set('sidebarCollapsed', c)} />}
                    />
                    <CardRow
                        icon={<FolderOpen size={18} />}
                        label={t('settings.general.useNativeFileDialog')}
                        description={t('settings.general.useNativeFileDialogDesc')}
                        onActivate={() => set('useNativeFileBrowser', !useNative)}
                        control={<Switch checked={useNative} onChange={(c) => set('useNativeFileBrowser', c)} />}
                    />
                    <CardRow
                        icon={<ListTree size={18} />}
                        label={t('settings.general.expandVfxSystems')}
                        description={t('settings.general.expandVfxSystemsDesc')}
                        onActivate={() => set('expandSystemsOnLoad', !expand)}
                        control={<Switch checked={expand} onChange={(c) => set('expandSystemsOnLoad', c)} />}
                    />
                </div>
            </FormGroup>

            <FormGroup label={t('settings.general.appUpdates')} icon={<RefreshCw size={15} />}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: '10px' }}>
                    <CardRow
                        icon={<Download size={18} />}
                        label={t('settings.general.autoUpdates')}
                        description={t('settings.general.autoUpdatesDesc')}
                        onActivate={() => void updateSettings({ autoUpdateEnabled: !autoUpdateEnabled })}
                        control={<Switch checked={autoUpdateEnabled} onChange={(checked) => void updateSettings({ autoUpdateEnabled: checked })} />}
                    />
                    <div className="settings-card" style={{ ...cardSurface, display: 'flex', flexDirection: 'column', gap: '10px' }}>
                        {version && <div style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>{t('settings.general.currentVersion', { version })}</div>}
                        <div style={{ display: 'flex', gap: '8px', flexWrap: 'wrap' }}>
                            <Button icon={<RefreshCw size={16} style={checking ? { animation: 'spin 1s linear infinite' } : undefined} />} variant="secondary" onClick={checkUpdate} disabled={checking}>
                                {checking ? t('settings.general.checking') : t('settings.general.checkForUpdates')}
                            </Button>
                            {/* The showcase appears once per version and is then
                                remembered, so this is the only way back to the notes
                                for the build you are already on. */}
                            <Button icon={<FileText size={16} />} variant="secondary" onClick={showUpdateNotes}>
                                {t('settings.general.patchNotes')}
                            </Button>
                            {pending && (
                                <Button icon={<Download size={16} />} variant="primary" onClick={() => installUpdate(pending).catch((e) => { log.error('installUpdate', e); setUpdateMessage(t('settings.general.installFailed')); })}>
                                    {t('settings.general.installAndRestart')}
                                </Button>
                            )}
                        </div>
                        {updateMessage && <div style={{ fontSize: '12px', color: 'var(--text-secondary)' }}>{updateMessage}</div>}
                    </div>
                </div>
            </FormGroup>
        </div>
    );
}

