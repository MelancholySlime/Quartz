//! Vertex-color recolor for static meshes (`.scb`).
//!
//! League `.scb` (`r3d2Mesh`) meshes can carry a per-vertex RGBA color block
//! (and, on `HasVcp`-flagged meshes, a per-face-corner color tail). That color
//! is baked into the mesh binary, not a `.bin` field, so the VFX/paint editors
//! never reach it. This module reads the mesh with RitoShark's `StaticMesh`,
//! applies a hue rotation to every vertex color, and writes the mesh back in
//! place — the one path to recolor a mesh's baked vertex colors.

use ritoshark::mesh::StaticMesh;
use ritoshark::prelude::{Parse, Serialize};
use std::path::Path;

/// Outcome of a recolor: how many colors were touched, so the UI can report
/// "recolored N vertex colors" or warn when a mesh had none.
#[derive(Debug, Clone, Copy)]
pub struct RecolorReport {
    pub vertex_colors: usize,
    pub vcp_colors: usize,
}

impl RecolorReport {
    pub fn total(&self) -> usize {
        self.vertex_colors + self.vcp_colors
    }
}

/// Hue-rotate every vertex color of the `.scb` at `path` by `hue_degrees`, in
/// place. Saturation and lightness (and alpha) are preserved. Thin wrapper over
/// [`hsl_shift_scb`] so the original command stays valid.
pub fn hue_shift_scb(path: &Path, hue_degrees: f32) -> Result<RecolorReport, String> {
    hsl_shift_scb(path, hue_degrees, 1.0, 1.0)
}

/// Shift every vertex color of the `.scb` at `path` in HSL: add `hue_degrees`
/// to hue, multiply saturation by `sat_mul` and lightness by `light_mul`
/// (both clamped to a sane range). Alpha is preserved. Returns how many colors
/// changed. A mesh with no color block is a no-op (`total() == 0`, file
/// untouched), not an error.
pub fn hsl_shift_scb(
    path: &Path,
    hue_degrees: f32,
    sat_mul: f32,
    light_mul: f32,
) -> Result<RecolorReport, String> {
    let mut mesh = read_mesh(path)?;
    let mut vertex_colors = 0usize;
    if let Some(colors) = mesh.colors.as_mut() {
        for c in colors.iter_mut() {
            let out = hsl_shift_rgba_u8(*c, hue_degrees, sat_mul, light_mul);
            if out != *c {
                *c = out;
                vertex_colors += 1;
            }
        }
    }
    write_if_changed(path, &mesh, vertex_colors)?;
    Ok(RecolorReport {
        vertex_colors,
        vcp_colors: 0,
    })
}

/// Tint every vertex color toward `target` RGB by `strength` (0..1): `0` keeps
/// the original, `1` replaces it fully. Alpha preserved. No-op on a colorless
/// mesh.
pub fn tint_scb(path: &Path, target: [u8; 3], strength: f32) -> Result<RecolorReport, String> {
    let s = strength.clamp(0.0, 1.0);
    let mut mesh = read_mesh(path)?;
    let mut vertex_colors = 0usize;
    if let Some(colors) = mesh.colors.as_mut() {
        for c in colors.iter_mut() {
            let out = [
                lerp_u8(c[0], target[0], s),
                lerp_u8(c[1], target[1], s),
                lerp_u8(c[2], target[2], s),
                c[3],
            ];
            if out != *c {
                *c = out;
                vertex_colors += 1;
            }
        }
    }
    write_if_changed(path, &mesh, vertex_colors)?;
    Ok(RecolorReport {
        vertex_colors,
        vcp_colors: 0,
    })
}

/// Replace the whole per-vertex color block of the `.scb` at `path` with
/// `colors` (one RGBA per mesh vertex, parallel to `positions`). Used to save a
/// 3D vertex-paint session. When the mesh had no color block this generates one
/// (forcing the 3.2 container + `vertex_type = 1`).
///
/// Errors when `colors.len()` does not match the mesh's vertex count, since the
/// reader always expects exactly `vertex_count` colors.
pub fn apply_vertex_colors_scb(path: &Path, colors: &[[u8; 4]]) -> Result<RecolorReport, String> {
    let mut mesh = read_mesh(path)?;
    let want = mesh.positions.len();
    if colors.len() != want {
        return Err(format!(
            "color count {} != mesh vertex count {} for {}",
            colors.len(),
            want,
            path.display()
        ));
    }
    // A fresh block needs the 3.2 container + indicator; a mesh that already had
    // colors keeps its version and just gets new values.
    if mesh.colors.is_none() {
        mesh.version = (3, 2);
        mesh.vertex_type = Some(1);
    }
    mesh.colors = Some(colors.to_vec());
    let out = mesh
        .to_bytes()
        .map_err(|e| format!("serialize {}: {e}", path.display()))?;
    std::fs::write(path, out).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(RecolorReport {
        vertex_colors: want,
        vcp_colors: 0,
    })
}

