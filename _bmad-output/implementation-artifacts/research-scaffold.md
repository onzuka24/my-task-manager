# Research: Tauri 2.11 + Svelte 5 macOS background app (scaffold & platform APIs)

Researched 2026-09-15 against live registries (crates.io, npm registry), GitHub source of the
exact published crate versions, and current official docs. Verified locally on Darwin 25.6
(macOS 26) where noted.

**Version floor confirmed live (2026-09-15):**

| Package | Latest stable | Released | Notes |
|---|---|---|---|
| `tauri` (crate) | **2.11.5** | 2026-07-01 | `3.0.0-alpha.0` published 2026-09-13 — do not use |
| `@tauri-apps/cli` | **2.11.4** | — | CLI version tracks separately from core; 2.11.4 is correct for core 2.11.5 |
| `@tauri-apps/api` | **2.11.1** | — | |
| `tauri-plugin-global-shortcut` | **2.3.2** | 2026-05-28 | requires `tauri` >= 2.10, Rust >= 1.77.2 |
| `tauri-plugin-autostart` | **2.5.1** | 2025-10-27 | |
| `create-tauri-app` | **4.7.4** | 2026-09-04 | |
| `create-vite` | **9.2.1** | — | |
| `svelte` | **5.57.0** | — | |

Sources: <https://crates.io/api/v1/crates/tauri>, <https://crates.io/crates/tauri-plugin-global-shortcut>,
<https://crates.io/crates/tauri-plugin-autostart>, <https://registry.npmjs.org/@tauri-apps/cli>,
<https://registry.npmjs.org/create-vite>, <https://registry.npmjs.org/svelte>

---

## 1. Scaffolding — Svelte (NOT SvelteKit) + TypeScript

### The trap: `create-tauri-app`'s "Svelte" template is SvelteKit

`create-tauri-app` 4.7.4 ships exactly two Svelte templates, `template-svelte` and
`template-svelte-ts`, and **both are SvelteKit projects**, not plain Svelte. Verified from the
published template source at tag `create-tauri-app-v4.7.4`:

```jsonc
// templates/template-svelte-ts/package.json.lte  (create-tauri-app v4.7.4)
"scripts": { "dev": "vite dev", "prepare": "svelte-kit sync || echo ''", ... },
"devDependencies": {
  "@sveltejs/adapter-static": "^3.0.10",
  "@sveltejs/kit": "^2.65.1",          // <-- SvelteKit
  "@sveltejs/vite-plugin-svelte": "^7.1.2",
  "svelte": "^5.56.3",
  "svelte-check": "^4.6.0",
  "typescript": "~6.0.3",
  "vite": "^8.0.16"
}
```
```ini
# templates/template-svelte-ts/.manifest
frontendDist = ../build     # SvelteKit adapter-static output, not Vite's dist/
```
Source: <https://github.com/tauri-apps/create-tauri-app/blob/dev/templates/template-svelte-ts/package.json.lte>,
template list: <https://github.com/tauri-apps/create-tauri-app/tree/dev/templates>

So there is **no** official Tauri template for plain Svelte + TS. Use Vite directly, then
`tauri init`.

### Recommended: Vite scaffold + `tauri init`

```bash
# 1. Plain Svelte 5 + TypeScript frontend (create-vite 9.2.1)
npm create vite@latest my-task-manager -- --template svelte-ts --no-interactive
cd my-task-manager
npm install

# 2. Add the Tauri shell
npm install -D @tauri-apps/cli@^2
npm install @tauri-apps/api@^2
npx tauri init \
  --app-name "My Task Manager" \
  --window-title "My Task Manager" \
  --frontend-dist ../dist \
  --dev-url http://localhost:1420 \
  --before-dev-command "npm run dev" \
  --before-build-command "npm run build" \
  --ci

# 3. Plugins (edits Cargo.toml, package.json, lib.rs and capabilities/default.json)
npx tauri add global-shortcut
npx tauri add autostart
```

`--no-interactive` is a current `create-vite` flag (needed for a non-prompting scaffold).
Sources: <https://vite.dev/guide/>, <https://v2.tauri.app/reference/cli/>

**What `create-vite --template svelte-ts` generates** (verified from the published
`create-vite@9.2.1` tarball, `package/template-svelte-ts/`):

```
index.html  vite.config.ts  svelte.config.js
tsconfig.json  tsconfig.app.json  tsconfig.node.json
src/main.ts  src/App.svelte  src/app.css  src/lib/Counter.svelte
public/favicon.svg  public/icons.svg  src/assets/*
```
```json
"devDependencies": {
  "@sveltejs/vite-plugin-svelte": "^7.3.0",
  "@tsconfig/svelte": "^5.0.8",
  "@types/node": "^24.13.3",
  "svelte": "^5.57.0",      // Svelte 5 — runes-era, matches the 5.57 target
  "svelte-check": "^4.7.6",
  "typescript": "~6.0.2",   // TypeScript 6
  "vite": "^8.3.0"          // Vite 8
}
```

