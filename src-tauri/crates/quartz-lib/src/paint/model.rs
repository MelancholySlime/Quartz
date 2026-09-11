//! VFX projection — walk a `Bin` tree into the systems/emitters/colors/materials
//! view the Paint UI renders. Each editable node (color, blend mode, material
//! param) carries a [`NodePath`] back into the live tree so [`super::recolor`]
//! and the blend-mode/material commands can mutate it in place.

use super::fnv1a_lower;
use ritoshark::bin::{Bin, BinValue};
use serde::Serialize;
use std::collections::HashMap;

/// One step from a parent field-map (embed/pointer) or list down to a child.
#[derive(Debug, Clone)]
pub enum Step {
    /// Into a field of an embed/pointer by its hash key.
    Field(u32),
    /// Into a list element by index.
    Index(usize),
}

/// A relocatable path to a live node: which resident bin, which top-level
/// entry, then the steps down to the value. Resolved against the resident bins
/// at edit time so we never hold a borrow across IPC. `bin` is the index into
/// the session's bins (main at 0, linked bins follow).
#[derive(Debug, Clone)]
pub struct NodePath {
    pub bin: usize,
    pub entry: usize,
    pub steps: Vec<Step>,
}

impl NodePath {
    pub fn root(bin: usize, entry: usize) -> NodePath {
        NodePath {
            bin,
            entry,
            steps: Vec::new(),
        }
    }

    fn child(&self, step: Step) -> NodePath {
        let mut steps = self.steps.clone();
        steps.push(step);
        NodePath {
            bin: self.bin,
            entry: self.entry,
            steps,
        }
    }

    /// Resolve to a mutable reference into the addressed bin's tree, or `None`
    /// if the bin index is out of range or the path no longer points at a value
    /// (tree shape changed).
    pub fn resolve_mut<'a>(
        &self,
        bins: &'a mut [crate::linked_bins::LoadedBin],
    ) -> Option<&'a mut BinValue> {
        let entry = bins.get_mut(self.bin)?.tree.entries.get_mut(self.entry)?;
        let mut steps = self.steps.iter();
        // First step must descend from the entry's field map.
        let first = steps.next()?;
        let mut cur: &mut BinValue = match first {
            Step::Field(h) => entry.fields.get_mut(h)?,
            Step::Index(_) => return None,
        };
        for step in steps {
            cur = match (step, cur) {
                (Step::Field(h), BinValue::Embed { fields, .. })
                | (Step::Field(h), BinValue::Pointer { fields, .. }) => fields.get_mut(h)?,
                (Step::Index(i), BinValue::List { items, .. }) => items.get_mut(*i)?,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// Resolve against a single tree (used by the recolor engine, which is
    /// handed the owning bin's tree directly).
    pub fn resolve_in_tree<'a>(&self, bin: &'a mut Bin) -> Option<&'a mut BinValue> {
        let entry = bin.entries.get_mut(self.entry)?;
        let mut steps = self.steps.iter();
        let first = steps.next()?;
        let mut cur: &mut BinValue = match first {
            Step::Field(h) => entry.fields.get_mut(h)?,
            Step::Index(_) => return None,
        };
        for step in steps {
            cur = match (step, cur) {
                (Step::Field(h), BinValue::Embed { fields, .. })
                | (Step::Field(h), BinValue::Pointer { fields, .. }) => fields.get_mut(h)?,
                (Step::Index(i), BinValue::List { items, .. }) => items.get_mut(*i)?,
                _ => return None,
            };
        }
        Some(cur)
    }
}

