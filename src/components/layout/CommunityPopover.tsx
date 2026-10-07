import { useEffect, useRef, Fragment } from 'react';
import { ExternalLink, ArrowRight, Github } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from '@/i18n';

const GROUPS = [
    [
        { icon: ExternalLink, key: 'community.website', fallback: 'Website', url: 'https://divineskins.gg' },
        { icon: ArrowRight, key: 'community.wiki', fallback: 'Wiki', url: 'https://wiki.divineskins.gg' },
    ],
    [
        { icon: Github, key: 'community.quartzGithub', fallback: 'Quartz on GitHub', url: 'https://github.com/LeagueToolkit/Quartz' },
        { icon: Github, key: 'community.ritosharkGithub', fallback: 'RitoShark on GitHub', url: 'https://github.com/RitoShark' },
    ],
];

function openExternal(url: string) {
    openUrl(url).catch(() => { window.open(url, '_blank'); });
}

interface CommunityPopoverProps {
    anchorRect: DOMRect;
    onClose: () => void;
}

/** Titlebar globe menu — the app's external destinations (Website, Wiki).
    Anchored under the globe button; closes on outside click or Escape. */
export function CommunityPopover({ anchorRect, onClose }: CommunityPopoverProps) {
    const { t } = useTranslation();
    const ref = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const onDown = (e: MouseEvent) => {
            if (ref.current && !ref.current.contains(e.target as Node)) onClose();
        };
        const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
        document.addEventListener('mousedown', onDown);
        document.addEventListener('keydown', onKey);
        return () => {
            document.removeEventListener('mousedown', onDown);
            document.removeEventListener('keydown', onKey);
        };
    }, [onClose]);

    const style: React.CSSProperties = {
        top: anchorRect.bottom + 8,
        right: window.innerWidth - anchorRect.right,
    };

    return (
        <div ref={ref} className="q-community-pop" style={style} role="menu">
            {GROUPS.map((group, gi) => (
                <Fragment key={gi}>
                    {gi > 0 && <div className="q-community-pop__sep" />}
                    {group.map((item) => {
                        const Icon = item.icon;
                        return (
                            <button
                                key={item.key}
                                type="button"
                                role="menuitem"
                                className="q-community-pop__item"
                                onClick={() => { onClose(); openExternal(item.url); }}
                            >
                                <Icon size={16} className="q-community-pop__icon" />
                                <span>{t(item.key) || item.fallback}</span>
                            </button>
                        );
                    })}
                </Fragment>
            ))}
        </div>
    );
}
