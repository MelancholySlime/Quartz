import { useEffect, useMemo, useRef } from 'react';
import type { KeyboardEvent, PointerEvent } from 'react';
import type { IndexedColorKeyframe, RGBA } from './colorEditorUtils';
import { clamp01 as clamp, gradientCss, rgbaToCss as cssColor } from './colorEditorUtils';
import './ColorTimeline.css';

interface GradientRampProps {
    frames: IndexedColorKeyframe[];
    selectedIndex: number;
    playhead: number;
    disabled: boolean;
    canRetime: boolean;
    canAdd: boolean;
    ignoreAlpha?: boolean;
    onSelect: (index: number) => void;
    onDraft: (index: number, rgba: RGBA, time: number) => void;
    onCommit: (index: number) => void;
    onCancel: () => void;
    onAdd: (time: number, channel?: number, value?: number) => void;
    onEditColor: (index: number, event: { currentTarget: Element }) => void;
}

interface RampDrag {
    element: HTMLButtonElement;
    pointerId: number;
    index: number;
    startX: number;
    width: number;
    time: number;
    rgba: RGBA;
    moved: boolean;
}

export function GradientRamp({
    frames, selectedIndex, playhead, disabled, canRetime, canAdd, ignoreAlpha = false,
    onSelect, onDraft, onCommit, onCancel, onAdd, onEditColor,
}: GradientRampProps) {
    const trackRef = useRef<HTMLDivElement>(null);
    const dragRef = useRef<RampDrag | null>(null);
    const suppressClickRef = useRef(false);
    const gradient = useMemo(() => gradientCss(frames, ignoreAlpha), [frames, ignoreAlpha]);

    const cancelDrag = () => {
        const drag = dragRef.current;
        if (!drag) return;
        dragRef.current = null;
        suppressClickRef.current = true;
        if (drag.element.hasPointerCapture(drag.pointerId)) drag.element.releasePointerCapture(drag.pointerId);
        onCancel();
    };

    useEffect(() => {
        if (disabled || !canRetime) cancelDrag();
    }, [disabled, canRetime]);

    const beginDrag = (event: PointerEvent<HTMLButtonElement>, frame: IndexedColorKeyframe) => {
        if (disabled || event.button !== 0 || dragRef.current) return;
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget.focus();
        onSelect(frame.index);
        if (!canRetime) return;
        dragRef.current = {
            element: event.currentTarget,
            pointerId: event.pointerId,
            index: frame.index,
            startX: event.clientX,
            width: Math.max(1, trackRef.current?.getBoundingClientRect().width ?? 1),
            time: frame.time,
            rgba: [...frame.rgba],
            moved: false,
        };
        event.currentTarget.setPointerCapture(event.pointerId);
    };

    const moveDrag = (event: PointerEvent<HTMLButtonElement>) => {
        if (disabled || !canRetime) { cancelDrag(); return; }
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        if (!drag.moved && Math.abs(event.clientX - drag.startX) < 3) return;
        event.preventDefault();
        drag.moved = true;
        onDraft(drag.index, [...drag.rgba], clamp(drag.time + (event.clientX - drag.startX) / drag.width));
    };

    const endDrag = (event: PointerEvent<HTMLButtonElement>, cancelled = false) => {
        const drag = dragRef.current;
        if (!drag || drag.pointerId !== event.pointerId) return;
        dragRef.current = null;
        suppressClickRef.current = cancelled || drag.moved;
        if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
        if (cancelled) onCancel();
        else if (drag.moved) onCommit(drag.index);
    };

    const handleKey = (event: KeyboardEvent<HTMLButtonElement>, frame: IndexedColorKeyframe) => {
        if (event.key === 'Escape' && dragRef.current) {
            event.preventDefault();
            event.stopPropagation();
            cancelDrag();
            return;
        }
        if (dragRef.current && ['ArrowLeft', 'ArrowRight', 'Home', 'End', 'Delete', 'Backspace', 'Enter', ' '].includes(event.key)) {
            event.preventDefault();
            event.stopPropagation();
            return;
        }
        if (disabled) return;
        if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            event.stopPropagation();
            if (!event.repeat) {
                suppressClickRef.current = false;
                onSelect(frame.index);
                onEditColor(frame.index, event);
            }
            return;
        }
        if (!canRetime || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();
        event.stopPropagation();
        onSelect(frame.index);
        const step = event.shiftKey ? 0.1 : 0.01;
        const time = event.key === 'Home' ? 0 : event.key === 'End' ? 1
            : clamp(frame.time + (event.key === 'ArrowRight' ? step : -step));
        if (time !== frame.time) {
            onDraft(frame.index, [...frame.rgba], time);
            onCommit(frame.index);
        }
    };

    return (
        <div
            className="ce-gradient"
            aria-label="Color gradient timeline"
            onPointerDownCapture={() => { suppressClickRef.current = false; }}
            onClickCapture={(event) => {
                if (!suppressClickRef.current) return;
                suppressClickRef.current = false;
                event.preventDefault();
                event.stopPropagation();
            }}
        >
            <div className="ce-gradient-track-area">
                <div
                    ref={trackRef}
                    className="ce-gradient-track ce-checker"
                    data-can-add={canAdd && !disabled}
                    title={canAdd && !disabled ? 'Click to add a color stop' : undefined}
                    onClick={(event) => {
                        if (disabled || !canAdd) return;
                        const rect = event.currentTarget.getBoundingClientRect();
                        onAdd(clamp((event.clientX - rect.left) / Math.max(1, rect.width)));
                    }}
                >
                    <div className="ce-gradient-fill" style={{ background: gradient }} />
                    <div className="ce-ramp-playhead" style={{ left: `${clamp(playhead) * 100}%` }} aria-hidden="true">
                        <span />
                    </div>
                </div>
                <div className="ce-gradient-handles">
                    {frames.map((frame, stopNumber) => (
                        <button
                            key={frame.index}
                            type="button"
                            role="slider"
                            className="ce-gradient-stop"
                            data-selected={frame.index === selectedIndex}
                            data-retime={canRetime}
                            aria-disabled={disabled}
                            tabIndex={disabled ? -1 : 0}
                            aria-label={`Color stop ${stopNumber + 1} time`}
                            aria-valuemin={0}
                            aria-valuemax={1}
                            aria-valuenow={Number(frame.time.toFixed(3))}
                            aria-valuetext={`${Math.round(frame.time * 100)} percent. Click or press Enter to edit color${ignoreAlpha ? '' : ' and alpha'}.`}
                            aria-orientation="horizontal"
                            title={`Stop ${stopNumber + 1} / ${Math.round(frame.time * 100)}% / Click to edit color${ignoreAlpha ? '' : ' and alpha'}${canRetime ? ' / Drag to retime' : ''}`}
                            style={{ left: `${clamp(frame.time) * 100}%` }}
                            onClick={(event) => {
                                event.stopPropagation();
                                if (!disabled) {
                                    onSelect(frame.index);
                                    onEditColor(frame.index, event);
                                }
                            }}
                            onPointerDown={(event) => beginDrag(event, frame)}
                            onPointerMove={moveDrag}
                            onPointerUp={(event) => endDrag(event)}
                            onPointerCancel={(event) => endDrag(event, true)}
                            onLostPointerCapture={(event) => endDrag(event, true)}
                            onBlur={cancelDrag}
                            onKeyDown={(event) => handleKey(event, frame)}
                        >
                            <span className="ce-stop-swatch ce-checker">
                                <span style={{ background: cssColor(frame.rgba, ignoreAlpha) }} />
                            </span>
                            <span className="ce-stop-label">{Math.round(frame.time * 100)}%</span>
                        </button>
                    ))}
                </div>
                <div className="ce-ramp-axis" aria-hidden="true">
                    {[0, 25, 50, 75, 100].map((tick) => <span key={tick} style={{ left: `${tick}%` }}>{tick}%</span>)}
                </div>
            </div>
            <div className="ce-timeline-hint">
                <span>{canRetime ? 'Click stops to edit color / Drag to change time' : 'Click a stop to edit its color'}</span>
                {canAdd && <span>Click the ramp to add a stop</span>}
            </div>
        </div>
    );
}
