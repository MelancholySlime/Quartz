import { FolderOpen, X } from 'lucide-react';
import type { RecentBin } from '@/lib/stores';
import { useTranslation } from '@/i18n';
import './binOpenLanding.css';

function relativeTime(iso: string): string {
    const elapsed = Math.max(0, Date.now() - new Date(iso).getTime());
    const minutes = Math.floor(elapsed / 60_000);
    if (minutes < 1) return 'now';
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
}

export interface BinOpenLandingProps {
    recentBins: RecentBin[];
    busy?: boolean;
    dragActive?: boolean;
    onOpen: () => void;
    onOpenRecent: (path: string) => void;
    onRemoveRecent: (path: string) => void;
    /* Wording overrides so non-bin callers (e.g. the Audio Splitter) get the
       same layout without the bin-specific copy. */
    title?: string;
    description?: string;
    actionLabel?: string;
    recentTitle?: string;
    footnote?: string;
}

/** The same no-document landing and history layout used by Bin Editor. */
export function BinOpenLanding({
    recentBins,
    busy = false,
    dragActive = false,
    onOpen,
    onOpenRecent,
    onRemoveRecent,
    title,
    description,
    actionLabel,
    recentTitle,
    footnote,
}: BinOpenLandingProps) {
    const { t } = useTranslation();
    const finalTitle = title ?? t('binLanding.title');
    const finalDesc = description ?? t('binLanding.description');
    const finalActionLabel = actionLabel ?? t('binLanding.actionLabel');
    const finalRecentTitle = recentTitle ?? t('binLanding.recentTitle');
    return (
        <div className={`bin-open-landing${dragActive ? ' is-dragging' : ''}`}>
            <div className="bin-open-landing__empty">
                <FolderOpen
                    size={48}
                    color="var(--accent-primary)"
                    strokeWidth={1.5}
                    style={{ display: 'block', marginBottom: 16 }}
                />
                <div className="bin-open-landing__title">{finalTitle}</div>
                <div className="bin-open-landing__description">{finalDesc}</div>
                <button type="button" className="dl-btn dl-btn--primary" onClick={onOpen} disabled={busy}>
                    <span className="dl-icon"><FolderOpen size={14} /></span>
                    <span>{finalActionLabel}</span>
                </button>
                {footnote && <div className="bin-open-landing__footnote">{footnote}</div>}
            </div>

            {recentBins.length > 0 && (
                <section className="bin-open-recent" aria-label={finalRecentTitle}>
                    <div className="bin-open-recent__heading">{finalRecentTitle}</div>
                    <div className="bin-open-recent__list">
                        {recentBins.map((bin) => (
                            <div
                                key={bin.path}
                                className="bin-open-recent__item"
                                role="button"
                                tabIndex={busy ? -1 : 0}
                                aria-disabled={busy}
                                onClick={() => { if (!busy) onOpenRecent(bin.path); }}
                                onKeyDown={(event) => {
                                    if (busy || (event.key !== 'Enter' && event.key !== ' ')) return;
                                    event.preventDefault();
                                    onOpenRecent(bin.path);
                                }}
                                title={bin.path}
                            >
                                <span className="bin-open-recent__info">
                                    <FolderOpen size={15} className="bin-open-recent__icon" />
                                    <span className="bin-open-recent__name">{bin.name}</span>
                                </span>
                                <span className="bin-open-recent__actions">
                                    <span className="bin-open-recent__date">{relativeTime(bin.lastOpened)}</span>
                                    <span
                                        role="button"
                                        tabIndex={0}
                                        className="bin-open-recent__remove"
                                        title={t('common.delete')}
                                        onClick={(event) => {
                                            event.stopPropagation();
                                            onRemoveRecent(bin.path);
                                        }}
                                        onKeyDown={(event) => {
                                            if (event.key !== 'Enter' && event.key !== ' ') return;
                                            event.preventDefault();
                                            event.stopPropagation();
                                            onRemoveRecent(bin.path);
                                        }}
                                    >
                                        <X size={13} />
                                    </span>
                                </span>
                            </div>
                        ))}
                    </div>
                </section>
            )}
        </div>
    );
}

export default BinOpenLanding;
