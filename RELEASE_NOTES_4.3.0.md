# 4.3.0 fixes Repath losing body and weapon textures, and makes an unpacked modpkg a standard mod project.

## What's new

- Unpacking a `.modpkg` now produces a standard mod project: `mod.config.json` at the top, the content under `content/<layer>/<wad>/`, and any hashtables the package declares under `hashes/`. It is the same layout Celestial and league-mod use, so a folder Quartz unpacks packs in either of them, and a folder they unpacked packs in Quartz.
- "Pack Modpkg" reads that layout back. Folders unpacked by an older Quartz still pack too.
- "Convert to .modpkg" (from a `.fantome` or a `.wad.client`) now writes the package's `game` hashtable, which names the chunks whose path the package cannot store directly, so other tools can put a name to them. A name from `files.txt` is only used when it really belongs to that chunk.

## Improvements

- The Asset Extractor no longer drops an asset just because the installed hash list has not caught up with the skin. When a bin names the file, that name is used directly, so a newly released skin extracts in full before the hashes update. The log says when this happened so you know to redownload them.
- When a bin cannot be read during extraction, the log now says so and what was lost, instead of quietly leaving files out.
- Hex-named chunks in an unpacked modpkg still get a real extension, and still repack to the same chunk.

## Fixes

- Fixed Repath in the Asset Extractor leaving out body and weapon textures. When an effect used the same texture as the model, the step that gathers effect textures into the skin's particles folder took the model's copy with it, and the model was left pointing at a file that no longer existed. Qiyana's weapon and Revenant Reign Viego's body were the reported cases. Taliyah was never affected, which is why it was hard to pin down. The Quick Repath wizard had the same problem and is fixed the same way.
- Fixed TFT companion extraction skipping files a pet's effects use from another character's folder in Companions.wad.

Thank you for testing Quartz. Please report any remaining issues on GitHub.
