import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { Activity, Check, ChevronDown, CirclePlus, LoaderCircle, Trash2, X } from 'lucide-react';
import {
    paintCreateColor, paintAnimateColor, paintDeanimateColor, paintAddKeyframe,
    paintDeleteKeyframe, paintSetKeyframe, paintSetBlendMode, type VfxModel,
} from '@/lib/api/paint';
import { openColorPicker, cleanupColorPickers } from './colorPickerController';
import { BLEND_MODES } from '../utils/blendModes';
import { GradientRamp } from './GradientRamp';
import { ValueCurve } from './ValueCurve';
import { ColorPreview } from './ColorPreview';
import {
    clamp01, cloneFrames, hexToRgb, rgbaToHex, rgbaToCss, sampleColor,
    type ColorEditorTarget, type EditorKeyframe, type RGBA,
} from './colorEditorUtils';
import './ColorEditorModal.css';

interface Props {
    target: ColorEditorTarget | null;
    onModel: (next: VfxModel, message: string) => void;
    onRefresh: (message: string) => void | Promise<void>;
    onClose: () => void;
}

export default function ColorEditorModal(props: Props) {
    return props.target ? createPortal(
        <ColorEditorSession {...props} target={props.target}
            key={`${props.target.sessionId}/${props.target.emitterKey}/${props.target.slot}`} />,
        document.body,
    ) : null;
}

