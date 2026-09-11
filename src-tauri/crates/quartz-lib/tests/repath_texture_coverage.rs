//! Does every MESH texture survive the Asset Extractor's "Repath" button?
//!
//! Users report that the repath flow sometimes ships a skin without its body or
//! weapon texture. This drives the exact code that button runs - clean extract,
//! then `repath_extracted` with the UI's default options (combine on, cleanup
//! off, consolidate ON) - against the real install, and checks two things:
//!
//!   1. EXTRACTION: every texture a non-VFX entry references (mesh textures,
//!      material samplers, HUD art) is on disk after the clean extract, unless
//!      the WAD itself does not hold it (Riot ships dead references).
//!   2. REPATH: after the rewrite, every non-VFX texture reference still points
//!      at a file. A `file =` reference is followed through `files.txt`, which
//!      is the only record of a re-hashed path. A reference whose file has
//!      landed under `*_particles/` is reported as MOVED BY CONSOLIDATE, since
//!      that is the failure mode being hunted.
//!
//! Gated on a real install; skips when absent. Targets and options come from
//! the environment so one binary covers many skins:
//!   QUARTZ_LEAGUE_ROOT       install root (default C:\Riot Games\League of Legends)
//!   QUARTZ_COVER_TARGETS     "taliyah:0,qiyana:0,viego:43" (the default: the three
//!                            reported skins, Viego 43 being Exalted "Revenant Reign")
//!   QUARTZ_COVER_CONSOLIDATE "0" to run the repath with consolidate off
//!   QUARTZ_COVER_SPLIT       "1" to also split VFX and animations into sibling bins
//!   QUARTZ_COVER_KEEP        "1" to leave the output on disk for inspection
//!
//! Run:
//!   cargo test -p quartz-lib --test repath_texture_coverage -- --nocapture

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use quartz_lib::bin::{read_bin, BinValue};
use quartz_lib::extractor::{extract_skin, repath_extracted, ExtractOptions, RepathOptions};

const TEXTURE_EXTS: [&str; 6] = [".tex", ".dds", ".png", ".jpg", ".scb", ".sco"];

fn league_root() -> Option<PathBuf> {
    let p = std::env::var("QUARTZ_LEAGUE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\Riot Games\League of Legends"));
    p.join("Game")
        .join("DATA")
        .join("FINAL")
        .join("Champions")
        .is_dir()
        .then_some(p)
}

fn targets() -> Vec<(String, u32)> {
    let raw = std::env::var("QUARTZ_COVER_TARGETS")
        .unwrap_or_else(|_| "taliyah:0,qiyana:0,viego:43".to_string());
    raw.split(',')
        .filter_map(|t| {
            let (c, s) = t.trim().split_once(':')?;
            Some((c.to_lowercase(), s.parse().ok()?))
        })
        .collect()
}

/// FNV1a-32 over the lowercased name: the BIN class-hash convention.
fn fnv1a_lower(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.to_lowercase().bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

fn norm(s: &str) -> String {
    s.replace('\\', "/").trim_start_matches('/').to_lowercase()
}

fn is_texture(p: &str) -> bool {
    let l = norm(p);
    (l.starts_with("assets/") || l.contains("/assets/")) && TEXTURE_EXTS.iter().any(|e| l.ends_with(e))
}

/// One reference found in a BIN: the path it names and where it came from.
#[derive(Debug, Clone)]
struct Ref {
    path: String,
    /// `bin#class` of the entry holding it, so a failure names the shape.
    source: String,
    /// Whether the reference was a `file =` hash (as opposed to a string).
    hashed: bool,
}

/// Every `File(u64)` hash in a value tree.
fn file_hashes(v: &BinValue, out: &mut Vec<u64>) {
    match v {
        BinValue::File(h) => {
            if *h != 0 {
                out.push(*h);
            }
        }
        BinValue::List { items, .. } => items.iter().for_each(|i| file_hashes(i, out)),
        BinValue::Pointer { fields, .. } | BinValue::Embed { fields, .. } => {
            fields.values().for_each(|x| file_hashes(x, out))
        }
        BinValue::Option { value: Some(inner), .. } => file_hashes(inner, out),
        BinValue::Map { entries, .. } => {
            for (k, val) in entries.iter() {
                file_hashes(k, out);
                file_hashes(val, out);
            }
        }
        _ => {}
    }
}

/// Every asset-looking string in a value tree.
fn strings(v: &BinValue, out: &mut Vec<String>) {
    match v {
        BinValue::String(s) => {
            if is_texture(s) {
                out.push(s.clone());
            }
        }
        BinValue::List { items, .. } => items.iter().for_each(|i| strings(i, out)),
        BinValue::Pointer { fields, .. } | BinValue::Embed { fields, .. } => {
            fields.values().for_each(|x| strings(x, out))
        }
        BinValue::Option { value: Some(inner), .. } => strings(inner, out),
        BinValue::Map { entries, .. } => {
            for (k, val) in entries.iter() {
                strings(k, out);
                strings(val, out);
            }
        }
        _ => {}
    }
}

fn bins_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            bins_under(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("bin")) {
            out.push(p);
        }
    }
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Resolve `file =` hashes: the mod's own `files.txt` first (the only record of
/// a re-hashed path), then the shared WAD dictionary.
struct Resolver {
    files_txt: HashMap<u64, String>,
    lmdb: Option<std::sync::Arc<heed::Env>>,
}

