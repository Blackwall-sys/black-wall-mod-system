[English](README.md) · [Português](README.pt.md) · [简体中文](README.zh.md)

# BWMS — Black Wall Mod System

**Native Cyberpunk 2077 modding runtime for macOS / Apple Silicon.**

100% Rust + redscript. No virtual machine, no Wine, no Windows streaming — the
mods run directly inside the native macOS build of the game.

> **Status: BETA 0.1.3** — early but real. This README is honest about what
> works today versus what is on the roadmap. Single-player only.

---

## What it does

BWMS is a native runtime and a set of data tools for the Apple Silicon build of
the game. As of 0.1.3:

- **In-game console + ImGui overlay** — a developer console rendered over the
  game via a Metal-based ImGui overlay. It is a **native command console, not a
  Lua interpreter**: the `lua` cargo feature is **OFF by default**, so the
  shipped build embeds no LuaJIT and `lua_stub.rs::run_code` is a no-op — it does
  **not** load CET Lua mods. Working verbs: `heal`, `level`, `money`, `give`,
  `remove`, `godmode`, `help`. It also translates two pasted CET-style lines
  (`Game.AddMoney(N)`, `Game.AddToInventory(...)`) into native calls.
- **Cheats** — god mode, carry capacity, damage and resource toggles, eddies,
  attributes, perks, vehicles, and similar single-player conveniences (~15),
  exposed as native redscript actions in a built-in **Settings > Mods** panel
  (BWMS's own redscript UI — **not** the PC NativeSettings framework).
- **3-level skip-boot** — a selector (Off / to the menu / straight to gameplay,
  zero input) with a boot loading screen and real progress bar; applies on the
  next boot.
- **Live TweakDB editing** — read and edit records in the running TweakDB
  (damage, stats, flats) without repacking archives.
- **Reflection for modders** — read and write fields and call methods by name
  against live game objects through the engine's RTTI.
- **Archive tools** — read and extract `.archive` containers. Loose-file
  `.archive` visual mods already load in-game.
- **Mod manager** — install, list, and remove mods transactionally.

Store parity: **Steam and GOG are tested and working**; Epic uses the same Mac
build but is not tested yet. Framework support (Codeware, ArchiveXL) is **not
implemented yet** — mods that depend on those frameworks won't load.

It is beta software. Expect rough edges. Always back up your saves before using
cheats (see the disclaimer at the bottom).

---

## Requirements

- macOS on **Apple Silicon** (M1 / M2 / M3 / M4).
- **Rust** (stable) installed via [rustup](https://rustup.rs), with the
  `aarch64-apple-darwin` target.
- `codesign` (ships with macOS) and `python3` (used by `build.sh`'s path-remap
  step). Note: recent macOS versions **do not** preinstall `python3`; if it's
  missing, install Apple's Command Line Tools (`xcode-select --install`) or
  python.org.
- A legitimate, installed copy of Cyberpunk 2077 (macOS build — Steam or GOG
  tested; Epic not tested yet).

You do **not** need the full Xcode app or Homebrew to build the runtime.

Add the build target once:

```sh
rustup target add aarch64-apple-darwin
```

---

## Build from source

These are the exact, reproducible commands. The runtime and all tools build
from crates.io dependencies plus the local crates included in this repository —
nothing else is required.

### 1. Runtime (the product dylib)

```sh
cd cp77-console
./build.sh
```

`build.sh` compiles with `cargo` in release mode, remaps build paths (for
privacy), strips the binary, sets the install-name to `@rpath`, signs the result
ad-hoc, and validates it by loading it with `dlopen`.

**Output:** `target/release/libcp77_console.dylib`

The `cp77-console` crate depends only on crates.io packages (`metal`, `imgui`,
`foreign-types`, etc.), so it builds on its own with no extra setup.

### 2. Data tools (optional)

Each tool is a standard Rust crate. Build any of them with:

```sh
cargo build --release
```

run from inside that tool's directory:

| Directory          | What it does                          |
| ------------------ | ------------------------------------- |
| `archive-tool`     | Read / extract `.archive` containers  |
| `tweakdb-tool`     | Read / edit `tweakdb.bin`             |
| `input-loader`     | Merge keybind / input definitions     |
| `mac-mod-manager`  | Install / list / remove mods          |
| `bwms`             | Unified command-line front-end        |

`bwms` and `mac-mod-manager` use the local `bwms-core` crate, which is included
in this repository — no external fetch is needed for it.

### 3. redscript scripts (in-game)

The redscript sources live in `r6/scripts/blackwall-mods/*.reds`. They are
compiled by the bundled `scc` redscript compiler **at install time** by the
installer — there is no separate manual compile step for end users.

---

## Install (end users)

For players who just want to run the mods (no development needed):

1. Download the release zip and unzip it.
2. Run **`INSTALAR.command`** (or `bwms-install.sh "<game dir>"` from a
   terminal).
3. Launch the game from **Steam (Play)** — not from Finder.

The installer adds an `LC_LOAD_DYLIB` entry to the game binary and re-signs the
`.app` ad-hoc while **preserving CDPR's original entitlements**. It uses only
base macOS tools (`codesign`, `xattr`): no password, no changes to SIP or
Gatekeeper, and it is fully reversible.

To uninstall:

```sh
INSTALAR.command --restore
```

or run `extras/DESINSTALAR.command`.

---

## Repository layout

```
cp77-console/            The runtime dylib (in-game console + ImGui overlay)
bwms-core/               Shared library (classify / theme / apply core)
bwms/                    Unified command-line tool
archive-tool/            Read / extract .archive containers
tweakdb-tool/            Read / edit tweakdb.bin
input-loader/            Merge keybind / input definitions
mac-mod-manager/         Install / list / remove mods
r6/scripts/blackwall-mods/   redscript sources (compiled at install time)
example-rust-plugin/     Example native plugin
INSTALAR.command         End-user installer (entry point)
bwms-install.sh          Installer script (terminal / scriptable)
```

---

## License

Dual-licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

at your option.

---

## Trademark / not affiliated

"Cyberpunk 2077" is a trademark of CD PROJEKT S.A.; this project is not
affiliated with or endorsed by CD PROJEKT.

This project ships **no game assets or data of any kind**. You must own a legal
copy of the game to use it.

---

## Notes

- **Single-player only.** There is no anti-cheat support and none is intended.
- **Back up your saves** before using cheats.
- BWMS is **free**. Donations are appreciated but never required.

Authored by **Blackwall**.

Project home: `https://github.com/Blackwall-sys/black-wall-mod-system`