**What `tauri init` generates:** a `src-tauri/` directory containing `Cargo.toml`,
`tauri.conf.json`, `build.rs`, `src/main.rs`, `src/lib.rs`, `icons/`, and
`capabilities/default.json`.

### Required Vite config additions for Tauri

```ts
// vite.config.ts
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,                 // don't hide Rust errors
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
```

### Answer to "does the template ship Svelte 5?"

Yes for both paths — `create-tauri-app` pins `svelte: ^5.56.3` and `create-vite` pins
`svelte: ^5.57.0`. Both also now pin **TypeScript 6.x and Vite 8.x**, which is newer than most
2025-era guides assume (those say TS 5.x / Vite 5-6).

---

## 2. LSUIElement / no Dock icon

There are **three** mechanisms in Tauri 2.11, and the correct answer is a combination.

### (a) `Info.plist` — `LSUIElement` (primary, Apple-blessed)

There is **no** `tauri.conf.json` key for `LSUIElement`. The `bundle.macOS.infoPlist` config key
is a **path to a plist file**, not an inline dictionary — verified against the generated JSON
schema:

```json
// crates/tauri-schema-generator/schemas/config.schema.json -> MacConfig.infoPlist
{
  "description": "Path to a Info.plist file to merge with the default Info.plist.\n\n Note that Tauri also looks for a `Info.plist` file in the same directory as the Tauri configuration file.",
  "type": ["string", "null"]
}
```
`MacConfig` properties in full: `frameworks`, `files`, `bundleVersion`, `bundleName`,
`minimumSystemVersion`, `exceptionDomain`, `signingIdentity`, `hardenedRuntime`,
`providerShortName`, `entitlements`, `infoPlist`, `dmg`.

So the idiomatic form is to just drop the file next to `tauri.conf.json`:

```xml
<!-- src-tauri/Info.plist -->
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>LSUIElement</key>
  <true/>
</dict>
</plist>
```

Tauri merges this into the generated `Info.plist` on `tauri build`, and **embeds it into the
binary during `tauri dev`** — so the no-Dock behavior is testable in dev, not only in the bundle.
Source: <https://v2.tauri.app/distribute/macos-application-bundle/>

> `LSUIElement` removes the Dock icon *and* the app's menu bar. Do not also set
> `bundle.macOS.minimumSystemVersion` to something below what you test on; it defaults to `10.13`.

### (b) `App::set_activation_policy` / `AppHandle::set_activation_policy` (runtime)

Both exist in 2.11.5 and are `#[cfg(target_os = "macos")]`. Note the two differ in signature —
`App` takes `&mut self` and returns `()`, `AppHandle` takes `&self` and returns `Result`:

```rust
// tauri-2.11.5/src/app.rs:1286  (App)
pub fn set_activation_policy(&mut self, activation_policy: ActivationPolicy);

// tauri-2.11.5/src/app.rs:640  (AppHandle)
pub fn set_activation_policy(&self, activation_policy: ActivationPolicy) -> crate::Result<()>;
```
```rust
tauri::Builder::default()
  .setup(|app| {
    #[cfg(target_os = "macos")]
    app.handle().set_activation_policy(tauri::ActivationPolicy::Accessory)?;
    Ok(())
  });
```
`ActivationPolicy` is re-exported at the crate root (`tauri-2.11.5/src/lib.rs:204`,
`pub use runtime::ActivationPolicy;`). Variants: `Regular`, `Accessory`, `Prohibited`.
The doc comment states the default is `NSApplicationActivationPolicyRegular`.

### (c) NEW in the 2.11 line — `set_dock_visibility`

This did **not** exist in 2025-era Tauri and is the cleanest runtime answer:

```rust
// tauri-2.11.5/src/app.rs:660  (AppHandle)
#[cfg(target_os = "macos")]
pub fn set_dock_visibility(&self, visible: bool) -> crate::Result<()>;

// tauri-2.11.5/src/app.rs:1307  (App, &mut self -> ())
#[cfg(target_os = "macos")]
pub fn set_dock_visibility(&mut self, visible: bool);
```
```rust
.setup(|app| {
  #[cfg(target_os = "macos")]
  app.handle().set_dock_visibility(false)?;
  Ok(())
})
```
Use this when you want to *toggle* Dock presence at runtime (e.g. a "show in Dock" preference)
without fighting the activation-policy/window-visibility interaction.

### State of the historical `set_activation_policy` issue

