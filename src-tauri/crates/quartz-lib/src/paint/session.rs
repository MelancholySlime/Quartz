//! Resident bin-session registry. A session holds the main skin bin plus every
//! resolved linked bin (via [`crate::linked_bins`]) as resident trees, one
//! merged VFX edit index whose keys are bin-prefixed and whose paths carry a
//! bin, the per-bin source formats, and a bounded undo stack of per-bin
//! entry-granular COW frames (see [`crate::undo`]). An edit clones only the
//! top-level entries it touches and marks only that bin dirty. Save writes ONLY
//! dirty bins, each back to its own file. Mirrors `crate::vfx_session`.

use super::model::{self, EditIndex, VfxModel};
use super::recolor::{self, ColorTargetSel, PaletteStop, RecolorOptions};
use crate::error::{Error, Result};
use crate::linked_bins::{self, LoadedBin};
use crate::undo::UndoFrame;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub type SessionId = u64;

const UNDO_CAP: usize = 50;

/// One reversible edit across the session: per-bin entry frames, so a single
/// undo reverses the last logical edit regardless of which bins it touched.
struct MultiUndoFrame {
    parts: Vec<(usize, UndoFrame)>,
}

impl MultiUndoFrame {
    fn swap_with(&mut self, bins: &mut [LoadedBin]) {
        for (bin_idx, frame) in self.parts.iter_mut() {
            if let Some(lb) = bins.get_mut(*bin_idx) {
                frame.swap_with(&mut lb.tree);
                lb.dirty = true;
            }
        }
    }
}

pub struct BinSession {
    pub id: SessionId,
    /// Index 0 is always the main bin; linked bins follow in `linked` order.
    pub bins: Vec<LoadedBin>,
    pub index: EditIndex,
    undo: Vec<MultiUndoFrame>,
    redo: Vec<MultiUndoFrame>,
}

impl BinSession {
    /// Reproject the edit index + view model from all resident bins. Called
    /// after any structural change (open, undo). Edits that only change
    /// vec4/u8 values keep the index valid, so they don't need a reproject.
    fn reproject(&mut self) -> VfxModel {
        let (model, index) = model::project_all(&self.bins);
        self.index = index;
        model
    }

    /// Commit a completed edit's frame to the undo stack. A fresh edit
    /// invalidates the redo history.
    fn push_undo(&mut self, frame: MultiUndoFrame) {
        if self.undo.len() >= UNDO_CAP {
            self.undo.remove(0);
        }
        self.undo.push(frame);
        self.redo.clear();
    }

    /// Capture one undo frame across the bins for the given `(bin, entry)`
    /// touch set, grouping entries per bin.
    fn capture(&self, touched: impl IntoIterator<Item = (usize, usize)>) -> MultiUndoFrame {
        let mut by_bin: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
        for (b, e) in touched {
            by_bin.entry(b).or_default().push(e);
        }
        let parts = by_bin
            .into_iter()
            .filter_map(|(b, entries)| {
                self.bins
                    .get(b)
                    .map(|lb| (b, UndoFrame::capture(&lb.tree, entries)))
            })
            .collect();
        MultiUndoFrame { parts }
    }

    /// Mark every bin in the touch set dirty.
    fn dirty_bins(&mut self, bins_touched: impl IntoIterator<Item = usize>) {
        for b in bins_touched {
            if let Some(lb) = self.bins.get_mut(b) {
                lb.dirty = true;
            }
        }
    }
}

static REGISTRY: OnceLock<RwLock<HashMap<SessionId, BinSession>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn registry() -> &'static RwLock<HashMap<SessionId, BinSession>> {
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Result of opening a file: the session id plus the initial VFX view.
pub struct OpenResult {
    pub session_id: SessionId,
    pub model: VfxModel,
}

/// Open a `.bin` (plus its resolvable linked bins) into a resident session and
/// register it. The main bin is index 0; linked bins follow. A `.py`/`.ritobin`
/// main opens as a single bin with no link resolution.
pub fn open(path: impl AsRef<Path>) -> Result<OpenResult> {
    let path = path.as_ref().to_path_buf();
    let bins = linked_bins::open_with_linked(&path)?;
    let (model, index) = model::project_all(&bins);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    registry().write().insert(
        id,
        BinSession {
            id,
            bins,
            index,
            undo: Vec::new(),
            redo: Vec::new(),
        },
    );
    Ok(OpenResult {
        session_id: id,
        model,
    })
}

/// Drop a session and free its trees. Returns false if the id was unknown.
pub fn close(id: SessionId) -> bool {
    registry().write().remove(&id).is_some()
}

fn with_session<R>(id: SessionId, f: impl FnOnce(&mut BinSession) -> R) -> Result<R> {
    let mut reg = registry().write();
    let session = reg
        .get_mut(&id)
        .ok_or_else(|| Error::InvalidInput(format!("No paint session with id {}", id)))?;
    Ok(f(session))
}

/// Reparse this session when one of its source BINs changed externally.
/// External disk state is authoritative, so local history is reset.
pub fn reload_if_changed(id: SessionId) -> Result<Option<VfxModel>> {
    with_session(id, |session| -> Result<Option<VfxModel>> {
        let Some(bins) = linked_bins::reload_if_changed(&session.bins)? else {
            return Ok(None);
        };
        session.bins = bins;
        session.undo.clear();
        session.redo.clear();
        Ok(Some(session.reproject()))
    })?
}

/// Recolor selected emitters. Snapshots for undo, mutates the tree, returns the
/// count modified. The caller fetches refreshed colors via [`model_of`] /
/// [`emitter_colors`] as needed.
#[allow(clippy::too_many_arguments)]
pub fn recolor_emitters(
    id: SessionId,
    emitter_keys: &[String],
    targets: &[ColorTargetSel],
    palette: &[PaletteStop],
    opts: &RecolorOptions,
) -> Result<usize> {
    with_session(id, |s| {
        // Every path a recolor can write lives under the selected emitters'
        // color targets — snapshot just those (bin, entry) pairs.
        let touched: Vec<(usize, usize)> = emitter_keys
            .iter()
            .filter_map(|k| s.index.emitter_colors.get(k))
            .flat_map(|slots| slots.values())
            .flat_map(|t| {
                // constant + values keyframes + every probability-table channel
                // node. Channel-table colors have empty constant/keyframes, so
                // without the channel paths their bins are never captured for undo
                // or marked dirty even though the recolor writes them.
                t.constant
                    .iter()
                    .chain(t.keyframes.iter())
                    .chain(t.channel_tables.iter().flatten().flatten())
            })
            .map(|p| (p.bin, p.entry))
            .collect();
        let bins_touched: Vec<usize> = touched.iter().map(|(b, _)| *b).collect();
        let frame = s.capture(touched);
        let n =
            recolor::recolor_emitters(&mut s.bins, &s.index, emitter_keys, targets, palette, opts);
        if n > 0 {
            s.dirty_bins(bins_touched);
            s.push_undo(frame);
        }
        n
    })
}

/// Recolor a single material color param.
pub fn set_material_param(
    id: SessionId,
    selection_key: &str,
    new_color: [f32; 4],
    preserve_alpha: bool,
) -> Result<bool> {
    with_session(id, |s| {
        let Some(path) = s.index.material_params.get(selection_key).cloned() else {
            return false;
        };
        let frame = s.capture([(path.bin, path.entry)]);
        let changed =
            recolor::recolor_material_param(&mut s.bins, &path, new_color, preserve_alpha, false);
        if changed {
            s.dirty_bins([path.bin]);
            s.push_undo(frame);
        }
        changed
    })
}

/// Set an emitter's blend mode, authoring the field when it used the default.
pub fn set_blend_mode(id: SessionId, emitter_key: &str, mode: u8) -> Result<bool> {
    set_blend_mode_bulk(id, &[emitter_key.to_owned()], mode).map(|count| count > 0)
}

/// The whole batch is one undo step, including newly authored blendMode fields.
pub fn set_blend_mode_bulk(id: SessionId, emitter_keys: &[String], mode: u8) -> Result<usize> {
    with_session(id, |s| {
        let mut seen = std::collections::HashSet::new();
        let nodes: Vec<_> = emitter_keys
            .iter()
            .filter(|key| seen.insert(key.as_str()))
            .filter_map(|key| s.index.emitter_nodes.get(key).cloned())
            .collect();
        let frame = s.capture(nodes.iter().map(|p| (p.bin, p.entry)));
        let mut changed = 0;
        let mut bins = Vec::new();
        for node in nodes {
            let Some(fields) = emitter_fields(&mut s.bins, &node) else {
                continue;
            };
            let hash = super::fnv1a_lower("blendMode");
            let current = match fields.get(&hash) {
                Some(BinValue::U8(value)) => *value,
                None => 0,
                _ => continue,
            };
            if current != mode {
                fields.insert(hash, BinValue::U8(mode));
                changed += 1;
                bins.push(node.bin);
            }
        }
        if changed > 0 {
            s.dirty_bins(bins);
            s.push_undo(frame);
            s.reproject();
        }
        changed
    })
}

