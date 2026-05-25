#!/usr/bin/env python3
"""
Retag all patches in assets/patches/**/*.json:
  - Reclassify artist-folder patches (Eno, PinkFloyd, etc.) to proper sound-type categories
  - Add 'inspired_by_*' tags for those patches
  - Add character / timbre tags to every patch based on name + category keywords
"""

import json, os, re, sys
from pathlib import Path

PATCHES_DIR = Path(__file__).parent.parent / "assets" / "patches"

# ---------------------------------------------------------------------------
# Artist-folder → (new_category, artist_tag) overrides
# Patches in these folders get their category changed and an artist tag added.
# ---------------------------------------------------------------------------
ARTIST_REMAP = {
    "Eno": {
        "Airport Shimmer":      ("Ambient",  "eno"),
        "Ambient Tape Bass":    ("Bass",     "eno"),
        "Discreet Music Bell":  ("Pluck",    "eno"),
        "Fourth World Choir":   ("Pad",      "eno"),
        "Oblique Pad":          ("Pad",      "eno"),
        "Tape Loop Drone":      ("Ambient",  "eno"),
    },
    "PinkFloyd": {
        "Comfortably Numb Choir": ("Pad",    "pink-floyd"),
        "Dark Side Bass":         ("Bass",   "pink-floyd"),
        "Division Bell Synth":    ("Lead",   "pink-floyd"),
        "Echoes Underwater":      ("Ambient","pink-floyd"),
        "Gilmour Lead":           ("Lead",   "pink-floyd"),
        "Shine On Pad":           ("Pad",    "pink-floyd"),
    },
    "Frahm": {
        "Felt Bass":         ("Bass",    "frahm"),
        "Felt Piano":        ("Keys",    "frahm"),
        "Keys Tail":         ("Keys",    "frahm"),
        "Mechanical Breath": ("Ambient", "frahm"),
    },
    "Zimmer": {
        "Endurance Pad":     ("Pad",     "zimmer"),
        "Interstellar Bass": ("Bass",    "zimmer"),
        "Organ Gravity":     ("Keys",    "zimmer"),
        "String Cluster":    ("Pad",     "zimmer"),
        "Sub Pulse Drone":   ("Ambient", "zimmer"),
        "Wormhole Texture":  ("Ambient", "zimmer"),
    },
    "Glass": {
        "Ostinato Bass": ("Bass", "glass"),
    },
    "Wagner": {
        "Tristan Bass": ("Bass", "wagner"),
    },
}

# ---------------------------------------------------------------------------
# Keyword → tags mapping (applied to lowercased patch name + category)
# Order matters: first match wins for conflicting tags, but all rules run.
# ---------------------------------------------------------------------------
KEYWORD_RULES = [
    # Timbre / synthesis type
    (r"\bdx7\b",            ["fm", "digital", "bright"]),
    (r"\bfm\b",             ["fm", "digital"]),
    (r"\bring\b",           ["bell", "digital", "glitchy"]),
    (r"\bnoise\b",          ["noise", "raw"]),
    (r"\bsupersaw\b",       ["digital", "lush", "bright"]),
    (r"\bsync\b(?!\s+lead)",["digital", "bright", "aggressive"]),
    (r"\bsquare\b",         ["analog", "bright"]),
    # Character
    (r"\bdark\b",           ["dark"]),
    (r"\bbright\b",         ["bright"]),
    (r"\bwarm\b",           ["warm"]),
    (r"\bcold\b",           ["cold"]),
    (r"\blush\b",           ["lush"]),
    (r"\braw\b",            ["raw"]),
    (r"\bsoft\b",           ["soft"]),
    (r"\baggressive\b",     ["aggressive"]),
    (r"\bglitch\b",         ["glitchy", "digital"]),
    # Movement / texture
    (r"\bdrone\b",          ["drone", "static"]),
    (r"\bevol",             ["evolving"]),
    (r"\bpuls",             ["pulsing", "rhythmic"]),
    (r"\btremolo\b",        ["pulsing", "evolving"]),
    (r"\btexture\b",        ["evolving", "drone"]),
    (r"\bloop\b",           ["evolving", "drone"]),
    (r"\btape\b",           ["analog", "warm", "evolving"]),
    (r"\becho\b",           ["long-release", "evolving"]),
    (r"\bshimmer\b",        ["bright", "evolving", "long-release"]),
    (r"\baurora\b",         ["evolving", "lush", "bright"]),
    (r"\bdrift\b",          ["evolving", "ambient"]),
    (r"\bsolar\b",          ["bright", "evolving"]),
    (r"\bspace\b",          ["dark", "evolving", "drone"]),
    (r"\bvoid\b",           ["dark", "drone", "evolving"]),
    (r"\bworm",             ["dark", "evolving", "digital"]),
    (r"\bphantom\b",        ["dark", "digital", "evolving"]),
    (r"\bghost\b",          ["dark", "cold", "digital"]),
    # Instrument/timbre type
    (r"\bbell\b",           ["bell", "bright"]),
    (r"\bharp\b",           ["plucked", "bright", "bell"]),
    (r"\bchoir\b",          ["choir", "lush"]),
    (r"\bstring",           ["strings", "lush"]),
    (r"\bpiano\b",          ["soft", "analog"]),
    (r"\bfelt\b",           ["soft", "analog"]),
    (r"\borgan\b",          ["analog", "warm"]),
    (r"\bbrass\b",          ["analog", "bright"]),
    # Synth brand → analog feel
    (r"\bjuno\b",           ["analog", "warm", "lush"]),
    (r"\bminimoog\b",       ["analog", "warm"]),
    (r"\bmoog\b",           ["analog", "warm"]),
    (r"\bob[-\s]",          ["analog", "lush", "warm"]),
    (r"\bprophet\b",        ["analog", "warm"]),
    (r"\btaurus\b",         ["analog", "dark", "drone"]),
    (r"\barp\b",            ["analog", "warm"]),
    (r"\bbrute\b",          ["analog", "raw", "aggressive"]),
    (r"\bserum\b",          ["digital", "bright"]),
    (r"\bvital\b",          ["digital", "bright"]),
    (r"\bprologue\b",       ["digital", "bright"]),
    (r"\bsub 37\b",         ["analog", "warm"]),
    # Attack / envelope feel
    (r"\bstab\b",           ["short-attack", "rhythmic"]),
    (r"\bclick\b",          ["short-attack", "percussive"]),
    (r"\bpluck\b",          ["plucked", "short-attack"]),
    (r"\bgravity\b",        ["evolving", "lush"]),
    # Context
    (r"\bacid\b",           ["analog", "aggressive"]),
    (r"\bfuzz\b",           ["aggressive", "raw", "analog"]),
    (r"\bmetal\b",          ["aggressive", "dark"]),
    (r"\bpower\b",          ["aggressive", "bright"]),
    (r"\blaser\b",          ["digital", "bright", "glitchy"]),
    (r"\br2d2\b",           ["digital", "glitchy", "bright"]),
    (r"\bgrowl\b",          ["aggressive", "digital"]),
    (r"\bneon\b",           ["bright", "digital"]),
    (r"\bkavinsky\b",       ["bright", "analog"]),
    (r"\bberlin\b",         ["cold", "digital", "evolving"]),
    (r"\bdetroit\b",        ["dark", "rhythmic"]),
    (r"\btechno\b",         ["digital", "aggressive", "rhythmic"]),
    (r"\bsynthwave\b",      ["bright", "analog"]),
    (r"\bretrowave\b",      ["bright", "digital"]),
    (r"\boutrun\b",         ["warm", "analog"]),
    (r"\bmidnight\b",       ["dark", "analog"]),
    (r"\bnight\b",          ["dark", "evolving"]),
    (r"\bmorning\b",        ["bright", "warm", "soft"]),
    (r"\bcrystal\b",        ["bright", "bell", "digital"]),
    (r"\bdeep\b",           ["dark", "analog"]),
    (r"\bstill\b",          ["drone", "soft"]),
    (r"\bsub\b",            ["dark", "analog"]),
    (r"\bwobble\b",         ["pulsing", "aggressive"]),
    (r"\bspectral\b",       ["digital", "evolving"]),
    (r"\bvoltage\b",        ["digital", "aggressive"]),
    (r"\bblues\b",          ["warm", "analog"]),
]

