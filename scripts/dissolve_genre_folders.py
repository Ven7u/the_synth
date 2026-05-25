#!/usr/bin/env python3
"""
Dissolve genre-named folders (Electronic, Synthwave, Rock, Cinematic) into
role-based categories, adding a genre tag to each patch.
"""
import json
import os
import shutil

PATCHES_DIR = os.path.join(os.path.dirname(__file__), "..", "assets", "patches")

MOVES = {
    # (source_folder, filename): (new_category, genre_tag)
    # Electronic
    ("Electronic", "Acid Drop.json"):    ("Bass",  "electronic"),
    ("Electronic", "Berlin Pad.json"):   ("Pad",   "electronic"),
    ("Electronic", "Detroit Bass.json"): ("Bass",  "electronic"),
    ("Electronic", "Techno Stab.json"):  ("Pulse", "electronic"),
    # Synthwave
    ("Synthwave", "Kavinsky Lead.json"):   ("Lead", "synthwave"),
    ("Synthwave", "Neon Drive.json"):      ("Lead", "synthwave"),
    ("Synthwave", "Outrun Bass.json"):     ("Bass", "synthwave"),
    ("Synthwave", "Retrowave Keys.json"):  ("Keys", "synthwave"),
    # Rock
    ("Rock", "Blues Tone.json"):   ("Lead", "rock"),
    ("Rock", "Fuzz Face.json"):    ("Lead", "rock"),
    ("Rock", "Metal Zone.json"):   ("Lead", "rock"),
    ("Rock", "Power Lead.json"):   ("Lead", "rock"),
    # Cinematic — drone/ambient patches go to Ambient, rest to Pad
    ("Cinematic", "01 Titan Rising.json"):       ("Pad",    "cinematic"),
    ("Cinematic", "02 Void Walker.json"):         ("Ambient","cinematic"),
    ("Cinematic", "03 Cathedral.json"):           ("Pad",    "cinematic"),
    ("Cinematic", "04 Interstellar Drift.json"):  ("Ambient","cinematic"),
    ("Cinematic", "05 Dark Matter.json"):         ("Pad",    "cinematic"),
    ("Cinematic", "06 Ascension.json"):           ("Pad",    "cinematic"),
    ("Cinematic", "07 Frozen Tundra.json"):       ("Pad",    "cinematic"),
    ("Cinematic", "08 War Drums.json"):           ("Pad",    "cinematic"),
    ("Cinematic", "09 Distant Horizon.json"):     ("Pad",    "cinematic"),
    ("Cinematic", "10 Eternal Flame.json"):       ("Pad",    "cinematic"),
    ("Cinematic", "11 Ghost Protocol.json"):      ("Pad",    "cinematic"),
    ("Cinematic", "12 Requiem.json"):             ("Pad",    "cinematic"),
    ("Cinematic", "13 Solar Wind.json"):          ("Pad",    "cinematic"),
    ("Cinematic", "14 Ancient Ruins.json"):       ("Pad",    "cinematic"),
    ("Cinematic", "15 Singularity.json"):         ("Pad",    "cinematic"),
    ("Cinematic", "16 Ember.json"):               ("Pad",    "cinematic"),
    ("Cinematic", "17 Leviathan.json"):           ("Pad",    "cinematic"),
    ("Cinematic", "18 Celestial Gate.json"):      ("Pad",    "cinematic"),
    ("Cinematic", "19 Last Rites.json"):          ("Pad",    "cinematic"),
    ("Cinematic", "20 Omega Point.json"):         ("Pad",    "cinematic"),
}

moved = 0
for (src_folder, filename), (new_category, genre_tag) in MOVES.items():
    src = os.path.join(PATCHES_DIR, src_folder, filename)
    dst_dir = os.path.join(PATCHES_DIR, new_category)
    dst = os.path.join(dst_dir, filename)

    if not os.path.exists(src):
        print(f"MISSING: {src}")
        continue

    os.makedirs(dst_dir, exist_ok=True)

    with open(src) as f:
        patch = json.load(f)

    patch["category"] = new_category
    tags = patch.setdefault("tags", [])
    if genre_tag not in tags:
        tags.append(genre_tag)

    with open(dst, "w") as f:
        json.dump(patch, f, indent=4)
        f.write("\n")

    os.remove(src)
    moved += 1
    print(f"  {src_folder}/{filename} → {new_category} (+{genre_tag})")

# Remove now-empty genre folders
for folder in ("Electronic", "Synthwave", "Rock", "Cinematic", "Synth"):
    folder_path = os.path.join(PATCHES_DIR, folder)
    if os.path.isdir(folder_path):
        remaining = os.listdir(folder_path)
        if not remaining:
            os.rmdir(folder_path)
            print(f"Removed empty folder: {folder}/")
        else:
            print(f"WARNING: {folder}/ not empty: {remaining}")

print(f"\nDone — {moved} patches moved.")
