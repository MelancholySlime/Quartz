/*
 * Color-picker controller — the non-component half of the picker, split out of
 * ColorPicker.tsx so that file exports ONLY a React component. Mixing component
 * and non-component exports in one module breaks React Fast Refresh (Vite then
 * *invalidates* the module instead of hot-updating it, so edits silently don't
 * apply). Keeping the imperative API here fixes HMR for the picker.
 */

/** Color-picker debug tracing. Toggle at runtime from the devtools console:
 *   window.__pickerDebug = true
 * Every stage of the pick → commit chain then logs with a `[picker]` prefix. */
export function pdbg(...args: unknown[]): void {
    if ((window as unknown as { __pickerDebug?: boolean }).__pickerDebug) {
        // eslint-disable-next-line no-console
        console.log('[picker]', ...args);
    }
}

/** Opt-in alpha support: pass `alpha` (0..1) to render an alpha slider; the
 *  picker then reports alpha changes through `onAlpha`. Omit for RGB-only. */
export interface ColorPickerOpts {
    alpha?: number;
    onAlpha?: (alpha: number) => void;
}

export interface PickerState {
    anchor: { left: number; top: number; bottom: number; right: number };
    hex: string;
    onCommit: (hex: string) => void;
    alpha: number | null;
    onAlpha?: (alpha: number) => void;
}

let openController: ((s: PickerState | null) => void) | null = null;

/** The mounted `ColorPickerHost` registers its open/close setter here. */
export function setPickerController(fn: ((s: PickerState | null) => void) | null): void {
    openController = fn;
}

export function openColorPicker(
    event: { currentTarget?: Element | null; target?: EventTarget | null },
    initialHex: string,
    onCommit: (hex: string) => void,
    opts?: ColorPickerOpts,
): void {
    const el = (event.currentTarget || event.target) as Element | null;
    const rect = el && 'getBoundingClientRect' in el
        ? (el as Element).getBoundingClientRect()
        : ({ left: 100, top: 100, bottom: 130, right: 130 } as DOMRect);
    pdbg('openColorPicker called', { initialHex, hasController: !!openController, hasAnchorEl: !!el, alpha: opts?.alpha });
    if (!openController) {
        pdbg('NO openController — a <ColorPickerHost/> is not mounted on this page; the picker cannot open');
    }
    openController?.({
        anchor: { left: rect.left, top: rect.top, bottom: rect.bottom, right: rect.right },
        hex: initialHex || '#808080',
        onCommit,
        alpha: opts?.alpha ?? null,
        onAlpha: opts?.onAlpha,
    });
}

export function cleanupColorPickers(): void {
    openController?.(null);
}
