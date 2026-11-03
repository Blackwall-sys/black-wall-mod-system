[English](README.md) · [Português](README.pt.md) · [简体中文](README.zh.md)

# BWMS — Black Wall Mod System

**Native Cyberpunk 2077 modding runtime for macOS / Apple Silicon.**

100% Rust + redscript. No virtual machine, no Wine, no Windows streaming — the
mods run directly inside the native macOS build of the game.

> **Status: BETA 0.1.5** — early but real. This README is honest about what
> works today versus what is on the roadmap. Single-player only.

---

## What it does

BWMS is a native runtime and a set of data tools for the Apple Silicon build of
the game. As of 0.1.5:

- **In-game console** — a developer console drawn over the game by BWMS's own
  Metal renderer (no third-party UI library). It is a **native command console**,
  with zero Lua in the code (0% Lua). Working verbs: `heal`, `level`, `money`, `give`,
  `remove`, `godmode`, `help`. BWMS's Rust also understands pasted CET lines
  (`Game.AddMoney(N)`, `Game.AddToInventory(...)` and `callm Namespace.Method(args)`)
  and turns them into native calls.
- **CET mods** — a CET mod made of `.lua` files runs from the `Mods/` folder: BWMS's own
  Rust reads and runs Windows `.lua` files, with no third-party code. Its window draws in
  the BWMS terminal. A mod that needs a `.dll` does not run. Tested in game with JB Third Person.
