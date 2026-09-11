/*
 * League VFX blend modes — ported from RubyRe's byte-verified table
 * (render/blend.ts, from Riot_VfxParticle_BuildBlendDepthState @0x1412CF8D0).
 *
 * Equation per channel: out = src*srcFactor  OP  dst*dstFactor, applied to
 * color and alpha independently. Premultiply (rgb*=a; a=1) applies to modes 0
 * and 2 ONLY, on the source, before compositing. Used to preview how an
 * emitter's color composites in-game.
 */

export interface BlendMode {
    value: number;
    name: string;
    /** rgb *= a on the source before compositing (modes 0 and 2 only). */
    premultiply: boolean;
}

/** The blend modes League actually uses, in value order. */
export const BLEND_MODES: BlendMode[] = [
    { value: 0, name: 'Additive (premult)', premultiply: true },
    { value: 1, name: 'Alpha', premultiply: false },
    { value: 2, name: 'Darken (inv-src)', premultiply: true },
    { value: 3, name: 'Opaque', premultiply: false },
    { value: 4, name: 'Additive', premultiply: false },
    { value: 5, name: 'Premultiplied', premultiply: false },
    { value: 6, name: 'Min (darken)', premultiply: false },
    { value: 7, name: 'Max (lighten)', premultiply: false },
    { value: 8, name: 'Dst-alpha', premultiply: false },
];

export function blendModeName(value: number): string {
    return BLEND_MODES.find((m) => m.value === value)?.name ?? `Mode ${value}`;
}

type Op = 'add' | 'min' | 'max';
/** Blend factor kinds referencing src/dst channels (League factor enum). */
type Factor = 'zero' | 'one' | 'srcColor' | 'oneMinusSrcColor' | 'srcAlpha' | 'oneMinusSrcAlpha' | 'dstAlpha' | 'oneMinusDstAlpha';

interface BlendEq {
    srcColor: Factor; dstColor: Factor;
    srcAlpha: Factor; dstAlpha: Factor;
    op: Op;
    /** Opaque: blending disabled, source replaces dest with alpha forced to 1. */
    opaque?: boolean;
}

/* Byte-verified factor set per mode (RubyRe render/blend.ts:54-95). */
const EQ: Record<number, BlendEq> = {
    0: { srcColor: 'one', dstColor: 'one', srcAlpha: 'one', dstAlpha: 'one', op: 'add' },
    1: { srcColor: 'srcAlpha', dstColor: 'oneMinusSrcAlpha', srcAlpha: 'srcAlpha', dstAlpha: 'oneMinusSrcAlpha', op: 'add' },
    2: { srcColor: 'zero', dstColor: 'oneMinusSrcColor', srcAlpha: 'zero', dstAlpha: 'oneMinusSrcAlpha', op: 'add' },
    3: { srcColor: 'one', dstColor: 'zero', srcAlpha: 'one', dstAlpha: 'zero', op: 'add', opaque: true },
    4: { srcColor: 'srcAlpha', dstColor: 'one', srcAlpha: 'srcAlpha', dstAlpha: 'one', op: 'add' },
    5: { srcColor: 'one', dstColor: 'oneMinusSrcAlpha', srcAlpha: 'one', dstAlpha: 'oneMinusSrcAlpha', op: 'add' },
    6: { srcColor: 'one', dstColor: 'one', srcAlpha: 'one', dstAlpha: 'one', op: 'min' },
    7: { srcColor: 'one', dstColor: 'one', srcAlpha: 'one', dstAlpha: 'one', op: 'max' },
    8: { srcColor: 'oneMinusDstAlpha', dstColor: 'dstAlpha', srcAlpha: 'one', dstAlpha: 'one', op: 'add' },
};

/** Evaluate a factor (0..1) given src/dst channel values (0..1). */
function factor(f: Factor, sC: number, sA: number, dA: number): number {
    switch (f) {
        case 'zero': return 0;
        case 'one': return 1;
        case 'srcColor': return sC;
        case 'oneMinusSrcColor': return 1 - sC;
        case 'srcAlpha': return sA;
        case 'oneMinusSrcAlpha': return 1 - sA;
        case 'dstAlpha': return dA;
        case 'oneMinusDstAlpha': return 1 - dA;
    }
}

/**
 * Composite a source pixel over a dest pixel with League blend `mode`.
 * All inputs/outputs are 0..1. Returns [r,g,b,a]. Straight (non-premultiplied)
 * src/dst in; the premultiply step (modes 0/2) is applied here.
 */
export function compositePixel(
    mode: number,
    src: [number, number, number, number],
    dst: [number, number, number, number],
): [number, number, number, number] {
    const m = BLEND_MODES.find((b) => b.value === mode);
    const eq = EQ[mode] ?? EQ[1];
    let [sr, sg, sb, sa] = src;
    if (m?.premultiply) { sr *= sa; sg *= sa; sb *= sa; sa = 1; }
    if (eq.opaque) return [sr, sg, sb, 1];
    const [dr, dg, db, da] = dst;
    const chan = (s: number, d: number, sf: Factor, df: Factor): number => {
        const a = s * factor(sf, s, sa, da);
        const b = d * factor(df, s, sa, da);
        if (eq.op === 'min') return Math.min(s, d);
        if (eq.op === 'max') return Math.max(s, d);
        return a + b;
    };
    return [
        Math.max(0, Math.min(1, chan(sr, dr, eq.srcColor, eq.dstColor))),
        Math.max(0, Math.min(1, chan(sg, dg, eq.srcColor, eq.dstColor))),
        Math.max(0, Math.min(1, chan(sb, db, eq.srcColor, eq.dstColor))),
        Math.max(0, Math.min(1, chan(sa, da, eq.srcAlpha, eq.dstAlpha))),
    ];
}
