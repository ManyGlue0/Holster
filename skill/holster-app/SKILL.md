---
name: holster-app
description: Use when making a new app, or adapting an existing one, so it shows up in the Holster dock (manifest, preview page, open route, activity signals) and looks and feels like it belongs in Holster.
---

# Building a Holster app

Holster is a Windows 11 dock that lives on the right screen edge as a slim frosted tab. Hovering it opens a column of app tiles; hovering a tile shows that app's **preview** to the left of the dock; clicking a tile **opens** the app. Holster is only the frame. Each app decides its icon, its color, its preview and how it opens, through one small manifest.

This file is everything you need: the contract, the required stack, the design language with real values, the taste rules, and how to ship each build. Holster itself and its example app (`examples/hello-holster`) follow all of it.

## Checklist

1. On start, check that `%APPDATA%\Holster\` exists (Holster is installed). If it does, write your manifest to `%APPDATA%\Holster\apps\<id>.json` (temp file, then rename).
2. Serve `/holster/health` and (panel apps) `/holster/open` on a **loopback-only** listener (`127.0.0.1`).
3. If your app has something worth a glance, ship a preview page **on disk** (`preview.path`) so it shows even when the app is closed, and keep its data in a file next to it. Make it transparent, theme-aware, status-aware and sized to its content (1.5). No preview is fine too: the tile then shows nothing on hover.
4. Panel apps: answer `POST /holster/open` fast, then slide your panel out. Window apps: be single-instance.
5. Optional: `POST` an `activity` signal to Holster when something meaningful happens.
6. Optional: show a notification when something needs the user now (1.8), and hear back on `notify.url`.
7. Check your app in Holster's Settings, Apps: no error, no warnings, status "Running".
8. Finish every round of changes with a new version and a release build, and tell the user the installer's full path (5).

---

## 1. The contract

### 1.1 Discovery

Holster writes `%APPDATA%\Holster\holster.json` while it runs (and removes it on exit):

```json
{ "version": "0.1.0", "port": 52710, "pid": 9192, "api": "http://127.0.0.1:52710" }
```

The port is random on every start. Read this file each time you need the API; don't cache the port across Holster restarts.

### 1.2 Registration

One manifest per app at `%APPDATA%\Holster\apps\<id>.json`. The file name **must** equal the manifest's `id`.

- Write it every time your app starts: ports and the exe path can change. Holster watches the folder and picks up added, changed and removed manifests live.
- Write atomically (write `.<id>.json.tmp`, then rename over the real file) so Holster never reads half a file. Skip the write if the content is unchanged.
- Only write it if `%APPDATA%\Holster\` exists. Don't install Holster's folder yourself.
- Users can also add a manifest from Holster's Settings ("Add app"); that copies the file into the folder.
- Order and visibility in the dock belong to the user and live in Holster's settings, never in your manifest.

### 1.3 Manifest

```json
{
  "schema": 1,
  "id": "notes",
  "name": "Notes",
  "icon": { "lucide": "notebook-pen" },
  "color": "#B8A9F0",
  "health": "http://127.0.0.1:47321/holster/health",
  "preview": { "path": "C:\\Path\\To\\Notes\\holster-preview\\index.html", "width": 320, "height": 260 },
  "open": { "mode": "panel", "url": "http://127.0.0.1:47321/holster/open" },
  "launch": { "exe": "C:\\Path\\To\\Notes.exe", "args": [] }
}
```

| Field | Required | Rules |
|---|---|---|
| `schema` | yes | `1`. |
| `id` | yes | Lowercase letters, digits, `-`, `_`, `.`; starts with a letter or digit; max 64. Equals the file name. |
| `name` | yes | Shown in Settings (and on the card that explains a failed open). |
| `icon` | yes | Either `{ "lucide": "<name>" }`: a [lucide.dev](https://lucide.dev/icons) icon (validated), drawn by Holster on a tile in your `color`. Or `{ "path": "<absolute path>", "size": "80%" }`: your own image (`.png`, `.svg`, `.jpg`, `.webp` or `.ico`, at most 1 MB) drawn on the same tile (your `color`, gloss and hairline stay). `size` is how much of the tile it covers, as a percentage from `"10%"` to `"100%"` (a bare `80` also reads as a percentage; never pixels); default `"80%"`. Use a square image with transparent background, at least 96 px. |
| `color` | yes | `#RGB` or `#RRGGBB`, or a palette name (`mint`, `peach`, `lavender`, `sky`, `butter`, `rose`, `sage`, `lilac`, `apricot`, `mist`). It fills the tile, draws the hover halo, and tints the tab on signals. Pick a soft pastel (see 3.2). |
| `health` | no | `GET` returns 2xx while the app runs. Strongly recommended. |
| `preview` | no | `path`: absolute path to a preview page on disk (preferred: shows whether or not the app runs). `url`: a live page served by the running app (used while it runs; without a `path`, nothing shows while it's stopped). At least one of them. `width` 140–720 and `height` 64–720 CSS px (defaults 320×240; out-of-range values are clamped with a warning). No preview: the tile shows nothing on hover. |
| `open.mode` | yes | `"panel"` (needs `open.url`) or `"window"` (needs `launch`). |
| `open.url` | panel | Holster `POST`s here to open your panel. |
| `launch` | window; recommended for panel | `exe`: absolute path. `args`: list of strings. Holster starts it when the app isn't running. |
| `notify.url` | no | Holster `POST`s here what happened to your notifications (1.8). |

**Every URL must be `http://127.0.0.1[:port]/...`.** `localhost`, other hosts and `https` are rejected. A broken manifest never breaks Holster: it shows in Settings with the exact error (bad JSON, mismatched id, unknown icon, non-loopback URL, missing field). Warnings (missing exe, missing preview file, clamped size, panel without launch) show in amber.

After writing the manifest you can ask Holster to validate it immediately: `POST {api}/apps/<id>/reload` answers `{ "ok": true, "warnings": [] }` or `422 { "ok": false, "error": "..." }`.

### 1.4 How Holster decides "running"

Polled once at start, on every manifest change, and every 2.5 s only while the dock or Settings is open (0.9 s timeout per check):

1. `health` returns 2xx, else
2. a process whose image path equals `launch.exe` is running, else
3. the `preview.url` returns 2xx, else
4. unknown ("No status" in Settings).

A running app gets a small dot beside its tile. Your preview page is told too (1.5), so it can show its own state.

### 1.5 The preview page

Shown in an iframe inside a card Holster draws (its material, hairline border, radius and shadow). Your page draws only the content.

**Two kinds; use either or both:**

- **On disk (`preview.path`), preferred.** Holster serves the page's folder itself, loopback only, at `http://127.0.0.1:<holster port>/apps/<id>/preview/<file>`, so the preview shows **whether or not your app is running**. Relative assets in that folder (CSS, JS, images, fonts, JSON) work; nothing outside it is served. Ship the page with your app (next to the exe or in its data folder). Keep what it shows in a data file beside it, such as `state.json`, which your app rewrites atomically when something changes. The page fetches it on load and on the `show` message.
- **Live (`preview.url`).** Served by your running app on loopback. Holster uses it while the app runs and falls back to the `path` page otherwise. Without a `path`, a stopped app shows nothing on hover.

Rules for both:

- **Transparent background**: `html, body { background: transparent; }`. Never paint your own card, border, radius or shadow.
- **Color scheme must match the theme.** Set `color-scheme: light` or `dark` on `:root` to follow the theme. If it differs from Holster's, Chromium paints an opaque backdrop behind your page.
- **Query parameters**: `?theme=light|dark&running=1|0`. Apply both on load.
- **Messages from Holster** (`message` events on `window`). Apply them without reloading:
  - `{ "holster": "theme", "value": "dark" }`: the theme changed.
  - `{ "holster": "status", "running": true }`: your app started or stopped.
  - `{ "holster": "show" }`: the card is coming into view; refresh your data now.
- **Size**: the manifest's `width`×`height` is the card size. To change it at runtime, post to the parent: `parent.postMessage({ holster: "resize", width: 320, height: 300 }, "*")`. Holster springs the card to the new size and keeps it on screen. Size to your content: never scroll, never clip.
- **Motion is Holster's**: the card slides out from behind the dock like a drawer, and your page rides its leading edge, fading in; on close it slides back and fades. Don't add your own entrance animation.
- **Light and fast**: Holster preloads every preview while the dock is open and keeps the pages alive for 20 s after it closes, so moving between tiles is a crossfade, never a reload. Render from local data; no spinners on hover; no heavy work on load. A page that hasn't loaded after 2.5 s is skipped.
- **Sandbox**: `allow-scripts allow-same-origin allow-forms`. No popups, no top-level navigation. Interactive controls are fine (it's your UI), but anything bigger belongs in your panel or window.
- The preview hides when the cursor leaves both its tile and the card (after a 250 ms grace). Don't rely on hover-only states that need longer.

Template:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <style>
      :root { --text: #1a1a1f; --text-2: #55555f; --text-3: #8b8b95; color-scheme: light; }
      :root[data-theme="dark"] { --text: #f2f2f5; --text-2: #b4b4be; --text-3: #85858f; color-scheme: dark; }
      html, body { margin: 0; height: 100%; overflow: hidden; background: transparent; color: var(--text);
        font: 13.5px/1.45 "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
        -webkit-font-smoothing: antialiased; user-select: none; cursor: default; }
      main { padding: 16px 18px 14px; }
      .label { font-size: 12px; font-weight: 600; color: var(--text-3); letter-spacing: 0.01em; }
    </style>
  </head>
  <body>
    <main><div class="label">Your app</div><div id="body"><!-- one glanceable thing --></div></main>
    <script>
      const q = new URLSearchParams(location.search);
      const setTheme = (t) => (document.documentElement.dataset.theme = t === "dark" ? "dark" : "light");
      const setRunning = (on) => document.body.classList.toggle("running", on);
      async function refresh() {
        // On-disk previews: read the snapshot your app keeps next to this page.
        const state = await fetch("state.json", { cache: "no-store" }).then((r) => r.json()).catch(() => null);
        /* render state into #body */
      }
      setTheme(q.get("theme"));
      setRunning(q.get("running") === "1");
      refresh();
      addEventListener("message", (e) => {
        if (e.data?.holster === "theme") setTheme(e.data.value);
        if (e.data?.holster === "status") setRunning(e.data.running);
        if (e.data?.holster === "show") refresh();
      });
    </script>
  </body>
</html>
```

### 1.6 Opening

**`panel`**: your app slides out its own side panel (see 3.8).

- Holster `POST`s `open.url` with JSON `{ "edge": "right", "anchorY": 433, "tabY": 512 }`, both in **physical screen pixels**:
  - `anchorY` is the vertical center of the clicked tile. **Open from it**: the panel unfolds out of the tile that was clicked.
  - `tabY` is the vertical center of Holster's tab, which is where the tab comes back once your panel is gone. **Close toward it**, not back to `anchorY`, so the panel folds into the edge exactly where the tab reappears. Read it on every open: the user can move the tab between opens.
  - A full-height panel that doesn't morph from a point can ignore both.
- Answer `2xx` immediately (Holster waits at most 4 s), then open the panel on your UI thread.
- Holster calls `AllowSetForegroundWindow(ASFW_ANY)` right before the request, so you may take the foreground. Still take focus the robust way (attach to the foreground thread's input, `BringWindowToTop`, `SetForegroundWindow`), as Holster does for window apps.
- While your panel is out, Holster's tab steps aside: the dock folds straight into the edge and the tab stays tucked until the panel is gone, then slides back. Holster recognizes the panel as a visible window of the process serving `open.url` (or of `launch.exe`) whose right side is within 40 px of the screen edge, wider than 160 px, overlapping the work area. Park or hide the panel when it closes; a slim tab of your own on the edge doesn't count. If no such window shows up within 3 s, the tab comes back.
- Not running: Holster runs `launch.exe`, waits up to 20 s for `health`, then calls `open.url`. Make `health` answer as soon as the open route works.
- Failures show on a small card next to the tile ("didn't respond to open", "couldn't open its panel (500)"). It's the only card Holster draws on its own.

**`window`**: Holster brings your main window forward if a process with your exe path is running and has a visible, unowned, titled top-level window (restoring it if minimized). Otherwise it runs `launch.exe launch.args` (working directory: the exe's folder). Be single-instance: a second launch should focus the first window and exit.

The dock collapses once the app is open.

### 1.7 Signals

```http
POST {api}/apps/<id>/signal
Content-Type: application/json

{ "kind": "activity" }
```

The tab glows in your app's color for about 2 s (or, while the dock is open, your tile pulses). During a fullscreen app the glow waits until the user is back. Send it when something meaningful happened that the user didn't do themselves (a model updated a project, a long job finished). Never on a timer.

Other endpoints: `GET {api}/status` returns `{ "running": true, "version": "..." }`. `POST {api}/apps/<id>/reload` re-reads manifests (see 1.3). `POST {api}/apps/<id>/notify` shows a notification (1.8).

Holster's API rejects requests whose `Host` isn't `127.0.0.1:<port>` or `localhost:<port>`, and browser requests from non-loopback origins.

### 1.8 Notifications

A notification is a small card that pops out of Holster's tab. The newest sits at the tab's height and older ones are pushed away from it; up to three show at once, and more wait their turn.

```http
POST {api}/apps/<id>/notify
Content-Type: application/json

{
  "id": "meeting-42",
  "title": "Meeting in 10 minutes",
  "body": "Design review with the team",
  "buttons": [
    { "id": "snooze", "label": "Snooze" },
    { "id": "join", "label": "Join", "primary": true }
  ],
  "timeout": 8
}
```

Holster answers `{ "ok": true, "id": "meeting-42", "muted": false }`, or `400` with the exact `error`.

| Field | Rules |
|---|---|
| `id` | Optional: your own id, 1–64 characters. Sending the same `id` again updates that notification in place (new content, its timer starts over). Without one, Holster makes one up and returns it. |
| `title` | Required (unless `page`). Up to 120 characters; two lines show. |
| `body` | Optional. Up to 500 characters; three lines show. |
| `buttons` | Optional, at most 2: `id` (1–64 characters), `label` (up to 24), `primary` (the filled one: the main action). |
| `timeout` | Seconds on screen: default 6, up to 3600. `0` stays until the user deals with it. |
| `page` | Instead of title, body and buttons: `{ "url": "http://127.0.0.1:…", "width": 340, "height": 120 }` (width 200–480, height 48–480). Your own page in Holster's card. Add `"dismiss": false` when the page draws its own way to dismiss it (see below). |

How it behaves:

- Holster draws the card in its own design language: your tile (icon in your color), your app's name, the title, the text and the buttons. Clicking the card opens your app the way the manifest says (a panel unfolds from the card's height); a button closes it; the `×` (on hover) or a flick into the edge dismisses it.
- A timed notification only counts down while it's on screen, and waits while the cursor is on it. Notifications wait, timers included, while the dock or Settings is open (your tile pulses instead), during a fullscreen app (when the user hides Holster for those), while an app panel is on the edge, and during Do not disturb.
- Nothing is stored: an expired notification is simply gone. Keep your own history if you need one.
- The user can mute your app's notifications in Settings. Holster then answers `"muted": true` and shows nothing.
- `DELETE {api}/apps/<id>/notify/<notification id>` takes one back (the meeting started, the job was cancelled). `404` if it's already gone.

**Hearing back.** Add `"notify": { "url": "http://127.0.0.1:<port>/holster/notify" }` to your manifest and Holster `POST`s what happened:

```json
{ "id": "meeting-42", "event": "action", "button": "snooze" }
```

`event` is `clicked` (the card itself; your app is being opened), `action` (a button, with its `button` id), `dismissed` (the `×`, or flicked away) or `expired` (its time ran out). Nothing is sent when you take one back yourself. Answer fast: Holster doesn't wait for you or retry.

**Custom pages** follow the preview rules (1.5): transparent, theme-aware (`?theme=` and `theme` messages), `color-scheme` matching the theme, no card or shadow of your own. Keep the top-right 32×32 px free: Holster draws the dismiss `×` there on hover. A page whose design has its own dismiss control (a skip button beside its main action) sends `"dismiss": false`: Holster then draws no `×`, and the page must send `close` itself, because a page card can't be flicked away (the page gets the pointer). Holster checks that `page.url` answers before it believes that: if it doesn't, the card keeps Holster's `×` and the response carries a `warning`. A `timeout` is a good second safety net. The page talks to Holster with `parent.postMessage(msg, "*")`:

- `{ holster: "resize", width, height }`: a new size.
- `{ holster: "action", button: "<id>" }`: closes the card and sends you `action` with that id.
- `{ holster: "open" }`: like a click on a card Holster draws.
- `{ holster: "close" }`: dismisses it.

When to notify: something the user wants to know now and can act on (a meeting starting, a long job done, a reply they're waiting for). Not progress chatter, not promotions, never on a timer. For "something happened, look when you like", a signal (1.7) is calmer.

### 1.9 Security

- Everything Holster talks to is `127.0.0.1`. Serve Holster routes on a listener bound to `127.0.0.1` only.
- **Holster routes must never be reachable through a public tunnel.** If your app also exposes itself (Tailscale Funnel, Cloudflare, ngrok), give the tunnel its own listener (or a gate that 404s everything else) and never route `/holster/*` there. A good pattern: the local listener serves everything; requests forwarded from the internet carry a marker header or arrive on a dedicated port, and get only the public routes.
- Validate the `Host` header on loopback listeners (DNS-rebinding guard).

### 1.10 Minimal server (Rust, axum)

```rust
use axum::{routing::{get, post}, Json, Router};
use serde_json::{json, Value};

pub async fn serve_holster(app_handle: tauri::AppHandle) -> anyhow::Result<u16> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?; // loopback only
    let port = listener.local_addr()?.port();
    let open_handle = app_handle.clone();
    let router = Router::new()
        .route("/holster/health", get(|| async { "ok" }))
        .route("/holster/open", post(move |Json(body): Json<Value>| {
            let h = open_handle.clone();
            async move {
                let anchor_y = body.get("anchorY").and_then(Value::as_f64); // open from here
                let tab_y = body.get("tabY").and_then(Value::as_f64); // close toward here
                // Return first; open the panel on the UI side.
                let _ = h.run_on_main_thread(move || { /* slide the panel out, take focus */ let _ = (anchor_y, tab_y); });
                Json(json!({ "ok": true }))
            }
        }))
        // What happened to a notification you sent (1.8).
        .route("/holster/notify", post(|Json(e): Json<Value>| async move {
            let _ = (&e["id"], &e["event"], &e["button"]); // e.g. snooze on "action"/"snooze"
            "ok"
        }));
    tokio::spawn(async move { axum::serve(listener, router).await });
    write_manifest(port)?;
    Ok(port)
}

