import { useEffect, useId, useMemo, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent, PointerEvent } from 'react';
import type { IndexedColorKeyframe, RGBA } from './colorEditorUtils';
import { clamp01 as clamp } from './colorEditorUtils';
import './ColorTimeline.css';

interface ValueCurveProps {
    frames: IndexedColorKeyframe[];
    selectedIndex: number;
    playhead: number;
    disabled: boolean;
    canRetime: boolean;
    canAdd: boolean;
    ignoreAlpha?: boolean;
    channel: 0 | 1 | 2 | 3;
    onSelect: (index: number) => void;
    onDraft: (index: number, rgba: RGBA, time: number) => void;
    onCommit: (index: number) => void;
    onCancel: () => void;
    onAdd: (time: number, channel?: number, value?: number) => void;
}

interface CurveDrag {
    element: SVGCircleElement;
    pointerId: number;
    index: number;
    startX: number;
    startY: number;
    width: number;
    height: number;
    time: number;
    rgba: RGBA;
    channel: 0 | 1 | 2 | 3;
    moved: boolean;
}

const CHANNEL_NAMES = ['Red', 'Green', 'Blue', 'Alpha'];
const CHANNEL_COLORS = ['#ee939b', '#8ed7b0', '#91b0ff', 'var(--accent-primary, #b1a0ff)'];
const TICKS = [0, 0.25, 0.5, 0.75, 1];
const PADDING = { left: 40, right: 18, top: 18, bottom: 36 };

