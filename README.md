# Holster

A slim frosted tab on the right edge of your screen that opens into a **vertical dock for your own apps**. Each app chooses its icon, its color, a live **preview** that slides out next to its tile, and how it **opens**: as its own side panel (like Anotes) or as a window. Holster is only the frame.

It's made by the same hands as Anotes: same stack, same tokens, same materials, same springs.

## Using it

- **Open the dock**: rest the cursor on the tab for a moment, click it, or press `Ctrl+Alt+Space` (change it in Settings; if another app already uses it, Settings says so).
- **Close the dock**: click anywhere outside it, click its grip, press `Esc` or the shortcut again, or open an app. Moving the cursor away never closes it, so a mis-hover can't make it vanish under you.
- **Previews**: rest on a tile. The preview slides out from behind the dock, even when the app isn't running (apps can ship their preview as a page on disk). Move straight into it to use it (diagonal paths over other tiles are fine). Moving to another tile crossfades between previews. Apps without a preview show nothing.
- **More than five apps**: scroll the column with the wheel. It moves a whole tile at a time, with soft fades where more apps are hidden.
- **Move it**: drag the tab, or the dock's grip, up or down.
- **Settings**: the gear card below the dock, or the tray icon. The back arrow returns to the dock. Reorder apps by dragging, hide them from the dock, remove a manifest, or add one. Theme (System, Light, Dark), shortcut, launch at startup, hiding during fullscreen apps, and hover timing (under "Hover timing").
- **Game mode**: while a fullscreen app is in front, the tab tucks away and never takes focus. Activity glows wait until you're back.
- **Ambient signal**: when an app reports activity, the tab glows in that app's color (or its tile pulses while the dock is open).

## Adding apps

Apps register themselves by writing a manifest to `%APPDATA%\Holster\apps\<id>.json` when they start. An app's tile is drawn in its color with Holster's gloss, and carries either a Lucide icon or the app's own image at a chosen share of the tile (`"size": "80%"` by default). Holster validates every manifest and watches the folder live; a broken one shows up in Settings with the exact error.

The whole contract, the required stack, and the design language with exact values are in one skill: [`skill/holster-app/SKILL.md`](skill/holster-app/SKILL.md). Give it to any agent that supports Agent Skills when you build or adapt an app.

[`examples/hello-holster`](examples/hello-holster) is the smallest complete app: a live-clock preview (with a runtime resize), `window` open mode, and an activity signal on `Space`. Run it with `pnpm hello` while Holster is running.

## Local API

Loopback only, on a random free port written to `%APPDATA%\Holster\holster.json` (`version`, `port`, `pid`, `api`):

| | |
|---|---|
| `GET /status` | `{ "running": true, "version": "0.1.0" }` |
| `POST /apps/{id}/signal` | body `{ "kind": "activity" }`: tints the tab in the app's color |
| `POST /apps/{id}/reload` | re-reads the manifests, answers with that app's validation result |
| `GET /apps/{id}/preview/{file}` | serves an app's on-disk preview folder (its `preview.path`), so previews work while the app is closed |

Requests must name the loopback host (DNS-rebinding guard); browser requests must come from loopback origins.

## Data

Everything lives in `%APPDATA%\Holster\`: `settings.json` (theme, shortcut, dock order, hidden apps, tab position, hover timing), `apps\` (manifests) and `holster.json` (while running).

## Development

Prerequisites: Windows 11, [Rust](https://rustup.rs) (stable, MSVC), Node 20+, pnpm, and the Visual Studio C++ Build Tools. WebView2 ships with Windows 11.

```sh
pnpm install
pnpm tauri dev        # Vite + the app
pnpm hello            # the example app (registers itself with Holster)
pnpm tauri build      # installer at target/release/bundle/nsis/Holster_0.1.0_x64-setup.exe
cargo test -p holster # backend unit tests
```

To try things without touching your real setup, set `APPDATA` to a sandbox folder before starting Holster and the example. To inspect the UI, set `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` and open `http://127.0.0.1:9333`.

`pnpm icons` regenerates the Lucide data (`src/lib/lucide-nodes.json` for rendering, `src-tauri/src/lucide-names.txt` for manifest validation). `pnpm app-icon` regenerates every app icon size from `src-tauri/icons/holster-icon.svg` with headless Edge.

### Layout

```
src-tauri/src/
  lib.rs         app setup: windows, plugins, tray, hotkey, API, watchers
  shell.rs       the three windows: spring-animated native bounds, geometry, acrylic, focus
  apps.rs        manifests: parsing, validation, dock order, folder watcher
  health.rs      running / not running, polled only while something is open
  process.rs     launching apps, finding and focusing their windows
  api.rs         the loopback API and holster.json
  hooks.rs       click-away and Escape for the dock without taking focus
  fullscreen.rs  game mode (shared with Anotes)
  settings.rs    settings.json
src/             React + TypeScript UI (Framer Motion, dnd-kit, Lucide), one bundle for all windows
skill/holster-app/SKILL.md   the app contract and design language
examples/hello-holster/      the example app and test fixture
```

### How it's drawn

Like Anotes, every surface is a frameless, always-on-top window whose **bounds are the visible shape**, so the real acrylic blur, the Windows 11 rounded corners and hit-testing follow every frame and nothing transparent can swallow clicks. Holster has three: the **main** window (the tab, which morphs into the app card, or into the settings panel), the **settings card** below the dock, and the **preview card** to its left. Shape changes animate the native window rect with per-edge springs on a thread paced by `DwmFlush`; the UI lays out at the final size and is revealed, not reflowed.

The dock opens on hover, so it never takes keyboard focus from what you're typing in. While it's open, low-level mouse and keyboard hooks (installed only for that time) notice a click outside Holster and the Escape key. The settings panel takes focus like the Anotes panel and closes when you click away.

Previews are iframes laid over Holster's material: an app's page on disk (served by Holster) or its live loopback page while it runs. Every preview stays loaded while the dock is open (and for 20 seconds after), so moving between tiles is a crossfade, never a reload. The card slides out from behind the dock like a drawer, the page riding its leading edge. The settings and preview cards stay shown but parked off screen when unused (a freshly shown window draws a hollow frame for its first frames), and they carry the veil in their native material, so edges the webview hasn't repainted yet never flicker.
