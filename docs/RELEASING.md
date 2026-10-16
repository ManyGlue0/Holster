# Releasing Holster

Every release ships two files on GitHub: the installer and `latest.json`. The
installer is what people download by hand; `latest.json` is what the
**Settings → Updates** button reads. This page is how that works, how it was
set up, and the steps for each release.

## How the updater works

Holster uses [`tauri-plugin-updater`](https://v2.tauri.app/plugin/updater/)
(code in `src-tauri/src/update.rs`, the UI in the Updates card in
`src/components/Settings.tsx`).

1. **Check.** Holster downloads
   `https://github.com/ManyGlue0/Holster/releases/latest/download/latest.json`
   (the endpoint in `src-tauri/tauri.conf.json` → `plugins.updater`) and
   compares its `version` with its own.
2. **Verify.** If it's newer, Holster downloads the installer named in `url`
   and checks it against `signature`, using the **public key** built into
   every Holster (`plugins.updater.pubkey`). A file that doesn't match is
   refused: "The signature verification failed".
3. **Install.** Holster tidies up (discovery file, tray icon), exits, and the
   installer runs in passive mode (a progress bar, no questions). It starts
   the new Holster when it's done. Apps and settings are kept.

The signature comes from a **private key** that never leaves the
maintainer's PC. Anyone can check a signature with the public key; only the
private key can make one. So even someone who took over the GitHub account
couldn't push an update Holster would install without that key file.

| | Where | Changes |
|---|---|---|
| Private key | `%USERPROFILE%\.tauri\holster.key`, password-protected, plus a backup | Never |
| Public key | `tauri.conf.json` → `plugins.updater.pubkey`, so inside every installer | Never |
| Signature | `latest.json` on each release (the build also writes it to a `.sig` file) | Every build |

The key pair is Holster's own (one key per product). It is not Windows code
signing: SmartScreen never sees it, and it costs nothing.

## One-time setup (done on 2026-10-17)

1. **Make the key pair**, in your own terminal so the password stays yours:

   ```powershell
   cd E:\GitProjects\Holster
   pnpm tauri signer generate -w "$env:USERPROFILE\.tauri\holster.key"
   ```

   It asks for a password and writes `holster.key` (private) and
   `holster.key.pub` (public) to `%USERPROFILE%\.tauri\`, outside the repo.
   Holster's key id is `AF7E47BC5F4CA5B1`.

2. **Back up `holster.key` and its password** (a password manager, a backup
   drive). If either is lost, no installed Holster can be updated by the
   button again: everyone would have to install a new version, carrying a new
   public key, by hand.

3. **Put the public key in Holster:** the one line in `holster.key.pub` goes
   in `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`.

4. **Save the password for release builds** as a Windows user variable. This
   prompt keeps it out of PowerShell's history file:

   ```powershell
   $p = Read-Host "Key password" -AsSecureString
   [Environment]::SetEnvironmentVariable("HOLSTER_SIGNING_KEY_PASSWORD", [Net.NetworkCredential]::new("", $p).Password, "User")
   Remove-Variable p
   ```

   `"User"` is the scope (your Windows account), not a name to replace.
   Check without showing it:
   `[bool][Environment]::GetEnvironmentVariable("HOLSTER_SIGNING_KEY_PASSWORD", "User")`
   → `True`. Optional: `HOLSTER_SIGNING_KEY` if the key ever lives somewhere
   other than the default path. Tauri's generic
   `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` work as a fallback.

`.gitignore` refuses `*.key` and `*.key.pub`, so a key copied into the
repo by mistake can't be committed.

## Each release

1. **Bump the version** in all three places: `package.json`,
   `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`. Every build gets a
   new version, even a small fix: the updater only installs a higher one.

2. **Build, signed:**

   ```powershell
   pnpm release:build
   ```

   That's `scripts/release-build.mjs`: it reads the key and password (from
   the shell or the user variables), runs `pnpm tauri build` with
   `src-tauri/tauri.release.conf.json` (which turns on
   `createUpdaterArtifacts`), and writes, in `target\release\bundle\nsis\`:

   - `Holster_<version>_x64-setup.exe`, the installer
   - `Holster_<version>_x64-setup.exe.sig`, its signature
   - `latest.json`, with the version, the release download URL and that signature

   `pnpm release:build -- --notes notes.md` also puts a file's text in
   `latest.json` as the update's notes. A plain `pnpm tauri build` still works
   for anyone without the key; it just isn't signed for the updater.

3. **Publish both files**, marked latest:

   ```powershell
   gh release create v<version> `
     target/release/bundle/nsis/Holster_<version>_x64-setup.exe `
     target/release/bundle/nsis/latest.json `
     --target main --title "Holster <version>" --notes-file notes.md --latest
   ```

   (`--target main`, not a short commit hash: gh rejects those.) A release
   without `latest.json` makes the Updates card report an error, because
   `releases/latest/download/latest.json` would be missing.

4. **Check it from outside:**
   `releases/latest/download/latest.json` should show the new version, and
   the downloaded installer should be byte-identical to the local one.

## Testing the updater without touching a real install

Build throwaway versions under another name, so they install to their own
folder and keep their own data: `--config` with `productName`
`HolsterUpdateTest`, identifier `app.holster.updtest`, versions `0.0.1` and
`0.0.2`, a throwaway key (`pnpm tauri signer generate --ci -p "" -w test.key`),
and `plugins.updater` pointing at `http://127.0.0.1:<port>/latest.json` with
`dangerousInsecureTransportProtocol: true`. Serve the 0.0.2 installer and a
`latest.json` from a local folder, install 0.0.1 with `/S`, and drive
`update_check` / `update_install` over CDP. Holster's data folder is named
after the product (`%APPDATA%\<productName>`), which is what keeps a test copy,
relaunched by the installer with the normal environment, away from the real
`%APPDATA%\Holster`. The installer only closes a running copy from its own
install folder. Afterwards run its `uninstall.exe /S` and delete
`%APPDATA%\HolsterUpdateTest`, `%LOCALAPPDATA%\app.holster.updtest` and its
`HolsterUpdateTest` autostart entry.

Checked this way on 2026-10-17: a real update (0.0.1 → 0.0.2, back up in
about a second), a tampered signature (refused, old version kept running),
and "already up to date".

## If something goes wrong

- **"Couldn't check for updates"**: the latest release is missing
  `latest.json`, or there's no network. Details are in
  `%APPDATA%\Holster\holster.log`.
- **The release build stops at signing**: the password variable is missing
  or wrong. Set it again (step 4 above).
- **The private key is lost or leaked**: make a new pair, put the new public
  key in `tauri.conf.json`, and release. Installed copies still trust only
  the old key, so tell people to install that release by hand once. After a
  leak, also delete the old key everywhere.
