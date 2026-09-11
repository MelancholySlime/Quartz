//! BNK/WPK tree builder — ports the Electron Quartz bnkLoader/bnkParser logic.
//!
//! Parses a BIN/WPK/BNK triple into the `BnkNode` tree the BnkExtract UI renders.
//! Event names come from the BIN, mapped to WEM ids through the events-BNK HIRC.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::bnk::{self, AudioEntry};
use super::event_mapper::{self, EventMapping};
use super::hirc;
use super::wpk;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioData {
    pub id: u32,
    pub data: Vec<u8>,
    pub offset: u32,
    pub length: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_modified: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BnkNode {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_data: Option<AudioData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<BnkNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_root: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bnk_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wpk_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadBanksResult {
    pub tree: BnkNode,
    pub audio_files: Vec<AudioData>,
    pub file_count: usize,
    #[serde(rename = "type")]
    pub kind: String,
}

impl BnkNode {
    fn dir(name: impl Into<String>) -> Self {
        let name = name.into();
        BnkNode {
            id: format!("dir-{name}"),
            name,
            audio_data: None,
            children: Some(Vec::new()),
            is_root: None,
            original_path: None,
            bnk_path: None,
            wpk_path: None,
            bin_path: None,
        }
    }

    fn leaf(name: String, audio: AudioData) -> Self {
        BnkNode {
            id: format!("audio-{}", audio.id),
            name,
            audio_data: Some(audio),
            children: Some(Vec::new()),
            is_root: None,
            original_path: None,
            bnk_path: None,
            wpk_path: None,
            bin_path: None,
        }
    }

    fn get_or_create_child(&mut self, name: &str) -> &mut BnkNode {
        let children = self.children.get_or_insert_with(Vec::new);
        let pos = children.iter().position(|c| c.name == name);
        let idx = match pos {
            Some(i) => i,
            None => {
                children.push(BnkNode::dir(name.to_string()));
                children.len() - 1
            }
        };
        &mut children[idx]
    }

    fn has_audio_child(&self, audio_id: u32) -> bool {
        self.children
            .as_ref()
            .map(|cs| {
                cs.iter()
                    .any(|c| c.audio_data.as_ref().is_some_and(|a| a.id == audio_id))
            })
            .unwrap_or(false)
    }
}

fn to_audio_data(entry: &AudioEntry) -> AudioData {
    AudioData {
        id: entry.id,
        length: entry.data.len() as u32,
        offset: 0,
        data: entry.data.clone(),
        is_modified: None,
    }
}

/// Build the tree from audio entries + event mappings (port of groupAudioFiles).
fn group_audio_files(
    entries: &[AudioEntry],
    mappings: &[EventMapping],
    root_name: &str,
) -> BnkNode {
    // Pre-index mappings by wem_id so the per-entry lookup below is O(1)
    // instead of a full scan of `mappings`. The old shape was O(entries ×
    // mappings) which explodes for champs with lots of skins × events.
    let mut mappings_by_wem: std::collections::HashMap<u32, Vec<&EventMapping>> =
        std::collections::HashMap::with_capacity(mappings.len());
    for m in mappings {
        mappings_by_wem.entry(m.wem_id).or_default().push(m);
    }

    let mut root = BnkNode::dir(root_name.to_string());
    /* Entries no mapping claimed. These land at the ROOT named `<id>.wem`, which is the
       "random names" symptom: the bank loaded fine, but nothing told the tree which event
       plays this wem. Distinct from `mappings.len()`, which counts event->wem edges and
       stays healthy even when most wems are unreachable. */
    let mut unmatched = 0usize;

    for entry in entries {
        let mut inserted = false;
        let matches = mappings_by_wem.get(&entry.id);
        let empty: Vec<&EventMapping> = Vec::new();
        let ms = matches.map(|v| v.as_slice()).unwrap_or(empty.as_slice());

        for m in ms {
            // Navigate switch -> event -> container -> music-segment.
            let mut path: Vec<String> = Vec::new();
            if let Some(switch_id) = m.switch_id {
                if switch_id != 0 {
                    path.push(switch_id.to_string());
                }
            }
            path.push(m.event_name.clone());
            if m.container_id != 0 {
                path.push(m.container_id.to_string());
            }
            if let Some(seg) = m.music_segment_id {
                if seg != 0 {
                    path.push(seg.to_string());
                }
            }

            let mut cursor = &mut root;
            for token in &path {
                cursor = cursor.get_or_create_child(token);
            }
            if !cursor.has_audio_child(entry.id) {
                let audio = to_audio_data(entry);
                cursor
                    .children
                    .get_or_insert_with(Vec::new)
                    .push(BnkNode::leaf(format!("{}.wem", entry.id), audio));
            }
            inserted = true;
        }

        if !inserted {
            unmatched += 1;
        }
        if !inserted && !root.has_audio_child(entry.id) {
            let audio = to_audio_data(entry);
            root.children
                .get_or_insert_with(Vec::new)
                .push(BnkNode::leaf(format!("{}.wem", entry.id), audio));
        }
    }

    let distinct_mapped = mappings_by_wem.len();
    eprintln!(
        "[load_banks]   tree: entries={} distinctMappedWems={} unmatched(at root)={}",
        entries.len(),
        distinct_mapped,
        unmatched,
    );

    root
}

fn sanitize_scope(value: &str) -> String {
    // Sanitize + collapse consecutive underscores + trim leading/trailing
    // underscores in a single pass. The old version replaced disallowed chars
    // with '_' and then ran `while out.contains("__") { replace("__", "_") }`
    // — quadratic on strings with long runs of underscores, and called for
    // every tree node during parse. This is O(n).
    let mut out = String::with_capacity(value.len());
    let mut prev_was_underscore = true; // start "true" so leading _'s are dropped
    for c in value.chars() {
        let mapped = if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
            c
        } else {
            '_'
        };
        if mapped == '_' {
            if !prev_was_underscore {
                out.push('_');
                prev_was_underscore = true;
            }
        } else {
            out.push(mapped);
            prev_was_underscore = false;
        }
    }
    // Drop trailing underscore if present.
    if out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "root".to_string()
    } else {
        out
    }
}

