import { useEffect, useMemo, useRef, useState } from 'react';
import { Box, Pause, Play, RotateCcw } from 'lucide-react';
import { ModelViewport } from '@/components/model-inspect/ModelViewport';
import { portResolveAssetPath } from '@/lib/api/wad';
import { resolveTextureDataUrl } from '@/lib/util/resolveTextureDataUrl';
import { blendModeName, compositePixel } from '../utils/blendModes';
import { clamp01, rgbaToHex, sampleColor, type RGBA } from './colorEditorUtils';
import './ColorPreview.css';

interface Props {
    keyframes: { rgba: RGBA; time: number }[];
    multiplier?: RGBA;
    blendMode: number;
    ignoreAlpha: boolean;
    texturePath: string | null;
    meshPath: string | null;
    binPath: string;
    animated: boolean;
    probability: boolean;
    onTime: (time: number) => void;
}

interface TexturePixels {
    assetKey: string;
    data: Uint8ClampedArray;
    width: number;
    height: number;
}

const WIDTH = 320;
const HEIGHT = 220;

export function ColorPreview({
    keyframes, multiplier, blendMode, ignoreAlpha, texturePath, meshPath,
    binPath, animated, probability, onTime,
}: Props) {
    const [playing, setPlaying] = useState(true);
    const [duration, setDuration] = useState(3);
    const [background, setBackground] = useState<'dark' | 'checker' | 'light'>('checker');
    const [view, setView] = useState<'flat' | 'mesh'>('flat');
    const [texture, setTexture] = useState<TexturePixels | null>(null);
    const [textureStatus, setTextureStatus] = useState<'loading' | 'ready' | 'missing'>('missing');
    const [meshAsset, setMeshAsset] = useState<{
        key: string; mesh: string | null; texture: string | null;
    } | null>(null);
    const canvasRef = useRef<HTMLCanvasElement>(null);
    const scrubberRef = useRef<HTMLInputElement>(null);
    const timeLabelRef = useRef<HTMLOutputElement>(null);
    const tintSwatchRef = useRef<HTMLSpanElement>(null);
    const tintLabelRef = useRef<HTMLOutputElement>(null);
    const timeRef = useRef(0);
    const onTimeRef = useRef(onTime);
    const drawRef = useRef<(time: number) => void>(() => {});
    const canAnimate = animated && keyframes.length > 1;
    const textureKey = JSON.stringify([binPath, texturePath]);
    const meshKey = JSON.stringify([binPath, meshPath, texturePath]);
    const currentTexture = texture?.assetKey === textureKey ? texture : null;
    const currentMesh = meshAsset?.key === meshKey ? meshAsset : null;
    const activeView = meshPath ? view : 'flat';

    useEffect(() => { onTimeRef.current = onTime; }, [onTime]);

    // Decode and rasterize once per asset change. Edits and animation only read
    // this small pixel buffer, so neither can trigger image/network work.
    useEffect(() => {
        let cancelled = false;
        let image: HTMLImageElement | null = null;
        const assetKey = JSON.stringify([binPath, texturePath]);
        setTexture(null);
        setTextureStatus(texturePath ? 'loading' : 'missing');
        if (texturePath) {
            void resolveTextureDataUrl(texturePath, binPath).then((url) => {
                if (cancelled) return;
                if (!url) { setTextureStatus('missing'); return; }
                image = new Image();
                image.onload = () => {
                    if (cancelled || !image) return;
                    try {
                        const scale = Math.min(248 / image.naturalWidth, 164 / image.naturalHeight);
                        const width = Math.max(1, Math.round(image.naturalWidth * scale));
                        const height = Math.max(1, Math.round(image.naturalHeight * scale));
                        const offscreen = document.createElement('canvas');
                        offscreen.width = width;
                        offscreen.height = height;
                        const context = offscreen.getContext('2d');
                        if (!context) { setTextureStatus('missing'); return; }
                        context.drawImage(image, 0, 0, width, height);
                        setTexture({ assetKey, width, height, data: context.getImageData(0, 0, width, height).data });
                        setTextureStatus('ready');
                    } catch {
                        setTextureStatus('missing');
                    }
                };
                image.onerror = () => { if (!cancelled) setTextureStatus('missing'); };
                image.src = url;
            }).catch(() => { if (!cancelled) setTextureStatus('missing'); });
        }
        return () => {
            cancelled = true;
            if (image) { image.onload = null; image.onerror = null; }
        };
    }, [texturePath, binPath]);

    // ModelViewport accepts disk paths only. Keep each response associated with
    // its original paths so a target change cannot briefly show the old mesh.
    useEffect(() => {
        let cancelled = false;
        setMeshAsset(null);
        if (meshPath) {
            const key = JSON.stringify([binPath, meshPath, texturePath]);
            void Promise.all([
                portResolveAssetPath(meshPath, binPath).catch(() => null),
                texturePath ? portResolveAssetPath(texturePath, binPath).catch(() => null) : Promise.resolve(null),
            ]).then(([mesh, textureDisk]) => {
                if (!cancelled) setMeshAsset({ key, mesh, texture: textureDisk });
            });
        }
        return () => { cancelled = true; };
    }, [meshPath, texturePath, binPath]);

    const backdrop = useMemo(() => {
        const pixels = new Uint8ClampedArray(WIDTH * HEIGHT * 4);
        for (let y = 0; y < HEIGHT; y++) {
            for (let x = 0; x < WIDTH; x++) {
                const i = (y * WIDTH + x) * 4;
                const checker = (Math.floor(x / 16) + Math.floor(y / 16)) % 2;
                const shade = background === 'light' ? 231 : background === 'dark' ? 19 : checker ? 46 : 35;
                pixels[i] = shade;
                pixels[i + 1] = shade + (background === 'light' ? 1 : 2);
                pixels[i + 2] = shade + (background === 'light' ? 3 : 7);
                pixels[i + 3] = 255;
            }
        }
        return pixels;
    }, [background]);

    useEffect(() => {
        const context = canvasRef.current?.getContext('2d');
        if (!context) return;
        const frame = context.createImageData(WIDTH, HEIGHT);
        const data = frame.data;
        const quadWidth = currentTexture?.width ?? 164;
        const quadHeight = currentTexture?.height ?? 164;
        const startX = Math.floor((WIDTH - quadWidth) / 2);
        const startY = Math.floor((HEIGHT - quadHeight) / 2);
        const source = currentTexture?.data;
        const src: RGBA = [1, 1, 1, 1];
        const dst: RGBA = [0, 0, 0, 1];
        drawRef.current = (time) => {
            const sampled = sampleColor(keyframes, time);
            const tint = sampled.map((value, channel) => value * (multiplier?.[channel] ?? 1)) as RGBA;
            if (ignoreAlpha) tint[3] = 1;
            const hex = rgbaToHex(tint).toUpperCase();
            if (scrubberRef.current) scrubberRef.current.value = String(time);
            if (timeLabelRef.current) timeLabelRef.current.textContent = `t ${time.toFixed(2)}`;
            if (tintSwatchRef.current) tintSwatchRef.current.style.backgroundColor = hex;
            if (tintLabelRef.current) tintLabelRef.current.textContent = `${hex} / ${Math.round(clamp01(tint[3]) * 100)}%`;
            if (activeView !== 'flat') return;
            data.set(backdrop);
            // Blend only within the actual quad. A transparent texel still
            // participates in opaque/min/darken and other engine blend modes.
            for (let y = 0; y < quadHeight; y++) {
                for (let x = 0; x < quadWidth; x++) {
                    const from = (y * quadWidth + x) * 4;
                    const to = ((startY + y) * WIDTH + startX + x) * 4;
                    src[0] = (source ? source[from] / 255 : 1) * tint[0];
                    src[1] = (source ? source[from + 1] / 255 : 1) * tint[1];
                    src[2] = (source ? source[from + 2] / 255 : 1) * tint[2];
                    src[3] = (source ? source[from + 3] / 255 : 1) * tint[3];
                    dst[0] = backdrop[to] / 255;
                    dst[1] = backdrop[to + 1] / 255;
                    dst[2] = backdrop[to + 2] / 255;
                    const result = compositePixel(blendMode, src, dst);
                    data[to] = result[0] * 255;
                    data[to + 1] = result[1] * 255;
                    data[to + 2] = result[2] * 255;
                    // The canvas represents an opaque framebuffer. Preserving
                    // blend alpha here would composite the result a second time.
                    data[to + 3] = 255;
                }
            }
            context.putImageData(frame, 0, 0);
        };
        drawRef.current(timeRef.current);
    }, [keyframes, multiplier, blendMode, ignoreAlpha, currentTexture, backdrop, activeView]);

    useEffect(() => {
        if (!canAnimate) {
            timeRef.current = 0;
            drawRef.current(0);
            onTimeRef.current(0);
            return;
        }
        if (!playing) return;
        let frame = 0;
        let previous: number | null = null;
        const tick = (now: number) => {
            if (previous !== null) timeRef.current = (timeRef.current + (now - previous) / (duration * 1000)) % 1;
            previous = now;
            drawRef.current(timeRef.current);
            onTimeRef.current(timeRef.current);
            frame = requestAnimationFrame(tick);
        };
        frame = requestAnimationFrame(tick);
        return () => cancelAnimationFrame(frame);
    }, [canAnimate, playing, duration]);

    const scrubTo = (time: number) => {
        setPlaying(false);
        timeRef.current = clamp01(time);
        drawRef.current(timeRef.current);
        onTimeRef.current(timeRef.current);
    };

    return (
        <section className="ce-preview" aria-label="Color preview">
            <div className="ce-preview-header">
                <h3>{probability ? 'Range preview' : 'Live preview'}</h3>
                {meshPath && (
                    <div className="ce-preview-tabs" role="group" aria-label="Preview geometry">
                        <button type="button" className={activeView === 'flat' ? 'is-active' : ''} aria-pressed={activeView === 'flat'} onClick={() => setView('flat')}>Flat</button>
                        <button type="button" className={activeView === 'mesh' ? 'is-active' : ''} aria-pressed={activeView === 'mesh'} onClick={() => setView('mesh')}><Box size={12} /> Mesh</button>
                    </div>
                )}
            </div>

            <div className="ce-preview-stage">
                <canvas ref={canvasRef} width={WIDTH} height={HEIGHT} className={activeView === 'flat' ? '' : 'ce-preview-hidden'} aria-label="Time-sampled color composited with the emitter blend mode" />
                {activeView === 'mesh' && (
                    <div className="ce-preview-mesh">
                        {currentMesh?.mesh ? (
                            <ModelViewport
                                key={currentMesh.mesh}
                                path={currentMesh.mesh}
                                texturePath={currentMesh.texture}
                                autoRotate={playing}
                                interactive
                                showGrid={false}
                                showSkybox={false}
                                autoResolveSkinMaterials={false}
                            />
                        ) : (
                            <div className="ce-preview-placeholder"><Box size={28} /><span>{currentMesh ? 'Mesh asset could not be resolved.' : 'Resolving mesh...'}</span></div>
                        )}
                    </div>
                )}
                <span className="ce-preview-stage-label">{activeView === 'mesh' ? 'Static material' : blendModeName(blendMode)}</span>
                {activeView === 'flat' && <span className="ce-preview-stage-time">{probability ? 'Range sample' : canAnimate ? 'Lifetime sample' : 'Constant'}</span>}
            </div>

            <div className="ce-preview-background-row">
                <span>Backdrop</span>
                <div className="ce-preview-backgrounds" role="group" aria-label="Preview backdrop">
                    {(['dark', 'checker', 'light'] as const).map((choice) => (
                        <button type="button" key={choice} className={`ce-preview-background ce-preview-background-${choice}${background === choice ? ' is-active' : ''}`} aria-label={`${choice === 'checker' ? 'Checkerboard' : choice === 'dark' ? 'Dark' : 'Light'} backdrop`} aria-pressed={background === choice} disabled={activeView === 'mesh'} onClick={() => setBackground(choice)} />
                    ))}
                </div>
                <output ref={timeLabelRef} className="ce-preview-time" aria-label={probability ? 'Range position' : 'Normalized preview time'}>t 0.00</output>
            </div>

            <div className="ce-preview-transport">
                <button type="button" className="ce-preview-play" disabled={!canAnimate} aria-label={playing && canAnimate ? 'Pause preview' : 'Play preview'} onClick={() => setPlaying((value) => !value)}>{playing && canAnimate ? <Pause size={15} fill="currentColor" /> : <Play size={15} fill="currentColor" />}</button>
                <input ref={scrubberRef} type="range" min="0" max="1" step="0.001" defaultValue="0" disabled={!canAnimate} aria-label={probability ? 'Scrub color range' : 'Scrub particle lifetime'} onPointerDown={() => setPlaying(false)} onChange={(event) => scrubTo(Number(event.target.value))} />
                <button type="button" className="ce-preview-reset" disabled={!canAnimate} aria-label="Reset preview to start" onClick={() => scrubTo(0)}><RotateCcw size={13} /></button>
                <select value={duration} aria-label="Preview loop duration" disabled={!canAnimate} onChange={(event) => setDuration(Number(event.target.value))}>
                    <option value={1.5}>1.5s</option>
                    <option value={3}>3s</option>
                    <option value={6}>6s</option>
                    <option value={10}>10s</option>
                </select>
            </div>

            <div className="ce-preview-sample">
                <span ref={tintSwatchRef} className="ce-preview-tint-swatch" />
                <span>Sample</span>
                <output ref={tintLabelRef}>#FFFFFF / 100%</output>
            </div>
            <p className="ce-preview-note">
                {activeView === 'mesh'
                    ? 'Static texture / Use Flat for animated color.'
                    : probability
                        ? 'Preview sweeps the authored random range.'
                        : canAnimate
                            ? 'Loop speed changes the preview only.'
                            : 'Constant tint across the particle.'}
                {ignoreAlpha && ' Fresnel tint uses RGB only.'}
            </p>
            {activeView === 'flat' && texturePath && textureStatus !== 'ready' && (
                <p className="ce-preview-asset-note">{textureStatus === 'loading' ? 'Loading texture...' : 'Texture unavailable. Showing a solid quad.'}</p>
            )}
        </section>
    );
}
