/*
 * Recolor Hub — an always-open card pinned top-left of the model inspector,
 * shown whenever the model is a `.scb`. Announces that the mesh can be recolored
 * and offers: HSL shift, tint, 3D vertex painting, and (for a colorless mesh)
 * generating a vertex-color block on demand. Disk ops write the `.scb` in place
 * and remount the viewport; painting stays live in the GPU buffer until Save.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { Paintbrush, Droplet, Sliders, Plus } from 'lucide-react';
import type { ModelSceneReady } from './ModelViewport';
import {
    meshRecolorHsl, meshRecolorTint, meshApplyVertexColors, meshGenerateVertexColors,
} from '@/lib/api/modelInspect';
import './model-inspect.css';

/** #rrggbb -> [r,g,b] bytes. */
function hexToRgb(hex: string): [number, number, number] {
    const n = parseInt(hex.slice(1), 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export interface RecolorHubProps {
    /** Getter for the live scene (null until mounted / during remount). */
    getScene: () => ModelSceneReady | null;
    /** Whether the current mesh already has a vertex-color block. */
    hasColors: boolean;
    /** Called after a disk write so the modal can remount the viewport. */
    onWroteDisk: () => void;
    /** The `.scb` disk path. */
    path: string;
}

export function RecolorHub({ getScene, hasColors, onWroteDisk, path }: RecolorHubProps) {
    const [hue, setHue] = useState(0);
    const [sat, setSat] = useState(100);   // percent multiplier
    const [light, setLight] = useState(100);
    const [tint, setTint] = useState('#8b5cf6');
    const [tintStrength, setTintStrength] = useState(50);
    const [paintColor, setPaintColor] = useState('#8b5cf6');
    const [brush, setBrush] = useState(6);
    const [painting, setPainting] = useState(false);
    const [dirty, setDirty] = useState(false); // unsaved paint strokes
    const [status, setStatus] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);

    const run = useCallback(async (fn: () => Promise<number>, label: string, remount: boolean) => {
        setBusy(true);
        setStatus(`${label}…`);
        try {
            const n = await fn();
            setStatus(n > 0 ? `${label}: ${n} vertices.` : 'No change.');
            if (remount && n > 0) onWroteDisk();
        } catch (e) {
            setStatus(`Failed: ${String(e)}`);
        } finally {
            setBusy(false);
        }
    }, [onWroteDisk]);

    // ── 3D painting: pointer handlers on the canvas host ─────────────────────
    const drawingRef = useRef(false);
    useEffect(() => {
        const ready = getScene();
        const host = ready?.hostElement;
        if (!painting || !ready || !host) return;

        ready.scene.ensureVertexColors();
        ready.scene.setControlsEnabled(false);
        const rgb = hexToRgb(paintColor);

        const paintFromEvent = (e: PointerEvent) => {
            const rect = host.getBoundingClientRect();
            const ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1;
            const ndcY = -(((e.clientY - rect.top) / rect.height) * 2 - 1);
            const hit = ready.scene.paintAtNdc(ndcX, ndcY, rgb, brush, 1);
            if (hit) setDirty(true);
        };
        const onDown = (e: PointerEvent) => { drawingRef.current = true; host.setPointerCapture(e.pointerId); paintFromEvent(e); };
        const onMove = (e: PointerEvent) => { if (drawingRef.current) paintFromEvent(e); };
        const onUp = (e: PointerEvent) => { drawingRef.current = false; try { host.releasePointerCapture(e.pointerId); } catch { /* ignore */ } };

        host.addEventListener('pointerdown', onDown);
        host.addEventListener('pointermove', onMove);
        host.addEventListener('pointerup', onUp);
        host.style.cursor = 'crosshair';
        return () => {
            host.removeEventListener('pointerdown', onDown);
            host.removeEventListener('pointermove', onMove);
            host.removeEventListener('pointerup', onUp);
            host.style.cursor = '';
            ready.scene.setControlsEnabled(true);
        };
    }, [painting, getScene, paintColor, brush]);

    const savePainting = useCallback(async () => {
        const ready = getScene();
        if (!ready) return;
        setBusy(true);
        setStatus('Saving painting…');
        try {
            const rgb = ready.scene.readVertexColors(); // per preview corner, RGB bytes
            const src = ready.scene.data.sourceIndices;
            const vertexCount = ready.scene.data.vertexCount;
            // De-flatten preview corners -> per mesh-vertex RGBA. Whole-shared-vertex
            // painting means every corner of a mesh vertex holds the same colour, so
            // last-write-wins is a no-op in practice. Alpha defaults to 255 (opaque);
            // the backend keeps existing alpha where the block already existed.
            const meshVerts = src.length ? Math.max(...Array.from(src)) + 1 : vertexCount;
            const out: [number, number, number, number][] = Array.from({ length: meshVerts }, () => [255, 255, 255, 255]);
            for (let k = 0; k < vertexCount; k++) {
                const m = src.length ? src[k] : k;
                out[m] = [rgb[k * 3], rgb[k * 3 + 1], rgb[k * 3 + 2], 255];
            }
            const n = await meshApplyVertexColors(path, out);
            setStatus(`Saved ${n} vertex colors.`);
            setDirty(false);
            onWroteDisk();
        } catch (e) {
            setStatus(`Save failed: ${String(e)}`);
        } finally {
            setBusy(false);
        }
    }, [getScene, path, onWroteDisk]);

    return (
        <div className="model-recolor-hub">
            <div className="model-recolor-hub__title"><Paintbrush size={14} /><span>Recolor</span></div>

            {!hasColors && (
                <div className="model-recolor-hub__section">
                    <div className="model-recolor-hub__note">This mesh has no vertex colors.</div>
                    <button type="button" className="model-inspect__option" disabled={busy}
                        onClick={() => void run(() => meshGenerateVertexColors(path, [255, 255, 255, 255]), 'Added colors', true)}>
                        <span><Plus size={13} /> Add Vertex Colors</span>
                    </button>
                </div>
            )}

            {hasColors && (
                <>
                    <div className="model-recolor-hub__section">
                        <div className="model-recolor-hub__head"><Sliders size={12} /><span>Adjust</span></div>
                        <label className="model-recolor-hub__row">Hue <input type="range" min={0} max={360} value={hue} onChange={(e) => setHue(+e.target.value)} /><b>{hue}°</b></label>
                        <label className="model-recolor-hub__row">Sat <input type="range" min={0} max={200} value={sat} onChange={(e) => setSat(+e.target.value)} /><b>{sat}%</b></label>
                        <label className="model-recolor-hub__row">Light <input type="range" min={0} max={200} value={light} onChange={(e) => setLight(+e.target.value)} /><b>{light}%</b></label>
                        <button type="button" className="model-inspect__option" disabled={busy}
                            onClick={() => void run(() => meshRecolorHsl(path, hue, sat / 100, light / 100), 'Adjusted', true)}>
                            <span>Apply Adjust</span>
                        </button>
                    </div>

                    <div className="model-recolor-hub__section">
                        <div className="model-recolor-hub__head"><Droplet size={12} /><span>Tint</span></div>
                        <label className="model-recolor-hub__row">Color <input type="color" value={tint} onChange={(e) => setTint(e.target.value)} /></label>
                        <label className="model-recolor-hub__row">Amount <input type="range" min={0} max={100} value={tintStrength} onChange={(e) => setTintStrength(+e.target.value)} /><b>{tintStrength}%</b></label>
                        <button type="button" className="model-inspect__option" disabled={busy}
                            onClick={() => void run(() => meshRecolorTint(path, hexToRgb(tint), tintStrength / 100), 'Tinted', true)}>
                            <span>Apply Tint</span>
                        </button>
                    </div>
                </>
            )}

            <div className="model-recolor-hub__section">
                <div className="model-recolor-hub__head"><Paintbrush size={12} /><span>Paint</span></div>
                <label className="model-recolor-hub__row">Color <input type="color" value={paintColor} onChange={(e) => setPaintColor(e.target.value)} /></label>
                <label className="model-recolor-hub__row">Brush <input type="range" min={0} max={40} value={brush} onChange={(e) => setBrush(+e.target.value)} /><b>{brush}</b></label>
                <button type="button" className={`model-inspect__option ${painting ? 'is-active' : ''}`}
                    onClick={() => setPainting((p) => !p)}>
                    <span>{painting ? 'Stop Painting' : 'Start Painting'}</span>
                </button>
                {dirty && (
                    <button type="button" className="model-inspect__option" disabled={busy} onClick={() => void savePainting()}>
                        <span>Save Painting</span>
                    </button>
                )}
            </div>

            {status && <div className="model-recolor-hub__status">{status}</div>}
        </div>
    );
}