/// Rewrite an emitter's texture path (the `string` node whose current value is
/// `old_path`). Returns the refreshed model so the edit index picks up the new
/// path value; `None` if the node could not be located or the value was
/// unchanged.
pub fn set_texture(
    id: SessionId,
    emitter_key: &str,
    old_path: &str,
    new_path: &str,
) -> Result<Option<VfxModel>> {
    with_session(id, |s| {
        let Some(path) = s
            .index
            .emitter_textures
            .get(emitter_key)
            .and_then(|m| m.get(old_path))
            .cloned()
        else {
            return None;
        };
        let frame = s.capture([(path.bin, path.entry)]);
        let changed = match path.resolve_mut(&mut s.bins) {
            Some(ritoshark::bin::BinValue::String(v)) => {
                if v != new_path {
                    *v = new_path.to_string();
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if !changed {
            return None;
        }
        s.dirty_bins([path.bin]);
        s.push_undo(frame);
        // The texture string is an index key, so relocate everything.
        Some(s.reproject())
    })
}

/// Set the alpha channel of an emitter color slot, per keyframe, preserving
/// RGB. `slot` is one of "color" | "birthColor" | "fresnelColor" |
/// "lingerColor". `alphas` are applied in the model's keyframe order: the
/// constant (if any) first, then the `values` list in order; extra/missing
/// entries are ignored. Returns the refreshed model, or `None` if nothing
/// changed / the slot wasn't found.
pub fn set_color_alpha(
    id: SessionId,
    emitter_key: &str,
    slot: &str,
    alphas: &[f32],
) -> Result<Option<VfxModel>> {
    let slot = match slot {
        "color" => model::ColorSlot::Color,
        "birthColor" => model::ColorSlot::BirthColor,
        "fresnelColor" => model::ColorSlot::FresnelColor,
        "lingerColor" => model::ColorSlot::LingerColor,
        _ => return Ok(None),
    };
    with_session(id, |s| {
        let Some(target) = s
            .index
            .emitter_colors
            .get(emitter_key)
            .and_then(|slots| slots.get(&slot))
            .cloned()
        else {
            return None;
        };
        // All nodes live in the same emitter entry; capture one frame.
        let entry = (target.color_path.bin, target.color_path.entry);
        let frame = s.capture([entry]);

        // Node order mirrors color_data_from_target: constant first, then list.
        let mut nodes: Vec<&model::NodePath> = Vec::new();
        if let Some(c) = target.constant.as_ref() {
            nodes.push(c);
        }
        nodes.extend(target.keyframes.iter());

        let mut changed = false;
        for (node, &a) in nodes.iter().zip(alphas.iter()) {
            if !a.is_finite() {
                continue;
            }
            let a = a.clamp(0.0, 1.0);
            if let Some(ritoshark::bin::BinValue::Vec4(v)) = node.resolve_mut(&mut s.bins) {
                if (v[3] - a).abs() > f32::EPSILON {
                    v[3] = a;
                    changed = true;
                }
            }
        }
        let offset = nodes.len();
        for (channels, &a) in target.channel_tables.iter().zip(alphas.iter().skip(offset)) {
            if !a.is_finite() {
                continue;
            }
            if let Some(path) = &channels[3] {
                if let Some(BinValue::F32(value)) = path.resolve_mut(&mut s.bins) {
                    let next = a.clamp(0.0, 1.0);
                    if *value != next {
                        *value = next;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            return None;
        }
        s.dirty_bins([entry.0]);
        s.push_undo(frame);
        Some(s.reproject())
    })
}

// ── Structural color edits: create / keyframes / (de)animate ─────────────────
//
// Unlike recolor/alpha (in-place vec4 mutation), these change tree SHAPE, so
// each reprojects afterward (the EditIndex slot paths shift). They resolve the
// emitter's own node (`EditIndex.emitter_nodes`) and mutate its field map or a
// nested color's `values`/`times` lists directly, using ritoshark BinValue
// construction (mirrors the `color_field` test template).

use ritoshark::bin::{BinType, BinValue};

fn slot_field_hashes(slot: model::ColorSlot) -> &'static [&'static str] {
    match slot {
        model::ColorSlot::Color => &["color"],
        model::ColorSlot::BirthColor => &["birthColor"],
        model::ColorSlot::FresnelColor => &["fresnelColor", "outlineColor"],
        model::ColorSlot::LingerColor => &["lingerColor", "SeparateLingerColor"],
    }
}

fn parse_slot(slot: &str) -> Option<model::ColorSlot> {
    Some(match slot {
        "color" => model::ColorSlot::Color,
        "birthColor" => model::ColorSlot::BirthColor,
        "fresnelColor" => model::ColorSlot::FresnelColor,
        "lingerColor" => model::ColorSlot::LingerColor,
        _ => return None,
    })
}

/// A vec4 `values` list from a slice of RGBA.
fn build_vec4_list(items: &[[f32; 4]]) -> BinValue {
    BinValue::List {
        is_list2: false,
        item: BinType::Vec4,
        items: items.iter().map(|v| BinValue::Vec4(*v)).collect(),
    }
}

/// An f32 `times` list.
fn build_f32_list(times: &[f32]) -> BinValue {
    BinValue::List {
        is_list2: false,
        item: BinType::F32,
        items: times.iter().map(|t| BinValue::F32(*t)).collect(),
    }
}

/// A fresh `ValueColor { constantValue }` embed.
fn build_value_color_constant(rgba: [f32; 4]) -> BinValue {
    let mut f = indexmap::IndexMap::new();
    f.insert(super::fnv1a_lower("constantValue"), BinValue::Vec4(rgba));
    BinValue::Embed {
        class: super::fnv1a_lower("ValueColor"),
        fields: f,
    }
}

/// Resolve the emitter's field map (`&mut IndexMap`) for `emitter_key`, given a
/// clone of its node path. Returns None if the emitter node isn't an embed/ptr.
fn emitter_fields<'a>(
    bins: &'a mut [LoadedBin],
    node: &model::NodePath,
) -> Option<&'a mut indexmap::IndexMap<u32, BinValue>> {
    match node.resolve_mut(bins) {
        Some(BinValue::Embed { fields, .. }) | Some(BinValue::Pointer { fields, .. }) => {
            Some(fields)
        }
        _ => None,
    }
}

/// The field-map holding a color slot's `values`/`times` lists: the ValueColor
/// embed's `dynamics` pointer fields when animated, else the embed's own fields.
/// Returns None when the color isn't an animatable embed with lists.
fn color_curve_fields<'a>(
    color: &'a mut BinValue,
) -> Option<&'a mut indexmap::IndexMap<u32, BinValue>> {
    let h_dynamics = super::fnv1a_lower("dynamics");
    let fields = match color {
        BinValue::Embed { fields, .. } | BinValue::Pointer { fields, .. } => fields,
        _ => return None,
    };
    // `has_dyn` is an owned bool, so the immutable borrow from `.get` ends before
    // the mutable branch below (avoids a get/get_mut borrow conflict).
    let has_dyn = matches!(
        fields.get(&h_dynamics),
        Some(BinValue::Pointer { .. } | BinValue::Embed { .. })
    );
    if has_dyn {
        match fields.get_mut(&h_dynamics) {
            Some(BinValue::Pointer { fields: df, .. } | BinValue::Embed { fields: df, .. }) => {
                Some(df)
            }
            _ => None,
        }
    } else {
        Some(fields)
    }
}

/// Look up the emitter's color field (by slot) as a mutable `BinValue`, plus a
/// touch (bin, entry) for undo. Resolves through the emitter node.
fn resolve_color_field<'a>(
    s: &'a mut BinSession,
    emitter_key: &str,
    slot: model::ColorSlot,
) -> Option<(&'a mut BinValue, (usize, usize))> {
    let node = s
        .index
        .emitter_colors
        .get(emitter_key)?
        .get(&slot)?
        .color_path
        .clone();
    let touch = (node.bin, node.entry);
    node.resolve_mut(&mut s.bins).map(|f| (f, touch))
}

/// Create a missing color using the slot's real schema. Fresnel is a nested
/// vec4; lifetime/birth/linger colors are ValueColor wrappers.
pub fn create_color(id: SessionId, emitter_key: &str, slot: &str) -> Result<Option<VfxModel>> {
    let Some(slot) = parse_slot(slot) else {
        return Ok(None);
    };
    with_session(id, |s| {
        if color_target(s, emitter_key, slot).is_some() {
            return None;
        }
        let node = s.index.emitter_nodes.get(emitter_key)?.clone();
        let mut next = node.resolve_mut(&mut s.bins)?.clone();
        let fields = color_fields(&mut next)?;
        let existing = slot_field_hashes(slot)
            .iter()
            .map(|name| super::fnv1a_lower(name))
            .find(|hash| fields.contains_key(hash));
        if let Some(hash) = existing {
            // An empty ValueColor is also missing from the model. Restore its
            // required constant without dropping other authored wrapper fields.
            match fields.get_mut(&hash)? {
                BinValue::Embed { class, fields } | BinValue::Pointer { class, fields }
                    if *class == super::fnv1a_lower("ValueColor") =>
                {
                    fields.insert(
                        super::fnv1a_lower("constantValue"),
                        BinValue::Vec4([1.0; 4]),
                    );
                }
                _ => return None,
            }
        } else if slot == model::ColorSlot::FresnelColor {
            let reflection = fields
                .entry(super::fnv1a_lower("reflectionDefinition"))
                .or_insert_with(|| BinValue::Pointer {
                    class: super::fnv1a_lower("VfxReflectionDefinitionData"),
                    fields: indexmap::IndexMap::new(),
                });
            if let BinValue::Pointer { class, .. } = reflection {
                // A null pointer cannot serialize child fields until its class
                // is authored; otherwise Create appears to work until reopen.
                if *class == 0 {
                    *class = super::fnv1a_lower("VfxReflectionDefinitionData");
                }
            }
            color_fields(reflection)?
                .insert(super::fnv1a_lower("fresnelColor"), BinValue::Vec4([1.0; 4]));
        } else {
            let name = if slot == model::ColorSlot::LingerColor {
                "SeparateLingerColor"
            } else {
                slot_field_hashes(slot)[0]
            };
            fields.insert(
                super::fnv1a_lower(name),
                build_value_color_constant([1.0; 4]),
            );
        }
        if slot == model::ColorSlot::LingerColor {
            fields.insert(
                super::fnv1a_lower("UseSeparateLingerColor"),
                BinValue::Flag(true),
            );
        }
        let touch = (node.bin, node.entry);
        let frame = s.capture([touch]);
        *node.resolve_mut(&mut s.bins)? = next;
        s.dirty_bins([touch.0]);
        s.push_undo(frame);
        Some(s.reproject())
    })
}