# Category-level character defaults (added unless already present)
CATEGORY_DEFAULTS = {
    "Bass":     [],
    "Lead":     [],
    "Pad":      ["long-release"],
    "Ambient":  ["long-release", "evolving"],
    "Cinematic":["long-release", "lush"],
    "Keys":     [],
    "Pluck":    ["plucked", "short-attack"],
    "Pulse":    ["short-attack", "rhythmic"],
    "Brass":    ["analog", "bright"],
    "FX":       ["digital"],
    "Electronic":["digital", "rhythmic"],
    "Synthwave":["bright"],
    "Rock":     ["analog", "aggressive"],
}

def name_tags(name: str, category: str) -> list[str]:
    text = (name + " " + category).lower()
    tags = []
    for pattern, add in KEYWORD_RULES:
        if re.search(pattern, text):
            for t in add:
                if t not in tags:
                    tags.append(t)
    # Category defaults
    for t in CATEGORY_DEFAULTS.get(category, []):
        if t not in tags:
            tags.append(t)
    return tags


def process_file(path: Path) -> bool:
    """Return True if file was modified."""
    raw = path.read_text(encoding="utf-8")
    try:
        patch = json.loads(raw)
    except json.JSONDecodeError as e:
        print(f"  SKIP (parse error): {path.name}: {e}", file=sys.stderr)
        return False

    folder = path.parent.name   # e.g. "Eno", "Bass", "Lead"
    original_name = patch.get("name", path.stem)
    original_category = patch.get("category", folder)

    tags: list[str] = list(patch.get("tags", []))  # preserve existing
    artist_tag = None

    # Artist remap
    if folder in ARTIST_REMAP:
        mapping = ARTIST_REMAP[folder]
        key = original_name
        if key in mapping:
            new_cat, artist_tag = mapping[key]
            patch["category"] = new_cat
        else:
            # Unmapped artist patch: keep category, add folder as tag
            artist_tag = folder.lower().replace(" ", "-")

    # Add artist tag
    if artist_tag and artist_tag not in tags:
        tags.insert(0, artist_tag)

    # Add character/timbre tags
    final_category = patch.get("category", original_category)
    for t in name_tags(original_name, final_category):
        if t not in tags:
            tags.append(t)

    patch["tags"] = tags

    new_raw = json.dumps(patch, indent=4, ensure_ascii=False)
    if new_raw.strip() == raw.strip():
        return False
    path.write_text(new_raw + "\n", encoding="utf-8")
    return True


def main():
    files = sorted(PATCHES_DIR.rglob("*.json"))
    print(f"Processing {len(files)} patches under {PATCHES_DIR}...")
    changed = 0
    for f in files:
        if process_file(f):
            changed += 1
            print(f"  updated: {f.relative_to(PATCHES_DIR)}")
    print(f"\nDone — {changed}/{len(files)} files updated.")


if __name__ == "__main__":
    main()
