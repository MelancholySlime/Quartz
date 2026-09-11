use quartz_lib::anim_preview::{self, AnimPreview};
use quartz_lib::model_preview::{self, ModelPreview};
use quartz_lib::skeleton::{self, SkeletonInfo};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSceneAssets {
    ground_path: Option<String>,
    skybox_path: Option<String>,
}

fn bundled_texture(app: &AppHandle, file_name: &str) -> Option<String> {
    let mut candidates = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(
            resource_dir
                .join("resources")
                .join("textures")
                .join(file_name),
        );
        candidates.push(resource_dir.join("textures").join(file_name));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("textures")
            .join(file_name),
    );

    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn model_inspect_scene_assets(app: AppHandle) -> ModelSceneAssets {
    ModelSceneAssets {
        ground_path: bundled_texture(&app, "ground_map.webp"),
        skybox_path: bundled_texture(&app, "riots_sru_skybox_cubemap.dds"),
    }
}

/// Parse an SCB/SCO/SKN on a blocking worker and return renderer-neutral
/// buffers.  WebGL stays in TypeScript; binary format handling stays native.
#[tauri::command]
pub async fn model_inspect_load(path: String) -> Result<ModelPreview, String> {
    tokio::task::spawn_blocking(move || {
        model_preview::load_model_preview(std::path::Path::new(&path)).map_err(String::from)
    })
    .await
    .map_err(|e| format!("model preview task failed: {e}"))?
}

/// Locate + parse the skeleton for a `.skn`. Tries the same-stem `<skn>.skl`
/// first, then the shared autodetect (a lone `.skl` beside it). Returns the
/// joint list with local + inverse-bind transforms for skinning.
#[tauri::command]
pub async fn model_inspect_skeleton(skn_path: String) -> Result<SkeletonInfo, String> {
    tokio::task::spawn_blocking(move || {
        let skn = Path::new(&skn_path);
        let same_stem = skn.with_extension("skl");
        let skl = if same_stem.is_file() {
            same_stem
        } else {
            skeleton::autodetect_skl(&skn_path, None, None).map_err(String::from)?
        };
        skeleton::read_skeleton_file(&skl).map_err(String::from)
    })
    .await
    .map_err(|e| format!("skeleton task failed: {e}"))?
}

/// Parse a `.anm` clip into per-joint keyframe tracks for playback.
#[tauri::command]
pub async fn model_inspect_animation(anm_path: String) -> Result<AnimPreview, String> {
    tokio::task::spawn_blocking(move || {
        anim_preview::load_anim_preview(&anm_path).map_err(String::from)
    })
    .await
    .map_err(|e| format!("animation task failed: {e}"))?
}

/// For a loose on-disk `.skn`, resolve the animation clips authored in its skin
/// bin to real `.anm` files on disk. Used when the model was opened directly
/// (file explorer) rather than through the WAD prep, which already returns them.
#[tauri::command]
pub async fn model_inspect_disk_animations(skn_path: String) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        quartz_lib::skin_preview::resolve_skn_disk_animations(Path::new(&skn_path))
    })
    .await
    .map_err(|e| format!("disk animation task failed: {e}"))
}

/// True when the `.scb` at `path` carries a per-vertex color block, so the UI
/// can offer "Recolor Vertex Colors" only on meshes it can actually edit.
#[tauri::command]
pub async fn mesh_has_vertex_colors(path: String) -> bool {
    tokio::task::spawn_blocking(move || {
        quartz_lib::mesh_recolor::scb_has_vertex_colors(Path::new(&path))
    })
    .await
    .unwrap_or(false)
}

/// Hue-rotate every vertex color of the `.scb` at `path` in place by
/// `hue_degrees` (saturation/lightness/alpha preserved). Returns the number of
/// vertex colors changed.
#[tauri::command]
pub async fn mesh_recolor_hue_shift(path: String, hue_degrees: f32) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        quartz_lib::mesh_recolor::hue_shift_scb(Path::new(&path), hue_degrees)
            .map(|r| r.total())
    })
    .await
    .map_err(|e| format!("mesh recolor task failed: {e}"))?
}

/// Shift every vertex color of the `.scb` at `path` in HSL: add `hue_degrees`,
/// scale saturation by `sat_mul` and lightness by `light_mul`. Returns the
/// number of vertex colors changed.
#[tauri::command]
pub async fn mesh_recolor_hsl(
    path: String,
    hue_degrees: f32,
    sat_mul: f32,
    light_mul: f32,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        quartz_lib::mesh_recolor::hsl_shift_scb(Path::new(&path), hue_degrees, sat_mul, light_mul)
            .map(|r| r.total())
    })
    .await
    .map_err(|e| format!("mesh recolor task failed: {e}"))?
}

/// Tint every vertex color of the `.scb` at `path` toward `target` (RGB, 0-255)
/// by `strength` (0..1). Returns the number of vertex colors changed.
#[tauri::command]
pub async fn mesh_recolor_tint(
    path: String,
    target: [u8; 3],
    strength: f32,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        quartz_lib::mesh_recolor::tint_scb(Path::new(&path), target, strength).map(|r| r.total())
    })
    .await
    .map_err(|e| format!("mesh recolor task failed: {e}"))?
}

/// Replace the whole per-vertex color block of the `.scb` at `path` with
/// `colors` (one `[r,g,b,a]` per mesh vertex, parallel to `positions`). Used to
/// save a 3D vertex-paint session. Errors when the count mismatches.
#[tauri::command]
pub async fn mesh_apply_vertex_colors(
    path: String,
    colors: Vec<[u8; 4]>,
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        quartz_lib::mesh_recolor::apply_vertex_colors_scb(Path::new(&path), &colors)
            .map(|r| r.total())
    })
    .await
    .map_err(|e| format!("mesh recolor task failed: {e}"))?
}

/// Ensure the `.scb` at `path` has a per-vertex color block, generating an
/// all-`fill` (RGBA) one when absent. No-op when colors already exist. Returns
/// the number of vertex colors generated (`0` when already present).
#[tauri::command]
pub async fn mesh_generate_vertex_colors(
    path: String,
    fill: [u8; 4],
) -> Result<usize, String> {
    tokio::task::spawn_blocking(move || {
        quartz_lib::mesh_recolor::ensure_vertex_colors_scb(Path::new(&path), fill).map(|r| r.total())
    })
    .await
    .map_err(|e| format!("mesh recolor task failed: {e}"))?
}
