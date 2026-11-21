# Install Crow

Crow runs on macOS 14 or later, on Apple Silicon and Intel. It is free software under the
GPL-3.0-or-later and its source is public. It is **not signed or notarized**: that needs a
paid Apple Developer ID, which the project does not have. macOS therefore blocks the first
open, once per installed version.

## From the DMG

1. Download `Crow-<version>.dmg` from the [Releases page](https://github.com/paraversal/file-crow/releases)
   and check it against the `.sha256` file next to it: `shasum -a 256 -c Crow-<version>.dmg.sha256`.
2. Open the DMG and drag **Crow** onto **Applications**.
3. Open Crow. macOS says it can't verify the app. Click **Done**.
4. Open **System Settings > Privacy & Security**, scroll to the Security section, click
   **Open Anyway** next to "Crow was blocked", and confirm with Touch ID or your password.
5. Open Crow again and choose **Open**. It starts normally from now on.

Terminal alternative for step 3 to 5, after copying the app to Applications:

```bash
xattr -dr com.apple.quarantine /Applications/Crow.app
```

## With Homebrew

```bash
brew install --cask paraversal/tap/crow
```

The cask installs the same unsigned app, so the first open is blocked the same way; follow
steps 3 to 5 above.

## Updating

Install the new version over the old one. Because the app is only ad hoc signed, macOS treats
each version as a new app: the first-open block comes back, and permissions you gave Crow
(Full Disk Access, Desktop, Documents, Downloads) must be granted again in
System Settings > Privacy & Security.

## Removing

Delete `Crow.app`. Settings, state, caches and logs are in `~/.config/crow`, `~/.local/state/crow`,
`~/Library/Caches/crow` and `~/Library/Logs/Crow`; delete those too for a clean removal.