- **Cheats** — god mode, carry capacity, damage and resource toggles, eddies,
  attributes, perks, vehicles, and similar single-player conveniences (~15),
  exposed as native redscript actions in a built-in **Settings > Mods** panel
  (BWMS's own redscript UI — **not** the PC NativeSettings framework).
- **3-level skip-boot** — a selector (Off / to the menu / straight to gameplay,
  zero input) with a boot loading screen and real progress bar; applies on the
  next boot. A fresh install comes set to "straight to gameplay".
- **Live TweakDB editing** — read and edit records in the running TweakDB
  (damage, stats, flats) without repacking archives.
- **Reflection for modders** — read and write fields and call methods by name
  against live game objects through the engine's RTTI.
- **Archive tools** — read and extract `.archive` containers. Loose-file
  `.archive` visual mods already load in-game.
- **Two ways to add a mod** — the game's own Mods menu (mod.io) and the `Mods/`
  folder in the game folder (Nexus and other sites). The badge in the top right
  corner and the console's Mods tab show the mods from both, each with a green,
  yellow or red light.

Store: **Steam, game version 2.5, is tested and working.** Version 2.3.1 is no
longer supported. The installer refuses GOG and Epic: bwms hooks addresses of
the Steam build, and on another build the game closes at boot
(`BWMS_IGNORE_BUILD_CHECK=1` forces the install, at your own risk). Frameworks:
Windows mods written for Codeware/ArchiveXL/TweakXL work within what bwms
covers — Virtual Atelier and Equipment-EX compile and load, and on 2.5 their
screens opened in game with the mod's items and controller (development build). `.xl` files in the game's `Mods/` folder go
into the runtime tables at boot; outside `Mods/`, `.archive.xl` sections are not
applied.

It is beta software. Expect rough edges. Always back up your saves before using
cheats (see the disclaimer at the bottom).

---

## What changed since 0.1.4

Full list, in Portuguese: [`docs/CHANGELOG-0.1.5-PUBLICO.md`](docs/CHANGELOG-0.1.5-PUBLICO.md).

- **Cyberpunk 2.5.** 0.1.5 is for game version 2.5 (Steam); 2.3.1 is no longer
  supported, and the installer checks 22 points of the binary. 2.5 brought
  native `CallbackSystem` (only `Session/*` and `Resource/*` events),
  `ScriptableService`, `GameInstance.GetCallbackSystem()` and UI methods
  (`GetParentWidget`, focus, text wrapping, `SetPath`): BWMS uses the game's
  where it fits and completes where it falls short — `Entity/*`, `Input/*`,
  `Component/*` events and the target filters stay BWMS's. On the Mac, 2.5
  does not fire the session events (the game never creates its own
  `CallbackSystem`, measured in game): BWMS delivers `Session/Start`,
  `Session/Ready` and `Session/End`, and `BeforeSave`, `AfterSave`, `Pause`
  and `Resume` do not fire. 2.5 dropped the `Log`/`LogChannel`/`LogWarning`/`LogError`/`Trace`
  declarations; BWMS declares them again so Windows mods compile. Mods written
  for Codeware keep compiling as before. Equipment-EX works: BWMS accepts its
  TweakDB writes (`SetFlat` of lists and text: 1898 of 1898; slot `CreateRecord`
  by short name: 54 of 54).
- **`Mods/` folder.** Drop a Nexus mod's folder as it comes in the zip
  (`archive/pc/mod/`, `r6/scripts/`, `r6/tweaks/`, or loose files) into `Mods/`
  in the game folder (next to `Cyberpunk2077.app`). On the next boot, with no
  manual step, BWMS uses its `.archive` (placed in the game's own mod folder,
  `archive/Mac/mod/BWMS-Mods/`, by clone with no extra disk space, and mounted
  by the game itself with the highest priority: wins over the base game and
  Phantom Liberty), `.xl` (`*.archive.xl` and `*.xl`: the runtime
  tables are generated at boot; `localization.onscreens` and `factories` go to
  2.5's own ArchiveXL, applied by the game), `.yaml`/`.tweak` (applied when a
  save loads, the same path as mod tweaks) and `.reds`. `Mods/` wins over the
  game and, in `.archive`, over mod.io; between two folders in `Mods/`, the first
  in path order (character by character, uppercase first) wins for archives and
  `.xl`, and the later one wins for tweaks.
- **Scripts compiled when the game opens.** The `.reds` from `r6/scripts`,
  `Mods/` and plugin scripts are compiled before the game reads the scripts, only
  when something changed. Measured outside the game, with a change: ~0.7 s with
  no broken mod, ~1.3 s with one, ~2 s in a cascade (a mod broken by another
  broken one); 3–5 ms without a change. In a real boot, 877 ms with Virtual
  Atelier and Equipment-EX, 48 `.reds` from `Mods/`. Without a change, also
  measured in game: 7 ms (boot of 2026-10-18).
- **A mod with a broken script leaves by itself.** If a `.reds` of a mod in
  `Mods/` does not compile, only that mod is left out; the other mods and BWMS
  load and the game opens. The mod is red in the report with the file:line of
  the error and what to do ("fix the file, or take the mod out of the folder").
  Nothing is deleted or moved. Proven in game on 2026-10-24 (development
  build). An error in BWMS itself or in a mod placed straight in `r6/scripts`
  (Windows style) keeps the old behavior: the game opens with the scripts of the
  last compile that worked, and the error goes to the report with file:line; no
  need to run the installer again. That case was tested outside the game, not in
  game.
- **Report.** `Mods/BWMS-RELATORIO.txt`, rewritten every time the game opens, in
  Portuguese and English: per mod, what loaded (archives, `.xl`, tweaks,
  `.reds`), what was left out and why, refused archives with the reason,
  resource conflicts between archives (who wins and how to flip it: rename the
  folder or the file), `.xl` sections BWMS does not apply yet, and the Scripts
  section. Each mod has a `luz / light` line (green: loaded whole, yellow:
  partly, red: does not work); the `[mod.io]` section lists the mod.io mods;
  paths are relative, with no path of your machine. A `.lua` shows as "CET mod: BWMS runs it from the Mods folder; whatever
  needs a .dll does not"; a `.dll` as "Windows plugin, does not load on a Mac".
- **Widget spawn in the user build.** The controller override on widget spawn
  (`Item:Controller`, used by Virtual Atelier and Equipment-EX), with the
  external library's dependency injected through RTTI, and the
  `InkWidget/Spawn` event with the `inkWidgetTarget` filter are now in the user
  build. The override and the injection work in synchronous and asynchronous
  spawns; the `InkWidget/Spawn` event comes only from synchronous spawns.
- **Less overhead.** What BWMS adds to each game script call went from ~33 ns
  (0.1.4) to ~8 ns, 4× less — same test on both sources, measured outside the
  game. The public dylib went from 2.76 MB to 1.99 MB (−28%). Boot time shows no
  measurable gain, and this is not an FPS measurement.
- **Zero third-party code in the product.** The dylib and `bwms-helper` link no
  third-party crate. The 0.1.4 dylib carried a third-party Metal binding, a C++
  UI library and a PNG decoder; the Metal bridge is now BWMS's own, and the
  splash image is read by macOS's own image framework.
- **Script files with BWMS's own names.** The `.reds` that carried another
  framework's name are now `bwms-*` / `ponte-*`. Updating from 0.1.4, the
  installer deletes the old names by an explicit list; other mods' files in the
  folder stay.
- **Two ways to add a mod; the BWMS library is gone.** A mod goes in through the
  game's own Mods menu (mod.io; account and internet to install) or the `Mods/`
  folder (Nexus and other sites). The BWMS library left: `BWMS/mods/<category>/`,
  `nexus/mods/`, the `bwms mods` command that installed, and the on/off switch
  in the Mods tab. Updating from 0.1.2–0.1.4, the installer moves each mod that
  was ON in the library to `Mods/<name>` by APFS clone (the original stays). A
  name already in `Mods/` is not overwritten (the mod stays where it was, and
  the report says to compare the two folders); a mod that was OFF stays in
  `BWMS/mods` and shows in the report; the files the library put in the game
  (archives with the BWMS prefix in `archive/Mac/content/`, tweaks, scripts)
  come out by the list it wrote. Nothing of the user's is deleted: the report
  suggests moving the `BWMS/` folder to the Trash.
- **Badge and Mods tab show both ways.** The badge in the top right corner and
  the console's Mods tab show each mod from `Mods/` and from mod.io with a green
  (loaded whole), yellow (partly) or red (does not work) light, and the count.
  Conflicts between `Mods/` and mod.io (and between mod.io mods) show who wins:
  in `.archive`, `Mods/` wins (the game mounts mod.io after it, in the same
  group). BWMS only reads mod.io: it never writes to its folder or to
  `EnabledMods.json`. No checkboxes. Before, the badge showed a grey "0" with
  mods in `Mods/`.
- **`factories` applied by the game.** 2.5's own ArchiveXL applies the
  `factories` section of the `.xl` files in `Mods/`; BWMS no longer has a hook or
  a table for it. If the game cannot apply it (for example, the `.csv` is in no
  archive), the mod turns yellow with the reason.
- **Hang before "press any key" fixed.** BWMS's hook on video opening did not
  pass one argument (the flags) on to Bink; depending on the boot, that stuck the
  game's reading thread. Fixed on 2026-10-24. Two other BWMS hooks were fixed
  too: one did not pass on an argument of the engine's update registration, and
  another, in an 8-byte stretch, overwrote the start of the next function.
- **"Clear wanted level" fixed.** The cheat asks the game's own wanted-level
  system for the same de-escalation the game uses. Before, it only wrote the
  wanted-level facts and the wanted level stayed.
- **Resource cache reader fixed.** Reading the resources already loaded
  (`BwmsDepotIsResourceLoaded`, `BwmsDepotLoadedResourceClass` and what depends
  on it) read an address that only worked by a memory coincidence, and could fail
  with archives in the `Mods/` folder. It now reads the map through the engine's
  loader. Checked in game with 4 archives in `Mods/`.

Proven in game on 2.5 (development build, 2026-10-23): boot to gameplay with the
save loaded on its own; tweaks through `Mods/` with a BWMS test mod (13 of 13
values in the TweakDB); `.archive` from `Mods/` mounted by the game (4 of 4); the
Cheats tab (God Mode, invisibility, clear wanted level); the Virtual Atelier and
Equipment-EX screens with the mod's items and controller (4 of 4). On 2026-10-24:
a mod with a broken script leaving by itself, the badge and the Mods tab with
mods from `Mods/` and mod.io, `factories` applied by the game, and the full test
script after the hang fix. Not proven yet: a real TweakXL mod from Nexus through
`Mods/`, a key delivered to a mod widget, and the public build from the zip
installed on 2.5. `.xl` through the game's native path: 2.5 ships its own
ArchiveXL, which reads `.json`, and the UI texts (`localization.onscreens`) of
`.xl` files in `Mods/` now go through it (proven with Equipment-EX and Virtual
Atelier), and so does `factories`. The other sections are still applied by
BWMS, and so are the texts when a mod.io mod or another game mod folder brings
texts too; `Mods/BWMS-RELATORIO.txt` says who applies each section.

## Official mods (mod.io) in 2.5 and BWMS

Game 2.5 has official mods through mod.io, in the game's Mods menu. Installing
them needs a CD PROJEKT RED account and internet; after that they run offline.
They stay in the game's menu; Nexus and manual mods go in `Mods/`. Those are the
two ways to add a mod. BWMS only reads the mod.io folder (it never writes to it
or to `EnabledMods.json`) and shows those mods in the badge, the Mods tab and the
report. BWMS needs no internet and no account. Game 2.5 marks with an icon a save made with mods in
the game's mod folder; BWMS puts the archives from `Mods/` there, so the save
comes out marked. On a Mac it is just the icon; achievement blocking on a Mac:
not measured.

---

## Requirements

- macOS on **Apple Silicon** (M1 / M2 / M3 / M4). Tested on macOS 27; macOS 26
  was not tested by us (there is one crash report with 0.1.4 on macOS 26.6.2).
- **Rust** (stable) installed via [rustup](https://rustup.rs), with the
  `aarch64-apple-darwin` target.
- `codesign` (ships with macOS). Nothing else: the Mach-O alignment step the
  build needs is a small Rust binary that builds along with everything else, so
  **no `python3` is required** — it used to be, and no longer is.
- A legitimate, installed copy of Cyberpunk 2077 (macOS build from Steam,
  version 2.5).

You do **not** need the full Xcode app or Homebrew to build the runtime.

Add the build target once:

```sh
rustup target add aarch64-apple-darwin
```

---

## Build from source

These are the exact, reproducible commands. The runtime and all tools build
only from the local crates included in this repository, with no crates.io
dependencies — nothing else is required.

### 1. Runtime (the product dylib)

```sh
cd cp77-console
./build-core.sh
```

`build-core.sh` compiles with `cargo` (profile `publico`), remaps build paths (for
privacy), applies the symbol-table alignment fix (`fix-symtab-align`, built from
`src/bin/`), sets a clean install-name, signs the result ad-hoc and prints the
alignment check (`stroff mod8=0` = loadable). With a nightly toolchain and `rust-src`
installed it also rebuilds `std` with `panic=immediate-abort` and the dylib comes out
smaller; on stable the only difference is size. If the game binary is on disk
(`CP77_DIR`), it first regenerates the address table in `dados/`; otherwise the table
already in this repository is used.

**Output:** `target/libcp77_console-publico.dylib`

The `cp77-console` crate has no crates.io dependencies: besides the standard library
it uses only `bwms-primitives` from this repository, so it builds offline with no
extra setup.

### 2. Data tools (optional)

The data tools are one Rust crate, `bwms`, with a subcommand per tool. Build it with:

```sh
cargo build --release
```

run from inside `bwms/`:

| Directory          | What it does                          |
| ------------------ | ------------------------------------- |
| `bwms`             | Unified command-line front-end; `bwms mods checar` compiles the game's scripts without touching the cache and points at each `native func` nothing registers, `bwms tweak` reads / edits `tweakdb.bin`, `bwms archive` reads / extracts `.archive` containers, `bwms input` merges keybind / input definitions |

`bwms` uses the local `bwms-primitives` crate, which is included
in this repository — no external fetch is needed for it.

### 3. redscript scripts (in-game)

The redscript sources live in `r6/scripts/blackwall-mods/*.reds`. They are
compiled by the bundled `scc` redscript compiler **at install time** by the
installer, and again when the game opens if a script changed — there is no
separate manual compile step for end users. The installer copies `scc` and
`libscc_lib.dylib` to `red4ext/`; that copy is what compiles when the game
opens.

---

## Install (end users)

For players who just want to run the mods (no development needed):

1. Download the release zip and unzip it.
2. Run **`INSTALAR.command`** (or `bwms-install.sh "<game dir>"` from a
   terminal).
3. Launch the game from **Steam (Play)** — not from Finder.

The installer adds an `LC_LOAD_DYLIB` entry to the game binary and re-signs the
`.app` ad-hoc with only the `com.apple.security.cs.*` relaxations the dylib
needs. CDPR's identity entitlements are dropped: they are bound to CDPR's
certificate, and on an ad-hoc signature macOS refuses to launch the game with
them. It uses only base macOS tools (`codesign`, `xattr`): no password, no
changes to SIP or Gatekeeper.

Before uninstalling, turn God Mode and invisibility off in the game and save:
the game itself writes God Mode into the save, and invisibility is an effect the
game marks to be saved.

To uninstall, run `extras/DESINSTALAR.command` (from a terminal:
`bwms-install.sh --restore "<game dir>"`). It puts the original binary back and
re-signs the game ad-hoc; CDPR's signature only comes back with Steam's "Verify
integrity of game files". If Steam already put its own binary back (a game update
or "Verify integrity"), it copies nothing over it and only removes the bwms
scripts. Optional full cleanup: delete `red4ext/`,
`Mods/BWMS-RELATORIO.txt` and, in `r6/cache/`, `final.redscripts.bwms-bak`,
`final.redscripts.bk` and `final.redscripts.ts`.

---

## Repository layout

```
cp77-console/            The runtime dylib (in-game console, native bridges)
bwms/                    Unified command-line tool (`bwms mods checar`: check the game's scripts; `bwms tweak`: read / edit tweakdb.bin; `bwms archive`: read / extract .archive containers; `bwms input`: merge keybind / input definitions) + `bwms-helper`, the small privilege-free helper used by the installer (`src/bin/`)
bwms-primitives/         Single source of what the runtime and the tools compute the same way (leaf crate): hashes, game addresses, TweakXL parser, `Mods/` runtime tables, library migration, pure cores
example-plugin/          Example native plugin (C ABI), for third-party authors
                         (the release package ships this same source in `engine/example-plugin/`)
r6/scripts/blackwall-mods/
                         redscript sources, compiled at install time — the same
                         set the released package ships (production only; no test
                         or diagnostic files). The path mirrors where they land
                         inside the game.
docs/                    Modder API, third-party mod sources, public changelog
INSTALAR.command         End-user installer (double-click entry point)
bwms-install.sh          Same installer, for terminal / scripting
```

Every crate is its own workspace (`cargo build --release` inside it). The runtime dylib is the
one exception — build it with `cp77-console/build-core.sh`, never bare `cargo build`, because
macOS 26+ needs a symbol-table alignment fix that the script applies.

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