/// Resolve a projected color target. Its indices are the public model indices:
/// constant first (when authored), followed by curve stops or channel ranges.
fn color_target(
    s: &BinSession,
    emitter_key: &str,
    slot: model::ColorSlot,
) -> Option<model::ColorTarget> {
    s.index.emitter_colors.get(emitter_key)?.get(&slot).cloned()
}

fn structural_color(target: &model::ColorTarget) -> bool {
    target.is_value_color && target.channel_tables.is_empty()
}

/// Apply a color replacement atomically after validating/building it off-tree.
/// One command contributes exactly one undo frame, including auto-animation.
fn replace_color(
    s: &mut BinSession,
    target: &model::ColorTarget,
    next: BinValue,
) -> Option<VfxModel> {
    let touch = (target.color_path.bin, target.color_path.entry);
    let frame = s.capture([touch]);
    *target.color_path.resolve_mut(&mut s.bins)? = next;
    s.dirty_bins([touch.0]);
    s.push_undo(frame);
    Some(s.reproject())
}

fn color_fields(color: &mut BinValue) -> Option<&mut indexmap::IndexMap<u32, BinValue>> {
    match color {
        BinValue::Embed { fields, .. } | BinValue::Pointer { fields, .. } => Some(fields),
        _ => None,
    }
}

/// Build dynamics in the existing ValueColor, retaining unknown fields and the
/// wrapper constant. Bare vec4 fields (especially Fresnel) are not animatable.
fn insert_curve(color: &mut BinValue, seed: [f32; 4], times: &[f32]) -> Option<()> {
    let fields = color_fields(color)?;
    let dynamics = fields
        .entry(super::fnv1a_lower("dynamics"))
        .or_insert_with(|| BinValue::Pointer {
            class: super::fnv1a_lower("VfxAnimatedColorVariableData"),
            fields: indexmap::IndexMap::new(),
        });
    if let BinValue::Pointer { class, .. } = dynamics {
        if *class == 0 {
            *class = super::fnv1a_lower("VfxAnimatedColorVariableData");
        }
    }
    let inner = color_fields(dynamics)?;
    inner.insert(super::fnv1a_lower("times"), build_f32_list(times));
    inner.insert(
        super::fnv1a_lower("values"),
        build_vec4_list(&vec![seed; times.len()]),
    );
    Some(())
}

/// Use exactly the same authored/fallback times the model displayed. Repair a
/// missing/short list before editing so existing stops do not jump in time.
fn aligned_curve_times(curve: &mut indexmap::IndexMap<u32, BinValue>, target: &model::ColorTarget) {
    curve.insert(super::fnv1a_lower("times"), build_f32_list(&target.times));
}

fn finite_keyframe(rgba: [f32; 4], time: f32) -> Result<([f32; 4], f32)> {
    if !time.is_finite() || rgba.iter().any(|v| !v.is_finite()) {
        return Err(Error::InvalidInput(
            "Color components and time must be finite".into(),
        ));
    }
    Ok((rgba.map(|v| v.clamp(0.0, 1.0)), time.clamp(0.0, 1.0)))
}

/// Promote a constant ValueColor to a two-stop lifetime curve.
pub fn animate_color(id: SessionId, emitter_key: &str, slot: &str) -> Result<Option<VfxModel>> {
    let Some(slot) = parse_slot(slot) else {
        return Ok(None);
    };
    with_session(id, |s| {
        let target = color_target(s, emitter_key, slot)?;
        if !structural_color(&target) || !target.keyframes.is_empty() {
            return None;
        }
        let mut next = target.color_path.resolve_mut(&mut s.bins)?.clone();
        let seed = constant_value(&next)?;
        insert_curve(&mut next, seed, &[0.0, 1.0])?;
        replace_color(s, &target, next)
    })
}

/// Collapse a lifetime curve to its first value, retaining other wrapper fields.
/// Probability tables represent random ranges and are never discarded here.
pub fn deanimate_color(id: SessionId, emitter_key: &str, slot: &str) -> Result<Option<VfxModel>> {
    let Some(slot) = parse_slot(slot) else {
        return Ok(None);
    };
    with_session(id, |s| {
        let target = color_target(s, emitter_key, slot)?;
        if !structural_color(&target) {
            return None;
        }
        let first = match target.keyframes.first()?.resolve_mut(&mut s.bins)? {
            BinValue::Vec4(value) => *value,
            _ => return None,
        };
        let mut next = target.color_path.resolve_mut(&mut s.bins)?.clone();
        let fields = color_fields(&mut next)?;
        for name in ["dynamics", "values", "times"] {
            fields.shift_remove(&super::fnv1a_lower(name));
        }
        fields.insert(super::fnv1a_lower("constantValue"), BinValue::Vec4(first));
        replace_color(s, &target, next)
    })
}

/// Append a stop; promoting a constant and appending is one atomic edit.
pub fn add_keyframe(
    id: SessionId,
    emitter_key: &str,
    slot: &str,
    rgba: [f32; 4],
    time: f32,
) -> Result<Option<VfxModel>> {
    let Some(slot) = parse_slot(slot) else {
        return Ok(None);
    };
    let (rgba, time) = finite_keyframe(rgba, time)?;
    with_session(id, |s| {
        let target = color_target(s, emitter_key, slot)?;
        if !structural_color(&target) {
            return None;
        }
        let mut next = target.color_path.resolve_mut(&mut s.bins)?.clone();
        if target.keyframes.is_empty() {
            let seed = constant_value(&next)?;
            insert_curve(&mut next, seed, &[0.0])?;
        } else {
            aligned_curve_times(color_curve_fields(&mut next)?, &target);
        }
        let curve = color_curve_fields(&mut next)?;
        match curve.get_mut(&super::fnv1a_lower("values"))? {
            BinValue::List {
                item: BinType::Vec4,
                items,
                ..
            } => items.push(BinValue::Vec4(rgba)),
            _ => return None,
        }
        match curve.get_mut(&super::fnv1a_lower("times"))? {
            BinValue::List { items, .. } => items.push(BinValue::F32(time)),
            _ => return None,
        }
        replace_color(s, &target, next)
    })
}

/// Map the public model index to its underlying curve-list index. The wrapper
/// constant is independently editable but is never a removable/retimable stop.
fn curve_index(target: &model::ColorTarget, index: usize) -> Option<usize> {
    let offset = usize::from(target.constant.is_some());
    let path = target.keyframes.get(index.checked_sub(offset)?)?;
    match path.steps.last()? {
        model::Step::Index(index) => Some(*index),
        _ => None,
    }
}

/// Delete a stop using the projected model index; retain at least one stop.
pub fn delete_keyframe(
    id: SessionId,
    emitter_key: &str,
    slot: &str,
    index: usize,
) -> Result<Option<VfxModel>> {
    let Some(slot) = parse_slot(slot) else {
        return Ok(None);
    };
    with_session(id, |s| {
        let target = color_target(s, emitter_key, slot)?;
        if !structural_color(&target) || target.keyframes.len() <= 1 {
            return None;
        }
        let index = curve_index(&target, index)?;
        let mut next = target.color_path.resolve_mut(&mut s.bins)?.clone();
        let curve = color_curve_fields(&mut next)?;
        aligned_curve_times(curve, &target);
        match curve.get_mut(&super::fnv1a_lower("values"))? {
            BinValue::List { items, .. } if index < items.len() => {
                items.remove(index);
            }
            _ => return None,
        }
        match curve.get_mut(&super::fnv1a_lower("times"))? {
            BinValue::List { items, .. } if index < items.len() => {
                items.remove(index);
            }
            _ => return None,
        }
        replace_color(s, &target, next)
    })
}