function ColorEditorSession({ target, onModel, onRefresh, onClose }: Props & { target: ColorEditorTarget }) {
    const [draft, setDraft] = useState(() => cloneFrames(target.keyframes));
    const draftRef = useRef(draft);
    const firstIndex = target.animated && target.constantIndex === 0 ? 1 : 0;
    const [selectedIndex, setSelectedIndex] = useState(firstIndex);
    const [channel, setChannel] = useState<0 | 1 | 2 | 3>(target.slot === 'fresnelColor' ? 0 : 3);
    const [playhead, setPlayhead] = useState(0);
    const [busy, setBusy] = useState(false);
    const [modified, setModified] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [status, setStatus] = useState('Changes apply to this session');
    const busyRef = useRef(false);
    const aliveRef = useRef(true);
    const dialogRef = useRef<HTMLDivElement>(null);
    const pendingRef = useRef<{ index: number; frame: EditorKeyframe } | null>(null);
    const inFlightRef = useRef<{ index: number; frame: EditorKeyframe } | null>(null);
    const timerRef = useRef<ReturnType<typeof setTimeout>>();
    const flushRef = useRef<() => Promise<boolean>>(async () => true);
    const savedFramesRef = useRef(cloneFrames(target.keyframes));
    const expectedSourceRef = useRef(target.keyframes);

    const showDraft = useCallback((frames: EditorKeyframe[]) => {
        draftRef.current = frames;
        setDraft(frames);
    }, []);

    useEffect(() => {
        if (target.keyframes !== expectedSourceRef.current) {
            // Undo/reload may reshape the list while a popover is open.
            pendingRef.current = null;
            if (timerRef.current) clearTimeout(timerRef.current);
            cleanupColorPickers();
        }
        savedFramesRef.current = cloneFrames(target.keyframes);
        const fresh = cloneFrames(target.keyframes);
        const pending = pendingRef.current ?? inFlightRef.current;
        if (pending && fresh[pending.index]) fresh[pending.index] = pending.frame;
        showDraft(fresh);
        setModified(!!pending);
        setSelectedIndex(index => Math.max(0, Math.min(index, fresh.length - 1)));
    }, [target.keyframes, showDraft]);

    useEffect(() => {
        aliveRef.current = true;
        const previous = document.activeElement as HTMLElement | null;
        let lastFocus: HTMLElement | SVGElement | null = null;
        const overflow = document.body.style.overflow;
        document.body.style.overflow = 'hidden';
        dialogRef.current?.focus();
        // The shared picker is a sibling portal. Include it in the dialog's focus
        // boundary, and handle Tab there as well as inside the modal itself.
        const focusGuard = (event: FocusEvent) => {
            const element = event.target as HTMLElement | null;
            if (element && dialogRef.current?.contains(element)) lastFocus = element;
            else if (!element?.closest('.paint-color-picker')) {
                if (lastFocus?.isConnected && !lastFocus.matches(':disabled,[aria-disabled="true"]')) lastFocus.focus();
                else dialogRef.current?.focus();
            }
        };
        const trapKey = (event: globalThis.KeyboardEvent) => {
            if (event.key === 'Escape' && document.querySelector('.paint-color-picker')) {
                event.preventDefault(); event.stopPropagation(); cleanupColorPickers();
                if (lastFocus?.isConnected) lastFocus.focus();
                else dialogRef.current?.focus();
            }
            if (event.key !== 'Tab') return;
            const nodes = Array.from(document.querySelectorAll<HTMLElement>(
                '.ce-dialog button:not(:disabled),.ce-dialog input:not(:disabled),.ce-dialog select:not(:disabled),.ce-dialog [tabindex="0"],.paint-color-picker input,.paint-color-picker button'))
                .filter(element => element.getClientRects().length > 0 && !element.matches('[aria-disabled="true"]'));
            const first = nodes[0], last = nodes[nodes.length - 1];
            if (event.shiftKey && (document.activeElement === first || document.activeElement === dialogRef.current)) {
                event.preventDefault(); last?.focus();
            } else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
        };
        document.addEventListener('focusin', focusGuard);
        document.addEventListener('keydown', trapKey, true);
        return () => {
            aliveRef.current = false;
            document.removeEventListener('focusin', focusGuard);
            document.removeEventListener('keydown', trapKey, true);
            if (timerRef.current) clearTimeout(timerRef.current);
            cleanupColorPickers();
            document.body.style.overflow = overflow;
            previous?.focus();
        };
    }, []);

    const probability = target.storage === 'probabilityTables';
    const ignoreAlpha = target.slot === 'fresnelColor';
    const multiplierIndex = target.animated ? target.constantIndex : null;
    const frames = useMemo(() => draft.map((frame, index) => ({ ...frame, index }))
        .filter(frame => frame.index !== multiplierIndex), [draft, multiplierIndex]);
    const selected = draft[selectedIndex];
    const isMultiplier = selectedIndex === multiplierIndex;
    const locked = busy || !!pendingRef.current;
    const canAdd = target.exists && target.supportsStructuralEdits;
    const canDelete = canAdd && target.animated && !isMultiplier && frames.length > 1;

    // Serialize mutations. A structural operation must never race a picker/drag.
    const run = async (command: () => Promise<VfxModel | null>, message: string, select?: (frames: EditorKeyframe[]) => number) => {
        if (busyRef.current || pendingRef.current) return false;
        busyRef.current = true;
        setBusy(true);
        setError(null);
        try {
            const next = await command();
            if (!aliveRef.current) return false;
            if (next) {
                const color = next.emitters.find(emitter => emitter.key === target.emitterKey)?.colors[target.slot];
                expectedSourceRef.current = color?.keyframes ?? [];
                const fresh = cloneFrames(color?.keyframes ?? []);
                savedFramesRef.current = fresh;
                const pending = pendingRef.current as { index: number; frame: EditorKeyframe } | null;
                const visible = cloneFrames(fresh);
                if (pending && visible[pending.index]) visible[pending.index] = pending.frame;
                showDraft(visible);
                if (select) setSelectedIndex(Math.max(0, select(fresh)));
                onModel(next, message);
                setStatus(message);
            } else {
                const fresh = cloneFrames(savedFramesRef.current);
                const pending = pendingRef.current as { index: number; frame: EditorKeyframe } | null;
                if (pending && fresh[pending.index]) fresh[pending.index] = pending.frame;
                showDraft(fresh);
                setStatus('No changes');
            }
            setModified(!!pendingRef.current);
            return true;
        } catch (reason) {
            if (aliveRef.current) {
                setError(reason instanceof Error ? reason.message : String(reason));
                pendingRef.current = null;
                showDraft(cloneFrames(savedFramesRef.current));
                setModified(false);
                setStatus('Change could not be applied');
                cleanupColorPickers();
            }
            return false;
        } finally {
            inFlightRef.current = null;
            busyRef.current = false;
            if (aliveRef.current) {
                setBusy(false);
                if (pendingRef.current) void flushRef.current();
            }
        }
    };

    const editDraft = (index: number, rgba: RGBA, time: number) => {
        if (!draftRef.current[index]) return;
        const next = cloneFrames(draftRef.current);
        next[index] = { rgba: [...rgba], time };
        showDraft(next);
        setModified(true);
    };
    const cancelDraft = () => {
        pendingRef.current = null;
        if (timerRef.current) clearTimeout(timerRef.current);
        showDraft(cloneFrames(savedFramesRef.current));
        setModified(false);
    };

    const commitFrame = async (index: number, frame = draftRef.current[index]) => {
        if (!frame) return true;
        if (busyRef.current || pendingRef.current) return false;
        const saved = savedFramesRef.current[index];
        if (saved && saved.time === frame.time && saved.rgba.every((value, c) => value === frame.rgba[c])) {
            setModified(false);
            return true;
        }
        inFlightRef.current = { index, frame };
        return run(() => paintSetKeyframe(target.sessionId, target.emitterKey, target.slot, index, frame.rgba, frame.time), 'Color keyframe updated');
    };
    const flushPicker = async () => {
        if (timerRef.current) clearTimeout(timerRef.current);
        if (busyRef.current) return false;
        const pending = pendingRef.current;
        pendingRef.current = null;
        return pending ? commitFrame(pending.index, pending.frame) : true;
    };
    flushRef.current = flushPicker;

    const editColor = (index: number, event: { currentTarget: Element }) => {
        if (locked || !draftRef.current[index]) return;
        setSelectedIndex(index);
        const update = (rgb?: [number, number, number], alpha?: number) => {
            if (!aliveRef.current) return;
            // Merge picker changes with current RGBA, never the color captured on open.
            const current = draftRef.current[index];
            if (!current) return;
            const rgba: RGBA = [...(rgb ?? current.rgba.slice(0, 3) as [number, number, number]), alpha ?? current.rgba[3]];
            editDraft(index, rgba, current.time);
            pendingRef.current = { index, frame: { rgba, time: current.time } };
            if (timerRef.current) clearTimeout(timerRef.current);
            timerRef.current = setTimeout(() => void flushRef.current(), 180);
        };
        const current = draftRef.current[index];
        openColorPicker(event, rgbaToHex(current.rgba), hex => update(hexToRgb(hex)), ignoreAlpha ? undefined : {
            alpha: current.rgba[3], onAlpha: alpha => update(undefined, alpha),
        });
    };

    const add = (time: number, activeChannel?: number, value?: number) => {
        if (locked || modified || !canAdd) return;
        cleanupColorPickers();
        const rgba = sampleColor(frames, time);
        if (activeChannel !== undefined && value !== undefined) rgba[activeChannel] = clamp01(value);
        void run(() => paintAddKeyframe(target.sessionId, target.emitterKey, target.slot, rgba, clamp01(time)),
            'Keyframe added', fresh => fresh.length - 1);
    };
    const remove = () => {
        if (locked || modified || !canDelete) return;
        cleanupColorPickers();
        void run(() => paintDeleteKeyframe(target.sessionId, target.emitterKey, target.slot, selectedIndex),
            'Keyframe deleted', fresh => Math.min(selectedIndex, fresh.length - 1));
    };
    const close = async () => {
        if (busyRef.current) return;
        cleanupColorPickers();
        const ok = await flushPicker();
        if (ok && !busyRef.current && !pendingRef.current) onClose();
    };

    const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
        const typing = (event.target as HTMLElement).matches('input,textarea,select,[contenteditable="true"]');
        if ((event.ctrlKey || event.metaKey) && (locked || modified)) event.stopPropagation();
        if (event.key === 'Escape') {
            event.preventDefault(); event.stopPropagation();
            if (document.querySelector('.paint-color-picker')) cleanupColorPickers();
            else if (modified) cancelDraft();
            else void close();
        } else if ((event.key === 'Delete' || event.key === 'Backspace') && !typing) {
            event.preventDefault(); event.stopPropagation(); remove();
        }
    };
    const changeBlend = async (mode: number) => {
        if (locked || modified) return;
        busyRef.current = true; setBusy(true); setError(null);
        try {
            const changed = await paintSetBlendMode(target.sessionId, target.emitterKey, mode);
            if (changed) await onRefresh('Blend mode updated');
            if (aliveRef.current) setStatus(changed ? 'Blend mode updated' : 'No changes');
        } catch (reason) {
            if (aliveRef.current) setError(reason instanceof Error ? reason.message : String(reason));
        } finally { busyRef.current = false; if (aliveRef.current) setBusy(false); }
    };
    const timelineProps = {
        frames, selectedIndex, playhead, disabled: locked, canRetime: target.supportsRetime,
        canAdd, ignoreAlpha, onSelect: (index: number) => {
            if (!locked) { cleanupColorPickers(); setSelectedIndex(index); }
        },
        onDraft: editDraft, onCommit: (index: number) => { void commitFrame(index); }, onCancel: cancelDraft, onAdd: add,
    };

    return <div className="ce-overlay" onMouseDown={event => { if (event.target === event.currentTarget) void close(); }}>
        <div className="ce-dialog" role="dialog" aria-modal="true" aria-labelledby="ce-title" tabIndex={-1} ref={dialogRef} onKeyDown={keyDown}>
            <header className="ce-header">
                <h2 id="ce-title">{target.title}</h2>
                <span className="ce-mode-badge">{!target.exists ? 'Not authored' : probability ? 'Random range' : target.animated ? 'Animated' : 'Constant'}</span>
                <button className="ce-close" aria-label="Close color editor" title="Close color editor" disabled={busy} onClick={() => void close()}><X size={20} /></button>
            </header>
            <div className="ce-body">
                <main className="ce-workspace">
                    <section className="ce-panel ce-ramp-panel">
                        <div className="ce-section-heading"><h3>Color over {probability ? 'range' : 'time'}</h3>
                            <div className="ce-ramp-tools"><span className="ce-count">{frames.length} {frames.length === 1 ? 'stop' : 'stops'}</span>
                                {target.exists && <button className="ce-button ce-small" disabled={locked || modified || !canAdd} onClick={() => add(playhead)}><CirclePlus size={14} /> Add stop</button>}</div></div>
                        {!target.exists ? <div className="ce-empty"><div className="ce-empty-icon"><CirclePlus size={30} /></div>
                            <h3>Give this emitter a {target.title.toLowerCase()}</h3><p>Start with white, then choose a color and shape its evolution.</p>
                            <button className="ce-button ce-primary" disabled={busy} onClick={() => void run(
                                () => paintCreateColor(target.sessionId, target.emitterKey, target.slot), `Created ${target.title}`, () => 0)}>
                                <CirclePlus size={16} /> Create {target.title}</button></div>
                            : <GradientRamp {...timelineProps} onEditColor={editColor} />}
                    </section>
                    <section className="ce-panel ce-curve-panel">
                        <div className="ce-section-heading"><h3 title={`Linear interpolation / ${probability ? 'random range' : 'normalized lifetime'}`}><Activity size={16} /> Value curve</h3>
                            <div className="ce-channels" aria-label="Curve channel">{(['R', 'G', 'B', 'A'] as const).map((name, index) =>
                                <button key={name} className={`ce-channel ce-channel-${name.toLowerCase()}`} aria-pressed={channel === index}
                                    title={index === 3 && ignoreAlpha ? 'Alpha is unused by fresnel' : ['Red', 'Green', 'Blue', 'Alpha'][index]}
                                    disabled={ignoreAlpha && index === 3} onClick={() => setChannel(index as 0 | 1 | 2 | 3)}>{name}</button>)}</div></div>
                        <ValueCurve {...timelineProps} channel={channel} />
                        <div className="ce-curve-note"><span className={`ce-channel-dot ce-dot-${channel}`} />
                            {['Red', 'Green', 'Blue', 'Alpha'][channel]} / {channel === 3 ? '0 transparent, 1 opaque' : '0 to 1 intensity'}
                            <span>{probability ? 'Range positions are fixed' : 'Drag a point to edit'}</span></div>
                    </section>
                    {probability && <div className="ce-notice">This color uses per-channel probability tables. Colors can be edited; adding, deleting and retiming stops are unavailable.</div>}
                    {ignoreAlpha && <div className="ce-notice">Fresnel is an RGB rim tint. Its alpha does not affect visibility, and this slot does not support animation.</div>}
                    <section className="ce-panel ce-inspector">
                        <div className="ce-section-heading"><div><h3>{isMultiplier ? probability ? 'Base color' : 'Fallback constant' : target.animated ? 'Selected stop' : 'Constant color'}</h3>
                            {isMultiplier && <p>{probability ? 'Scales the colors in this random range.' : 'Used when the color has no lifetime curve.'}</p>}</div>
                            <button className="ce-icon-button ce-delete" aria-label="Delete selected stop" title={canDelete ? 'Delete selected stop (Delete)' : 'Keep at least one curve stop'} disabled={locked || modified || !canDelete} onClick={remove}><Trash2 size={16} /></button></div>
                        {selected && <div className="ce-inspector-fields">
                            <button className="ce-color-input" title="Open color picker" disabled={locked} onClick={event => editColor(selectedIndex, event)}>
                                <span className="ce-color-chip" style={{ background: rgbaToCss(selected.rgba, ignoreAlpha) }} /><span>{rgbaToHex(selected.rgba).toUpperCase()}</span><ChevronDown size={14} /></button>
                            <NumberField key={`${selectedIndex}-time`} label="Time" value={selected.time} disabled={locked || !target.supportsRetime || isMultiplier}
                                onDraft={value => editDraft(selectedIndex, selected.rgba, value)} onCommit={() => void commitFrame(selectedIndex)} onCancel={cancelDraft} />
                            {(['R', 'G', 'B', 'A'] as const).map((label, c) => <NumberField key={`${selectedIndex}-${label}`} label={label}
                                value={selected.rgba[c]} disabled={locked || (ignoreAlpha && c === 3)}
                                onDraft={value => { const rgba: RGBA = [...draftRef.current[selectedIndex].rgba]; rgba[c] = value; editDraft(selectedIndex, rgba, draftRef.current[selectedIndex].time); }}
                                onCommit={() => void commitFrame(selectedIndex)} onCancel={cancelDraft} />)}
                        </div>}
                        {multiplierIndex !== null && draft[multiplierIndex] && <button className="ce-multiplier" disabled={locked} aria-pressed={isMultiplier} onClick={() => setSelectedIndex(multiplierIndex)}>
                            <span style={{ background: rgbaToCss(draft[multiplierIndex].rgba) }} /> {probability ? 'Base color' : 'Fallback constant'} <span className="ce-mono">{rgbaToHex(draft[multiplierIndex].rgba).toUpperCase()}</span></button>}
                    </section>
                </main>
                <aside className="ce-sidebar">
                        <ColorPreview keyframes={frames} multiplier={probability && multiplierIndex !== null ? draft[multiplierIndex]?.rgba : undefined}
                            blendMode={target.blendMode} ignoreAlpha={ignoreAlpha} texturePath={target.texturePath}
                            meshPath={target.meshPath} binPath={target.binPath} animated={target.animated && frames.length > 1}
                            probability={probability} onTime={setPlayhead} />
                    <section className="ce-panel ce-settings"><label className="ce-label" htmlFor="ce-blend">Emitter blend mode</label>
                        <select id="ce-blend" value={target.blendMode} disabled={locked || modified} onChange={event => void changeBlend(Number(event.target.value))}>
                            {!BLEND_MODES.some(mode => mode.value === target.blendMode) && <option value={target.blendMode}>Mode {target.blendMode}</option>}
                            {BLEND_MODES.map(mode => <option key={mode.value} value={mode.value}>{mode.value} / {mode.name}</option>)}
                        </select>
                        <div className="ce-divider" /><div className="ce-animation-label"><strong>Animate color</strong>
                            <button className="ce-switch" role="switch" aria-checked={target.animated} aria-label="Animate color" disabled={locked || modified || !target.exists || !target.supportsStructuralEdits}
                                onClick={() => { cleanupColorPickers(); void run(() => target.animated
                                    ? paintDeanimateColor(target.sessionId, target.emitterKey, target.slot)
                                    : paintAnimateColor(target.sessionId, target.emitterKey, target.slot), target.animated ? 'Color made constant' : 'Color animated', () => target.animated ? 0 : 1); }}><span /></button></div>
                        {target.animated && !probability && <p className="ce-hint">Turning animation off keeps the first curve color.</p>}
                    </section>
                </aside>
            </div>
            {error && <div className="ce-error" role="alert"><span>{error}</span><button className="ce-icon-button" aria-label="Dismiss error" onClick={() => setError(null)}><X size={14} /></button></div>}
            <footer className="ce-footer"><span role="status">{busy ? <LoaderCircle size={13} className="ce-spinning" /> : modified ? <span className="ce-draft-dot" /> : <Check size={13} />}
                {busy ? 'Applying changes...' : modified ? 'Live draft' : status}</span><span>Ctrl Z / Undo · Delete / Remove stop · Save in Paint</span></footer>
        </div>
    </div>;
}

function NumberField({ label, value, disabled, onDraft, onCommit, onCancel }: {
    label: string; value: number; disabled: boolean;
    onDraft: (value: number) => void; onCommit: () => void; onCancel: () => void;
}) {
    const [text, setText] = useState(String(Number(value.toFixed(4))));
    const editing = useRef(false);
    const changed = useRef(false);
    useEffect(() => { if (!editing.current) setText(String(Number(value.toFixed(4)))); }, [value]);
    return <label className="ce-number-field"><span>{label}</span><input type="number" min={0} max={1} step={0.01} value={text} disabled={disabled}
        onFocus={() => { editing.current = true; changed.current = false; }}
        onChange={event => { setText(event.target.value); const next = event.target.valueAsNumber; if (Number.isFinite(next)) { changed.current = true; onDraft(clamp01(next)); } }}
        onBlur={() => { editing.current = false; setText(String(Number(value.toFixed(4)))); if (changed.current) onCommit(); changed.current = false; }}
        onKeyDown={event => {
            if (event.key === 'Enter') { event.preventDefault(); event.currentTarget.blur(); }
            if (event.key === 'Escape') { event.stopPropagation(); changed.current = false; onCancel(); event.currentTarget.blur(); }
        }} /></label>;
}