fn write_manifest(port: u16) -> anyhow::Result<()> {
    let Some(appdata) = std::env::var_os("APPDATA") else { return Ok(()) };
    let dir = std::path::PathBuf::from(appdata).join("Holster");
    if !dir.is_dir() { return Ok(()) } // Holster isn't installed
    let apps = dir.join("apps");
    std::fs::create_dir_all(&apps)?;
    let base = format!("http://127.0.0.1:{port}/holster");
    let manifest = json!({
        "schema": 1, "id": "my-app", "name": "My App",
        "icon": { "lucide": "sparkles" }, "color": "#89BCF2",
        "health": format!("{base}/health"),
        // Shipped next to the exe; the app rewrites state.json beside it.
        "preview": { "path": std::env::current_exe()?.with_file_name("holster-preview").join("index.html"),
                     "width": 300, "height": 180 },
        "open": { "mode": "panel", "url": format!("{base}/open") },
        "launch": { "exe": std::env::current_exe()?, "args": [] },
        "notify": { "url": format!("{base}/notify") }
    });
    let text = serde_json::to_string_pretty(&manifest)?;
    let file = apps.join("my-app.json");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
        let tmp = apps.join(".my-app.json.tmp");
        std::fs::write(&tmp, &text)?;
        std::fs::rename(&tmp, &file)?;
    }
    Ok(())
}
```

`examples/hello-holster` in the Holster repo is a complete, dependency-free example: an on-disk live-clock preview that follows the running state and resizes itself at runtime, a std HTTP health route, `window` mode, a Space-key signal, and notifications (`N` for one that goes away by itself, `Shift+N` for one that stays; Snooze really snoozes).

---

## 2. The stack

**New apps use exactly this stack:**

- **Tauri 2** with a **Rust** backend; **React + TypeScript + Vite** front end (Vite target `chrome120`).
- **Framer Motion** for all animation. **Lucide** for all icons (`lucide-react` in the UI; icon names in manifests).
- **zustand** for UI state; **dnd-kit** for reordering; **axum + tokio** for local HTTP; **rusqlite** (bundled, WAL) when you need storage.
- **`window-vibrancy`** plus the accent-policy acrylic below for the material.
- Plugins: `tauri-plugin-single-instance` (always), `tauri-plugin-global-shortcut` (if you have a hotkey), `tauri-plugin-autostart` (only touch it in release builds), `tauri-plugin-dialog`, `tauri-plugin-opener`.
- **Single instance**, a **tray icon** (left click toggles, menu: Open, Settings…, separator, Quit), and **no taskbar button** (`skipTaskbar: true`, `WS_EX_TOOLWINDOW`). The exception is a `window` app that genuinely needs one.
- Main window config for edge surfaces: `transparent: true, decorations: false, alwaysOnTop: true, skipTaskbar: true, resizable: false, visible: false, focus: false, shadow: false, dragDropEnabled: false`.
- Release profile: `codegen-units = 1, lto = true, opt-level = "s", panic = "abort", strip = true`. NSIS installer, `installMode: currentUser`.

**Adapting an existing app with another stack:** only its Holster-facing surfaces (the preview page and the panel or window it opens) must follow the design language below. The contract is language-agnostic.

---

## 3. The design language

Real values from Holster. Copy them; don't approximate them.

### 3.1 Tokens

```css
:root {
  --font: "Segoe UI Variable Text", "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
  --font-display: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", system-ui, sans-serif;
  --font-mono: "Cascadia Code", "Cascadia Mono", Consolas, ui-monospace, monospace;

  --veil: rgba(249, 249, 251, 0.6);          /* over the acrylic, under content */
  --surface: rgba(255, 255, 255, 0.7);       /* cards, rows, inputs at rest */
  --surface-strong: rgba(255, 255, 255, 0.92);
  --hover: rgba(20, 20, 40, 0.05);
  --press: rgba(20, 20, 40, 0.08);
  --border: rgba(20, 20, 40, 0.08);          /* hairlines */
  --border-strong: rgba(20, 20, 40, 0.14);
  --text: #1a1a1f;
  --text-2: #55555f;
  --text-3: #8b8b95;
  --accent: #6b5bd2;
  --accent-soft: rgba(107, 91, 210, 0.12);
  --focus: rgba(107, 91, 210, 0.32);
  --danger: #d4495b;
  --warn: #b7791f;
  --pop-bg: rgba(252, 252, 254, 0.97);
  --shadow-card: 0 1px 1px rgba(20, 20, 40, 0.03), 0 2px 6px rgba(20, 20, 40, 0.04);
  --shadow-lift: 0 2px 4px rgba(20, 20, 40, 0.05), 0 12px 28px rgba(20, 20, 40, 0.1);
  --shadow-pop: 0 10px 34px rgba(20, 20, 40, 0.16), 0 2px 8px rgba(20, 20, 40, 0.06);
  --scrim: rgba(250, 250, 252, 0.45);
  --code-bg: rgba(20, 20, 40, 0.045);
  color-scheme: light;
}