/// Ensure the `.scb` at `path` has a per-vertex color block, generating an
/// all-`fill` one when absent (forcing the 3.2 container + `vertex_type = 1`).
/// A no-op when colors already exist. Returns the vertex count on generation,
/// `0` when the mesh already had colors.
pub fn ensure_vertex_colors_scb(path: &Path, fill: [u8; 4]) -> Result<RecolorReport, String> {
    let mut mesh = read_mesh(path)?;
    if mesh.colors.as_ref().is_some_and(|c| !c.is_empty()) {
        return Ok(RecolorReport {
            vertex_colors: 0,
            vcp_colors: 0,
        });
    }
    let n = mesh.positions.len();
    mesh.version = (3, 2);
    mesh.vertex_type = Some(1);
    mesh.colors = Some(vec![fill; n]);
    let out = mesh
        .to_bytes()
        .map_err(|e| format!("serialize {}: {e}", path.display()))?;
    std::fs::write(path, out).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(RecolorReport {
        vertex_colors: n,
        vcp_colors: 0,
    })
}

/// Read + parse the `.scb`/`.sco` at `path` into a `StaticMesh`.
fn read_mesh(path: &Path) -> Result<StaticMesh, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    StaticMesh::from_bytes(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Serialize + write `mesh` back to `path` only when `changed > 0`.
fn write_if_changed(path: &Path, mesh: &StaticMesh, changed: usize) -> Result<(), String> {
    if changed > 0 {
        let out = mesh
            .to_bytes()
            .map_err(|e| format!("serialize {}: {e}", path.display()))?;
        std::fs::write(path, out).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Linear blend of two bytes by `t` (0..1).
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let v = a as f32 + (b as f32 - a as f32) * t;
    v.round().clamp(0.0, 255.0) as u8
}

/// True when the `.scb` at `path` carries a per-vertex color block — used to
/// gate the recolor UI so the action only offers itself on meshes it can edit.
pub fn scb_has_vertex_colors(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    match StaticMesh::from_bytes(&bytes) {
        Ok(mesh) => mesh.colors.as_ref().is_some_and(|c| !c.is_empty()),
        Err(_) => false,
    }
}

/// Hue-rotate one RGBA byte color, preserving saturation, lightness, and alpha.
fn hue_shift_rgba_u8(c: [u8; 4], hue_degrees: f32) -> [u8; 4] {
    hsl_shift_rgba_u8(c, hue_degrees, 1.0, 1.0)
}

/// Shift one RGBA byte color in HSL: add `hue_degrees`, multiply saturation by
/// `sat_mul` and lightness by `light_mul` (clamped), preserving alpha.
fn hsl_shift_rgba_u8(c: [u8; 4], hue_degrees: f32, sat_mul: f32, light_mul: f32) -> [u8; 4] {
    let r = c[0] as f32 / 255.0;
    let g = c[1] as f32 / 255.0;
    let b = c[2] as f32 / 255.0;
    let (h, s, l) = rgb_to_hsl(r, g, b);
    let nh = (h + hue_degrees / 360.0).rem_euclid(1.0);
    let ns = (s * sat_mul).clamp(0.0, 1.0);
    let nl = (l * light_mul).clamp(0.0, 1.0);
    let (nr, ng, nb) = hsl_to_rgb(nh, ns, nl);
    [
        (nr * 255.0).round().clamp(0.0, 255.0) as u8,
        (ng * 255.0).round().clamp(0.0, 255.0) as u8,
        (nb * 255.0).round().clamp(0.0, 255.0) as u8,
        c[3],
    ]
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let l = (max + min) / 2.0;
    if delta == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let mut h = if max == r {
        let mut hue = ((g - b) / delta) % 6.0;
        if g < b {
            hue += 6.0;
        }
        hue
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h /= 6.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let conv = |t: f32| -> f32 {
        let mut t = t;
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 1.0 / 2.0 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (conv(h + 1.0 / 3.0), conv(h), conv(h - 1.0 / 3.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_shift_360_is_identity() {
        let c = [200, 40, 120, 255];
        // A full turn returns (within rounding) the same color.
        let out = hue_shift_rgba_u8(c, 360.0);
        for i in 0..4 {
            assert!((out[i] as i32 - c[i] as i32).abs() <= 1, "channel {i}");
        }
    }

    #[test]
    fn hue_shift_preserves_alpha() {
        let out = hue_shift_rgba_u8([10, 20, 30, 77], 120.0);
        assert_eq!(out[3], 77);
    }

    #[test]
    fn grey_is_unchanged_by_hue() {
        // Zero saturation: hue rotation can't move a grey.
        let c = [128, 128, 128, 255];
        assert_eq!(hue_shift_rgba_u8(c, 90.0), c);
    }
}
