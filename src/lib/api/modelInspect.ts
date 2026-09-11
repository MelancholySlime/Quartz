import { invokeCommand } from './core';

export interface ModelGroup {
    name: string;
    indexStart: number;
    indexCount: number;
}

export interface ModelPreviewData {
    name: string;
    kind: 'static' | 'skinned';
    version: string;
    positions: number[];
    normals: number[];
    uvs: number[];
    colors: number[];
    /** 4 bone influence indices per vertex (skinned meshes only). */
    boneIndices: number[];
    /** 4 bone weights per vertex, parallel to boneIndices. */
    boneWeights: number[];
    indices: number[];
    /** Static (.scb/.sco) only: mesh vertex index each flattened preview vertex
     *  came from. Empty for skinned meshes (preview index == mesh vertex). */
    sourceIndices: number[];
    groups: ModelGroup[];
    vertexCount: number;
    triangleCount: number;
    suggestedTexture: string | null;
    suggestedTextures: Record<string, string>;
    suggestedHiddenGroups: string[];
    suggestedModelScale: number;
}

export interface ModelSceneAssets {
    groundPath: string | null;
    skyboxPath: string | null;
}

/** One skeleton joint with local + inverse-bind transforms (TRS). Quaternions
 *  are `[x, y, z, w]`. */
export interface JointPreview {
    id: number;
    name: string;
    parentId: number;
    hash: number;
    localTranslation: [number, number, number];
    localScale: [number, number, number];
    localRotation: [number, number, number, number];
    inverseBindTranslation: [number, number, number];
    inverseBindScale: [number, number, number];
    inverseBindRotation: [number, number, number, number];
}

export interface SkeletonPreview {
    name: string;
    assetName: string;
    joints: JointPreview[];
    /** Maps a mesh's local bone-influence index to a joint id. */
    influences: number[];
}

/** One keyframe of one joint: a TRS pose at `time` seconds. */
export interface AnimFramePreview {
    time: number;
    translation: [number, number, number];
    rotation: [number, number, number, number];
    scale: [number, number, number];
}

export interface AnimTrackPreview {
    jointHash: number;
    frames: AnimFramePreview[];
}

export interface AnimPreview {
    name: string;
    fps: number;
    frameCount: number;
    durationSeconds: number;
    tracks: AnimTrackPreview[];
}

/** A submesh-visibility event: while `[startFrame, endFrame]` is live during the
 *  clip, HIDE `hide` and SHOW `show`. Tokens are submesh names or `0x`-hashes. */
export interface SubmeshVisEvent {
    startFrame: number | null;
    endFrame: number | null;
    show: string[];
    hide: string[];
}

export interface PreparedClipMember {
    name: string;
    /** Extracted `.anm` disk path. */
    anmPath: string;
    events: SubmeshVisEvent[];
    loops: boolean;
}

/** A resolved animation clip: its extracted `.anm`, submesh-visibility events, and
 *  (for a sequencer) its ordered member queue. */
export interface PreparedClip {
    name: string;
    /** Extracted `.anm` disk path (the first member's, for a sequencer). */
    anmPath: string | null;
    members: PreparedClipMember[];
    events: SubmeshVisEvent[];
    loops: boolean;
}

/** Parse SCB/SCO/SKN natively and return WebGL-ready buffers. */
export function modelInspectLoad(path: string): Promise<ModelPreviewData> {
    return invokeCommand<ModelPreviewData>('model_inspect_load', { path });
}

export function modelInspectSceneAssets(): Promise<ModelSceneAssets> {
    return invokeCommand<ModelSceneAssets>('model_inspect_scene_assets');
}

/** Locate + parse the skeleton for a `.skn` (same-stem `.skl` or a sibling). */
export function modelInspectSkeleton(sknPath: string): Promise<SkeletonPreview> {
    return invokeCommand<SkeletonPreview>('model_inspect_skeleton', { sknPath });
}

/** Parse a `.anm` clip into per-joint keyframe tracks. */
export function modelInspectAnimation(anmPath: string): Promise<AnimPreview> {
    return invokeCommand<AnimPreview>('model_inspect_animation', { anmPath });
}

/** Resolve a loose on-disk .skn's animation clips (from its skin bin) to real
 *  .anm files on disk. Returns [] when the model was not opened from a project
 *  tree (e.g. a bare .skn with no skin bin nearby). */
export function modelInspectDiskAnimations(sknPath: string): Promise<string[]> {
    return invokeCommand<string[]>('model_inspect_disk_animations', { sknPath });
}

/** True when a `.scb` carries a per-vertex color block, so the recolor action
 *  only offers itself on meshes it can actually edit. */
export function meshHasVertexColors(path: string): Promise<boolean> {
    return invokeCommand<boolean>('mesh_has_vertex_colors', { path });
}

/** Hue-rotate every vertex color of a `.scb` in place by `hueDegrees`
 *  (saturation/lightness/alpha preserved). Resolves to the number of colors
 *  changed. */
export function meshRecolorHueShift(path: string, hueDegrees: number): Promise<number> {
    return invokeCommand<number>('mesh_recolor_hue_shift', { path, hueDegrees });
}

/** Shift every vertex color of a `.scb` in HSL: add `hueDegrees`, scale
 *  saturation by `satMul` and lightness by `lightMul`. Resolves to # changed. */
export function meshRecolorHsl(path: string, hueDegrees: number, satMul: number, lightMul: number): Promise<number> {
    return invokeCommand<number>('mesh_recolor_hsl', { path, hueDegrees, satMul, lightMul });
}

/** Tint every vertex color of a `.scb` toward `target` (RGB 0-255) by
 *  `strength` (0..1). Resolves to # changed. */
export function meshRecolorTint(path: string, target: [number, number, number], strength: number): Promise<number> {
    return invokeCommand<number>('mesh_recolor_tint', { path, target, strength });
}

/** Replace a `.scb`'s whole per-vertex color block (one [r,g,b,a] per mesh
 *  vertex). Used to save a 3D vertex-paint session. */
export function meshApplyVertexColors(path: string, colors: [number, number, number, number][]): Promise<number> {
    return invokeCommand<number>('mesh_apply_vertex_colors', { path, colors });
}

/** Ensure a `.scb` has a vertex-color block, generating an all-`fill` (RGBA)
 *  one when absent. No-op when colors already exist. Resolves to # generated. */
export function meshGenerateVertexColors(path: string, fill: [number, number, number, number]): Promise<number> {
    return invokeCommand<number>('mesh_generate_vertex_colors', { path, fill });
}