:root[data-theme="dark"] {
  --veil: rgba(24, 24, 28, 0.52);
  --surface: rgba(255, 255, 255, 0.055);
  --surface-strong: rgba(48, 48, 54, 0.96);
  --hover: rgba(255, 255, 255, 0.06);
  --press: rgba(255, 255, 255, 0.09);
  --border: rgba(255, 255, 255, 0.08);
  --border-strong: rgba(255, 255, 255, 0.14);
  --text: #f2f2f5;
  --text-2: #b4b4be;
  --text-3: #85858f;
  --accent: #a596f5;
  --accent-soft: rgba(165, 150, 245, 0.16);
  --focus: rgba(165, 150, 245, 0.38);
  --danger: #f0798a;
  --warn: #e9b65c;
  --pop-bg: rgba(44, 44, 50, 0.97);
  --shadow-card: 0 1px 1px rgba(0, 0, 0, 0.12), 0 2px 6px rgba(0, 0, 0, 0.12);
  --shadow-lift: 0 2px 4px rgba(0, 0, 0, 0.18), 0 12px 28px rgba(0, 0, 0, 0.32);
  --shadow-pop: 0 10px 34px rgba(0, 0, 0, 0.42), 0 2px 8px rgba(0, 0, 0, 0.2);
  --scrim: rgba(16, 16, 20, 0.45);
  --code-bg: rgba(255, 255, 255, 0.06);
  color-scheme: dark;
}