- **tauri#2258** "feat(macos): expose `set_activation_policy`" — **closed / completed**
  2021-08-13. The API is long since shipped. <https://github.com/tauri-apps/tauri/issues/2258>
- **tauri#5122** "[bug] set_activation_policy() breaks window.show()" — **closed / completed**
  2024-02-02, one comment, the reporter's issue was not reproducible for others.
  <https://github.com/tauri-apps/tauri/issues/5122>

So the "known open bug" a 2025-trained model would warn about is **closed**. What remains real is
the AppKit-level behavior: under `Accessory`, `window.show()` alone does not bring the window
forward reliably — you must call `set_focus()` (see §6), and calling `set_focus()` can transiently
promote Dock presence. Belt-and-braces recipe used by current tray/background apps:

```rust
.setup(|app| {
  #[cfg(target_os = "macos")]
  {
    // Info.plist LSUIElement covers launch time; the runtime call covers
    // the dev-run / re-promotion cases.
    app.handle().set_activation_policy(tauri::ActivationPolicy::Accessory)?;
  }
  Ok(())
})
```

**Recommendation for this app:** ship `src-tauri/Info.plist` with `LSUIElement` **and** call
`set_activation_policy(Accessory)` in `setup()`. They cover different failure modes of the same
requirement.

---

## 3. Pre-created hidden window — exact `tauri.conf.json` v2 shape

All keys verified against the live `config.schema.json` (`WindowConfig`).

```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "My Task Manager",
  "version": "0.1.0",
  "identifier": "com.example.mytaskmanager",
  "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  },
  "app": {
    "macOSPrivateApi": true,
    "windows": [
      {
        "label": "main",
        "title": "My Task Manager",
        "width": 720,
        "height": 480,
        "center": true,
        "resizable": false,

        "visible": false,          // start hidden  (default true)
        "focus": false,            // do not steal focus at launch (default true)

        "alwaysOnTop": true,       // (default false)
        "visibleOnAllWorkspaces": true,   // follow the user across Spaces
        "decorations": false,      // no title bar / borders (default true)
        "transparent": true,       // REQUIRES macos-private-api
        "shadow": true,            // (default true)
        "skipTaskbar": true,       // NO-OP ON macOS (Windows/Linux only)

        "titleBarStyle": "Overlay",
        "hiddenTitle": true,
        "acceptFirstMouse": true
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": "app",
    "macOS": {
      "minimumSystemVersion": "10.15"
    }
  }
}
```

### Key-by-key (schema descriptions + defaults, verbatim from the schema)

| Key | Default | Notes |
|---|---|---|
| `visible` | `true` | "Whether the window is visible or not." Set `false` to pre-create hidden. |
| `focus` | `true` | "Whether the window will be initially focused or not." |
| `alwaysOnTop` | `false` | |
| `decorations` | `true` | "Whether the window should have borders and bars." |
| `skipTaskbar` | `false` | "If `true`, hides the window icon from the taskbar **on Windows and Linux**." — **no macOS effect** |
| `transparent` | `false` | "**on `macOS` this requires the `macos-private-api` feature flag, enabled under `tauri > macOSPrivateApi`.** WARNING: Using private APIs on `macOS` prevents your application from being accepted to the App Store." |
| `shadow` | `true` | |
| `create` | `true` | "Whether Tauri should create this window at app startup or not." — leave `true`; you want the window pre-created. |
| `titleBarStyle` | `Visible` | `Visible` \| `Transparent` \| `Overlay` |
| `hiddenTitle` | `false` | |
| `trafficLightPosition` | `null` | requires `titleBarStyle: "Overlay"` **and** `decorations: true` |
| `windowEffects` | `null` | requires the window to be transparent |

### Which require `macos-private-api`

Only **`transparent`** (and `windowEffects`, which requires transparency). `visible`,
`alwaysOnTop`, `decorations`, `skipTaskbar`, `shadow`, `focus` do **not**.

Enabling it is a **two-sided** switch — both must be set or the build errors:

```json
// tauri.conf.json — note this moved from `tauri.macOSPrivateApi` (v1) to `app.macOSPrivateApi` (v2)
"app": { "macOSPrivateApi": true }
```
```toml
# src-tauri/Cargo.toml
[dependencies]
tauri = { version = "2.11.5", features = ["macos-private-api"] }
```
The Cargo feature `macos-private-api` is confirmed in `tauri-2.11.5/Cargo.toml:108`.
Schema doc for the config key: "MacOS private API configuration. Enables the transparent
background API and sets the `fullScreenEnabled` preference to `true`."

> If you do not actually need a transparent/vibrancy window, **skip `macos-private-api`** —
> it disqualifies the app from the Mac App Store.

### v1 → v2 renames that bite

