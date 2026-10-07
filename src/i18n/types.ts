export type SupportedLocale = 'en-US' | 'zh-CN';

export interface LocaleOption {
    value: SupportedLocale;
    label: string;
}

export const SUPPORTED_LOCALES: LocaleOption[] = [
    { value: 'en-US', label: 'English' },
    { value: 'zh-CN', label: '简体中文' },
];

export const DEFAULT_LOCALE: SupportedLocale = 'en-US';

export type TranslationParams = Record<string, string | number>;