html, body, #root { margin: 0; height: 100%; overflow: hidden; background: transparent; }
body { font-family: var(--font); font-size: 13.5px; line-height: 1.45; color: var(--text);
  -webkit-font-smoothing: antialiased; user-select: none; cursor: default; }
button { font: inherit; color: inherit; background: none; border: 0; padding: 0; cursor: pointer; }
:focus { outline: none; }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; border-radius: 6px; }
::selection { background: var(--focus); }
```

**Type scale.** Body 13.5/1.45. Secondary 12.5. Hints and meta 11.5–12 in `--text-3`. Section labels 12px, weight 600, `--text-3`, letter-spacing 0.01em, sentence case (never uppercase). Display (`--font-display`): wordmark/panel title 19px/650/-0.02em; card titles 15px/600/-0.01em; hero titles 26px/650/-0.022em; empty-state titles 16px/600. Numbers that change use `font-variant-numeric: tabular-nums`.

**Radii.** Hairline cards and settings groups 12px; project cards and tiles 14px; sheets 18px; buttons and inputs 9px (8px for 30px inputs); small icon buttons 7px; chips and pills fully round; icon wells 9px at 30px, 13px at 44px, 16px at 52px.

**Spacing.** 2, 4, 6, 8, 10, 12, 14, 16, 22, 34. Panel headers `16px 16px 10px`; scroll areas `scrollbar-gutter: stable both-edges` with 6px side padding; settings rows min-height 46px with hairlines between; controls 32px tall (26px small).

**Scrollbars.** 10px, invisible thumbs that appear on hover (`--border-strong`, 3px inset). Docks and previews never show one.

### 3.2 The palette

Ten pastels, each with a light and a dark swatch. Use `bg` for washes, `strong` for fills, rings and progress, `ink` for text and icons on `bg`.

| Name | Light bg / strong / ink | Dark bg / strong / ink |
|---|---|---|
| mint | `#E2F5EB` `#46B086` `#1D5741` | `#1E3A2F` `#7AD6AF` `#CDF2E1` |
| peach | `#FDE7DC` `#E8916A` `#773C20` | `#41291E` `#F2A887` `#FBDACB` |
| lavender | `#ECE7FB` `#8D79DF` `#42338B` | `#2E2848` `#B2A2F5` `#E2DBFD` |
| sky | `#E1EEFC` `#5797DE` `#1D4B7C` | `#1E2F47` `#89BCF2` `#D4E7FB` |
| butter | `#FAF1D2` `#D2A82E` `#6A500D` | `#3A331E` `#EACA68` `#F6EAC2` |
| rose | `#FCE3E9` `#DF7592` `#7A253E` | `#42232C` `#F29AB1` `#FBD8E1` |
| sage | `#E7EEDF` `#86A271` `#3B4F2D` | `#293224` `#A8C394` `#DCE7D3` |
| lilac | `#F3E5F7` `#B679C9` `#5B2B6D` | `#38273E` `#D19EE1` `#F0DBF6` |
| apricot | `#FDEAD5` `#E99F55` `#764311` | `#3F2F1D` `#F3BA7F` `#FBE2C8` |
| mist | `#E7EBF0` `#8897AB` `#374252` | `#292E36` `#A8B4C4` `#DDE2E9` |

