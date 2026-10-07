import { Hammer } from 'lucide-react';
import { useTranslation } from '@/i18n';

export function Placeholder({ title }: { title: string }) {
    const { t } = useTranslation();
    return (
        <div className="grid h-full place-items-center">
            <div className="q-glass flex flex-col items-center gap-3 px-10 py-12 text-center">
                <div
                    className="grid h-14 w-14 place-items-center rounded-2xl"
                    style={{ background: 'color-mix(in srgb, var(--accent) 14%, transparent)', color: 'var(--accent)' }}
                >
                    <Hammer size={26} />
                </div>
                <h1 className="text-xl font-semibold text-white/90">{title}</h1>
                <p className="max-w-xs text-sm text-white/40">
                    {t('placeholder.notPorted')}
                </p>
            </div>
        </div>
    );
}
