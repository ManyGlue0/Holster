# Notes for Claude

## Releasing

Releases are signed for the in-app updater. Follow
[docs/RELEASING.md](docs/RELEASING.md) every time. In short:

- Bump the version in `package.json`, `src-tauri/tauri.conf.json` and
  `src-tauri/Cargo.toml`. Every build gets a new version.
- Build with `pnpm release:build`, not `pnpm tauri build`. It signs the
  installer and writes `latest.json`.
- Attach **both** the installer and `latest.json` to the GitHub release
  (`gh release create ... --target main --latest`). Without `latest.json`, the
  Settings → Updates button breaks for everyone.

The private key lives in `%USERPROFILE%\.tauri\holster.key`, outside the
repo. Its password is the user variable `HOLSTER_SIGNING_KEY_PASSWORD`, read
by the build script. Never print, copy, move or commit either; `.gitignore`
blocks `*.key` files as a safety net. The public key in `tauri.conf.json`
must never change unless the key pair is replaced (see the guide).

## Other conventions

- The README stays version-free: it links `releases/latest`.
- Never stop the user's installed Holster
  (`%LOCALAPPDATA%\Holster\holster.exe`). Test with a sandbox `APPDATA` and
  the `app.holster.dev` identifier (README → Development).