Your manifest `color` should sit in the dark-`strong` family (`#B2A2F5`, `#89BCF2`, `#7AD6AF`…): soft, light, saturated just enough. Holster draws the tile in that color with a 26%→0 white top gradient, an inner top highlight and a 1px inner hairline. A Lucide icon is drawn 22 px in a deep shade of the same hue (HSL lightness 24%) on light fills, or white on dark ones. An image icon is drawn at its `size` share of the tile, untinted, so pick a `color` it reads well on. Status colors: online `#3fbf7f` (with a 3px 16–18% halo), pending `#e2ab3f`, danger as `--danger`.

### 3.3 Material

Every surface is a native window whose **bounds are the visible shape**: no transparent regions, no fake CSS blur. The frost is real acrylic from the window accent policy, which stays frosted while unfocused (the DWM system backdrop turns flat when inactive):

```rust
// SetWindowCompositionAttribute(hwnd, WCA_ACCENT_POLICY = 19,
//   AccentPolicy { state: 4 /* ACRYLICBLURBEHIND */, flags: 2, gradient, animation: 0 })
// gradient is ABGR: dark 0x5C201C1C, light 0x66F4F2F2.
// First turn the system backdrop off: DwmSetWindowAttribute(hwnd, 38 /* SYSTEMBACKDROP_TYPE */, &1u32 /* NONE */).
// Fallback when unavailable: window_vibrancy::apply_blur(window, Some(dark ? (28,26,26,160) : (244,244,246,160))).
```