| Tauri v1 | Tauri v2 |
|---|---|
| `package.productName`, `package.version` | top-level `productName`, `version` |
| `tauri` (root object) | `app` |
| `tauri.windows` | `app.windows` |
| `tauri.macOSPrivateApi` | `app.macOSPrivateApi` |
| `tauri.windows[].fileDropEnabled` | `app.windows[].dragDropEnabled` |
| `tauri.systemTray` | `app.trayIcon` |
| `build.distDir` | `build.frontendDist` |
| `build.devPath` | `build.devUrl` |
| `build.withGlobalTauri` | `app.withGlobalTauri` |
| `tauri.bundle` | top-level `bundle` |
| `tauri.bundle.identifier` | top-level `identifier` |
| `tauri.allowlist` | removed → capabilities/permissions |

Source: <https://v2.tauri.app/start/migrate/from-tauri-1/>, schema:
<https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-schema-generator/schemas/config.schema.json>

---

## 4. `tauri-plugin-global-shortcut` 2.3.2 — exact Rust API

All signatures below are copied from the **published 2.3.2 crate source**
(`tauri-plugin-global-shortcut-2.3.2/src/lib.rs`).

### Cargo

```toml
# src-tauri/Cargo.toml
[target."cfg(not(any(target_os = \"android\", target_os = \"ios\")))".dependencies]
tauri-plugin-global-shortcut = "2.3.2"
```
Crate deps: `tauri >= 2.10`, `global-hotkey 0.8` (with `serde`), `thiserror 2`,
`tauri-plugin 2.5` (build). Rust >= 1.77.2.

### The handler type

```rust
// src/lib.rs:39
type HandlerFn<R> = Box<dyn Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>;
```

So the handler closure signature is **`Fn(&AppHandle<R>, &Shortcut, ShortcutEvent)`** — note the
event is taken **by value**, not by reference.

### Builder registration (register at startup)

```rust
// src/lib.rs:380
pub fn with_handler<F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static>(
    mut self, handler: F
) -> Self;

// src/lib.rs:356 / :366
pub fn with_shortcut<T>(mut self, shortcut: T) -> Result<Self>
where T: TryInto<ShortcutWrapper>, T::Error: std::error::Error;
pub fn with_shortcuts<S, T>(mut self, shortcuts: S) -> Result<Self>;

pub fn build(self) -> TauriPlugin<R>;   // src/lib.rs:388
```

### Runtime registration (register/unregister later, e.g. user rebinds the hotkey)

```rust
// GlobalShortcutExt::global_shortcut() -> &GlobalShortcut<R>
pub fn register<S>(&self, shortcut: S) -> Result<()>;                        // :131
pub fn on_shortcut<S, F>(&self, shortcut: S, handler: F) -> Result<()>       // :143
    where F: Fn(&AppHandle<R>, &Shortcut, ShortcutEvent) + Send + Sync + 'static;
pub fn register_multiple<S, T>(&self, shortcuts: S) -> Result<()>;           // :153
pub fn on_shortcuts<S, T, F>(&self, shortcuts: S, handler: F) -> Result<()>; // :167
pub fn unregister<S: TryInto<ShortcutWrapper>>(&self, shortcut: S) -> Result<()>;  // :182
pub fn unregister_multiple<...>(&self, shortcuts: S) -> Result<()>;          // :193
pub fn unregister_all(&self) -> Result<()>;                                  // :220
pub fn is_registered<S: TryInto<ShortcutWrapper>>(&self, shortcut: S) -> bool; // :232
```

### Pressed vs Released — the exact filter

The plugin **re-exports** the types from `global-hotkey`:

```rust
// src/lib.rs:22-24
pub use global_hotkey::{
    hotkey::{Code, HotKey as Shortcut, Modifiers},
    GlobalHotKeyEvent as ShortcutEvent,
    HotKeyState as ShortcutState,
};
```

And in `global-hotkey` 0.8.0, `state` is a **public struct field**, not a method:

```rust
// global-hotkey-0.8.0/src/lib.rs:66
pub enum HotKeyState { Pressed, Released }

// global-hotkey-0.8.0/src/lib.rs:76
pub struct GlobalHotKeyEvent {
    pub id: u32,
    pub state: HotKeyState,
}
```

**The handler fires for both Pressed and Released.** Filter with the `state` field:

```rust
if event.state == ShortcutState::Pressed { /* ... */ }
```

(`docs.rs` prose mentioning an `event.state()` *method* is wrong for 2.3.2 — the source has a
public field. `HotKey` likewise exposes `pub mods: Modifiers`, `pub key: Code`, `pub id: u32`.)

### Complete working registration