export function ValueCurve({
    frames, selectedIndex, playhead, disabled, canRetime, canAdd, ignoreAlpha = false, channel,
    onSelect, onDraft, onCommit, onCancel, onAdd,
}: ValueCurveProps) {
    const svgRef = useRef<SVGSVGElement>(null);
    const dragRef = useRef<CurveDrag | null>(null);
    const suppressClickRef = useRef(false);
    const [width, setWidth] = useState(620);
    const [height, setHeight] = useState(246);
    const fillId = useId();
    const inertAlpha = ignoreAlpha && channel === 3;
    const plotWidth = Math.max(1, width - PADDING.left - PADDING.right);
    const plotHeight = height - PADDING.top - PADDING.bottom;
    const x = (time: number) => PADDING.left + clamp(time) * plotWidth;
    const y = (value: number) => PADDING.top + (1 - clamp(value)) * plotHeight;
    const sorted = useMemo(() => [...frames].sort((a, b) => a.time - b.time), [frames]);
    const points = sorted.length ? [
        `${x(0)},${y(sorted[0].rgba[channel])}`,
        ...sorted.map((frame) => `${x(frame.time)},${y(frame.rgba[channel])}`),
        `${x(1)},${y(sorted[sorted.length - 1].rgba[channel])}`,
    ].join(' ') : '';

    const cancelDrag = () => {
        const drag = dragRef.current;
        if (!drag) return;
        dragRef.current = null;
        suppressClickRef.current = true;
        if (drag.element.hasPointerCapture(drag.pointerId)) drag.element.releasePointerCapture(drag.pointerId);
        onCancel();
    };

    useEffect(() => {
        if (disabled || inertAlpha) cancelDrag();
    }, [disabled, inertAlpha]);

    useEffect(() => {
        const svg = svgRef.current;
        if (!svg) return;
        const observer = new ResizeObserver(([entry]) => {
            if (entry.contentRect.width > 0) setWidth(Math.max(120, Math.round(entry.contentRect.width)));
            // Match the viewBox to the available height so compact layouts keep
            // full-size axis labels and the same 28px point hit targets.
            if (entry.contentRect.height > 0) setHeight(Math.max(100, Math.round(entry.contentRect.height)));
        });
        observer.observe(svg);
        return () => observer.disconnect();
    }, []);

    const beginDrag = (event: PointerEvent<SVGCircleElement>, frame: IndexedColorKeyframe) => {
        if (disabled || inertAlpha || event.button !== 0 || dragRef.current) return;
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.focus();
        onSelect(frame.index);
        const rect = svgRef.current!.getBoundingClientRect();
        dragRef.current = {
            element: event.currentTarget,
            pointerId: event.pointerId,
            index: frame.index,
            startX: event.clientX,
            startY: event.clientY,
            width: Math.max(1, rect.width * plotWidth / width),
            height: Math.max(1, rect.height * plotHeight / height),
            time: frame.time,
            rgba: [...frame.rgba],
            channel,
            moved: false,
        };
        event.currentTarget.setPointerCapture(event.pointerId);
    };

    const moveDrag = (event: PointerEvent<SVGCircleElement>) => {
        if (disabled || inertAlpha) { cancelDrag(); return; }
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        const dx = event.clientX - drag.startX;
        const dy = event.clientY - drag.startY;
        if (!drag.moved && Math.hypot(canRetime ? dx : 0, dy) < 3) return;
        event.preventDefault();
        drag.moved = true;
        const rgba: RGBA = [...drag.rgba];
        rgba[drag.channel] = clamp(drag.rgba[drag.channel] - dy / drag.height);
        onDraft(drag.index, rgba, canRetime ? clamp(drag.time + dx / drag.width) : drag.time);
    };

    const endDrag = (event: PointerEvent<SVGCircleElement>, cancelled = false) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        dragRef.current = null;
        suppressClickRef.current = cancelled || drag.moved;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        if (cancelled) onCancel();
        else if (drag.moved) onCommit(drag.index);
    };

    const handleKey = (event: KeyboardEvent<SVGCircleElement>, frame: IndexedColorKeyframe) => {
        if (event.key === 'Escape' && dragRef.current) {
            event.preventDefault();
            event.stopPropagation();
            cancelDrag();
            return;
        }
        if (dragRef.current && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'Delete', 'Backspace', 'Enter', ' '].includes(event.key)) {
            event.preventDefault();
            event.stopPropagation();
            return;
        }
        if (disabled || inertAlpha) return;
        if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            onSelect(frame.index);
            return;
        }
        if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        event.stopPropagation();
        onSelect(frame.index);
        const step = event.shiftKey ? 0.1 : 0.01;
        const rgba: RGBA = [...frame.rgba];
        let time = frame.time;
        if (event.key === 'ArrowUp' || event.key === 'ArrowDown') rgba[channel] = clamp(rgba[channel] + (event.key === 'ArrowUp' ? step : -step));
        if (canRetime && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) time = clamp(time + (event.key === 'ArrowRight' ? step : -step));
        if (event.key === 'Home') rgba[channel] = 0;
        if (event.key === 'End') rgba[channel] = 1;
        if (time !== frame.time || rgba[channel] !== frame.rgba[channel]) {
            onDraft(frame.index, rgba, time);
            onCommit(frame.index);
        }
    };

    return (
        <div
            className="ce-curve"
            style={{ '--ce-channel-color': CHANNEL_COLORS[channel] } as CSSProperties}
            onPointerDownCapture={() => { suppressClickRef.current = false; }}
            onClickCapture={(event) => {
                if (!suppressClickRef.current) return;
                suppressClickRef.current = false;
                event.preventDefault();
                event.stopPropagation();
            }}
        >
            <svg
                ref={svgRef}
                className="ce-curve-svg"
                width="100%"
                height={height}
                viewBox={`0 0 ${width} ${height}`}
                preserveAspectRatio="none"
                role="group"
                aria-label={`${CHANNEL_NAMES[channel]} curve. Drag points or use arrow keys to edit. Shift adjusts by ten percent.`}
                data-can-add={canAdd && !disabled && !inertAlpha}
                onClick={(event) => {
                    if (disabled || !canAdd || inertAlpha) return;
                    const rect = event.currentTarget.getBoundingClientRect();
                    const px = (event.clientX - rect.left) * width / Math.max(1, rect.width);
                    const py = (event.clientY - rect.top) * height / Math.max(1, rect.height);
                    if (px < PADDING.left || px > width - PADDING.right || py < PADDING.top || py > height - PADDING.bottom) return;
                    onAdd(clamp((px - PADDING.left) / plotWidth), channel, clamp(1 - (py - PADDING.top) / plotHeight));
                }}
            >
                <defs>
                    <linearGradient id={fillId} x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" stopColor="var(--ce-channel-color)" stopOpacity="0.22" />
                        <stop offset="100%" stopColor="var(--ce-channel-color)" stopOpacity="0.02" />
                    </linearGradient>
                </defs>
                <rect className="ce-curve-plot" x={PADDING.left} y={PADDING.top} width={plotWidth} height={plotHeight} rx="4" />
                <g className="ce-curve-grid" aria-hidden="true">
                    {TICKS.map((tick) => (
                        <g key={tick}>
                            <line x1={x(tick)} y1={y(0)} x2={x(tick)} y2={y(1)} />
                            <line x1={x(0)} y1={y(tick)} x2={x(1)} y2={y(tick)} />
                            <text x={PADDING.left - 10} y={y(tick) + 4} textAnchor="end">{tick.toFixed(2)}</text>
                            <text x={x(tick)} y={height - 15} textAnchor="middle">{Math.round(tick * 100)}%</text>
                        </g>
                    ))}
                </g>
                {points && (
                    <g className="ce-curve-drawing" aria-hidden="true">
                        <polygon points={`${x(0)},${y(0)} ${points} ${x(1)},${y(0)}`} fill={`url(#${fillId})`} />
                        <polyline className="ce-curve-line" points={points} />
                    </g>
                )}
                <line className="ce-curve-playhead" x1={x(playhead)} y1={y(1)} x2={x(playhead)} y2={y(0)} aria-hidden="true" />
                {frames.map((frame, stopNumber) => (
                    <g key={frame.index} className="ce-curve-point" data-selected={frame.index === selectedIndex} data-disabled={disabled || inertAlpha}>
                        <circle className="ce-curve-point-halo" cx={x(frame.time)} cy={y(frame.rgba[channel])} r="10" aria-hidden="true" />
                        <circle className="ce-curve-point-dot" cx={x(frame.time)} cy={y(frame.rgba[channel])} r="5" aria-hidden="true" />
                        <circle
                            className="ce-curve-point-hit"
                            cx={x(frame.time)}
                            cy={y(frame.rgba[channel])}
                            r="14"
                            role="slider"
                            tabIndex={disabled || inertAlpha ? -1 : 0}
                            aria-label={`Stop ${stopNumber + 1} ${CHANNEL_NAMES[channel]}`}
                            aria-valuemin={0}
                            aria-valuemax={1}
                            aria-valuenow={Number(frame.rgba[channel].toFixed(3))}
                            aria-valuetext={`${frame.rgba[channel].toFixed(2)} at ${Math.round(frame.time * 100)} percent time`}
                            aria-disabled={disabled || inertAlpha}
                            aria-orientation="vertical"
                            onClick={(event) => { event.stopPropagation(); if (!disabled) onSelect(frame.index); }}
                            onPointerDown={(event) => beginDrag(event, frame)}
                            onPointerMove={moveDrag}
                            onPointerUp={(event) => endDrag(event)}
                            onPointerCancel={(event) => endDrag(event, true)}
                            onLostPointerCapture={(event) => endDrag(event, true)}
                            onBlur={cancelDrag}
                            onKeyDown={(event) => handleKey(event, frame)}
                        >
                            <title>{`Stop ${stopNumber + 1} / ${CHANNEL_NAMES[channel]} ${frame.rgba[channel].toFixed(2)} / Time ${Math.round(frame.time * 100)}%`}</title>
                        </circle>
                    </g>
                ))}
            </svg>
            <div className="ce-timeline-hint ce-curve-hint">
                <span>{inertAlpha ? 'Fresnel uses RGB only. Alpha has no visible effect.' : canRetime ? 'Drag points to shape the curve' : 'Drag points vertically to adjust the channel'}</span>
                <span>{inertAlpha ? '' : 'Arrow keys for precision / Shift for larger steps'}</span>
            </div>
        </div>
    );
}