Also: `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND` for Windows 11 corners; subclass the window and answer `WM_NCACTIVATE` with `wParam = 1` so the acrylic keeps its active look; while a surface must not steal focus, set `WS_EX_NOACTIVATE` and answer `WM_MOUSEACTIVATE` with `MA_NOACTIVATE`. Re-apply the material on theme change.

On top of the acrylic, content sits on `--veil`. Surfaces that grow a lot (Holster's preview and settings cards) bake the veil into the accent tint instead (dark `0xB01D1919`, light `0xC2F9F7F7`) and draw no CSS veil: the webview fills newly exposed edges a frame late, and with the veil in the material those edges already look finished. Surfaces that touch the screen edge extend 10px (×scale) past it (the overhang), so only the inner corners are rounded. Skip the overhang when another monitor or a right-side taskbar is there. Lay content out with `right: overhang`.

### 3.4 Geometry

- **Tab**: 14×88 CSS px, flush with the edge, 6px slide-out on hover. Grip: 3×26 bar at `left: 5px`, centered, `--text` at 20% opacity (34% on hover). Light sheen gradient `rgba(255,255,255,.34) → .06 at 55% → .14` (dark `.12 → .02 → .05`). The tab is dragged vertically with a 4px click-vs-drag threshold; its center is persisted as a fraction of the work-area height (default 0.42), clamped 8px from the work-area ends.
- **Dock** (Holster): tiles 46px, 8px apart, radius 14; card padding 10px top and bottom, 16px left (the grip lives there) and 10px right; exactly 5 tiles visible. Settings card 50px tall, 8px below. Previews sit 8px left of the dock, centered on the tile and clamped 8px inside the work area.
- **Side panel**: right edge, 10px from the edge, 12px from the work-area top and bottom; default width 420, minimum 340, wide 640 for detail pages, resizable from its inner edge; never wider than the work area minus 48px.
- **Preview cards**: 140–720 × 64–720. Small cards are 240–320 wide. Inner padding 14–18px.

### 3.5 Motion

All UI animation uses Framer Motion springs:

```ts
export const spring = { type: "spring", stiffness: 520, damping: 38, mass: 0.8 } as const; // controls, pills, popovers
export const softSpring = { type: "spring", stiffness: 300, damping: 30 } as const;          // calmer moves
export const rowSpring = { type: "spring", stiffness: 520, damping: 34 } as const;           // rows/tiles arriving, delay index * 0.04
export const pageSpring = { type: "spring", stiffness: 420, damping: 40 } as const;          // routes, lists, scrolling, height
export const reveal = { duration: 0.32, ease: [0.16, 1, 0.3, 1] } as const;                 // content once its window unfolded
export const conceal = { duration: 0.09, ease: "easeIn" } as const;                         // content before its window folds
```

Recipes: rows enter from `{ opacity: 0, x: 14, scale: 0.98 }`; panels reveal from `{ opacity: 0, filter: "blur(6px)", scale: 0.985 }`; route changes slide 28px in and 20px out with `pageSpring` (opacity 0.18 s); toasts rise 16px with `{ stiffness: 500, damping: 36 }`; presses scale to 0.94–0.97 (`whileTap`), never above 1; popovers enter from `{ opacity: 0, scale: 0.96, y: -4 }`; theme changes crossfade with a 260 ms view transition.

**Window morphs** animate the native window rect, one spring per edge `[left, top, right, bottom]`, on a dedicated thread paced by `DwmFlush` (2 ms integration substeps, settled within 0.25 px). Content is laid out at the final size, anchored to the edge, and revealed once the shape is within 6 px of its target. Leaving: fade the content first (70–90 ms), then fold the shape.

| Morph | ω (rad/s) [l, t, r, b] | ζ | delay (s) |
|---|---|---|---|
| Tab → panel | 21, 17, 21, 17 | .78, .9, .9, .9 | 0, .035, 0, .035 |
| Panel → tab | 24, 26, 24, 26 | 1 | .03, 0, .03, 0 |
| Tab → peek/dock card | 22, 19, 22, 19 | .8, .86, .9, .86 | 0, .025, 0, .025 |
| Card → tab | 25, 27, 25, 27 | 1 | .025, 0, .025, 0 |
| Tab hover slide | 26 | .72 | 0 |
| Resize / glide between sizes | 19–24 | .92 | 0 |

The shape always goes out from the edge first (slight overshoot), then grows in height.

### 3.6 Theming

Three options everywhere: **System, Light, Dark** (segmented control). Resolve to light or dark, set `document.documentElement.dataset.theme`, set the native window theme, and re-apply the material tint. A preview follows Holster's theme (1.5), not its own setting.

### 3.7 Preview cards

- One glanceable thing: the next step, a live number, the latest few items. It answers "what's going on in this app?" in under a second.
- Start with a 12px/600 `--text-3` label (your app or the subject), optionally with a 6px status dot in your color.
- Up to three rows. Neutral surfaces (`--surface` + `--border`, radius 12) for rows, color only for meaning (progress, status). A good model: a Lucide icon inside a circular progress ring in the item's color, a 13px/600 title and an 11.5px `--text-3` description, all truncated with ellipsis.
- No scrollbars, no empty space: size the card to the content and `resize` when the content changes.
- Designed states: empty ("Nothing in progress", with a quiet icon), loading (render the last known state, never a spinner on hover), error (one calm line in `--text-3`).

### 3.8 Side panels

- Same window technique: a frameless, always-on-top, skip-taskbar window whose bounds morph from a slim tab or point on the right edge into the panel (geometry 3.4, springs 3.5). The panel's close grip is drawn exactly like the tab's grip (3×26 at `left: 5px`, centered) inside a 14×48 hit area, so it doesn't move as the panel unfolds.
- Opened from Holster, the panel unfolds out of the clicked tile (`anchorY`) and folds back toward Holster's tab (`tabY`), never back to the tile: the tab slides out right where the panel disappeared (1.6). Keep your own seed for when the panel is opened some other way.
- Opening takes focus robustly (on the window's own thread: attach to the foreground thread's input, `BringWindowToTop`, `SetForegroundWindow`) and remembers the previous foreground window. Closing gives focus back to it.
- Closes on Escape (innermost overlay first), the hotkey again, clicking the grip, or a click outside (blur debounced 100 ms, ignored for the first 350 ms, never while a menu, sheet, picker or IME composition is open). A **session-only pin** (Lucide `pin`, filled when on, `pin-off` on hover) keeps it open until the panel closes.
- Header: 19px display title on the left, icon buttons (32px, radius 9) on the right. Body: grouped cards (`--surface`, hairline `--border`, radius 12, 4px 12px padding) with 46px rows separated by hairlines, labels with an 11.5px hint below.
- Hide during fullscreen apps (a game, a video, a presentation): tuck the tab into the edge, never take focus, and queue ambient signals until the user is back.

---

## 4. Taste rules

- **Calm, native, expensive-feeling.** Soft pastels on frosted glass, hairline borders, generous but dense layout. If it looks like a web page, it's wrong.
- **No default browser styling**: no native scrollbars, outlines, selects, checkboxes, tooltips (`title` only on icon buttons as an accessible name), context menus or text selection outside inputs. Block zoom and reload shortcuts.
- **No system dialogs where a styled one fits.** Confirm inline ("Remove" / "Cancel" in place), undo with a toast, warn in the row that has the problem. File pickers are the exception.
- **No jittery hover lifts.** Hover is a color, opacity or halo change. Movement belongs to presses (scale down) and to things actually moving.
- **No visual noise**: one accent per surface, color carries meaning, no gradients except the tile highlight and the tab sheen, no drop shadows on flat rows, no badges for things that don't need attention.
- **Every interaction animates with the shared springs.** Nothing pops in. Nothing fades slower than 0.32 s.
- **Empty, loading and error states are designed, not blank**: an icon in a soft accent well, a 16px/600 title, one sentence of help, and the next action.
- **Respect the user's attention**: never steal focus on hover, never animate on a timer, signal only when it matters, and stay out of the way of fullscreen apps.
- Sentence case everywhere. Short, plain labels ("Add app", "Not running", "Open folder").

---

## 5. Shipping each build

Work isn't finished until the user has something to install. At the end of every round of changes:

1. **Bump the version, every build.** Never ship two different builds under one version: the installer would replace the app without anyone being able to tell which build is running. Patch (`0.1.0` → `0.1.1`) for fixes and small changes, minor (`0.1.1` → `0.2.0`) for new features. Change it everywhere it lives and keep them identical. For a Tauri app that's `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` (then let the build update `Cargo.lock`), plus any README line that names the version or the installer file.
2. **Build the release installer** (`pnpm tauri build` for the stack above), after the tests pass. A dev run is not a build.
3. **Check that the installer exists and is new**: the file name carries the new version and its timestamp is from this build. It's under `target\release\bundle\nsis\` (`src-tauri\target\...` when the app isn't a Cargo workspace), named `<productName>_<version>_x64-setup.exe`.
4. **End your reply with the installer's full absolute path**, e.g. `E:\Code\Notes\target\release\bundle\nsis\Notes_0.1.1_x64-setup.exe`, and the new version number. Never report an installer from an earlier build as the new one.