```rust
// src-tauri/src/lib.rs
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(desktop)]
            {
                let toggle = Shortcut::new(
                    Some(Modifiers::SUPER | Modifiers::SHIFT),
                    Code::Space,
                );
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_shortcut(toggle)?
                        .with_handler(move |app, shortcut, event| {
                            // fires TWICE per keypress; keep only the down edge
                            if event.state != ShortcutState::Pressed {
                                return;
                            }
                            if shortcut == &toggle {
                                if let Some(w) = app.get_webview_window("main") {
                                    if w.is_visible().unwrap_or(false) {
                                        let _ = w.hide();
                                        #[cfg(target_os = "macos")]
                                        let _ = app.hide();   // see §6
                                    } else {
                                        let _ = w.show();
                                        let _ = w.set_focus();
                                    }
                                }
                            }
                        })
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### String shortcuts

`Shortcut: FromStr` and `TryFrom<&str>` are implemented, so `"CmdOrCtrl+Shift+Space"` /
`"alt+space"` work anywhere a `TryInto<ShortcutWrapper>` is accepted
(`global-hotkey-0.8.0/src/hotkey.rs:145`).

### Permissions (only needed if you drive it from JS)

```json
// src-tauri/capabilities/default.json
"permissions": [
  "global-shortcut:allow-register",
  "global-shortcut:allow-unregister",
  "global-shortcut:allow-is-registered"
]
```
Pure-Rust registration needs **no** capability entry.

Sources: <https://v2.tauri.app/plugin/global-shortcut/>,
crate source <https://static.crates.io/crates/tauri-plugin-global-shortcut/tauri-plugin-global-shortcut-2.3.2.crate>

---

## 5. `tauri-plugin-autostart` 2.5.1 — exact setup + macOS caveats

### Cargo + npm

```bash
cd src-tauri
cargo add tauri-plugin-autostart --target 'cfg(any(target_os = "macos", windows, target_os = "linux"))'
cd .. && npm install @tauri-apps/plugin-autostart   # only if driving from JS
```
```toml
# resolves to
[target.'cfg(any(target_os = "macos", windows, target_os = "linux"))'.dependencies]
tauri-plugin-autostart = "2.5.1"
```

### Registration

Short form:
```rust
#[cfg(desktop)]
app.handle().plugin(tauri_plugin_autostart::init(
    tauri_plugin_autostart::MacosLauncher::LaunchAgent,
    Some(vec!["--from-autostart"]),   // argv passed on login launch
))?;
```

Builder form (2.5.x, lets you set the Login Items display name):
```rust
use tauri_plugin_autostart::{Builder, MacosLauncher};

app.handle().plugin(
    Builder::new()
        .macos_launcher(MacosLauncher::LaunchAgent)   // default
        .app_name("My Task Manager")                   // defaults to package_info().name
        .arg("--from-autostart")
        .build(),
)?;
```

### API

```rust
// src/lib.rs — AutoLaunchManager
pub fn enable(&self)     -> Result<()>;
pub fn disable(&self)    -> Result<()>;
pub fn is_enabled(&self) -> Result<bool>;

// trait ManagerExt<R: Runtime>
fn autolaunch(&self) -> State<'_, AutoLaunchManager>;
```
```rust
use tauri_plugin_autostart::ManagerExt;
let m = app.autolaunch();
m.enable()?;
let on = m.is_enabled()?;
m.disable()?;
```
(The source carries a `// TODO: Rename these to `autostart` or `auto_start` in v3` comment on
`autolaunch()` — expect a rename in the 3.x line.)

JS side:
```ts
import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';
await enable(); await isEnabled(); await disable();
```
```json
// capabilities/default.json — only if using the JS API
"permissions": ["autostart:allow-enable", "autostart:allow-disable", "autostart:allow-is-enabled"]
```

### macOS caveats (verified from 2.5.1 source, `src/lib.rs`)

1. **It is NOT SMAppService.** The plugin wraps the `auto-launch` crate (v0.5) and offers exactly
   two macOS strategies:
   ```rust
   pub enum MacosLauncher { #[default] LaunchAgent, AppleScript }
   ```
   `LaunchAgent` writes a plist into `~/Library/LaunchAgents/`. `AppleScript` adds a Login Item via
   System Events. Neither uses the modern `SMAppService` / `ServiceManagement` framework. There is
   no SMAppService path in 2.5.1.

2. **`LaunchAgent` registers the inner Unix executable, not the `.app`.** The source explicitly
   special-cases this — the `.app`-path rewrite only happens for `AppleScript`:
   ```rust
   let exe_path = current_exe.canonicalize()?.display().to_string();
   let parts: Vec<&str> = exe_path.split(".app/").collect();
   let app_path = if parts.len() == 2 && matches!(self.macos_launcher, MacosLauncher::AppleScript) {
       format!("{}.app", parts.first().unwrap())   // /Applications/Example.app
   } else {
       exe_path                                     // /Applications/Example.app/Contents/MacOS/Example
   };
   ```
   Consequence: with `LaunchAgent`, System Settings → General → Login Items shows the entry as a
   raw **Unix executable**, not as your app with its icon. The source comment acknowledges this.
   Use `MacosLauncher::AppleScript` if you care about how the Login Items row looks;
   use `LaunchAgent` if you want it to work without `System Events` automation consent.