// ── Serialized view shapes (camelCase to the frontend) ──────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorKeyframe {
    pub rgba: [f32; 4],
    pub time: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorData {
    pub keyframes: Vec<ColorKeyframe>,
    /// True when this is a single constant value (vs. an animated list).
    pub is_constant: bool,
    pub storage: ColorStorage,
    /// Index of the constant in the projected list. On a curve this is the
    /// wrapper's constant, not a lifetime stop, so it cannot be retimed/deleted.
    pub constant_index: Option<usize>,
    pub supports_structural_edits: bool,
    pub supports_retime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorStorage {
    Constant,
    Curve,
    ProbabilityTables,
}

impl ColorTarget {
    fn view(&self, keyframes: Vec<ColorKeyframe>) -> ColorData {
        let storage = if !self.channel_tables.is_empty() {
            ColorStorage::ProbabilityTables
        } else if !self.keyframes.is_empty() {
            ColorStorage::Curve
        } else {
            ColorStorage::Constant
        };
        ColorData {
            keyframes,
            is_constant: storage == ColorStorage::Constant,
            storage,
            constant_index: self.constant.as_ref().map(|_| 0),
            supports_structural_edits: self.is_value_color
                && storage != ColorStorage::ProbabilityTables,
            supports_retime: storage == ColorStorage::Curve,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmitterTexture {
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmitterColors {
    pub color: Option<ColorData>,
    pub birth_color: Option<ColorData>,
    pub fresnel_color: Option<ColorData>,
    pub linger_color: Option<ColorData>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VfxEmitter {
    pub key: String,
    pub name: String,
    pub system_key: String,
    pub blend_mode: u8,
    pub textures: Vec<EmitterTexture>,
    pub colors: EmitterColors,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VfxSystem {
    pub key: String,
    pub name: String,
    pub particle_name: Option<String>,
    pub emitter_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterialParam {
    pub name: String,
    pub values: [f32; 4],
    pub is_color: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VfxMaterial {
    pub key: String,
    pub name: String,
    pub color_params: Vec<MaterialParam>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VfxStats {
    pub system_count: usize,
    pub emitter_count: usize,
    pub material_count: usize,
    pub color_param_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VfxModel {
    pub systems: Vec<VfxSystem>,
    pub system_order: Vec<String>,
    pub emitters: Vec<VfxEmitter>,
    pub materials: Vec<VfxMaterial>,
    pub material_order: Vec<String>,
    pub stats: VfxStats,
}

// ── Edit-target index (Rust-side only; not serialized) ──────────────────────

/// Which of the four emitter colors a path targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSlot {
    Color,
    BirthColor,
    FresnelColor,
    LingerColor,
}

/// Live locations for every editable node, keyed by the same string keys the
/// serialized model exposes. Built alongside [`VfxModel`] and held in the
/// session so edits can relocate nodes without re-walking the whole tree.
#[derive(Debug, Default)]
pub struct EditIndex {
    /// `emitterKey` → (slot → path to the ValueColor embed or simple vec4).
    pub emitter_colors: HashMap<String, HashMap<ColorSlot, ColorTarget>>,
    /// `emitterKey` → path to the emitter's own embed/pointer node (its field
    /// map). Lets an edit insert a brand-new field (e.g. a missing color) onto
    /// the emitter, which the per-slot `ColorTarget`s can't express.
    pub emitter_nodes: HashMap<String, NodePath>,
    /// `emitterKey` → path to the `blendMode: u8` node.
    pub blend_modes: HashMap<String, NodePath>,
    /// `"mat::<materialKey>::<paramName>"` → path to the param `value: vec4`.
    pub material_params: HashMap<String, NodePath>,
    /// `emitterKey` → (current texture path string → path to that `string` node).
    /// Lets an edit relocate the exact texture node from its old value.
    pub emitter_textures: HashMap<String, HashMap<String, NodePath>>,
}

/// Where a color's editable vec4 nodes live.
#[derive(Debug, Clone)]
pub struct ColorTarget {
    /// The color field itself, including nested reflectionDefinition colors.
    pub color_path: NodePath,
    /// Bare vec4 fields cannot legally be replaced with ValueColor embeds.
    pub is_value_color: bool,
    /// Path to the constant vec4 node, if present (`constantValue` or a simple vec4).
    pub constant: Option<NodePath>,
    /// Paths to each keyframe vec4 in the `values` list, in order.
    pub keyframes: Vec<NodePath>,
    /// The keyframe times read from the bin's `times` list, parallel to `keyframes`.
    ///
    /// Recorded at projection time so a post-edit refresh reproduces the same view the
    /// initial open produced. Without it the refresh has to guess even spacing, and any
    /// color whose real times are uneven shifts its gradient after every recolor.
    pub times: Vec<f32>,
    /// Per-channel probability-table scatter paths, present when the color is
    /// authored as a `VfxAnimatedColorVariableData.probabilityTables` block
    /// instead of a `values: list[vec4]`.
    ///
    /// The color there is stored as up to four parallel `VfxProbabilityTableData`
    /// tables — one per RGBA channel, in vec4 component order (table[0]=R,
    /// [1]=G, [2]=B, [3]=A), each holding a `keyValues: list[f32]`. There is no
    /// vec4 node to edit; the color lives spread across those f32 lists.
    ///
    /// `channel_tables[k]` corresponds to synthesized keyframe `keyframes`
    /// index `k` (the k-th entry across every channel's `keyValues`), and holds
    /// `[Option<pathR>, pathG, pathB, pathA]` to each channel's F32 node for
    /// that key. Channels a table doesn't author are `None` (and default to the
    /// synthesized keyframe value on write, i.e. left unchanged).
    pub channel_tables: Vec<[Option<NodePath>; 4]>,
}

// ── Projection ──────────────────────────────────────────────────────────────

struct Hashes {
    vfx_system: u32,
    complex_emitter: u32,
    simple_emitter: u32,
    emitter_name: u32,
    particle_name: u32,
    blend_mode: u32,
    constant_value: u32,
    values: u32,
    dynamics: u32,
    times: u32,
    probability_tables: u32,
    key_values: u32,
    key_times: u32,
    reflection_definition: u32,
    color_slots: Vec<(ColorSlot, Vec<u32>)>,
    textures: Vec<(u32, &'static str)>,
    static_material: u32,
    sampler_values: u32,
    static_material_def: u32,
    param_name: u32,
    param_value: u32,
}

impl Hashes {
    fn new() -> Self {
        Hashes {
            vfx_system: fnv1a_lower("VfxSystemDefinitionData"),
            complex_emitter: fnv1a_lower("ComplexEmitterDefinitionData"),
            simple_emitter: fnv1a_lower("SimpleEmitterDefinitionData"),
            emitter_name: fnv1a_lower("emitterName"),
            particle_name: fnv1a_lower("particleName"),
            blend_mode: fnv1a_lower("blendMode"),
            constant_value: fnv1a_lower("constantValue"),
            values: fnv1a_lower("values"),
            dynamics: fnv1a_lower("dynamics"),
            times: fnv1a_lower("times"),
            probability_tables: fnv1a_lower("probabilityTables"),
            key_values: fnv1a_lower("keyValues"),
            key_times: fnv1a_lower("keyTimes"),
            reflection_definition: fnv1a_lower("reflectionDefinition"),
            // Field names that carry each color, in priority order.
            color_slots: vec![
                (ColorSlot::Color, vec![fnv1a_lower("color")]),
                (ColorSlot::BirthColor, vec![fnv1a_lower("birthColor")]),
                (
                    ColorSlot::FresnelColor,
                    vec![fnv1a_lower("fresnelColor"), fnv1a_lower("outlineColor")],
                ),
                (
                    ColorSlot::LingerColor,
                    vec![
                        fnv1a_lower("lingerColor"),
                        fnv1a_lower("SeparateLingerColor"),
                    ],
                ),
            ],
            textures: vec![
                (fnv1a_lower("texture"), "Main Texture"),
                (fnv1a_lower("particleColorTexture"), "Color Texture"),
                (fnv1a_lower("erosionMapName"), "Erosion Map"),
                (fnv1a_lower("textureMult"), "Mult Texture"),
                (fnv1a_lower("paletteTexture"), "Palette"),
                (fnv1a_lower("normalMap"), "Normal Map"),
                (fnv1a_lower("normalMapTexture"), "Normal Map"),
            ],
            static_material: fnv1a_lower("StaticMaterialDef"),
            static_material_def: fnv1a_lower("StaticMaterialShaderParamDef"),
            sampler_values: fnv1a_lower("paramValues"),
            param_name: fnv1a_lower("name"),
            param_value: fnv1a_lower("value"),
        }
    }
}

/// Project a single bin into the VFX view plus edit index (single-bin sessions
/// and tests). Equivalent to [`project_all`] over one main bin.
pub fn project(bin: &Bin) -> (VfxModel, EditIndex) {
    let lb = crate::linked_bins::LoadedBin {
        path: std::path::PathBuf::new(),
        role: crate::linked_bins::BinRole::Main,
        source_format: crate::linked_bins::SourceFormat::Bin,
        tree: bin.clone(),
        dirty: false,
        link_str: None,
        mtime: None,
    };
    project_all(std::slice::from_ref(&lb))
}

/// Project every resident bin into one merged VFX view plus edit index. Keys are
/// bin-prefixed (`${bin}:${path_hash}`) so systems/emitters/materials stay
/// unambiguous when linked bins share a path hash, and every `NodePath` carries
/// its bin so edits route to the right tree. Main bin first.
pub fn project_all(bins: &[crate::linked_bins::LoadedBin]) -> (VfxModel, EditIndex) {
    let h = Hashes::new();
    let mut model = VfxModel {
        systems: Vec::new(),
        system_order: Vec::new(),
        emitters: Vec::new(),
        materials: Vec::new(),
        material_order: Vec::new(),
        stats: VfxStats::default(),
    };
    let mut index = EditIndex::default();

    for (bin_idx, lb) in bins.iter().enumerate() {
        let bin = &lb.tree;
        for (entry_idx, entry) in bin.entries.iter().enumerate() {
            if entry.class_hash == h.vfx_system {
                project_system(bin, bin_idx, entry_idx, &h, &mut model, &mut index);
            } else if entry.class_hash == h.static_material {
                project_material(bin, bin_idx, entry_idx, &h, &mut model, &mut index);
            }
        }
    }

    model.stats.system_count = model.systems.len();
    model.stats.emitter_count = model.emitters.len();
    model.stats.material_count = model.materials.len();
    (model, index)
}

fn entry_key(bin: &Bin, bin_idx: usize, entry_idx: usize) -> String {
    // The entry's path hash gives a stable per-entry key; hex form matches the
    // hashed-name convention the rest of the app uses for unresolved names. The
    // bin index prefix keeps keys unique when linked bins repeat a path hash.
    format!("{}:{:08x}", bin_idx, bin.entries[entry_idx].path_hash)
}

fn project_system(
    bin: &Bin,
    bin_idx: usize,
    entry_idx: usize,
    h: &Hashes,
    model: &mut VfxModel,
    index: &mut EditIndex,
) {
    let entry = &bin.entries[entry_idx];
    let system_key = entry_key(bin, bin_idx, entry_idx);
    let base = NodePath::root(bin_idx, entry_idx);

    let particle_name = entry.fields.get(&h.particle_name).and_then(string_of);
    let display_name = particle_name
        .clone()
        .unwrap_or_else(|| short_name(&system_key));

    let mut system = VfxSystem {
        key: system_key.clone(),
        name: display_name,
        particle_name,
        emitter_keys: Vec::new(),
    };

    // Emitters live under ComplexEmitterDefinitionData / SimpleEmitterDefinitionData
    // list fields (a list of embed/pointer emitters).
    for (field_hash, field_val) in entry.fields.iter() {
        if *field_hash != h.complex_emitter && *field_hash != h.simple_emitter {
            continue;
        }
        let list_path = base.child(Step::Field(*field_hash));
        if let BinValue::List { items, .. } = field_val {
            for (i, item) in items.iter().enumerate() {
                let emitter_path = list_path.child(Step::Index(i));
                if let Some(emitter) = project_emitter(
                    item,
                    &emitter_path,
                    &system_key,
                    system.emitter_keys.len(),
                    h,
                    index,
                ) {
                    system.emitter_keys.push(emitter.key.clone());
                    model.emitters.push(emitter);
                }
            }
        }
    }

    if !system.emitter_keys.is_empty() || entry.class_hash == h.vfx_system {
        model.system_order.push(system_key.clone());
        model.systems.push(system);
    }
}

fn project_emitter(
    item: &BinValue,
    path: &NodePath,
    system_key: &str,
    index_in_system: usize,
    h: &Hashes,
    index: &mut EditIndex,
) -> Option<VfxEmitter> {
    let fields = match item {
        BinValue::Embed { fields, .. } | BinValue::Pointer { fields, .. } => fields,
        _ => return None,
    };

    let key = format!("{}__emitter_{}", system_key, index_in_system);
    let name = fields
        .get(&h.emitter_name)
        .and_then(string_of)
        .unwrap_or_else(|| "Unnamed".to_string());

    let blend_mode = match fields.get(&h.blend_mode) {
        Some(BinValue::U8(v)) => *v,
        _ => 0,
    };
    if fields.contains_key(&h.blend_mode) {
        index
            .blend_modes
            .insert(key.clone(), path.child(Step::Field(h.blend_mode)));
    }

    let mut textures = Vec::new();
    for (tex_hash, label) in &h.textures {
        if let Some(p) = fields.get(tex_hash).and_then(string_of) {
            // Index the node so an edit can relocate it from its old path value.
            index
                .emitter_textures
                .entry(key.clone())
                .or_default()
                .insert(p.clone(), path.child(Step::Field(*tex_hash)));
            if !textures
                .iter()
                .any(|t: &EmitterTexture| t.path == p && t.label == *label)
            {
                textures.push(EmitterTexture {
                    label: label.to_string(),
                    path: p,
                });
            }
        }
    }
    // The labeled list above only reaches seven known top-level fields. An emitter
    // can reference textures through other fields or nested structures
    // (reflectionDefinition, alphaErosionDefinition, primitive mesh defs, sub
    // emitters, …). Walk the whole emitter subtree for any texture-path string and
    // add the ones the labeled pass missed, so the hover list matches Port's
    // "every texture the emitter references" (Port: `collect_texture_strings`).
    // These extras are display-only (labeled generically); the labeled entries keep
    // their editable node paths in `emitter_textures`.
    let mut all_tex: Vec<String> = Vec::new();
    let mut all_mesh: Vec<String> = Vec::new();
    collect_asset_paths(item, &mut all_tex, &mut all_mesh);
    for p in all_tex {
        if !textures.iter().any(|t| t.path.eq_ignore_ascii_case(&p)) {
            textures.push(EmitterTexture {
                label: "Texture".to_string(),
                path: p,
            });
        }
    }
    // Mesh assets (.scb/.sco/.skn) the emitter draws through — Port keeps these
    // in a separate list; Paint surfaces them in the same hover list with a Mesh
    // label so nothing an emitter references is hidden.
    for p in all_mesh {
        if !textures.iter().any(|t| t.path.eq_ignore_ascii_case(&p)) {
            textures.push(EmitterTexture {
                label: "Mesh".to_string(),
                path: p,
            });
        }
    }
    // Index EVERY reachable texture/mesh node (not just the 7 labeled top-level
    // fields) so a path edit on a nested one (reflection / mesh def / sub-emitter)
    // actually resolves and saves instead of silently no-op'ing. The labeled loop
    // above already inserted the top-level ones; `or_insert` keeps those.
    if !textures.is_empty() {
        let node_map = index.emitter_textures.entry(key.clone()).or_default();
        collect_texture_nodes(item, path, node_map);
    }

    let mut targets: HashMap<ColorSlot, ColorTarget> = HashMap::new();
    let mut colors = EmitterColors {
        color: None,
        birth_color: None,
        fresnel_color: None,
        linger_color: None,
    };
    for (slot, names) in &h.color_slots {
        for name_hash in names {
            if let Some(field) = fields.get(name_hash) {
                let field_path = path.child(Step::Field(*name_hash));
                if let Some((data, target)) = project_color(field, &field_path, h) {
                    match slot {
                        ColorSlot::Color => colors.color = Some(data),
                        ColorSlot::BirthColor => colors.birth_color = Some(data),
                        ColorSlot::FresnelColor => colors.fresnel_color = Some(data),
                        ColorSlot::LingerColor => colors.linger_color = Some(data),
                    }
                    targets.insert(*slot, target);
                    break;
                }
            }
        }
    }

    // `fresnelColor` is often nested inside a `reflectionDefinition`
    // (VfxReflectionDefinitionData) pointer rather than sitting on the emitter,
    // so the top-level scan above misses it. When the FresnelColor slot is still
    // empty, look one level into reflectionDefinition and project its
    // `fresnelColor` (with the nested node path, so a recolor writes back into
    // the reflection struct).
    if !targets.contains_key(&ColorSlot::FresnelColor) {
        if let Some(BinValue::Pointer { fields: rf, .. } | BinValue::Embed { fields: rf, .. }) =
            fields.get(&h.reflection_definition)
        {
            let h_fresnel = fnv1a_lower("fresnelColor");
            if let Some(field) = rf.get(&h_fresnel) {
                let field_path = path
                    .child(Step::Field(h.reflection_definition))
                    .child(Step::Field(h_fresnel));
                if let Some((data, target)) = project_color(field, &field_path, h) {
                    colors.fresnel_color = Some(data);
                    targets.insert(ColorSlot::FresnelColor, target);
                }
            }
        }
    }

    if !targets.is_empty() {
        index.emitter_colors.insert(key.clone(), targets);
    }
    // Always record the emitter's own node path so an edit can add a missing
    // color field to it later (independent of which colors it currently has).
    index.emitter_nodes.insert(key.clone(), path.clone());

    Some(VfxEmitter {
        key,
        name,
        system_key: system_key.to_string(),
        blend_mode,
        textures,
        colors,
    })
}

/// Rebuild one color view from its edit target against the live tree — the
/// partial-refresh path after a recolor. Mirrors `project_color`'s shape: the
/// constant contributes a t=0 keyframe and list entries reuse the times captured
/// when the color was first projected, so a refreshed swatch matches the one drawn
/// on open.
pub(crate) fn color_data_from_target(
    bins: &mut [crate::linked_bins::LoadedBin],
    target: &ColorTarget,
) -> Option<ColorData> {
    let mut keyframes = Vec::new();
    if let Some(p) = &target.constant {
        if let Some(BinValue::Vec4(v)) = p.resolve_mut(bins) {
            keyframes.push(ColorKeyframe {
                rgba: *v,
                time: 0.0,
            });
        }
    }
    let count = target.keyframes.len();
    for (i, p) in target.keyframes.iter().enumerate() {
        if let Some(BinValue::Vec4(v)) = p.resolve_mut(bins) {
            /* Reuse the times captured when the color was projected. Recomputing them
            as even spacing here made a refreshed swatch disagree with the one drawn
            on open, so a gradient visibly shifted after every recolor even though the
            bin was unchanged. Fall back to even spacing only for targets recorded
            before times were tracked. */
            let time = target.times.get(i).copied().unwrap_or({
                if count <= 1 {
                    0.0
                } else {
                    i as f32 / (count - 1) as f32
                }
            });
            keyframes.push(ColorKeyframe { rgba: *v, time });
        }
    }

    // Probability-table channels: reconstruct each synthesized keyframe's vec4
    // from its per-channel `keyValues` f32 nodes. Without this the post-recolor
    // refresh returns None for a channel-table color (empty constant + keyframes),
    // so the swatch renders transparent until a full reload re-projects it.
    // `channel_tables` follows the `values` keyframes, matching `project_color`.
    let base = target.keyframes.len();
    for (k, chans) in target.channel_tables.iter().enumerate() {
        let mut rgba = [1.0f32, 1.0, 1.0, 1.0];
        for (ci, cp) in chans.iter().enumerate() {
            if let Some(p) = cp {
                if let Some(BinValue::F32(f)) = p.resolve_mut(bins) {
                    rgba[ci] = *f;
                }
            }
        }
        let time = target.times.get(base + k).copied().unwrap_or(0.0);
        keyframes.push(ColorKeyframe { rgba, time });
    }

    if keyframes.is_empty() {
        return None;
    }
    Some(target.view(keyframes))
}

/// Assemble a full per-emitter color view from its slot map (partial refresh).
pub(crate) fn emitter_colors_from_targets(
    bins: &mut [crate::linked_bins::LoadedBin],
    slots: &HashMap<ColorSlot, ColorTarget>,
) -> EmitterColors {
    let mut get = |slot: ColorSlot| -> Option<ColorData> {
        slots
            .get(&slot)
            .and_then(|t| color_data_from_target(bins, t))
    };
    EmitterColors {
        color: get(ColorSlot::Color),
        birth_color: get(ColorSlot::BirthColor),
        fresnel_color: get(ColorSlot::FresnelColor),
        linger_color: get(ColorSlot::LingerColor),
    }
}

/// Parse a color field, which is either a `ValueColor` embed (constantValue +
/// values/times) or a bare vec4. Returns the view data and the edit target.
fn project_color(
    field: &BinValue,
    path: &NodePath,
    h: &Hashes,
) -> Option<(ColorData, ColorTarget)> {
    match field {
        // Simple vec4 color (fresnelColor/outlineColor/lingerColor: vec4 = {...}).
        BinValue::Vec4(v) => Some((
            ColorData {
                keyframes: vec![ColorKeyframe {
                    rgba: *v,
                    time: 0.0,
                }],
                is_constant: true,
                storage: ColorStorage::Constant,
                constant_index: Some(0),
                supports_structural_edits: false,
                supports_retime: false,
            },
            ColorTarget {
                color_path: path.clone(),
                is_value_color: false,
                constant: Some(path.clone()),
                keyframes: Vec::new(),
                times: Vec::new(),
                channel_tables: Vec::new(),
            },
        )),
        // ValueColor embed/pointer.
        BinValue::Embed { fields, class } | BinValue::Pointer { fields, class } => {
            let mut keyframes = Vec::new();
            let mut target = ColorTarget {
                color_path: path.clone(),
                is_value_color: *class == fnv1a_lower("ValueColor"),
                constant: None,
                keyframes: Vec::new(),
                times: Vec::new(),
                channel_tables: Vec::new(),
            };

            if let Some(BinValue::Vec4(v)) = fields.get(&h.constant_value) {
                keyframes.push(ColorKeyframe {
                    rgba: *v,
                    time: 0.0,
                });
                target.constant = Some(path.child(Step::Field(h.constant_value)));
            }

            // Animated colors nest `values`/`times` inside a `dynamics` pointer
            // (VfxAnimatedColorVariableData). Descend into it when present;
            // otherwise fall back to `values` directly on the embed.
            let (values_owner_path, values_fields) = match fields.get(&h.dynamics) {
                Some(BinValue::Pointer { fields: df, .. } | BinValue::Embed { fields: df, .. }) => {
                    (path.child(Step::Field(h.dynamics)), df)
                }
                _ => (path.clone(), fields),
            };

            // Probability-table color form takes PRIORITY over `values`: when the
            // animated color authors `probabilityTables` (one VfxProbabilityTableData
            // per RGBA channel, vec4 order, each with `keyValues: list[f32]` + a
            // parallel `keyTimes`), that is where the real color lives and the
            // `values` list is just a placeholder (typically a single `{1,1,1,1}`).
            // Editing `values` there changes nothing rendered, so project the tables
            // instead and record where each channel's key lives so the recolor engine
            // can scatter new values back. `has_color_tables` guards the `values`
            // fallback below so we don't also emit the placeholder as a keyframe.
            let mut has_color_tables = false;
            if let Some(BinValue::List { items: tables, .. }) =
                values_fields.get(&h.probability_tables)
            {
                let tables_path = values_owner_path.child(Step::Field(h.probability_tables));
                // Per channel: (keyValues f32 vec, keyTimes f32 vec, path to keyValues).
                let mut chans: Vec<(usize, Vec<f32>, Vec<f32>, NodePath)> = Vec::new();
                for (ci, tbl) in tables.iter().enumerate().take(4) {
                    if let BinValue::Pointer { fields: tf, .. }
                    | BinValue::Embed { fields: tf, .. } = tbl
                    {
                        let kv = f32_list(tf.get(&h.key_values));
                        let kt = f32_list(tf.get(&h.key_times));
                        let tpath = tables_path
                            .child(Step::Index(ci))
                            .child(Step::Field(h.key_values));
                        chans.push((ci, kv, kt, tpath));
                    }
                }
                // Synthesized keyframe count = max keyValues length across channels.
                // Zero when every table is empty (a `VfxProbabilityTableData {}` with
                // no keyValues, as authored on inert channels) — then this is not a
                // real color-table block and we leave `has_color_tables` false so the
                // `values`/constant path still drives the color.
                let nkeys = chans
                    .iter()
                    .map(|(_, kv, _, _)| kv.len())
                    .max()
                    .unwrap_or(0);
                if nkeys > 0 {
                    has_color_tables = true;
                    for k in 0..nkeys {
                        let mut rgba = [1.0f32, 1.0, 1.0, 1.0];
                        let mut scatter: [Option<NodePath>; 4] = [None, None, None, None];
                        let mut time = if nkeys <= 1 {
                            0.0
                        } else {
                            k as f32 / (nkeys - 1) as f32
                        };
                        for (ci, kv, kt, tpath) in &chans {
                            if let Some(val) = kv.get(k) {
                                rgba[*ci] = *val;
                                scatter[*ci] = Some(tpath.child(Step::Index(k)));
                            }
                            if *ci == 0 {
                                if let Some(t) = kt.get(k) {
                                    time = *t;
                                }
                            }
                        }
                        keyframes.push(ColorKeyframe { rgba, time });
                        target.channel_tables.push(scatter);
                        target.times.push(time);
                    }
                }
            }

            if !has_color_tables {
                if let Some(BinValue::List { items, .. }) = values_fields.get(&h.values) {
                    // Prefer the real `times` list; fall back to even spacing.
                    let times: Vec<f32> = match values_fields.get(&h.times) {
                        Some(BinValue::List { items: t, .. }) => t
                            .iter()
                            .filter_map(|v| match v {
                                BinValue::F32(f) => Some(*f),
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    let values_path = values_owner_path.child(Step::Field(h.values));
                    let count = items.len();
                    for (i, item) in items.iter().enumerate() {
                        if let BinValue::Vec4(v) = item {
                            let time = times.get(i).copied().unwrap_or_else(|| {
                                if count <= 1 {
                                    0.0
                                } else {
                                    i as f32 / (count - 1) as f32
                                }
                            });
                            keyframes.push(ColorKeyframe { rgba: *v, time });
                            target.keyframes.push(values_path.child(Step::Index(i)));
                            target.times.push(time);
                        }
                    }
                }
            }

            if keyframes.is_empty() {
                return None;
            }
            // A channel-table color animates across its keyValues, so it is not a
            // constant even though `keyframes` (the vec4-list target) is empty.
            Some((target.view(keyframes), target))
        }
        _ => None,
    }
}

/// Read a bin field as a `Vec<f32>`, tolerating a missing / non-list value
/// (returns empty). Used to lift `keyValues` / `keyTimes` scalar lists.
fn f32_list(v: Option<&BinValue>) -> Vec<f32> {
    match v {
        Some(BinValue::List { items, .. }) => items
            .iter()
            .filter_map(|x| match x {
                BinValue::F32(f) => Some(*f),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn project_material(
    bin: &Bin,
    bin_idx: usize,
    entry_idx: usize,
    h: &Hashes,
    model: &mut VfxModel,
    index: &mut EditIndex,
) {
    let entry = &bin.entries[entry_idx];
    let material_key = entry_key(bin, bin_idx, entry_idx);
    let base = NodePath::root(bin_idx, entry_idx);

    let mut color_params = Vec::new();

    // Static-material color params live in a list of StaticMaterialShaderParamDef
    // embeds under `paramValues`, each { name: string, value: vec4 }.
    if let Some(BinValue::List { items, .. }) = entry.fields.get(&h.sampler_values) {
        let list_path = base.child(Step::Field(h.sampler_values));
        for (i, item) in items.iter().enumerate() {
            let pf = match item {
                BinValue::Embed { fields, .. } | BinValue::Pointer { fields, .. } => fields,
                _ => continue,
            };
            let name = match pf.get(&h.param_name).and_then(string_of) {
                Some(n) => n,
                None => continue,
            };
            if let Some(BinValue::Vec4(v)) = pf.get(&h.param_value) {
                let selection_key = format!("mat::{}::{}", material_key, name);
                let value_path = list_path
                    .child(Step::Index(i))
                    .child(Step::Field(h.param_value));
                index.material_params.insert(selection_key, value_path);
                color_params.push(MaterialParam {
                    name,
                    values: *v,
                    is_color: true,
                });
                model.stats.color_param_count += 1;
            }
        }
    }

    let _ = h.static_material_def; // reserved: some bins nest params differently
    if color_params.is_empty() {
        return;
    }

    model.material_order.push(material_key.clone());
    model.materials.push(VfxMaterial {
        key: material_key.clone(),
        name: short_name(&material_key),
        color_params,
    });
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn string_of(v: &BinValue) -> Option<String> {
    match v {
        BinValue::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Collect every texture (.dds/.tex/.png/.jpg) and mesh (.scb/.sco/.skn) path
/// string anywhere in a value subtree, each deduped in walk order
/// (case-insensitive). Mirrors the VFX-port collectors so Paint lists the same
/// assets Port does — not just a fixed field list.
fn collect_asset_paths(value: &BinValue, textures: &mut Vec<String>, meshes: &mut Vec<String>) {
    match value {
        BinValue::String(s) => {
            let l = s.to_ascii_lowercase();
            if l.ends_with(".dds")
                || l.ends_with(".tex")
                || l.ends_with(".png")
                || l.ends_with(".jpg")
            {
                if !textures.iter().any(|e| e.eq_ignore_ascii_case(s)) {
                    textures.push(s.clone());
                }
            } else if l.ends_with(".scb") || l.ends_with(".sco") || l.ends_with(".skn") {
                if !meshes.iter().any(|e| e.eq_ignore_ascii_case(s)) {
                    meshes.push(s.clone());
                }
            }
        }
        BinValue::List { items, .. } => {
            items
                .iter()
                .for_each(|v| collect_asset_paths(v, textures, meshes));
        }
        BinValue::Pointer { fields, .. } | BinValue::Embed { fields, .. } => {
            fields
                .values()
                .for_each(|v| collect_asset_paths(v, textures, meshes));
        }
        BinValue::Option {
            value: Some(inner), ..
        } => collect_asset_paths(inner, textures, meshes),
        BinValue::Map { entries, .. } => {
            for (k, v) in entries {
                collect_asset_paths(k, textures, meshes);
                collect_asset_paths(v, textures, meshes);
            }
        }
        _ => {}
    }
}

/// Index every texture/mesh string reachable from `value` by a resolvable
/// `NodePath` (Field into embed/pointer, Index into list) into `out`, keyed by
/// its path value. This makes nested textures (reflectionDefinition, mesh defs,
/// sub-emitters, …) EDITABLE, not just display-only — `set_texture` looks the
/// node up by its current string. Options/Maps are not addressable by the
/// current `Step` enum, so a texture at or below one stays display-only (it is
/// still shown, just not repathable). First writer wins so an already-indexed
/// labeled top-level field is never clobbered.
fn collect_texture_nodes(
    value: &BinValue,
    path: &NodePath,
    out: &mut HashMap<String, NodePath>,
) {
    match value {
        BinValue::String(s) => {
            let l = s.to_ascii_lowercase();
            let is_asset = l.ends_with(".dds")
                || l.ends_with(".tex")
                || l.ends_with(".png")
                || l.ends_with(".jpg")
                || l.ends_with(".scb")
                || l.ends_with(".sco")
                || l.ends_with(".skn");
            if is_asset {
                out.entry(s.clone()).or_insert_with(|| path.clone());
            }
        }
        BinValue::List { items, .. } => {
            for (i, v) in items.iter().enumerate() {
                collect_texture_nodes(v, &path.child(Step::Index(i)), out);
            }
        }
        BinValue::Pointer { fields, .. } | BinValue::Embed { fields, .. } => {
            for (h, v) in fields.iter() {
                collect_texture_nodes(v, &path.child(Step::Field(*h)), out);
            }
        }
        // Option / Map inner nodes have no NodePath Step, so they can't be
        // resolved for an in-place edit; leave them display-only.
        _ => {}
    }
}

/// Trim a slash-delimited path to its last segment, capped at 40 chars. Strips a
/// leading `<bin>:` prefix first so a bin-prefixed hex key (`0:1a2b3c4d`) reads
/// as just the hash in the UI, not the internal bin index.
fn short_name(full: &str) -> String {
    if full.is_empty() {
        return "Unknown".to_string();
    }
    // Drop a leading numeric bin prefix (`<digits>:`) if present.
    let unprefixed = match full.split_once(':') {
        Some((head, tail)) if head.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() => tail,
        _ => full,
    };
    let last = unprefixed.rsplit('/').next().unwrap_or(unprefixed);
    if last.len() > 40 {
        format!("{}...", &last[..37])
    } else {
        last.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indexmap::IndexMap;
    use ritoshark::bin::{Bin, BinEntry, BinType};

    /// Build a one-system, one-emitter bin with a constant base color and two
    /// birthColor keyframes, then assert the projection + edit index.
    #[test]
    fn projects_system_emitter_colors() {
        let h_emitter_name = fnv1a_lower("emitterName");
        let h_blend = fnv1a_lower("blendMode");
        let h_color = fnv1a_lower("color");
        let h_birth = fnv1a_lower("birthColor");
        let h_constant = fnv1a_lower("constantValue");
        let h_values = fnv1a_lower("values");
        let h_complex = fnv1a_lower("ComplexEmitterDefinitionData");

        // color: ValueColor { constantValue: vec4 }
        let mut color_fields = IndexMap::new();
        color_fields.insert(h_constant, BinValue::Vec4([0.5, 0.2, 0.1, 1.0]));
        // birthColor: ValueColor { values: list[vec4]{ a, b } }
        let mut birth_fields = IndexMap::new();
        birth_fields.insert(
            h_values,
            BinValue::List {
                is_list2: false,
                item: BinType::Vec4,
                items: vec![
                    BinValue::Vec4([1.0, 0.0, 0.0, 1.0]),
                    BinValue::Vec4([0.0, 0.0, 1.0, 1.0]),
                ],
            },
        );

        let mut emitter_fields = IndexMap::new();
        emitter_fields.insert(h_emitter_name, BinValue::String("Sparkles".into()));
        emitter_fields.insert(h_blend, BinValue::U8(3));
        emitter_fields.insert(
            h_color,
            BinValue::Embed {
                class: 0xAABB,
                fields: color_fields,
            },
        );
        emitter_fields.insert(
            h_birth,
            BinValue::Embed {
                class: 0xAABB,
                fields: birth_fields,
            },
        );

        let mut system_fields = IndexMap::new();
        system_fields.insert(
            h_complex,
            BinValue::List {
                is_list2: false,
                item: BinType::Embed,
                items: vec![BinValue::Embed {
                    class: 0xCCDD,
                    fields: emitter_fields,
                }],
            },
        );

        let bin = Bin {
            entries: vec![BinEntry {
                path_hash: 0x1234_5678,
                class_hash: fnv1a_lower("VfxSystemDefinitionData"),
                fields: system_fields,
            }],
            ..Bin::new()
        };

        let (model, index) = project(&bin);
        assert_eq!(model.systems.len(), 1);
        assert_eq!(model.emitters.len(), 1);

        let emitter = &model.emitters[0];
        assert_eq!(emitter.name, "Sparkles");
        assert_eq!(emitter.blend_mode, 3);

        let color = emitter.colors.color.as_ref().expect("base color");
        assert!(color.is_constant);
        assert_eq!(color.keyframes.len(), 1);
        assert_eq!(color.keyframes[0].rgba, [0.5, 0.2, 0.1, 1.0]);

        let birth = emitter.colors.birth_color.as_ref().expect("birth color");
        assert!(!birth.is_constant);
        assert_eq!(birth.keyframes.len(), 2);

        // Edit index has both colors + the blend mode for this emitter.
        let slots = index
            .emitter_colors
            .get(&emitter.key)
            .expect("color targets");
        assert!(slots.contains_key(&ColorSlot::Color));
        assert!(slots.contains_key(&ColorSlot::BirthColor));
        assert!(index.blend_modes.contains_key(&emitter.key));

        // The base-color target's constant path resolves back to the live vec4.
        let mut bins_mut = vec![crate::linked_bins::LoadedBin {
            path: std::path::PathBuf::new(),
            role: crate::linked_bins::BinRole::Main,
            source_format: crate::linked_bins::SourceFormat::Bin,
            tree: bin.clone(),
            dirty: false,
            link_str: None,
            mtime: None,
        }];
        let target = &slots[&ColorSlot::Color];
        let node = target.constant.as_ref().unwrap().resolve_mut(&mut bins_mut);
        assert!(matches!(node, Some(BinValue::Vec4(_))));
    }
}
