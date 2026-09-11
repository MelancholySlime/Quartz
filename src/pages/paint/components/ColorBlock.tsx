/*
 * ColorBlock Component
 * Displays a color or gradient, clickable to import to palette.
 * Uses a wrapper div with solid background to prevent bleed-through.
 * Ported 1:1 from the Electron Quartz paint2 ColorBlock.
 */

import React from 'react';
import { Box } from '@mui/material';
import { Plus, Pencil, Copy } from 'lucide-react';

interface ColorKeyframe {
    rgba: number[];
    time: number;
}

interface ColorBlockProps {
    colors?: ColorKeyframe[];
    title: string;
    variant?: 'standard' | 'secondary' | 'wide';
    /** Open the full color editor (pencil icon + block click + right-click). */
    onEdit?: () => void;
    /** Copy this color's keyframes into the top palette (copy icon). */
    onCopy?: () => void;
    /** Open the editor for a missing color slot. */
    onCreate?: () => void;
    /** Right-click to open the color editor. */
    onContextMenu?: (e: React.MouseEvent) => void;
    /** Draw the swatch opaque (ignore the color's alpha) while still reporting the
     *  true alpha in the tooltip. Used for Fresnel/rim colors, whose `.a` is inert
     *  in the engine — an authored `.a = 0` fresnel is still a visible rim tint, so
     *  it must not render as a transparent/empty chip. */
    ignoreAlpha?: boolean;
}