3. **macOS 13+ "Background Items Added" notification.** Any LaunchAgent registration triggers the
   system notification and an entry under "Allow in the Background". For an **unsigned /
   ad-hoc-signed** app macOS cannot attribute it to a developer, so the alert and the Login Items
   row show the bare binary/team-less name. It still *functions* unsigned — it is a plain
   user-domain LaunchAgent plist, no signature check — but the UX is poor and Gatekeeper may
   quarantine the app on first launch from a downloaded copy.

4. **Dev-mode footgun.** `current_exe()` in `tauri dev` points at
   `src-tauri/target/debug/<binary>`, so calling `enable()` during development registers your debug
   build to launch on login. Guard it behind `#[cfg(not(debug_assertions))]` or a settings toggle.

5. **Pairs correctly with `LSUIElement`.** Launching the inner executable still resolves
   `NSBundle.mainBundle` to the enclosing `.app`, so the bundled `Info.plist` `LSUIElement` is
   honored on a LaunchAgent login launch — the app starts with no Dock icon.

Sources: <https://v2.tauri.app/plugin/autostart/>,
crate source <https://static.crates.io/crates/tauri-plugin-autostart/tauri-plugin-autostart-2.5.1.crate>,
<https://github.com/tauri-apps/plugins-workspace/issues/634>

---

## 6. Returning focus to the previously-frontmost app

### State of the underlying Tauri issue

**tauri#7540 "[bug] Hiding a window on macOS doesn't unfocus" is still OPEN** (no `closed_at`).
Maintainer `amrbashir` (2024-08-14): *"This is not an issue to fix IMO, handing focus off to
another window upon hiding is tricky… If your app consists of only 1 window, I've also seen some
people call `app.hide()`."* So there is no dedicated "restore previous app" Tauri API and none is
planned. <https://github.com/tauri-apps/tauri/issues/7540>

### Option A (recommended) — `AppHandle::hide()`

Tauri 2.11.5 ships a macOS-only `hide()`/`show()` pair on `App`/`AppHandle` that maps to
`NSApplication`'s hide, which is exactly the Cmd-H path AppKit uses to hand activation back:

```rust
// tauri-2.11.5/src/app.rs:1097
/// Hides the application.
#[cfg(target_os = "macos")]
pub fn hide(&self) -> crate::Result<()>;

// tauri-2.11.5/src/app.rs:1086
/// Shows the application, but does not automatically focus it.
#[cfg(target_os = "macos")]
pub fn show(&self) -> crate::Result<()>;
```

Single-window background app — this is the whole answer:

```rust
fn toggle(app: &tauri::AppHandle) -> tauri::Result<()> {
    let w = app.get_webview_window("main").unwrap();
    if w.is_visible()? {
        w.hide()?;
        #[cfg(target_os = "macos")]
        app.hide()?;              // AppKit returns activation to the previous app
    } else {
        w.show()?;
        w.set_focus()?;           // show() alone does NOT focus under Accessory policy
    }
    Ok(())
}
```

Note the asymmetry the docs make explicit: `AppHandle::show()` "does not automatically focus" —
always follow with `WebviewWindow::set_focus()`.

### Option B (explicit) — remember and re-activate via `objc2`

Use when you have multiple windows, or want deterministic restore rather than AppKit's heuristic.
Tauri 2.11.5 already depends on `objc2 0.6` / `objc2-app-kit 0.3` (`tauri-2.11.5/Cargo.toml:367`,
`:405`), so matching those versions avoids a duplicate-crate link error:

```toml
# src-tauri/Cargo.toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-app-kit = { version = "0.3", features = ["NSWorkspace", "NSRunningApplication", "NSApplication"] }
```

```rust
#[cfg(target_os = "macos")]
mod focus {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
    use std::sync::Mutex;

    static PREVIOUS: Mutex<Option<Retained<NSRunningApplication>>> = Mutex::new(None);

    /// Call IMMEDIATELY BEFORE showing/focusing your window.
    pub fn remember_frontmost() {
        unsafe {
            let ws = NSWorkspace::sharedWorkspace();
            *PREVIOUS.lock().unwrap() = ws.frontmostApplication();
        }
    }

    /// Call AFTER hiding your window.
    pub fn restore_previous() {
        if let Some(app) = PREVIOUS.lock().unwrap().take() {
            unsafe {
                app.activateWithOptions(NSApplicationActivationOptions::empty());
            }
        }
    }
}
```

