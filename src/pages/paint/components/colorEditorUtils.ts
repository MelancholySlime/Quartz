import type { ColorData, ColorSlot, VfxEmitter } from '@/lib/api/paint';

export type RGBA = [number, number, number, number];
export interface EditorKeyframe { rgba: RGBA; time: number }
export interface IndexedColorKeyframe extends EditorKeyframe { index: number }
export interface ColorEditorTarget {
    sessionId: number;
    emitterKey: string;
    slot: ColorSlot;
    title: string;
    keyframes: { rgba: number[]; time: number }[];
    exists: boolean;
    animated: boolean;
    storage: ColorData['storage'];
    constantIndex: number | null;
    supportsStructuralEdits: boolean;
    supportsRetime: boolean;
    blendMode: number;
    texturePath: string | null;
    meshPath: string | null;
    binPath: string;
}

export const clamp01 = (value: number) => Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;
export const cloneFrames = (frames: { rgba: number[]; time: number }[]): EditorKeyframe[] =>
    frames.map(frame => ({ rgba: [...frame.rgba] as RGBA, time: frame.time }));
export const rgbaToHex = (rgba: number[]) => `#${rgba.slice(0, 3).map(v => Math.round(clamp01(v) * 255).toString(16).padStart(2, '0')).join('')}`;
export const hexToRgb = (hex: string): [number, number, number] => {
    const value = parseInt(hex.replace('#', ''), 16);
    return [(value >> 16 & 255) / 255, (value >> 8 & 255) / 255, (value & 255) / 255];
};
export const rgbaToCss = (rgba: number[], ignoreAlpha = false) =>
    `rgba(${rgba.slice(0, 3).map(v => Math.round(clamp01(v) * 255)).join(',')},${ignoreAlpha ? 1 : clamp01(rgba[3] ?? 1)})`;

/** Never reorder the editable list: backend indices include the wrapper constant. */
export function sampleColor(frames: EditorKeyframe[], time: number): RGBA {
    if (!frames.length) return [1, 1, 1, 1];
    const sorted = [...frames].sort((a, b) => a.time - b.time);
    if (time < sorted[0].time) return [...sorted[0].rgba];
    for (let i = 1; i < sorted.length; i++) {
        const low = sorted[i - 1], high = sorted[i];
        if (time < high.time) {
            const fraction = clamp01((time - low.time) / (high.time - low.time));
            return low.rgba.map((v, channel) => v + (high.rgba[channel] - v) * fraction) as RGBA;
        }
    }
    return [...sorted[sorted.length - 1].rgba];
}

export function gradientCss(frames: EditorKeyframe[], ignoreAlpha = false): string {
    if (!frames.length) return 'transparent';
    if (frames.length === 1) return rgbaToCss(frames[0].rgba, ignoreAlpha);
    // Dense samples preserve straight RGBA interpolation, including transparent keys.
    return `linear-gradient(90deg, ${Array.from({ length: 101 }, (_, i) => `${rgbaToCss(sampleColor(frames, i / 100), ignoreAlpha)} ${i}%`).join(',')})`;
}

export function colorTargetFields(emitter: VfxEmitter, slot: ColorSlot) {
    const color = emitter.colors[slot];
    return {
        keyframes: color?.keyframes ?? [],
        exists: color != null,
        animated: color != null && !color.isConstant,
        storage: color?.storage ?? 'constant' as ColorData['storage'],
        constantIndex: color?.constantIndex ?? null,
        supportsStructuralEdits: color?.supportsStructuralEdits ?? false,
        supportsRetime: color?.supportsRetime ?? false,
        blendMode: emitter.blendMode,
        meshPath: emitter.textures.find(texture => texture.label === 'Mesh')?.path ?? null,
        texturePath: (emitter.textures.find(texture => texture.label === 'Main Texture')
            ?? emitter.textures.find(texture => texture.label !== 'Mesh'))?.path ?? null,
    };
}