/// Update the public model index in place. Probability-table keys scatter RGBA
/// to authored channels only; their random-distribution times stay unchanged.
pub fn set_keyframe(
    id: SessionId,
    emitter_key: &str,
    slot: &str,
    index: usize,
    rgba: [f32; 4],
    time: f32,
) -> Result<Option<VfxModel>> {
    let Some(slot) = parse_slot(slot) else {
        return Ok(None);
    };
    let (rgba, time) = finite_keyframe(rgba, time)?;
    with_session(id, |s| {
        let target = color_target(s, emitter_key, slot)?;
        let offset = usize::from(target.constant.is_some());
        let constant = index == 0 && target.constant.is_some();
        let list_index = if constant {
            None
        } else {
            Some(index.checked_sub(offset)?)
        };
        let vec_path = if constant {
            target.constant.as_ref()
        } else {
            target.keyframes.get(list_index?)
        };
        let channels = list_index
            .and_then(|i| i.checked_sub(target.keyframes.len()))
            .and_then(|i| target.channel_tables.get(i));
        if vec_path.is_none() && channels.is_none() {
            return None;
        }
        let touch = (target.color_path.bin, target.color_path.entry);
        let frame = s.capture([touch]);
        let mut changed = false;
        if let Some(path) = vec_path {
            if let Some(BinValue::Vec4(value)) = path.resolve_mut(&mut s.bins) {
                if *value != rgba {
                    *value = rgba;
                    changed = true;
                }
            }
        }
        if let Some(channels) = channels {
            for (channel, path) in channels.iter().enumerate() {
                if let Some(path) = path {
                    if let Some(BinValue::F32(value)) = path.resolve_mut(&mut s.bins) {
                        if *value != rgba[channel] {
                            *value = rgba[channel];
                            changed = true;
                        }
                    }
                }
            }
        }
        if !constant && vec_path.is_some() {
            let projected_index = list_index?;
            let raw_index = curve_index(&target, index)?;
            if target.times.get(projected_index).copied() != Some(time) {
                let (color, _) = resolve_color_field(s, emitter_key, slot)?;
                let curve = color_curve_fields(color)?;
                aligned_curve_times(curve, &target);
                if let Some(BinValue::List { items, .. }) =
                    curve.get_mut(&super::fnv1a_lower("times"))
                {
                    if let Some(value) = items.get_mut(raw_index) {
                        *value = BinValue::F32(time);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            return None;
        }
        s.dirty_bins([touch.0]);
        s.push_undo(frame);
        Some(s.reproject())
    })
}

fn constant_value(color: &BinValue) -> Option<[f32; 4]> {
    match color {
        BinValue::Vec4(v) => Some(*v),
        BinValue::Embed { fields, .. } | BinValue::Pointer { fields, .. } => {
            match fields.get(&super::fnv1a_lower("constantValue")) {
                Some(BinValue::Vec4(v)) => Some(*v),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Undo the last mutating edit. Returns the refreshed model, or `None` if the
/// undo stack was empty.
pub fn undo(id: SessionId) -> Result<Option<VfxModel>> {
    with_session(id, |s| {
        match s.undo.pop() {
            Some(mut frame) => {
                // Swap the stored entries back in; the frame now holds the
                // undone state and parks on the redo stack.
                frame.swap_with(&mut s.bins);
                s.redo.push(frame);
                Some(s.reproject())
            }
            None => None,
        }
    })
}

/// Redo the last undone edit. Returns the refreshed model, or null if there's
/// nothing to redo.
pub fn redo(id: SessionId) -> Result<Option<VfxModel>> {
    with_session(id, |s| match s.redo.pop() {
        Some(mut frame) => {
            frame.swap_with(&mut s.bins);
            if s.undo.len() >= UNDO_CAP {
                s.undo.remove(0);
            }
            s.undo.push(frame);
            Some(s.reproject())
        }
        None => None,
    })
}

/// Re-fetch the full VFX model (after edits, to refresh views).
pub fn model_of(id: SessionId) -> Result<VfxModel> {
    with_session(id, |s| {
        let (model, _) = model::project_all(&s.bins);
        model
    })
}

/// Refreshed color views for just `emitter_keys`, read from the live tree —
/// the partial payload a recolor returns instead of a whole-model
/// reprojection (O(selected emitters), not O(file)).
pub fn emitter_colors_of(
    id: SessionId,
    emitter_keys: &[String],
) -> Result<HashMap<String, model::EmitterColors>> {
    with_session(id, |s| {
        let mut out = HashMap::new();
        let BinSession { bins, index, .. } = s;
        for key in emitter_keys {
            if let Some(slots) = index.emitter_colors.get(key) {
                out.insert(key.clone(), model::emitter_colors_from_targets(bins, slots));
            }
        }
        out
    })
}

/// Save the session. With `out_path = None`, writes ONLY the dirty bins, each
/// back to its own file in its own format (via [`crate::linked_bins::save_dirty`]),
/// and returns the paths written. With `out_path = Some(dest)`, this is a
/// Save-As of the MAIN bin (index 0) to `dest`; linked bins are not written.
pub fn save(id: SessionId, out_path: Option<PathBuf>, force: bool) -> Result<Vec<PathBuf>> {
    with_session(id, |s| -> Result<Vec<PathBuf>> {
        match out_path {
            None => linked_bins::save_dirty_checked(&mut s.bins, force),
            Some(dest) => {
                let main = s
                    .bins
                    .get_mut(0)
                    .ok_or_else(|| Error::InvalidInput("Session has no bins".to_string()))?;
                let bytes = crate::bin::write_bin(&main.tree)
                    .map_err(|e| Error::InvalidInput(e.to_string()))?;
                std::fs::write(&dest, bytes).map_err(|e| Error::io_with_path(e, &dest))?;
                main.dirty = false;
                Ok(vec![dest])
            }
        }
    })?
}

/* Self-contained edit tests.
 *
 * These build their own bins on disk rather than needing a real skin, so they
 * always run. They cover the shapes a VFX colour actually takes - a bare vec4,
 * a `constantValue`, an animated `values` list, and the `dynamics`-nested form -
 * and assert that an edit reaches the BYTES, survives a save/reopen round-trip,
 * and replays byte-exact through undo/redo. */
#[cfg(test)]
mod edit_tests {
    use super::*;
    use crate::bin::write_bin;
    use crate::paint::fnv1a_lower as fh;
    use crate::paint::recolor::{ColorTargetSel, PaletteStop, RecolorMode, RecolorOptions};
    use indexmap::IndexMap;
    use ritoshark::bin::{Bin, BinEntry, BinType, BinValue};

    /// Colour field shapes a VfxEmitter can carry.
    enum ColorShape {
        /// `color: vec4 = {...}`
        BareVec4([f32; 4]),
        /// `color: embed = ValueColor { constantValue: vec4 }`
        Constant([f32; 4]),
        /// `ValueColor { values: list[vec4] }`
        Values(Vec<[f32; 4]>),
        /// `ValueColor { dynamics: pointer { values: list[vec4], times: list[f32] } }`
        Dynamics(Vec<[f32; 4]>),
        /// `ValueColor { constantValue: vec4, dynamics: { values, times } }` -
        /// both a constant AND keyframes, the case the editor lists together.
        ConstantPlusDynamics([f32; 4], Vec<[f32; 4]>),
    }

    fn vec4_list(items: &[[f32; 4]]) -> BinValue {
        BinValue::List {
            is_list2: false,
            item: BinType::Vec4,
            items: items.iter().map(|v| BinValue::Vec4(*v)).collect(),
        }
    }

    fn f32_list(n: usize) -> BinValue {
        BinValue::List {
            is_list2: false,
            item: BinType::F32,
            items: (0..n)
                .map(|i| {
                    BinValue::F32(if n <= 1 {
                        0.0
                    } else {
                        i as f32 / (n - 1) as f32
                    })
                })
                .collect(),
        }
    }

    fn color_field(shape: &ColorShape) -> BinValue {
        match shape {
            ColorShape::BareVec4(v) => BinValue::Vec4(*v),
            ColorShape::Constant(v) => {
                let mut f = IndexMap::new();
                f.insert(fh("constantValue"), BinValue::Vec4(*v));
                BinValue::Embed {
                    class: fh("ValueColor"),
                    fields: f,
                }
            }
            ColorShape::Values(vs) => {
                let mut f = IndexMap::new();
                f.insert(fh("values"), vec4_list(vs));
                f.insert(fh("times"), f32_list(vs.len()));
                BinValue::Embed {
                    class: fh("ValueColor"),
                    fields: f,
                }
            }
            ColorShape::Dynamics(vs) => {
                let mut inner = IndexMap::new();
                inner.insert(fh("values"), vec4_list(vs));
                inner.insert(fh("times"), f32_list(vs.len()));
                let mut f = IndexMap::new();
                f.insert(
                    fh("dynamics"),
                    BinValue::Pointer {
                        class: fh("VfxAnimatedColorVariableData"),
                        fields: inner,
                    },
                );
                BinValue::Embed {
                    class: fh("ValueColor"),
                    fields: f,
                }
            }
            ColorShape::ConstantPlusDynamics(c, vs) => {
                let mut inner = IndexMap::new();
                inner.insert(fh("values"), vec4_list(vs));
                inner.insert(fh("times"), f32_list(vs.len()));
                let mut f = IndexMap::new();
                f.insert(fh("constantValue"), BinValue::Vec4(*c));
                f.insert(
                    fh("dynamics"),
                    BinValue::Pointer {
                        class: fh("VfxAnimatedColorVariableData"),
                        fields: inner,
                    },
                );
                BinValue::Embed {
                    class: fh("ValueColor"),
                    fields: f,
                }
            }
        }
    }

    /// One system, one complex emitter, with `color` set to `shape`.
    fn bin_with_color(shape: ColorShape) -> Bin {
        let mut emitter = IndexMap::new();
        emitter.insert(fh("emitterName"), BinValue::String("Test".into()));
        emitter.insert(fh("blendMode"), BinValue::U8(1));
        emitter.insert(fh("color"), color_field(&shape));

        let mut sys = IndexMap::new();
        sys.insert(fh("particleName"), BinValue::String("TestSystem".into()));
        sys.insert(
            fh("complexEmitterDefinitionData"),
            BinValue::List {
                is_list2: false,
                item: BinType::Embed,
                items: vec![BinValue::Embed {
                    class: fh("VfxEmitterDefinitionData"),
                    fields: emitter,
                }],
            },
        );

        let mut bin = Bin::new();
        bin.version = 3;
        bin.entries.push(BinEntry {
            path_hash: 0x1234_5678,
            class_hash: fh("VfxSystemDefinitionData"),
            fields: sys,
        });
        bin
    }

    fn write_temp(bin: &Bin, name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("quartz-paint-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}-{}.bin", name, std::process::id()));
        std::fs::write(&path, write_bin(bin).unwrap()).unwrap();
        path
    }

    fn only_emitter_key(id: SessionId) -> String {
        with_session(id, |s| {
            s.index
                .emitter_colors
                .keys()
                .next()
                .cloned()
                .expect("emitter should have a colour target")
        })
        .unwrap()
    }

    /// Every alpha the model currently reports for the `color` slot, in the
    /// same order the editor lists (and the writer zips) them.
    fn alphas_of(id: SessionId, key: &str) -> Vec<f32> {
        with_session(id, |s| {
            let m = s.reproject();
            let e = m
                .emitters
                .iter()
                .find(|e| e.key == key)
                .expect("emitter in model");
            e.colors
                .color
                .as_ref()
                .map(|c| c.keyframes.iter().map(|k| k.rgba[3]).collect())
                .unwrap_or_default()
        })
        .unwrap()
    }

    fn session_bytes(id: SessionId) -> Vec<u8> {
        with_session(id, |s| {
            let mut out = Vec::new();
            for lb in &s.bins {
                out.extend(write_bin(&lb.tree).unwrap());
            }
            out
        })
        .unwrap()
    }

    /* ── alpha ─────────────────────────────────────────────────────────────── */

    /// Setting alpha must reach every keyframe, for EVERY colour shape - the
    /// bug being guarded here is a writer that only understands one of them and
    /// silently no-ops on the rest.
    #[test]
    fn set_alpha_writes_every_color_shape() {
        let cases: Vec<(&str, ColorShape, usize)> = vec![
            ("bare_vec4", ColorShape::BareVec4([1.0, 0.0, 0.0, 1.0]), 1),
            ("constant", ColorShape::Constant([1.0, 0.0, 0.0, 1.0]), 1),
            (
                "values",
                ColorShape::Values(vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]]),
                2,
            ),
            (
                "dynamics",
                ColorShape::Dynamics(vec![
                    [1.0, 0.0, 0.0, 1.0],
                    [0.0, 1.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0, 1.0],
                ]),
                3,
            ),
            (
                "constant_plus_dynamics",
                ColorShape::ConstantPlusDynamics(
                    [1.0, 1.0, 1.0, 1.0],
                    vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
                ),
                3,
            ),
        ];

        for (name, shape, expected_kfs) in cases {
            let path = write_temp(&bin_with_color(shape), name);
            let id = open(&path).unwrap().session_id;
            let key = only_emitter_key(id);

            let before = alphas_of(id, &key);
            assert_eq!(
                before.len(),
                expected_kfs,
                "{name}: editor should list {expected_kfs} keyframe(s)"
            );

            // Drive every keyframe to a distinct value so a mis-zip is visible.
            let wanted: Vec<f32> = (0..before.len()).map(|i| 0.1 + 0.15 * i as f32).collect();
            let out = set_color_alpha(id, &key, "color", &wanted).unwrap();
            assert!(out.is_some(), "{name}: alpha edit reported no change");

            let after = alphas_of(id, &key);
            for (i, (got, want)) in after.iter().zip(wanted.iter()).enumerate() {
                assert!(
                    (got - want).abs() < 1e-6,
                    "{name}: keyframe {i} alphaは {got}, expected {want}"
                );
            }
            close(id);
            let _ = std::fs::remove_file(&path);
        }
    }

    /// The edit must survive save + reopen. This is the "it didn't save" report:
    /// an in-memory change that never reaches the file looks fine until reload.
    #[test]
    fn set_alpha_survives_save_and_reopen() {
        let path = write_temp(
            &bin_with_color(ColorShape::Dynamics(vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
            ])),
            "alpha_roundtrip",
        );

        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        set_color_alpha(id, &key, "color", &[0.25, 0.75]).unwrap();
        let written = save(id, None, true).unwrap();
        assert_eq!(written.len(), 1, "the dirty main bin should be written");
        close(id);

        let id2 = open(&path).unwrap().session_id;
        let key2 = only_emitter_key(id2);
        let reloaded = alphas_of(id2, &key2);
        assert_eq!(reloaded.len(), 2);
        assert!((reloaded[0] - 0.25).abs() < 1e-6, "got {reloaded:?}");
        assert!((reloaded[1] - 0.75).abs() < 1e-6, "got {reloaded:?}");
        close(id2);
        let _ = std::fs::remove_file(&path);
    }

    /// Alpha edits must not disturb RGB.
    #[test]
    fn set_alpha_preserves_rgb() {
        let rgb = [0.2, 0.4, 0.6, 1.0];
        let path = write_temp(&bin_with_color(ColorShape::Constant(rgb)), "alpha_rgb");
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        set_color_alpha(id, &key, "color", &[0.33]).unwrap();

        let got = with_session(id, |s| {
            let m = s.reproject();
            m.emitters
                .iter()
                .find(|e| e.key == key)
                .and_then(|e| e.colors.color.as_ref())
                .map(|c| c.keyframes[0].rgba)
                .unwrap()
        })
        .unwrap();
        assert!((got[0] - rgb[0]).abs() < 1e-6, "r changed: {got:?}");
        assert!((got[1] - rgb[1]).abs() < 1e-6, "g changed: {got:?}");
        assert!((got[2] - rgb[2]).abs() < 1e-6, "b changed: {got:?}");
        assert!((got[3] - 0.33).abs() < 1e-6, "a not applied: {got:?}");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// Out-of-range input clamps instead of writing a nonsense alpha.
    #[test]
    fn set_alpha_clamps_out_of_range() {
        let path = write_temp(
            &bin_with_color(ColorShape::Values(vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
            ])),
            "alpha_clamp",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        set_color_alpha(id, &key, "color", &[-5.0, 9.0]).unwrap();

        let after = alphas_of(id, &key);
        assert!((after[0] - 0.0).abs() < 1e-6, "got {after:?}");
        assert!((after[1] - 1.0).abs() < 1e-6, "got {after:?}");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// A no-op edit must report "nothing changed" rather than dirtying the bin
    /// and pushing an empty undo step.
    #[test]
    fn set_alpha_same_value_is_a_noop() {
        let path = write_temp(
            &bin_with_color(ColorShape::Constant([1.0, 0.0, 0.0, 0.5])),
            "alpha_noop",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        let before = session_bytes(id);
        let out = set_color_alpha(id, &key, "color", &[0.5]).unwrap();
        assert!(
            out.is_none(),
            "re-applying the same alpha reported a change"
        );
        assert_eq!(before, session_bytes(id), "no-op still mutated the bytes");
        assert!(undo(id).unwrap().is_none(), "no-op pushed an undo frame");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// Fewer alphas than keyframes edits only the leading ones (zip semantics),
    /// and extra alphas are ignored rather than panicking.
    #[test]
    fn set_alpha_handles_length_mismatch() {
        let path = write_temp(
            &bin_with_color(ColorShape::Values(vec![
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ])),
            "alpha_mismatch",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        // Short input: only the first keyframe moves.
        set_color_alpha(id, &key, "color", &[0.2]).unwrap();
        let a = alphas_of(id, &key);
        assert!((a[0] - 0.2).abs() < 1e-6, "got {a:?}");
        assert!(
            (a[1] - 1.0).abs() < 1e-6,
            "trailing keyframe changed: {a:?}"
        );

        // Long input: extras are dropped, no panic.
        set_color_alpha(id, &key, "color", &[0.9, 0.9, 0.9, 0.9, 0.9]).unwrap();
        let b = alphas_of(id, &key);
        assert_eq!(b.len(), 3);
        assert!(b.iter().all(|v| (v - 0.9).abs() < 1e-6), "got {b:?}");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// An unknown slot name must be rejected, not silently applied elsewhere.
    #[test]
    fn set_alpha_unknown_slot_is_rejected() {
        let path = write_temp(
            &bin_with_color(ColorShape::Constant([1.0, 0.0, 0.0, 1.0])),
            "alpha_slot",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        assert!(set_color_alpha(id, &key, "notAColor", &[0.5])
            .unwrap()
            .is_none());
        assert!(set_color_alpha(id, "no-such-emitter", "color", &[0.5])
            .unwrap()
            .is_none());
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /* ── undo / redo ───────────────────────────────────────────────────────── */

    /// Alpha edits must replay byte-exact both directions.
    #[test]
    fn alpha_undo_redo_is_byte_exact() {
        let path = write_temp(
            &bin_with_color(ColorShape::ConstantPlusDynamics(
                [1.0, 1.0, 1.0, 1.0],
                vec![[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]],
            )),
            "alpha_undo",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        let s0 = session_bytes(id);
        set_color_alpha(id, &key, "color", &[0.1, 0.2, 0.3]).unwrap();
        let s1 = session_bytes(id);
        assert_ne!(s0, s1, "edit did not change the bytes");

        set_color_alpha(id, &key, "color", &[0.6, 0.7, 0.8]).unwrap();
        let s2 = session_bytes(id);
        assert_ne!(s1, s2);

        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), s1, "undo #1 did not restore state 1");
        undo(id).unwrap().unwrap();
        assert_eq!(
            session_bytes(id),
            s0,
            "undo #2 did not restore the original"
        );
        redo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), s1, "redo #1 mismatch");
        redo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), s2, "redo #2 mismatch");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /* ── recolor ───────────────────────────────────────────────────────────── */

    fn recolor_opts(preserve_alpha: bool) -> RecolorOptions {
        RecolorOptions {
            mode: RecolorMode::Linear,
            ignore_black_white: false,
            preserve_alpha,
            hsl_shift: (0.0, 0.0, 0.0),
            hue_target: None,
            seed: 7,
        }
    }

    fn two_stop_palette() -> Vec<PaletteStop> {
        vec![
            PaletteStop {
                vec4: [1.0, 0.0, 0.0, 1.0],
                time: 0.0,
            },
            PaletteStop {
                vec4: [0.0, 0.0, 1.0, 1.0],
                time: 1.0,
            },
        ]
    }

    /// Recolor must reach the bytes and survive save + reopen.
    #[test]
    fn recolor_survives_save_and_reopen() {
        let path = write_temp(
            &bin_with_color(ColorShape::Dynamics(vec![
                [1.0, 1.0, 1.0, 1.0],
                [0.5, 0.5, 0.5, 1.0],
            ])),
            "recolor_roundtrip",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        let before = with_session(id, |s| {
            let m = s.reproject();
            m.emitters
                .iter()
                .find(|e| e.key == key)
                .and_then(|e| e.colors.color.as_ref())
                .map(|c| c.keyframes.iter().map(|k| k.rgba).collect::<Vec<_>>())
                .unwrap()
        })
        .unwrap();

        let n = recolor_emitters(
            id,
            &[key.clone()],
            &[ColorTargetSel::All],
            &two_stop_palette(),
            &recolor_opts(true),
        )
        .unwrap();
        assert!(n > 0, "recolor changed nothing");
        save(id, None, true).unwrap();
        close(id);

        let id2 = open(&path).unwrap().session_id;
        let key2 = only_emitter_key(id2);
        let after = with_session(id2, |s| {
            let m = s.reproject();
            m.emitters
                .iter()
                .find(|e| e.key == key2)
                .and_then(|e| e.colors.color.as_ref())
                .map(|c| c.keyframes.iter().map(|k| k.rgba).collect::<Vec<_>>())
                .unwrap()
        })
        .unwrap();
        assert_ne!(before, after, "recolor did not persist to disk");
        close(id2);
        let _ = std::fs::remove_file(&path);
    }

    /// `preserve_alpha` must leave a non-1.0 alpha untouched while RGB changes.
    #[test]
    fn recolor_preserve_alpha_keeps_alpha() {
        let path = write_temp(
            &bin_with_color(ColorShape::Values(vec![
                [1.0, 1.0, 1.0, 0.25],
                [0.5, 0.5, 0.5, 0.75],
            ])),
            "recolor_alpha",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        recolor_emitters(
            id,
            &[key.clone()],
            &[ColorTargetSel::All],
            &two_stop_palette(),
            &recolor_opts(true),
        )
        .unwrap();

        let a = alphas_of(id, &key);
        assert!((a[0] - 0.25).abs() < 1e-6, "alpha 0 not preserved: {a:?}");
        assert!((a[1] - 0.75).abs() < 1e-6, "alpha 1 not preserved: {a:?}");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// Recolor undo/redo replays byte-exact.
    #[test]
    fn recolor_undo_redo_is_byte_exact_synthetic() {
        let path = write_temp(
            &bin_with_color(ColorShape::Dynamics(vec![
                [1.0, 1.0, 1.0, 1.0],
                [0.2, 0.4, 0.6, 1.0],
            ])),
            "recolor_undo",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        let s0 = session_bytes(id);
        recolor_emitters(
            id,
            &[key.clone()],
            &[ColorTargetSel::All],
            &two_stop_palette(),
            &recolor_opts(true),
        )
        .unwrap();
        let s1 = session_bytes(id);
        assert_ne!(s0, s1);

        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), s0, "recolor undo was not byte-exact");
        redo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), s1, "recolor redo was not byte-exact");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// An alpha edit followed by a recolor must both persist - interleaving two
    /// different edit kinds is where entry-granular undo frames can collide.
    #[test]
    fn alpha_then_recolor_both_persist() {
        let path = write_temp(
            &bin_with_color(ColorShape::Dynamics(vec![
                [1.0, 1.0, 1.0, 1.0],
                [0.2, 0.4, 0.6, 1.0],
            ])),
            "alpha_then_recolor",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);

        set_color_alpha(id, &key, "color", &[0.3, 0.4]).unwrap();
        let after_alpha = session_bytes(id);

        recolor_emitters(
            id,
            &[key.clone()],
            &[ColorTargetSel::All],
            &two_stop_palette(),
            &recolor_opts(true),
        )
        .unwrap();

        // Alpha must have survived the recolor (preserve_alpha = true).
        let a = alphas_of(id, &key);
        assert!((a[0] - 0.3).abs() < 1e-6, "recolor clobbered alpha: {a:?}");
        assert!((a[1] - 0.4).abs() < 1e-6, "recolor clobbered alpha: {a:?}");

        undo(id).unwrap().unwrap();
        assert_eq!(
            session_bytes(id),
            after_alpha,
            "undo of recolor did not return to the post-alpha state"
        );
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// Saving with nothing dirty must not rewrite files.
    #[test]
    fn save_with_no_changes_writes_nothing() {
        let path = write_temp(
            &bin_with_color(ColorShape::Constant([1.0, 0.0, 0.0, 1.0])),
            "save_clean",
        );
        let id = open(&path).unwrap().session_id;
        let written = save(id, None, true).unwrap();
        assert!(written.is_empty(), "clean session wrote files: {written:?}");
        close(id);
        let _ = std::fs::remove_file(&path);
    }

    /// Undo must clear the dirty flag's effect: after undoing back to the
    /// original state the saved file must match the original bytes.
    #[test]
    fn undo_then_save_restores_original_bytes() {
        let bin = bin_with_color(ColorShape::Values(vec![
            [1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
        ]));
        let original = write_bin(&bin).unwrap();
        let path = write_temp(&bin, "undo_save");

        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        set_color_alpha(id, &key, "color", &[0.1, 0.2]).unwrap();
        undo(id).unwrap().unwrap();
        save(id, None, true).unwrap();
        close(id);

        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(
            on_disk, original,
            "undo + save did not restore the original file bytes"
        );
        let _ = std::fs::remove_file(&path);
    }

    fn projected_color(id: SessionId) -> model::ColorData {
        model_of(id).unwrap().emitters[0]
            .colors
            .color
            .clone()
            .unwrap()
    }

    fn test_emitter_fields(bin: &mut Bin) -> &mut IndexMap<u32, BinValue> {
        let BinValue::List { items, .. } = bin.entries[0]
            .fields
            .get_mut(&fh("complexEmitterDefinitionData"))
            .unwrap()
        else {
            panic!("emitter list")
        };
        color_fields(&mut items[0]).unwrap()
    }

    #[test]
    fn keyframe_indices_include_wrapper_constant_and_roundtrip() {
        let wrapper = [0.2, 0.3, 0.4, 0.5];
        let first = [1.0, 0.0, 0.0, 1.0];
        let last = [0.0, 0.0, 1.0, 0.5];
        let path = write_temp(
            &bin_with_color(ColorShape::ConstantPlusDynamics(wrapper, vec![first, last])),
            "keyframe_indices",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        let original = session_bytes(id);
        let metadata = projected_color(id);
        assert_eq!(metadata.storage, model::ColorStorage::Curve);
        assert_eq!(metadata.constant_index, Some(0));
        assert!(metadata.supports_retime && metadata.supports_structural_edits);

        let new_wrapper = [0.8, 0.9, 0.7, 0.6];
        set_keyframe(id, &key, "color", 0, new_wrapper, 0.8)
            .unwrap()
            .unwrap();
        let view = projected_color(id);
        assert_eq!(view.keyframes[0].rgba, new_wrapper);
        assert_eq!(view.keyframes[0].time, 0.0);
        assert_eq!(view.keyframes[1].rgba, first);
        let replacement = [0.3, 0.5, 0.7, 0.9];
        set_keyframe(id, &key, "color", 2, replacement, 0.25)
            .unwrap()
            .unwrap();
        let view = projected_color(id);
        assert_eq!(view.keyframes[1].rgba, first);
        assert_eq!(view.keyframes[2].rgba, replacement);
        assert_eq!(view.keyframes[2].time, 0.25);
        let before_delete = session_bytes(id);
        assert!(delete_keyframe(id, &key, "color", 0).unwrap().is_none());
        assert!(delete_keyframe(id, &key, "color", 99).unwrap().is_none());
        assert!(set_keyframe(id, &key, "color", 99, [0.0; 4], 0.0)
            .unwrap()
            .is_none());
        assert!(set_keyframe(id, &key, "color", 2, replacement, 0.25)
            .unwrap()
            .is_none());
        assert_eq!(session_bytes(id), before_delete);
        delete_keyframe(id, &key, "color", 1).unwrap().unwrap();
        let view = projected_color(id);
        assert_eq!(view.keyframes.len(), 2);
        assert_eq!(view.keyframes[1].rgba, replacement);
        assert_eq!(view.keyframes[1].time, 0.25);
        assert!(delete_keyframe(id, &key, "color", 1).unwrap().is_none());
        let edited = session_bytes(id);
        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), before_delete);
        undo(id).unwrap().unwrap();
        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), original);
        assert!(
            undo(id).unwrap().is_none(),
            "invalid/no-op edits must not add undo frames"
        );
        for _ in 0..3 {
            redo(id).unwrap().unwrap();
        }
        assert_eq!(session_bytes(id), edited);
        save(id, None, true).unwrap();
        close(id);
        let reopened = open(&path).unwrap().session_id;
        assert_eq!(session_bytes(reopened), edited);
        assert_eq!(projected_color(reopened).keyframes[1].time, 0.25);
        close(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn auto_animation_and_add_are_one_undo_step() {
        let path = write_temp(
            &bin_with_color(ColorShape::Constant([0.2, 0.3, 0.4, 0.5])),
            "add_atomic",
        );
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        let original = session_bytes(id);
        add_keyframe(id, &key, "color", [0.9, 0.8, 0.7, 0.6], 0.37)
            .unwrap()
            .unwrap();
        let view = projected_color(id);
        assert_eq!(view.keyframes.len(), 3);
        assert_eq!(view.keyframes[2].time, 0.37);
        let edited = session_bytes(id);
        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), original);
        assert!(undo(id).unwrap().is_none());
        redo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), edited);
        assert!(add_keyframe(id, &key, "color", [f32::NAN; 4], 0.0).is_err());
        assert!(set_keyframe(id, &key, "color", 1, [0.0; 4], f32::INFINITY).is_err());
        assert_eq!(session_bytes(id), edited);
        close(id);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn missing_times_keep_displayed_positions_and_use_supplied_new_time() {
        let mut bin = bin_with_color(ColorShape::Values(vec![[1.0; 4], [0.5; 4]]));
        let color = test_emitter_fields(&mut bin).get_mut(&fh("color")).unwrap();
        color_curve_fields(color)
            .unwrap()
            .shift_remove(&fh("times"));
        let path = write_temp(&bin, "missing_curve_times");
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        add_keyframe(id, &key, "color", [0.25; 4], 0.3)
            .unwrap()
            .unwrap();
        let times: Vec<_> = projected_color(id)
            .keyframes
            .iter()
            .map(|k| k.time)
            .collect();
        assert_eq!(times, vec![0.0, 1.0, 0.3]);
        set_keyframe(id, &key, "color", 0, [1.0; 4], 0.8)
            .unwrap()
            .unwrap();
        let times: Vec<_> = projected_color(id)
            .keyframes
            .iter()
            .map(|k| k.time)
            .collect();
        assert_eq!(
            times,
            vec![0.8, 1.0, 0.3],
            "retiming must never sort the stored list"
        );
        close(id);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn animate_deanimate_preserve_wrapper_metadata_and_noops() {
        let mut bin = bin_with_color(ColorShape::Constant([0.2, 0.4, 0.6, 0.8]));
        color_fields(test_emitter_fields(&mut bin).get_mut(&fh("color")).unwrap())
            .unwrap()
            .insert(fh("otherMetadata"), BinValue::U32(123));
        let path = write_temp(&bin, "animate_wrapper");
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        let original = session_bytes(id);
        assert!(deanimate_color(id, &key, "color").unwrap().is_none());
        animate_color(id, &key, "color").unwrap().unwrap();
        assert_eq!(projected_color(id).keyframes.len(), 3);
        assert!(animate_color(id, &key, "color").unwrap().is_none());
        with_session(id, |s| {
            let (color, _) = resolve_color_field(s, &key, model::ColorSlot::Color).unwrap();
            assert_eq!(
                color_fields(color).unwrap().get(&fh("otherMetadata")),
                Some(&BinValue::U32(123))
            );
        })
        .unwrap();
        deanimate_color(id, &key, "color").unwrap().unwrap();
        assert_eq!(session_bytes(id), original);
        undo(id).unwrap().unwrap();
        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), original);
        assert!(undo(id).unwrap().is_none());
        close(id);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn probability_table_edits_scatter_and_structural_commands_leave_bytes_intact() {
        let mut bin = bin_with_color(ColorShape::Dynamics(vec![[1.0; 4]]));
        let tables = (0..4)
            .map(|channel| {
                let mut fields = IndexMap::new();
                let values = if channel == 1 {
                    vec![0.3]
                } else {
                    vec![0.2, 0.7]
                };
                fields.insert(fh("keyValues"), build_f32_list(&values));
                fields.insert(fh("keyTimes"), build_f32_list(&[0.1, 0.9]));
                BinValue::Pointer {
                    class: fh("VfxProbabilityTableData"),
                    fields,
                }
            })
            .collect();
        let color = test_emitter_fields(&mut bin).get_mut(&fh("color")).unwrap();
        color_curve_fields(color).unwrap().insert(
            fh("probabilityTables"),
            BinValue::List {
                is_list2: false,
                item: BinType::Pointer,
                items: tables,
            },
        );
        let path = write_temp(&bin, "probability_edit");
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        let original = session_bytes(id);
        let view = projected_color(id);
        assert_eq!(view.storage, model::ColorStorage::ProbabilityTables);
        assert!(!view.supports_retime && !view.supports_structural_edits);
        assert_eq!(view.constant_index, None);
        assert!(animate_color(id, &key, "color").unwrap().is_none());
        assert!(deanimate_color(id, &key, "color").unwrap().is_none());
        assert!(add_keyframe(id, &key, "color", [0.5; 4], 0.5)
            .unwrap()
            .is_none());
        assert!(delete_keyframe(id, &key, "color", 0).unwrap().is_none());
        assert_eq!(session_bytes(id), original);
        assert!(undo(id).unwrap().is_none());
        set_keyframe(id, &key, "color", 1, [0.8, 0.6, 0.4, 0.5], 0.25)
            .unwrap()
            .unwrap();
        let view = projected_color(id);
        assert_eq!(
            view.keyframes[1].rgba,
            [0.8, 1.0, 0.4, 0.5],
            "unauthored channels must stay absent"
        );
        assert_eq!(
            view.keyframes[1].time, 0.9,
            "probability times are distribution coordinates"
        );
        set_color_alpha(id, &key, "color", &[0.15, 0.35])
            .unwrap()
            .unwrap();
        assert_eq!(alphas_of(id, &key), vec![0.15, 0.35]);
        with_session(id, |s| {
            let (color, _) = resolve_color_field(s, &key, model::ColorSlot::Color).unwrap();
            let curve = color_curve_fields(color).unwrap();
            assert_eq!(
                curve.get(&fh("values")),
                Some(&vec4_list(&[[1.0; 4]])),
                "leave placeholder curve unchanged"
            );
        })
        .unwrap();
        let edited = session_bytes(id);
        undo(id).unwrap().unwrap();
        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), original);
        redo(id).unwrap().unwrap();
        redo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), edited);
        recolor_emitters(
            id,
            &[key.clone()],
            &[ColorTargetSel::All],
            &two_stop_palette(),
            &recolor_opts(true),
        )
        .unwrap();
        let partial = emitter_colors_of(id, &[key.clone()])
            .unwrap()
            .remove(&key)
            .unwrap()
            .color
            .unwrap();
        assert_eq!(
            serde_json::to_value(&partial).unwrap(),
            serde_json::to_value(projected_color(id)).unwrap()
        );
        let edited = session_bytes(id);
        save(id, None, true).unwrap();
        close(id);
        let reopened = open(&path).unwrap().session_id;
        assert_eq!(session_bytes(reopened), edited);
        close(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn create_uses_real_slot_shapes_and_missing_blend_mode_is_editable() {
        let mut bin = bin_with_color(ColorShape::Constant([1.0; 4]));
        test_emitter_fields(&mut bin).shift_remove(&fh("blendMode"));
        // Empty ValueColor and null reflection pointers both project as absent.
        test_emitter_fields(&mut bin).insert(
            fh("birthColor"),
            BinValue::Embed {
                class: fh("ValueColor"),
                fields: IndexMap::new(),
            },
        );
        test_emitter_fields(&mut bin).insert(
            fh("reflectionDefinition"),
            BinValue::Pointer {
                class: 0,
                fields: IndexMap::new(),
            },
        );
        let path = write_temp(&bin, "create_slots");
        let id = open(&path).unwrap().session_id;
        let key = only_emitter_key(id);
        let original = session_bytes(id);
        assert!(!set_blend_mode(id, &key, 0).unwrap());
        assert!(set_blend_mode(id, &key, 5).unwrap());
        assert_eq!(model_of(id).unwrap().emitters[0].blend_mode, 5);
        undo(id).unwrap().unwrap();
        assert_eq!(session_bytes(id), original);
        assert_eq!(
            set_blend_mode_bulk(id, &[key.clone(), key.clone()], 2).unwrap(),
            1
        );
        assert!(!set_blend_mode(id, &key, 2).unwrap());
        for slot in ["birthColor", "fresnelColor", "lingerColor"] {
            create_color(id, &key, slot).unwrap().unwrap();
            assert!(create_color(id, &key, slot).unwrap().is_none());
        }
        let view = model_of(id).unwrap();
        let fresnel = view.emitters[0].colors.fresnel_color.as_ref().unwrap();
        assert!(!fresnel.supports_structural_edits);
        assert!(animate_color(id, &key, "fresnelColor").unwrap().is_none());
        assert!(add_keyframe(id, &key, "fresnelColor", [0.0; 4], 0.5)
            .unwrap()
            .is_none());
        set_keyframe(id, &key, "fresnelColor", 0, [0.2, 0.4, 0.6, 0.0], 0.0)
            .unwrap()
            .unwrap();
        with_session(id, |s| {
            let node = s.index.emitter_nodes.get(&key).unwrap().clone();
            let fields = emitter_fields(&mut s.bins, &node).unwrap();
            assert_eq!(
                fields.get(&fh("UseSeparateLingerColor")),
                Some(&BinValue::Flag(true))
            );
            assert!(fields.contains_key(&fh("SeparateLingerColor")));
            assert!(!fields.contains_key(&fh("fresnelColor")));
            let reflection =
                color_fields(fields.get_mut(&fh("reflectionDefinition")).unwrap()).unwrap();
            assert_eq!(
                reflection.get(&fh("fresnelColor")),
                Some(&BinValue::Vec4([0.2, 0.4, 0.6, 0.0]))
            );
        })
        .unwrap();
        let edited = session_bytes(id);
        save(id, None, true).unwrap();
        close(id);
        let reopened = open(&path).unwrap().session_id;
        assert_eq!(session_bytes(reopened), edited);
        close(reopened);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn empty_probability_shells_keep_curve_editing_and_survive_add() {
        for values in [vec![], vec![[0.3; 4]]] {
            let mut bin =
                bin_with_color(ColorShape::ConstantPlusDynamics([1.0; 4], values.clone()));
            let color = test_emitter_fields(&mut bin).get_mut(&fh("color")).unwrap();
            let curve = color_curve_fields(color).unwrap();
            let shells = BinValue::List {
                is_list2: false,
                item: BinType::Pointer,
                items: (0..4)
                    .map(|_| BinValue::Pointer {
                        class: fh("VfxProbabilityTableData"),
                        fields: IndexMap::new(),
                    })
                    .collect(),
            };
            curve.insert(fh("probabilityTables"), shells.clone());
            curve.insert(fh("otherMetadata"), BinValue::U32(42));
            let path = write_temp(&bin, &format!("empty_probability_shells_{}", values.len()));
            let id = open(&path).unwrap().session_id;
            let key = only_emitter_key(id);
            let view = projected_color(id);
            assert_ne!(view.storage, model::ColorStorage::ProbabilityTables);
            assert!(view.supports_structural_edits);
            add_keyframe(id, &key, "color", [0.6; 4], 0.7)
                .unwrap()
                .unwrap();
            with_session(id, |s| {
                let (color, _) = resolve_color_field(s, &key, model::ColorSlot::Color).unwrap();
                let curve = color_curve_fields(color).unwrap();
                assert_eq!(curve.get(&fh("probabilityTables")), Some(&shells));
                assert_eq!(curve.get(&fh("otherMetadata")), Some(&BinValue::U32(42)));
            })
            .unwrap();
            close(id);
            let _ = std::fs::remove_file(path);
        }
    }

    /// Opt-in real-asset smoke. Always edits a temporary copy, including when
    /// QUARTZ_TEST_BIN refers to a user's working skin.
    #[test]
    #[ignore = "set QUARTZ_TEST_BIN to an Irelia Skin55 BIN and run explicitly"]
    fn real_color_editor_smoke() {
        let source = PathBuf::from(std::env::var_os("QUARTZ_TEST_BIN").expect("QUARTZ_TEST_BIN"));
        let original_source = std::fs::read(&source).unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("skin.bin");
        std::fs::write(&path, &original_source).unwrap();
        let id = open(&path).unwrap().session_id;
        let main_keys = with_session(id, |s| {
            s.index
                .emitter_nodes
                .iter()
                .filter(|(_, node)| node.bin == 0)
                .map(|(key, _)| key.clone())
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap();
        let model = model_of(id).unwrap();
        let meshes = model
            .emitters
            .iter()
            .filter(|e| e.textures.iter().any(|t| t.label == "Mesh"))
            .count();
        assert!(meshes > 0, "fixture must contain mesh emitters");
        let mut probability_emitters: Vec<_> = model
            .emitters
            .iter()
            .filter(|e| main_keys.contains(&e.key))
            .filter(|e| {
                e.colors
                    .birth_color
                    .as_ref()
                    .is_some_and(|c| c.storage == model::ColorStorage::ProbabilityTables)
            })
            .collect();
        probability_emitters.sort_by_key(|emitter| {
            let name = emitter.name.to_ascii_lowercase();
            !(name.contains("vortex") || name.contains("petal"))
        });
        assert!(
            !probability_emitters.is_empty(),
            "fixture must contain birthColor probability ranges"
        );
        println!(
            "Real fixture: {} emitters, {} mesh emitters, {} probability birthColors",
            model.emitters.len(),
            meshes,
            probability_emitters.len()
        );
        for emitter in probability_emitters.iter().take(3) {
            let color = emitter.colors.birth_color.as_ref().unwrap();
            let (table_index, channel) = with_session(id, |s| {
                let target = color_target(s, &emitter.key, model::ColorSlot::BirthColor).unwrap();
                target
                    .channel_tables
                    .iter()
                    .enumerate()
                    .find_map(|(key, channels)| {
                        channels
                            .iter()
                            .position(Option::is_some)
                            .map(|channel| (key, channel))
                    })
                    .unwrap()
            })
            .unwrap();
            let key_index = table_index + usize::from(color.constant_index.is_some());
            let key = &color.keyframes[key_index];
            let mut rgba = key.rgba;
            rgba[channel] = if rgba[channel] < 0.5 { 0.8 } else { 0.2 };
            let original = session_bytes(id);
            assert!(add_keyframe(id, &emitter.key, "birthColor", rgba, 0.5)
                .unwrap()
                .is_none());
            assert!(delete_keyframe(id, &emitter.key, "birthColor", key_index)
                .unwrap()
                .is_none());
            assert!(deanimate_color(id, &emitter.key, "birthColor")
                .unwrap()
                .is_none());
            assert_eq!(session_bytes(id), original);
            let edited = set_keyframe(id, &emitter.key, "birthColor", key_index, rgba, key.time)
                .unwrap()
                .unwrap();
            let edited = edited
                .emitters
                .iter()
                .find(|e| e.key == emitter.key)
                .unwrap()
                .colors
                .birth_color
                .as_ref()
                .unwrap();
            assert_eq!(edited.keyframes[key_index].rgba[channel], rgba[channel]);
            assert_eq!(edited.keyframes[key_index].time, key.time);
            undo(id).unwrap().unwrap();
            assert_eq!(session_bytes(id), original);
            redo(id).unwrap().unwrap();
            println!("Probability recolor + undo/redo: {}", emitter.name);
        }
        let emitter = model
            .emitters
            .iter()
            .find(|e| {
                main_keys.contains(&e.key)
                    && e.colors.color.as_ref().is_some_and(|c| {
                        c.storage == model::ColorStorage::Curve && c.supports_retime
                    })
            })
            .unwrap();
        let color = emitter.colors.color.as_ref().unwrap();
        let index = usize::from(color.constant_index.is_some());
        set_keyframe(
            id,
            &emitter.key,
            "color",
            index,
            color.keyframes[index].rgba,
            0.371,
        )
        .unwrap()
        .unwrap();
        let expected = session_bytes(id);
        let expected_model = serde_json::to_value(model_of(id).unwrap()).unwrap();
        assert_eq!(save(id, None, true).unwrap(), vec![path.clone()]);
        close(id);
        let reopened = open(&path).unwrap().session_id;
        assert_eq!(session_bytes(reopened), expected);
        assert_eq!(
            serde_json::to_value(model_of(reopened).unwrap()).unwrap(),
            expected_model
        );
        close(reopened);
        assert_eq!(
            std::fs::read(source).unwrap(),
            original_source,
            "source fixture must remain untouched"
        );
        println!(
            "Curve retime + complete model save/reopen: {}",
            emitter.name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bin::write_bin;
    use crate::paint::recolor::RecolorMode;
    use std::time::Instant;

    fn real_bin_path() -> Option<PathBuf> {
        let p = std::env::var("QUARTZ_TEST_BIN")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(r"D:\updated skins\Frozen Locke\data\locke_vfx_skin0.bin")
            });
        p.is_file().then_some(p)
    }

    /// Byte fingerprint of the whole session (all resident bins concatenated),
    /// so undo/redo replay is verified across linked bins too.
    fn bytes_of(id: SessionId) -> Vec<u8> {
        with_session(id, |s| {
            let mut out = Vec::new();
            for lb in &s.bins {
                out.extend(write_bin(&lb.tree).unwrap());
            }
            out
        })
        .unwrap()
    }

    fn opts(seed: u64) -> RecolorOptions {
        RecolorOptions {
            mode: RecolorMode::Linear,
            ignore_black_white: false,
            preserve_alpha: true,
            hsl_shift: (0.0, 0.0, 0.0),
            hue_target: None,
            seed,
        }
    }

    fn palette(a: [f32; 4], b: [f32; 4]) -> Vec<PaletteStop> {
        vec![
            PaletteStop { vec4: a, time: 0.0 },
            PaletteStop { vec4: b, time: 1.0 },
        ]
    }

    fn some_emitter_keys(id: SessionId, n: usize) -> Vec<String> {
        with_session(id, |s| {
            let mut keys: Vec<String> = s.index.emitter_colors.keys().cloned().collect();
            keys.sort();
            keys.truncate(n);
            keys
        })
        .unwrap()
    }

    /// Recolor twice with different palettes, then undo/redo must replay every
    /// state byte-exact — the corruption check for entry-granular undo.
    #[test]
    fn recolor_undo_redo_replays_byte_exact() {
        let Some(path) = real_bin_path() else {
            eprintln!("skipping: real test bin not found (set QUARTZ_TEST_BIN)");
            return;
        };
        let opened = open(&path).unwrap();
        let id = opened.session_id;
        let keys = some_emitter_keys(id, 6);
        if keys.is_empty() {
            eprintln!("skipping: no emitters with color targets");
            close(id);
            return;
        }

        let s0 = bytes_of(id);
        let n1 = recolor_emitters(
            id,
            &keys,
            &[ColorTargetSel::All],
            &palette([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]),
            &opts(1),
        )
        .unwrap();
        assert!(n1 > 0, "first recolor changed nothing");
        let s1 = bytes_of(id);
        assert_ne!(s0, s1);

        let n2 = recolor_emitters(
            id,
            &keys,
            &[ColorTargetSel::All],
            &palette([0.0, 1.0, 0.2, 1.0], [0.6, 0.0, 0.8, 1.0]),
            &opts(2),
        )
        .unwrap();
        assert!(n2 > 0, "second recolor changed nothing");
        let s2 = bytes_of(id);
        assert_ne!(s1, s2);

        undo(id).unwrap().expect("undo #1");
        assert_eq!(bytes_of(id), s1, "undo #1 diverged");
        undo(id).unwrap().expect("undo #2");
        assert_eq!(bytes_of(id), s0, "undo #2 diverged from pristine");
        assert!(undo(id).unwrap().is_none());

        redo(id).unwrap().expect("redo #1");
        assert_eq!(bytes_of(id), s1, "redo #1 diverged");
        redo(id).unwrap().expect("redo #2");
        assert_eq!(bytes_of(id), s2, "redo #2 diverged");
        assert!(redo(id).unwrap().is_none());
        close(id);
    }

    /// Perf report: old per-click cost (whole-tree clone) vs entry-granular
    /// recolor. Run with:
    /// `cargo test -p quartz-lib --release bench_recolor -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_recolor_vs_whole_tree_clone() {
        let Some(path) = real_bin_path() else {
            eprintln!("skipping: real test bin not found (set QUARTZ_TEST_BIN)");
            return;
        };
        let opened = open(&path).unwrap();
        let id = opened.session_id;
        let keys = some_emitter_keys(id, 6);
        if keys.is_empty() {
            eprintln!("skipping: no emitters with color targets");
            close(id);
            return;
        }

        let (n_entries, clone_avg) = with_session(id, |s| {
            let iters = 10u32;
            let t0 = Instant::now();
            for _ in 0..iters {
                std::hint::black_box(s.bins[0].tree.clone());
            }
            (s.bins[0].tree.entries.len(), t0.elapsed() / iters)
        })
        .unwrap();

        let pals = [
            palette([1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0]),
            palette([0.0, 1.0, 0.2, 1.0], [0.6, 0.0, 0.8, 1.0]),
        ];
        let iters = 20u32;
        let t0 = Instant::now();
        for i in 0..iters {
            let n = recolor_emitters(
                id,
                &keys,
                &[ColorTargetSel::All],
                &pals[(i % 2) as usize],
                &opts(i as u64),
            )
            .unwrap();
            assert!(n > 0);
        }
        let recolor_avg = t0.elapsed() / iters;

        let t0 = Instant::now();
        let pairs = 10u32;
        for _ in 0..pairs {
            undo(id).unwrap().unwrap();
            redo(id).unwrap().unwrap();
        }
        let pair_avg = t0.elapsed() / pairs;

        println!(
            "paint bench: {} entries, {} emitters selected | whole-tree clone (old per-click cost) {:?} | \
             recolor click {:?} | undo+redo pair (incl. reprojection) {:?}",
            n_entries,
            keys.len(),
            clone_avg,
            recolor_avg,
            pair_avg
        );
        close(id);
    }
}