Signature confirmed in `objc2-app-kit` 0.3.2:
```rust
pub fn activateWithOptions(&self, options: NSApplicationActivationOptions) -> bool;
pub fn activateFromApplication_options(&self, application: &NSRunningApplication,
                                       options: NSApplicationActivationOptions) -> bool;
```
Both are gated behind the `NSRunningApplication` cargo feature.
<https://docs.rs/objc2-app-kit/0.3.2/objc2_app_kit/struct.NSRunningApplication.html>

`activateFromApplication_options` is the "cooperative activation" variant macOS 14+ prefers, and
is more reliable than bare `activateWithOptions` under macOS's newer activation restrictions —
pass your own `NSRunningApplication.currentApplication()` as the donor.

### Option C (best UX, most work) — never take focus at all: `tauri-nspanel`

Convert the window to a **non-activating `NSPanel`** so showing it never steals activation from the
frontmost app (the Spotlight/Alfred/Raycast model). Then there is nothing to restore.

```toml
tauri-nspanel = { git = "https://github.com/ahkohd/tauri-nspanel", branch = "v2.1" }
```
Supports Tauri v2; docs at <https://docs.aremu.dev/tauri-nspanel/>. This is the approach the
community converged on for exactly this problem and is cited in tauri#7540 as the fix.

**Recommendation for this app:** start with **Option A** (`app.hide()` — zero dependencies, works
for a single-window app). If you observe focus landing on the wrong app, layer in **Option B**.
Reach for **Option C** only if you need the window to appear without ever deactivating the
user's current app (e.g. you want to keep the user's text cursor alive behind the overlay).

---

## 7. Measuring idle memory including WKWebView helper processes

### Why it's hard

WKWebView helpers (`com.apple.WebKit.WebContent`, `.GPU`, `.Networking`) are XPC services launched
by `launchd`, so `ps -o ppid` reports **1** for all of them — they never appear under your app's
process tree or RSS:

```
  PID  PPID UCOMM               RSS      VSZ
 5623     1 com.apple.WebKit  34848 507404512
```
`argv` is just the XPC bundle path, carrying no hint of the owning app. Apple's mapping mechanism
("responsible pid") is what Activity Monitor's hierarchy view uses and **has no public API** —
<https://developer.apple.com/forums/thread/95612>.

### The tools

- **`/usr/bin/footprint`** — the correct memory metric on modern macOS. Reports
  `phys_footprint`, the same number Activity Monitor's "Memory" column shows and the number the
  jetsam/memory-limit system enforces. Superior to RSS (which double-counts shared pages).
  ```
  footprint -p <pid>            # one process
  footprint -a                  # everything
  footprint --noCategories      # just the total
  footprint -f bytes            # raw bytes instead of formatted
  footprint -j out.json         # JSON output
  footprint --sample 0.5 --sample-duration 60   # sample over time
  ```
- `vmmap -summary <pid>` — region-level breakdown.
- `heap <pid>` — malloc-zone breakdown.

All three are present at `/usr/bin/` on stock macOS (verified on Darwin 25.6).

### Concrete method — resolve the responsible pid, then sum footprints

`responsibility_get_pid_responsible_for_pid()` lives in `libSystem` and is callable from Python
via `ctypes` with no sudo and no Xcode. **Verified working on this machine (Darwin 25.6).**

Save as `scripts/appmem.py`:

```python
#!/usr/bin/env python3
"""Total phys_footprint for a macOS app INCLUDING its WebKit XPC helpers.
Usage: python3 appmem.py <app-process-name-or-pid>"""
import ctypes, ctypes.util, subprocess, sys

libc = ctypes.CDLL(ctypes.util.find_library("System"))
resp = libc.responsibility_get_pid_responsible_for_pid
resp.argtypes = [ctypes.c_int]; resp.restype = ctypes.c_int

target = sys.argv[1]
ps = subprocess.run(["ps", "-Ao", "pid=,comm="], capture_output=True, text=True).stdout
procs = [(int(l.split(None, 1)[0]), l.split(None, 1)[1]) for l in ps.splitlines() if l.strip()]

roots = [int(target)] if target.isdigit() else [p for p, c in procs if target in c]
if not roots:
    sys.exit(f"no process matching {target}")

group = set(roots)
for p, _c in procs:
    if resp(p) in roots:
        group.add(p)

total = 0
for p in sorted(group):
    out = subprocess.run(["footprint", "-p", str(p), "--noCategories", "-f", "bytes"],
                         capture_output=True, text=True).stdout
    fp = 0
    for line in out.splitlines():
        if "phys_footprint" in line:
            fp = int("".join(ch for ch in line.split(":")[-1] if ch.isdigit()) or 0)
    name = dict(procs).get(p, "?").split("/")[-1]
    print(f"{p:>7}  {fp/1048576:9.1f} MB  {name}")
    total += fp
print(f"{'TOTAL':>7}  {total/1048576:9.1f} MB  ({len(group)} processes)")
```

