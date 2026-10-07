import type { SelectedSkin } from '../types';
import { useTranslation } from '@/i18n';

interface Props {
    selectedSkins: SelectedSkin[];
    statusMessage: string;
    isExtracting: boolean;
    isRepathing: boolean;
    isPreviewing: boolean;
    isSetupValid: boolean;
    onExtract: () => void;
    onRepath: () => void;
    onClearAll: () => void;
}

/* Bottom action bar (mirrors PortBottomControls). Left: selection count + names.
   Center: latest status message. Right: Extract / Repath / Clear. Renders only
   when at least one skin is selected. */
export function SelectionActionBar({
    selectedSkins,
    statusMessage,
    isExtracting,
    isRepathing,
    isPreviewing,
    isSetupValid,
    onExtract,
    onRepath,
    onClearAll,
}: Props) {
    const { t } = useTranslation();
    const hasSelection = selectedSkins.length > 0;
    const busy = isExtracting || isRepathing || isPreviewing;
    const disabledAction = busy || !isSetupValid || !hasSelection;
    /* Say WHY the button is dead. A disabled Extract with a skin plainly
       selected reads as a broken button, and the missing setup is on a
       different page, so there is nothing on screen to connect it to. */
    const blockedReason = !isSetupValid
        ? t('assetExtractorPage.actionBar.setupMissing')
        : !hasSelection
          ? t('assetExtractorPage.actionBar.selectSkinDesc')
          : '';
    const names = selectedSkins
        .map((s) => `${s.name}${s.champion?.name ? ` (${s.champion.name})` : ''}`)
        .join(', ');

    return (
        <div className="ae-bottom-bar">
            <div className="ae-bottom-bar__group">
                <span className="dl-badge"><span className="dl-badge__dot" />{t('assetExtractorPage.actionBar.selected', { count: selectedSkins.length })}</span>
                {hasSelection
                    ? <span className="ae-bottom-bar__names" title={names}>{names}</span>
                    : <span className="ae-bottom-bar__names" style={{ color: 'var(--text-muted)' }}>{t('assetExtractorPage.actionBar.noSkinsSelected')}</span>}
            </div>

            {statusMessage
                ? <span className="ae-bottom-bar__status">{statusMessage}</span>
                : blockedReason && <span className="ae-bottom-bar__status">{blockedReason}</span>}

            <div className="ae-bottom-bar__group">
                <button
                    className="dl-btn dl-btn--sm ae-extract-btn"
                    onClick={onExtract}
                    disabled={disabledAction}
                    title={blockedReason || t('assetExtractorPage.actionBar.extractTooltip')}
                >
                    {isExtracting ? t('assetExtractorPage.actionBar.extracting') : t('assetExtractorPage.actionBar.extract')}
                </button>
                <button
                    className="dl-btn dl-btn--sm dl-btn--primary"
                    onClick={onRepath}
                    disabled={disabledAction}
                    title={blockedReason || t('assetExtractorPage.actionBar.repathTooltip')}
                >
                    {isRepathing ? t('assetExtractorPage.actionBar.repathing') : t('assetExtractorPage.actionBar.repath')}
                </button>
                <button className="dl-btn dl-btn--sm dl-btn--secondary" onClick={onClearAll} disabled={busy || !hasSelection}>
                    {t('assetExtractorPage.actionBar.clearAll')}
                </button>
            </div>
        </div>
    );
}
