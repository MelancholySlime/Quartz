import { useUiPrefsStore } from '@/lib/stores/uiPrefsStore';
import enUS, { type Translations } from './locales/en-US';
import zhCN from './locales/zh-CN';
import { type SupportedLocale, type TranslationParams, DEFAULT_LOCALE, SUPPORTED_LOCALES } from './types';

export * from './types';
export type { Translations };

const translations: Record<SupportedLocale, Translations> = {
    'en-US': enUS,
    'zh-CN': zhCN,
};

type Leaves<T> = T extends object
    ? {
          [K in keyof T]: `${Exclude<K, symbol>}${Leaves<T[K]> extends never ? '' : `.${Leaves<T[K]>}`}`;
      }[keyof T]
    : never;

export type TranslationKey = Leaves<Translations>;

function getNestedValue(obj: unknown, path: string): string | undefined {
    if (!obj || typeof obj !== 'object') return undefined;
    const parts = path.split('.');
    let current: unknown = obj;
    for (const part of parts) {
        if (current && typeof current === 'object' && part in (current as Record<string, unknown>)) {
            current = (current as Record<string, unknown>)[part];
        } else {
            return undefined;
        }
    }
    return typeof current === 'string' ? current : undefined;
}

function interpolate(template: string, params?: TranslationParams): string {
    if (!params) return template;
    return template.replace(/\{(\w+)\}/g, (match, key) => {
        if (key in params) {
            return String(params[key]);
        }
        return match;
    });
}

/**
 * Translates a key with fallback: Selected Locale -> en-US -> raw key.
 */
export function t(key: TranslationKey | (string & {}), params?: TranslationParams, localeOverride?: SupportedLocale): string {
    const currentLocale = localeOverride || useUiPrefsStore.getState().locale || DEFAULT_LOCALE;

    // 1. Try selected locale
    let text = getNestedValue(translations[currentLocale], key);

    // 2. Fallback to English
    if (!text && currentLocale !== DEFAULT_LOCALE) {
        text = getNestedValue(translations[DEFAULT_LOCALE], key);
    }

    // 3. Fallback to raw key
    if (!text) {
        return key;
    }

    return interpolate(text, params);
}

/**
 * Reactive hook for React components. Updates component whenever locale changes.
 */
export function useTranslation() {
    const locale = useUiPrefsStore((s) => s.locale) || DEFAULT_LOCALE;
    const setLocale = useUiPrefsStore((s) => s.setLocale);

    const translate = (key: TranslationKey | (string & {}), params?: TranslationParams) => {
        return t(key, params, locale);
    };

    return {
        t: translate,
        locale,
        setLocale,
        supportedLocales: SUPPORTED_LOCALES,
    };
}