/// Assign stable, collision-free ids to every node (port of scopeTreeNodeIds).
fn scope_ids(node: &mut BnkNode, scope_key: &str, trail: &[String]) {
    let token = sanitize_scope(if !node.name.is_empty() {
        &node.name
    } else {
        &node.id
    });
    let mut scoped_trail = trail.to_vec();
    scoped_trail.push(token);
    node.id = format!("{scope_key}::{}", scoped_trail.join("::"));

    if let Some(children) = node.children.as_mut() {
        let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
        for child in children.iter_mut() {
            let child_token = sanitize_scope(if !child.name.is_empty() {
                &child.name
            } else {
                &child.id
            });
            let count = counts.entry(child_token.clone()).or_insert(0);
            *count += 1;
            scope_ids(
                child,
                scope_key,
                &scoped_trail_with(&scoped_trail, &child_token, *count),
            );
        }
    }
}

/// Helper mirroring scopeTreeNodeIds: the child's trail is parent trail + `token~n`.
fn scoped_trail_with(parent_trail: &[String], token: &str, count: u32) -> Vec<String> {
    let mut t = parent_trail.to_vec();
    t.push(format!("{token}~{count}"));
    t
}

fn normalize_slashes(value: &str) -> String {
    value.replace('\\', "/")
}

fn skin_seg_to_number(seg: &str) -> Option<u32> {
    let seg = seg.trim().to_lowercase();
    if seg.is_empty() {
        return None;
    }
    if seg == "base" || seg == "root" {
        return Some(0);
    }
    if let Some(rest) = seg.strip_prefix("skin") {
        return rest.trim_start_matches('0').parse().ok().or({
            if rest.chars().all(|c| c == '0') {
                Some(0)
            } else {
                None
            }
        });
    }
    let trimmed = seg.trim_start_matches('0');
    if trimmed.is_empty() && !seg.is_empty() {
        return Some(0);
    }
    trimmed.parse().ok()
}

