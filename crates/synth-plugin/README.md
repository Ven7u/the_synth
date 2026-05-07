# Installing synth-plugin in Ableton Live (macOS)

You should have received a file called **`synth-plugin.vst3`** (it looks like a single file but is actually a folder bundle — that's normal). If it came as a `.zip`, double-click to unzip it first.

## 1. Put the plugin in the right folder

1. Open **Finder**.
2. In the menu bar, click **Go → Go to Folder…** (or press ⇧⌘G).
3. Paste this path and hit Return:

   ```
   ~/Library/Audio/Plug-Ins/VST3
   ```

   If the folder doesn't exist, create it.
4. Drag **`synth-plugin.vst3`** into that folder.

## 2. Remove the macOS quarantine flag (important!)

Because the plugin wasn't downloaded from the App Store, macOS will silently block it. You need to clear the quarantine flag once.

1. Open the **Terminal** app (⌘Space, type "Terminal", press Return).
2. Paste this command and hit Return:

   ```sh
   xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/VST3/synth-plugin.vst3
   ```

3. No output means it worked. You can close Terminal.

> Skip this step and the plugin will not show up in Ableton, or it will show up but fail to load with no clear error.

## 3. Make Ableton see the plugin

1. Open **Ableton Live**.
2. Go to **Settings** (⌘,) → **Plug-Ins**.
3. Make sure **Use VST3 Plug-In System Folders** is turned **On**.
4. Click **Rescan**. (If nothing shows up, hold **⌥ Option** while clicking Rescan to force a full rescan.)

## 4. Use it

1. In the left-hand browser, click **Plug-Ins**.
2. Open **VST3** and find **synth-plugin**.
3. Drag it onto a MIDI track.
4. Arm the track for recording (the small circle button), play some MIDI notes, and you should hear sound.

## Updating to a new version

When you receive a new build:

1. Quit Ableton.
2. Replace the old `synth-plugin.vst3` in `~/Library/Audio/Plug-Ins/VST3/` with the new one.
3. Re-run the `xattr` command from step 2 above.
4. Reopen Ableton — no rescan needed unless something major changed.

## Troubleshooting

- **Plugin doesn't appear in the browser.** Quit Ableton fully (⌘Q, not just close the window) and reopen. If still missing, hold ⌥ while clicking **Rescan**.
- **Plugin appears but the track stays silent / Ableton shows an error loading it.** You almost certainly skipped step 2. Run the `xattr` command, then quit and reopen Ableton.
- **"synth-plugin.vst3 cannot be opened because the developer cannot be verified."** Same fix — run the `xattr` command in step 2.

If something else goes wrong, send a screenshot of Ableton's **Settings → Plug-Ins** screen plus a short description of what you tried.