impl Resolver {
    fn new(content: &Path) -> Self {
        let mut files_txt = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(content.join("files.txt")) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                files_txt.insert(quartz_lib::hash::xxh64(&norm(line)), line.to_string());
            }
        }
        let lmdb = quartz_lib::hash::get_hash_dir()
            .ok()
            .and_then(|d| quartz_lib::hash::get_wad_env(&d.to_string_lossy()));
        Self { files_txt, lmdb }
    }

    fn resolve_many(&self, hashes: &[u64]) -> HashMap<u64, String> {
        let mut out = HashMap::new();
        let mut rest = Vec::new();
        for h in hashes {
            match self.files_txt.get(h) {
                Some(p) => {
                    out.insert(*h, p.clone());
                }
                None => rest.push(*h),
            }
        }
        if let (Some(env), false) = (&self.lmdb, rest.is_empty()) {
            rest.sort_unstable();
            rest.dedup();
            let r = quartz_lib::hash::resolve_hashes_lmdb_bulk(&rest, env);
            for h in &rest {
                if let Some(p) = r.get(h) {
                    out.insert(*h, p.to_string());
                }
            }
        }
        out
    }
}

/// Collect texture references from every BIN under `content`, split into
/// non-VFX (mesh, material, HUD) and VFX, with `file =` hashes resolved.
/// Unresolvable `file =` hashes are returned separately: a reference we cannot
/// even name is still a reference the game will follow.
fn collect_refs(
    content: &Path,
    resolver: &Resolver,
) -> (Vec<Ref>, Vec<Ref>, Vec<(u64, String)>) {
    let vfx_class = fnv1a_lower("VfxSystemDefinitionData");
    let mut bins = Vec::new();
    bins_under(content, &mut bins);

    let mut non_vfx = Vec::new();
    let mut vfx = Vec::new();
    let mut unresolved = Vec::new();

    for bin_path in &bins {
        let Ok(bytes) = std::fs::read(bin_path) else { continue };
        let Ok(bin) = read_bin(&bytes) else {
            eprintln!("  !! unreadable BIN {}", bin_path.display());
            continue;
        };
        let label = bin_path
            .strip_prefix(content)
            .unwrap_or(bin_path)
            .to_string_lossy()
            .replace('\\', "/");

        // Resolve every file hash of this bin in one batch.
        let mut all_hashes = Vec::new();
        for e in &bin.entries {
            for v in e.fields.values() {
                file_hashes(v, &mut all_hashes);
            }
        }
        all_hashes.sort_unstable();
        all_hashes.dedup();
        let resolved = resolver.resolve_many(&all_hashes);

        for e in &bin.entries {
            let source = format!("{}#{:08x}", label, e.class_hash);
            let bucket = if e.class_hash == vfx_class { &mut vfx } else { &mut non_vfx };
            for v in e.fields.values() {
                let mut ss = Vec::new();
                strings(v, &mut ss);
                for s in ss {
                    bucket.push(Ref { path: s, source: source.clone(), hashed: false });
                }
                let mut hs = Vec::new();
                file_hashes(v, &mut hs);
                for h in hs {
                    match resolved.get(&h) {
                        Some(p) if is_texture(p) => {
                            bucket.push(Ref { path: p.clone(), source: source.clone(), hashed: true })
                        }
                        Some(_) => {}
                        None => {
                            if e.class_hash != vfx_class {
                                unresolved.push((h, source.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    (non_vfx, vfx, unresolved)
}

/// Case-insensitive existence: BIN strings carry Riot's casing, files are lowercase.
fn on_disk(root: &Path, reference: &str) -> bool {
    let rel = norm(reference);
    root.join(&rel).is_file() || root.join(reference.replace('\\', "/")).is_file()
}

/// The hex-named file a `file =` hash the dictionary cannot name comes out as.
///
/// That is the designed fallback, not a loss: the extractor selects such a
/// chunk by its hash and writes it as `<hex>.<sniffed ext>`, and a hex stem at a
/// WAD root packs back under the same hash. So an unresolvable reference is only
/// a problem when no such file exists either.
fn hex_on_disk(root: &Path, hash: u64) -> Option<String> {
    let stem = format!("{hash:016x}");
    let mut all = Vec::new();
    files_under(root, &mut all);
    all.iter()
        .find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .map(|n| n.split('.').next().unwrap_or("") == stem)
                .unwrap_or(false)
        })
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
}

/// Where a basename actually sits under `root`, for the failure report.
fn locate(root: &Path, reference: &str) -> Vec<String> {
    let want = norm(reference);
    let base = want.rsplit('/').next().unwrap_or(&want).to_string();
    let mut all = Vec::new();
    files_under(root, &mut all);
    all.iter()
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_lowercase() == base)
                .unwrap_or(false)
        })
        .map(|p| {
            p.strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn mesh_textures_survive_the_repath_button() {
    let Some(root) = league_root() else {
        eprintln!("skipping: no League install");
        return;
    };
    let consolidate = std::env::var("QUARTZ_COVER_CONSOLIDATE").map(|v| v != "0").unwrap_or(true);
    let split = std::env::var("QUARTZ_COVER_SPLIT").map(|v| v == "1").unwrap_or(false);
    let keep = std::env::var("QUARTZ_COVER_KEEP").map(|v| v == "1").unwrap_or(false);

    let mut failures: Vec<String> = Vec::new();

    for (champ, skin) in targets() {
        eprintln!("\n================ {champ} skin{skin} (consolidate={consolidate} split={split}) ================");
        let out = std::env::temp_dir().join(format!(
            "quartz-cover-{champ}-{skin}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&out);
        std::fs::create_dir_all(&out).unwrap();

        // 1) Clean extract, exactly as the Repath button does it.
        let summary = match extract_skin(
            ExtractOptions {
                league_root: &root,
                champion: &champ,
                skin_id: skin,
                output_dir: &out,
                include_vo: false,
                clean: true,
                chroma_id: None,
                preserve_hud_icons2d: true,
                skip_sfx: true,
                folder_name: None,
            },
            |_| {},
        ) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{champ} skin{skin}: extract failed: {e}"));
                continue;
            }
        };
        let content = PathBuf::from(&summary.output_dir);
        eprintln!("extracted {} files -> {}", summary.files, content.display());

        // The WAD's own index, to tell a dead Riot reference from a dropped one.
        let wad = root
            .join("Game/DATA/FINAL/Champions")
            .join(format!("{}.wad.client", quartz_lib::extractor::wad_stem_for_name(&champ)));
        let toc: HashSet<u64> = quartz_lib::wad::read_wad_toc(&wad)
            .map(|t| t.iter().map(|e| e.path_hash).collect())
            .unwrap_or_default();

        // 2) PRE-repath: every non-VFX texture the WAD holds must be on disk.
        let resolver = Resolver::new(&content);
        let (non_vfx, vfx, unresolved) = collect_refs(&content, &resolver);
        let mut mesh_paths: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in &non_vfx {
            mesh_paths.entry(norm(&r.path)).or_default().insert(format!(
                "{}{}",
                r.source,
                if r.hashed { " (file=)" } else { "" }
            ));
        }
        let vfx_paths: BTreeSet<String> = vfx.iter().map(|r| norm(&r.path)).collect();
        eprintln!(
            "non-VFX texture refs: {}  VFX texture refs: {}  shared: {}  unresolved file= in non-VFX: {}",
            mesh_paths.len(),
            vfx_paths.len(),
            mesh_paths.keys().filter(|p| vfx_paths.contains(*p)).count(),
            unresolved.len()
        );
        for (h, src) in &unresolved {
            match hex_on_disk(&content, *h) {
                Some(f) => eprintln!("  .. file= {h:016x} in {src}: no dictionary name, extracted by hash as {f}"),
                None => eprintln!("  ?? file= {h:016x} in {src}: no dictionary name AND no hex file on disk"),
            }
        }

        let mut missing_after_extract = Vec::new();
        for (p, srcs) in &mesh_paths {
            if on_disk(&content, p) {
                continue;
            }
            let in_wad = toc.contains(&quartz_lib::hash::xxh64(p));
            if !in_wad {
                continue;
            }
            // The modern and legacy (`jade_*`) trees share one archive and an
            // extraction targets exactly one of them (see `jade_split.rs`). A
            // modern skin bin that names a file under the other tree, as Annie
            // skin60 does for its loadscreen and circle icon, leaves that file
            // behind by design. Reported, not failed.
            let crosses_tree = p.contains("/characters/jade_") != champ.starts_with("jade_");
            if crosses_tree {
                eprintln!("  -- left behind by the modern/legacy split: {p}  <- {:?}", srcs);
                continue;
            }
            missing_after_extract.push(format!("{p}  <- {:?}", srcs));
        }
        if !missing_after_extract.is_empty() {
            eprintln!("\nEXTRACTION dropped {} texture(s) the WAD holds:", missing_after_extract.len());
            for m in &missing_after_extract {
                eprintln!("  {m}");
            }
            failures.push(format!(
                "{champ} skin{skin}: extraction dropped {} texture(s) the WAD holds",
                missing_after_extract.len()
            ));
        }

        // 3) Repath with the UI's defaults.
        if let Err(e) = repath_extracted(RepathOptions {
            content_dir: &content,
            champion: &champ,
            skin_id: skin,
            creator_name: "cover",
            project_name: "",
            combine_linked: true,
            cleanup_unused: false,
            skip_sfx: true,
            skip_vo: true,
            split_vfx: split,
            split_anm: split,
            consolidate_assets: consolidate,
            wad_folder_override: None,
        }) {
            failures.push(format!("{champ} skin{skin}: repath failed: {e}"));
            continue;
        }

        // 4) POST-repath: every non-VFX texture reference must still point at a file.
        let resolver = Resolver::new(&content);
        let (non_vfx, _vfx, unresolved) = collect_refs(&content, &resolver);
        let mut dangling: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in &non_vfx {
            // Same rule as before the repath: a file the modern/legacy split left
            // behind keeps its vanilla path and resolves to the game's own copy.
            let crosses_tree =
                norm(&r.path).contains("/characters/jade_") != champ.starts_with("jade_");
            if crosses_tree {
                continue;
            }
            if !on_disk(&content, &r.path) {
                dangling.entry(norm(&r.path)).or_default().insert(format!(
                    "{}{}",
                    r.source,
                    if r.hashed { " (file=)" } else { "" }
                ));
            }
        }
        // A hash neither files.txt nor the dictionary can name is fine as long as
        // its chunk is on disk under the hex name: that is the fallback for a
        // dictionary older than the skin, and it packs back under the same hash.
        let unresolved: Vec<(u64, String)> = unresolved
            .into_iter()
            .filter(|(h, src)| match hex_on_disk(&content, *h) {
                Some(f) => {
                    eprintln!("  .. file= {h:016x} in {src} kept as hex file {f}");
                    false
                }
                None => true,
            })
            .collect();
        if !unresolved.is_empty() {
            eprintln!(
                "\nAFTER REPATH: {} non-VFX file= reference(s) resolve to nothing and have no hex file on disk:",
                unresolved.len()
            );
            for (h, src) in &unresolved {
                eprintln!("  {h:016x} in {src}");
            }
        }
        if !dangling.is_empty() {
            eprintln!("\nAFTER REPATH: {} non-VFX texture reference(s) dangle:", dangling.len());
            for (p, srcs) in &dangling {
                let where_ = locate(&content, p);
                let verdict = if where_.iter().any(|w| w.contains("_particles/")) {
                    "MOVED BY CONSOLIDATE"
                } else if where_.is_empty() {
                    "FILE ABSENT"
                } else {
                    "ELSEWHERE"
                };
                eprintln!("  [{verdict}] {p}");
                for s in srcs {
                    eprintln!("        ref from {s}");
                }
                for w in where_ {
                    eprintln!("        found at {w}");
                }
            }
            failures.push(format!(
                "{champ} skin{skin}: {} non-VFX texture reference(s) dangle after repath",
                dangling.len()
            ));
        }
        if !unresolved.is_empty() {
            failures.push(format!(
                "{champ} skin{skin}: {} non-VFX file= reference(s) unresolvable after repath",
                unresolved.len()
            ));
        }
        if dangling.is_empty() && unresolved.is_empty() && missing_after_extract.is_empty() {
            eprintln!("OK: {champ} skin{skin}: all {} non-VFX texture refs resolve", non_vfx.len());
        }

        if keep {
            eprintln!("kept: {}", content.display());
        } else {
            let _ = std::fs::remove_dir_all(&out);
        }
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