/// Infer likely BIN paths from a skin audio path (port of inferBinCandidatesFromAudioPath).
fn infer_bin_candidates(audio_path: &str) -> Vec<String> {
    if audio_path.is_empty() {
        return Vec::new();
    }
    let normalized = normalize_slashes(audio_path);
    let lower = normalized.to_lowercase();

    // Match /characters/<champ>/skins/<skin>/
    let marker = "/characters/";
    let chars_idx = match lower.find(marker) {
        Some(i) => i,
        None => return Vec::new(),
    };
    let after = &lower[chars_idx + marker.len()..];
    let champ = match after.split('/').next() {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => return Vec::new(),
    };
    let skins_marker = format!("/characters/{champ}/skins/");
    let skin_seg = match lower.find(&skins_marker) {
        Some(i) => {
            let rest = &lower[i + skins_marker.len()..];
            rest.split('/').next().unwrap_or("").to_string()
        }
        None => return Vec::new(),
    };
    let skin_num = skin_seg_to_number(&skin_seg);

    let assets_idx = lower.find("/assets/");
    let data_idx = lower.find("/data/");
    let root: String = if let Some(i) = assets_idx {
        audio_path[..i].to_string()
    } else if let Some(i) = data_idx {
        audio_path[..i].to_string()
    } else {
        Path::new(audio_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    let mut candidates: Vec<String> = Vec::new();
    let mut push = |p: PathBuf| {
        let s = p.to_string_lossy().to_string();
        if !s.is_empty() && !candidates.contains(&s) {
            candidates.push(s);
        }
    };

    let root = PathBuf::from(root);
    let data_skins = root
        .join("data")
        .join("characters")
        .join(&champ)
        .join("skins");
    let data_champion = root.join("data").join("characters").join(&champ);

    if let Some(n) = skin_num {
        push(data_skins.join(format!("skin{n}.bin")));
        push(data_skins.join(format!("skin{n:02}.bin")));
    }
    if !skin_seg.is_empty() {
        push(data_skins.join(format!("{skin_seg}.bin")));
    }
    push(data_skins.join("root.bin"));
    push(data_champion.join(format!("{champ}.bin")));
    push(
        data_champion
            .join("animations")
            .join(format!("skin{}.bin", skin_num.unwrap_or(0))),
    );

    candidates
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Read all entries from a BNK or WPK buffer, sorted by id.
fn read_entries(data: &[u8]) -> Result<Vec<AudioEntry>, String> {
    if data.len() < 4 {
        return Err("Audio file too small".into());
    }
    let mut entries = match &data[0..4] {
        b"BKHD" => bnk::BnkFile::parse(data)?.read_all_entries(data)?,
        b"r3d2" => wpk::WpkFile::parse(data)?.read_all_entries(data)?,
        _ => return Err("Unknown audio format".into()),
    };
    entries.sort_by_key(|e| e.id);
    Ok(entries)
}

/// Resolve event mappings from a BIN + events BNK. Falls back to plain BIN
/// strings (each name hashed) so audio still groups when HIRC mapping is absent.
fn resolve_mappings(bin_data: &[u8], events_bnk: Option<&[u8]>) -> Vec<EventMapping> {
    let events = event_mapper::extract_bin_events(bin_data);

    if let Some(bnk_data) = events_bnk {
        /* Report each stage separately. A bank that resolves to raw wem ids can fail at
           three different points and they look identical in the UI: no event NAMES out of
           the BIN, a HIRC that would not parse, or a HIRC that parsed but whose events do
           not reach any of this bank's wems. */
        match hirc::parse_hirc_from_bnk(bnk_data) {
            Ok(Some(hirc_data)) => {
                let mapped = event_mapper::map_events_to_wem(&events, &hirc_data);
                eprintln!(
                    "[load_banks] hirc ok: events(bin)={} hirc.events={} actions={} sounds={} randomCntr={} switchCntr={} -> mapped={}",
                    events.len(),
                    hirc_data.events.len(),
                    hirc_data.event_actions.len(),
                    hirc_data.sounds.len(),
                    hirc_data.random_containers.len(),
                    hirc_data.switch_containers.len(),
                    mapped.len(),
                );
                /* Container CHILD counts, not just container counts. Every Play action in
                   a champion bank targets a container rather than a Sound directly, so if
                   the child lists come back empty the walk reaches nothing and mapped is 0
                   - while the container count still looks healthy. That is the difference
                   between "the HIRC parsed" and "the HIRC parsed correctly". */
                let rc_children: usize =
                    hirc_data.random_containers.iter().map(|c| c.sound_ids.len()).sum();
                let sc_children: usize =
                    hirc_data.switch_containers.iter().map(|c| c.children.len()).sum();
                eprintln!(
                    "[load_banks]   container children: random={rc_children} switch={sc_children}"
                );
                if !mapped.is_empty() {
                    return mapped;
                }
            }
            Ok(None) => eprintln!(
                "[load_banks] events bank has NO HIRC section (events(bin)={})",
                events.len()
            ),
            Err(e) => eprintln!("[load_banks] HIRC parse FAILED: {e} (events(bin)={})", events.len()),
        }
    } else {
        eprintln!(
            "[load_banks] no events bank supplied (events(bin)={})",
            events.len()
        );
    }

    // Fallback: treat each BIN string's hash as a direct wem id.
    events
        .into_iter()
        .map(|e| EventMapping {
            event_name: e.name,
            wem_id: e.hash,
            container_id: 0,
            music_segment_id: None,
            switch_id: None,
        })
        .collect()
}

/// Orchestrates a full BNK/WPK/BIN load (port of bnkLoader.loadBanks).
pub fn load_banks(
    bnk_path: &str,
    wpk_path: &str,
    bin_path: &str,
) -> Result<Option<LoadBanksResult>, String> {
    let read = |p: &str| -> Option<Vec<u8>> {
        if p.is_empty() {
            return None;
        }
        std::fs::read(p).ok()
    };

    // Build the ordered, de-duplicated list of BIN candidates.
    let source_audio = if !wpk_path.is_empty() {
        wpk_path
    } else {
        bnk_path
    };
    let mut bin_candidates: Vec<String> = Vec::new();
    let mut push_bin = |c: &str| {
        if !c.is_empty() && Path::new(c).exists() && !bin_candidates.contains(&c.to_string()) {
            bin_candidates.push(c.to_string());
        }
    };
    push_bin(bin_path);
    for c in infer_bin_candidates(source_audio) {
        push_bin(&c);
    }

    let mut used_bin_path = String::new();
    let mut mappings: Vec<EventMapping> = Vec::new();
    let events_bnk_data = read(bnk_path);

    /* What actually arrived. A group whose events bank or BIN never made it this far
       resolves to raw wem ids, and that is indistinguishable in the UI from a mapping
       that ran and found nothing - so log the inputs BEFORE any of the passes. */
    eprintln!(
        "[load_banks] in: events_bnk={:?} audio={:?} bin={:?} | events_bnk_read={} bin_candidates={}",
        file_name(bnk_path),
        file_name(wpk_path),
        file_name(bin_path),
        events_bnk_data.is_some(),
        bin_candidates.len(),
    );

    // First pass: BIN + events BNK mapping.
    if events_bnk_data.is_some() && !bin_candidates.is_empty() {
        for candidate in &bin_candidates {
            if let Ok(bin_data) = std::fs::read(candidate) {
                let mapped = resolve_mappings(&bin_data, events_bnk_data.as_deref());
                if !mapped.is_empty() {
                    mappings = mapped;
                    used_bin_path = candidate.clone();
                    break;
                }
            }
        }
    }

    // Second pass: plain BIN strings fallback.
    if mappings.is_empty() {
        for candidate in &bin_candidates {
            if let Ok(bin_data) = std::fs::read(candidate) {
                let mapped = resolve_mappings(&bin_data, None);
                if !mapped.is_empty() {
                    mappings = mapped;
                    used_bin_path = candidate.clone();
                    break;
                }
            }
        }
    }

    // Pick the audio source — WPK preferred, else BNK.
    let (entries, final_type) = if let Some(data) = read(wpk_path) {
        let entries = read_entries(&data)?;
        let kind = if !bnk_path.is_empty() {
            "bnk+wpk"
        } else {
            "wpk"
        };
        (entries, kind.to_string())
    } else if let Some(data) = read(bnk_path) {
        (read_entries(&data)?, "bnk".to_string())
    } else {
        (Vec::new(), String::new())
    };

    if entries.is_empty() {
        return Ok(None);
    }

    let source_name = if !wpk_path.is_empty() {
        file_name(wpk_path)
    } else if !bnk_path.is_empty() {
        file_name(bnk_path)
    } else {
        "root".to_string()
    };
    let original_path = if !wpk_path.is_empty() {
        wpk_path
    } else {
        bnk_path
    };

    let scope_key = sanitize_scope(if !original_path.is_empty() {
        original_path
    } else {
        &source_name
    });

    let audio_files: Vec<AudioData> = entries.iter().map(to_audio_data).collect();
    let file_count = audio_files.len();

    // The outcome, in the terms the tree is built from: mappings==0 means every node is
    // named by raw wem id, which is exactly the "random names" symptom.
    eprintln!(
        "[load_banks] out: kind={final_type} wems={} mappings={} used_bin={:?}",
        entries.len(),
        mappings.len(),
        file_name(&used_bin_path),
    );

    let mut tree = group_audio_files(&entries, &mappings, &source_name);
    scope_ids(&mut tree, &scope_key, &[]);

    tree.is_root = Some(true);
    tree.original_path = Some(original_path.to_string());
    tree.bnk_path = Some(bnk_path.to_string());
    tree.wpk_path = Some(wpk_path.to_string());
    tree.bin_path = Some(if !used_bin_path.is_empty() {
        used_bin_path
    } else if !bin_path.is_empty() && Path::new(bin_path).exists() {
        bin_path.to_string()
    } else {
        String::new()
    });

    Ok(Some(LoadBanksResult {
        tree,
        audio_files,
        file_count,
        kind: final_type,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_collapses_underscores() {
        assert_eq!(sanitize_scope("a//b\\c"), "a_b_c");
        assert_eq!(sanitize_scope("***"), "root");
    }

    #[test]
    fn skin_number_parsing() {
        assert_eq!(skin_seg_to_number("skin07"), Some(7));
        assert_eq!(skin_seg_to_number("base"), Some(0));
        assert_eq!(skin_seg_to_number("12"), Some(12));
    }

    #[test]
    fn groups_unmapped_audio_under_root() {
        let entries = vec![
            AudioEntry {
                id: 10,
                data: vec![1, 2, 3],
            },
            AudioEntry {
                id: 20,
                data: vec![4, 5],
            },
        ];
        let tree = group_audio_files(&entries, &[], "root");
        assert_eq!(tree.children.as_ref().unwrap().len(), 2);
    }
}