function ColorBlock({ colors, title, variant = 'standard', onEdit, onCopy, onCreate, onContextMenu, ignoreAlpha }: ColorBlockProps) {
    const dimensions = ({
        standard: { width: 40, height: 26 },
        secondary: { width: 34, height: 24 },
        wide: { width: 110, height: 26 },
    } as Record<string, { width: number; height: number }>)[variant] || { width: 24, height: 24 };

    if (!colors || colors.length === 0) {
        return (
            <Box
                component="button"
                type="button"
                disabled={!onCreate}
                aria-label={`Add ${title}`}
                aria-haspopup="dialog"
                onClick={(event) => { event.stopPropagation(); onCreate?.(); }}
                onContextMenu={onContextMenu}
                title={`Add ${title} / Open color editor`}
                sx={{
                    ...dimensions,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    padding: 0,
                    borderRadius: 'var(--radius-sm)',
                    border: '1px solid var(--border)',
                    background: 'var(--bg-tertiary)',
                    color: 'var(--accent-primary)',
                    opacity: 0.5,
                    cursor: onCreate ? 'pointer' : 'default',
                    flexShrink: 0,
                    transition: 'opacity 0.12s, border-color 0.12s, background 0.12s',
                    '& .paint-color-add': { opacity: 0, transition: 'opacity 0.12s' },
                    '&:hover:not(:disabled), &:focus-visible': {
                        opacity: 1,
                        borderColor: 'var(--accent-primary)',
                        background: 'color-mix(in srgb, var(--accent-primary) 10%, var(--bg-tertiary))',
                        '& .paint-color-add': { opacity: 1 },
                    },
                    '&:focus-visible': { outline: '2px solid var(--accent-primary)', outlineOffset: 2 },
                }}
            >
                <Plus className="paint-color-add" size={16} aria-hidden="true" />
            </Box>
        );
    }

    // Render the color with its ACTUAL alpha so transparency shows directly in
    // the swatch (a checkerboard behind it makes low alpha read at a glance).
    const rgbaToCSS = (rgba: number[]): string => {
        if (!rgba || rgba.length < 3) return 'transparent';
        const toInt = (val: number) => Math.round(Math.max(0, Math.min(1, val)) * 255);
        // Fresnel (ignoreAlpha) draws opaque since its alpha doesn't affect the
        // rendered rim; every other swatch shows its real alpha.
        const a = ignoreAlpha ? 1 : (rgba[3] !== undefined ? Math.max(0, Math.min(1, rgba[3])) : 1);
        return `rgba(${toInt(rgba[0])}, ${toInt(rgba[1])}, ${toInt(rgba[2])}, ${a})`;
    };

    const alphaOf = (c: ColorKeyframe) => (c.rgba[3] !== undefined ? c.rgba[3] : 1);
    // Keep fully-transparent keyframes in the gradient now (they're the point).
    const renderList = colors;

    let background: string;
    if (renderList.length === 1) {
        background = rgbaToCSS(renderList[0].rgba);
    } else {
        const sorted = [...renderList].sort((a, b) => a.time - b.time);
        const stops: string[] = [];
        stops.push(`${rgbaToCSS(sorted[0].rgba)} 0%`);
        sorted.forEach(c => {
            stops.push(`${rgbaToCSS(c.rgba)} ${c.time * 100}%`);
        });
        stops.push(`${rgbaToCSS(sorted[sorted.length - 1].rgba)} 100%`);
        background = `linear-gradient(90deg, ${stops.join(', ')})`;
    }

    const hasAlpha = renderList.some(c => alphaOf(c) < 0.999);
    const alphaText = colors.map(c => alphaOf(c).toFixed(2)).join(', ');
    const tooltipContent = colors.length === 1
        ? `${title}: ${colors[0].rgba.map(v => v.toFixed(2)).join(', ')}\nAlpha: ${alphaText}\n(click or the pencil to edit / copy icon adds to palette)`
        : `${title}: ${colors.length} keyframes\nAlpha: ${alphaText}\n(click or the pencil to edit / copy icon adds to palette)`;

    return (
        <Box
            onClick={(e) => { e.stopPropagation(); onEdit?.(); }}
            onContextMenu={onContextMenu}
            title={tooltipContent}
            sx={{
                ...dimensions,
                borderRadius: 'var(--radius-sm)',
                border: '1px solid var(--border)',
                backgroundColor: 'var(--bg-primary)',
                cursor: 'pointer',
                flexShrink: 0,
                overflow: 'hidden',
                position: 'relative',
                transition: 'transform 0.1s, border-color 0.1s',
                '& .paint-color-actions': { opacity: 0, transition: 'opacity 0.12s' },
                '&:hover': {
                    transform: 'translateY(-1px)',
                    borderColor: 'var(--accent-primary)',
                    '& .paint-color-actions': { opacity: 1 },
                },
            }}
        >
            {/* Checkerboard, only when there's transparency to show through. */}
            {hasAlpha && (
                <Box
                    sx={{
                        position: 'absolute',
                        inset: 0,
                        borderRadius: '3px',
                        backgroundColor: '#888',
                        backgroundImage:
                            'linear-gradient(45deg, rgba(255,255,255,.3) 25%, transparent 25%),' +
                            'linear-gradient(-45deg, rgba(255,255,255,.3) 25%, transparent 25%),' +
                            'linear-gradient(45deg, transparent 75%, rgba(255,255,255,.3) 75%),' +
                            'linear-gradient(-45deg, transparent 75%, rgba(255,255,255,.3) 75%)',
                        backgroundSize: '8px 8px',
                        backgroundPosition: '0 0, 0 4px, 4px -4px, -4px 0',
                    }}
                />
            )}
            <Box
                sx={{
                    position: 'absolute',
                    inset: 0,
                    background,
                    borderRadius: '3px',
                }}
            />
            {/* Hover actions: pencil edits (same as click / right-click), copy
                imports the colors into the top palette. A dark scrim keeps the
                icons legible over any swatch colour. */}
            <Box
                className="paint-color-actions"
                sx={{
                    position: 'absolute',
                    inset: 0,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    gap: '3px',
                    borderRadius: '3px',
                    background: 'rgba(0,0,0,0.42)',
                    backdropFilter: 'blur(1px)',
                }}
            >
                {onEdit && (
                    <Box
                        component="button"
                        type="button"
                        aria-label={`Edit ${title}`}
                        title={`Edit ${title}`}
                        onClick={(e) => { e.stopPropagation(); onEdit(); }}
                        sx={iconBtn}
                    >
                        <Pencil size={12} aria-hidden="true" />
                    </Box>
                )}
                {onCopy && (
                    <Box
                        component="button"
                        type="button"
                        aria-label={`Copy ${title} to palette`}
                        title={`Copy ${title} to palette`}
                        onClick={(e) => { e.stopPropagation(); onCopy(); }}
                        sx={iconBtn}
                    >
                        <Copy size={12} aria-hidden="true" />
                    </Box>
                )}
            </Box>
        </Box>
    );
}

const iconBtn = {
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: 18,
    height: 18,
    padding: 0,
    border: 'none',
    borderRadius: '4px',
    background: 'rgba(255,255,255,0.14)',
    color: '#fff',
    cursor: 'pointer',
    transition: 'background 0.12s',
    '&:hover': { background: 'var(--accent-primary)' },
    '&:focus-visible': { outline: '2px solid var(--accent-primary)', outlineOffset: 1 },
} as const;

export default React.memo(ColorBlock);