Verified output (Mail.app used as a WKWebView-hosting stand-in):

```
$ python3 appmem.py Mail
   1389      323.2 MB  Mail
   2157       85.8 MB  com.apple.WebKit.GPU
   2158        9.9 MB  com.apple.WebKit.Networking
   2991       16.0 MB  SkyMailExtensionApp
   5623       65.1 MB  com.apple.WebKit.WebContent.EnhancedSecurity
   5624       59.0 MB  com.apple.WebKit.WebContent.EnhancedSecurity
   9794       28.7 MB  com.apple.WebKit.WebContent.EnhancedSecurity
  TOTAL      587.7 MB  (7 processes)
```

Run it as `python3 appmem.py "My Task Manager"` against the **bundled release build**
(`tauri build` → `/Applications/…`), never `tauri dev` — the dev build carries debug symbols and
the Vite dev server in the mix.

### Fallback without the private symbol (baseline diff)

If you'd rather not touch a private symbol, snapshot WebKit pids before and after launching:

```bash
before=$(pgrep -f 'com.apple.WebKit' | sort)
open -a "My Task Manager"; sleep 10
after=$(pgrep -f 'com.apple.WebKit' | sort)
new=$(comm -13 <(echo "$before") <(echo "$after"))
app=$(pgrep -x "My Task Manager")
footprint -p $app $(echo $new | tr '\n' ' ')
```
Works only when nothing else spawns WebKit processes during the window.

### Idle-measurement hygiene for this app

- Measure **after** the hidden window has loaded its webview once (a `visible: false` window still
  creates a live `WKWebView` and its WebContent process — that is the whole point of pre-creating
  it, and it is where the resident cost lives).
- Set `"backgroundThrottling"` on the window config (a real `WindowConfig` key) to let the hidden
  webview throttle timers.
- Let the app sit idle 2-5 minutes before sampling; WebKit reclaims aggressively on a delay.
  `footprint --sample 1 --sample-duration 300` captures the decay curve.
- Expect a floor of roughly **3 processes per Tauri app** (app + WebContent + Networking, plus GPU
  once anything renders) — a "small" always-resident Tauri app realistically lands in the
  100-200 MB total range, not the ~10 MB the Rust binary's own RSS suggests.

Sources: <https://docs.webkit.org/Infrastructure/MemoryInspection.html>,
<https://developer.apple.com/forums/thread/95612>, `footprint(1)` / `vmmap(1)` on macOS 26.

---

## Appendix — deltas from 2025-era assumptions

1. `create-tauri-app`'s Svelte template is **SvelteKit**, and always has been in the v2 line —
   there is no official plain-Svelte template. Use `create-vite --template svelte-ts`.
2. Current templates pin **Vite 8** and **TypeScript 6**, not Vite 5/6 and TS 5.
3. `bundle.macOS.infoPlist` is a **path string**, not an inline dictionary. Inline
   `"infoPlist": { "LSUIElement": true }` is invalid config in Tauri 2.
4. **`set_dock_visibility(bool)` is new** on `App`/`AppHandle` in the 2.11 line — a cleaner
   runtime Dock toggle than `set_activation_policy`.
5. `AppHandle::set_activation_policy` now returns `crate::Result<()>` (v1 and early v2 had a
   `&mut self`, unit-returning form only on `App`).
6. tauri#5122 (`set_activation_policy` breaks `window.show()`) is **closed/completed** since
   2024-02-02. The remaining real constraint is just that `show()` must be followed by
   `set_focus()`.
7. tauri#7540 (hiding doesn't unfocus) is **still open** and explicitly won't-fix-ish; use
   `AppHandle::hide()`.
8. `ShortcutState` is a **public field** `event.state`, not a `.state()` method, in
   global-shortcut 2.3.2 — the docs.rs prose suggesting a method is misleading.
9. `tauri-plugin-autostart` 2.5.1 still uses `auto-launch` 0.5 (LaunchAgent / AppleScript).
   **No SMAppService.** The `LaunchAgent` mode registers the inner Unix executable, producing an
   ugly Login Items entry; `AppleScript` mode registers the `.app`.
10. Tauri 3.0.0-alpha.0 landed 2026-09-13 across core and plugins. Pin to the 2.x line
    (`tauri = "2.11.5"`, `@tauri-apps/cli = "2.11.4"`) and avoid `@latest`/`@next` drift.
