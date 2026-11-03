[English](README.md) · [Português](README.pt.md) · [简体中文](README.zh.md)

# BWMS — Black Wall Mod System

**面向 macOS / Apple Silicon 的原生 Cyberpunk 2077 模组运行时。**

100% Rust + redscript。没有虚拟机，没有兼容层，没有 Windows 串流——模组直接运行在游戏的原生 macOS 版本内部。

> **状态：BETA 0.1.5** —— 早期但真实。本 README 如实说明今天哪些功能可用、哪些还在路线图上。仅限单人模式。

---

## 它能做什么

BWMS 是一个原生运行时，外加一套针对 Apple Silicon 版游戏的数据工具。截至 0.1.5：

- **游戏内控制台** —— 由 BWMS 自己的 Metal 渲染器在游戏之上绘制的开发者控制台（不使用第三方界面库）。它是一个**原生命令控制台**，代码中零 Lua（0% Lua）。命令：`heal`、`level`、`money`、`give`、`remove`、`godmode`、`help`。BWMS 的 Rust 代码还能理解粘贴进来的 Windows Lua 控制台代码行（`Game.AddMoney(N)`、`Game.AddToInventory(...)` 和 `callm Namespace.Method(args)`），并把它们翻译成原生调用。
- **`.lua` mod** —— 由 `.lua` 文件组成的 Windows mod 可以从 `Mods/` 文件夹运行：BWMS 自己的 Rust 代码读取并执行 Windows 的 `.lua` 文件，不含第三方代码。mod 的窗口显示在 BWMS 终端里。需要 `.dll` 的 mod 不能运行。已在游戏中用一个第三人称视角 mod 测试。
- **作弊功能（Cheats）** —— 无敌模式、负重容量、伤害与资源开关、欧元（eddies）、属性、专长、载具，以及类似的单人便利功能（约 15 个），作为原生 redscript 动作暴露在内置的 **设置 > Mods** 面板里（BWMS 自己的 redscript UI）。
- **三档跳过启动** —— 一个选择器（关闭 / 到主菜单 / 直接进入游戏，零输入），带有启动加载画面和真实进度条；从下一次启动开始生效。新安装默认就是"直接进入游戏"。
- **实时 TweakDB 编辑** —— 在运行中的 TweakDB 中读取和编辑记录（伤害、属性、flats），无需重新打包归档文件。
- **面向模组作者的反射（Reflection）** —— 通过引擎的 RTTI，按名称读写字段、调用方法，作用于游戏的实时对象。
- **归档工具** —— 读取并提取 `.archive` 容器。散装（loose）`.archive` 视觉 mod 已经能在游戏内加载。
- **放入 mod 的两种方式** —— 游戏自己的 Mods 菜单（mod.io）和游戏文件夹里的 `Mods/` 文件夹（Nexus 和其他网站）。右上角的标记和控制台的 Mods 选项卡会显示两者的 mod，每个 mod 带一盏绿、黄或红灯。

商店：**Steam 上的 2.5 版游戏已测试可用。** 2.3.1 版不再受支持。安装程序会拒绝 GOG 和 Epic：bwms 使用 Steam build 的地址，在其他 build 上游戏会在启动时关闭（`BWMS_IGNORE_BUILD_CHECK=1` 可强制安装，风险自负）。Windows mod：`.reds` 调用游戏本身没有的函数的 mod、带 `.xl` 的 mod 和带 `.yaml`/`.tweak` 的 mod，在 bwms 覆盖范围内可用 —— 两个带有自己界面的 Windows mod 可以编译并加载，在 2.5 上它们的界面已在游戏内打开，显示该 mod 的物品和 controller（开发版 build）。游戏 `Mods/` 文件夹里的 `.xl` 在启动时进入运行时表；在 `Mods/` 之外，`.archive.xl` 段落不会被应用。

这是 beta 软件。请预期会有粗糙之处。使用作弊功能前请务必备份你的存档（见底部免责声明）。

---

## 自 0.1.4 以来的变化

完整列表（葡萄牙语）：[`docs/CHANGELOG-0.1.5-PUBLICO.md`](docs/CHANGELOG-0.1.5-PUBLICO.md)。

- **Cyberpunk 2.5。** 0.1.5 适用于 2.5 版游戏（Steam）；2.3.1 版不再受支持，安装程序会检查二进制文件的 22 个点。2.5 原生带来了 `CallbackSystem`（只有 `Session/*` 和 `Resource/*` 事件）、`ScriptableService`、`GameInstance.GetCallbackSystem()` 以及界面方法（`GetParentWidget`、焦点、文本换行、`SetPath`）：BWMS 在合适的地方使用游戏自带的，在不足的地方补齐 —— `Entity/*`、`Input/*`、`Component/*` 事件和目标过滤器仍由 BWMS 提供。在 Mac 上 2.5 不触发会话事件（游戏从不创建它自己的 `CallbackSystem`，游戏内实测）：BWMS 提供 `Session/Start`、`Session/Ready` 和 `Session/End`，`BeforeSave`、`AfterSave`、`Pause`、`Resume` 不会触发。2.5 去掉了 `Log`/`LogChannel`/`LogWarning`/`LogError`/`Trace` 的声明；BWMS 重新声明它们，让 Windows mod 能编译。`.reds` 调用游戏本身没有的函数的 Windows mod 照常编译。写入 TweakDB 的 Windows mod 可以用了：BWMS 接受它的写入（列表和文本的 `SetFlat`：1898 个中 1898 个；按短名的槽位 `CreateRecord`：54 个中 54 个）。
- **`Mods/` 文件夹。** 把 Nexus 上下载的 mod 文件夹按 zip 里的原样（`archive/pc/mod/`、`r6/scripts/`、`r6/tweaks/` 或散装文件）放进游戏文件夹里的 `Mods/`（与 `Cyberpunk2077.app` 同级）。下一次启动时，无需任何手动步骤，BWMS 就会使用其中的 `.archive`（放进游戏自己的 mod 文件夹 `archive/Mac/mod/BWMS-Mods/`，以克隆方式，不占额外磁盘空间，由游戏自己以最高优先级挂载：覆盖基础游戏和 Phantom Liberty）、`.xl`（`*.archive.xl` 和 `*.xl`：运行时表在启动时生成；`localization.onscreens` 和 `factories` 交给 2.5 自带的 `.xl` 支持，由游戏应用）、`.yaml`/`.tweak`（走与 mod tweak 相同的路径，在读取存档时应用）以及 `.reds`。`Mods/` 优先于游戏本身，在 `.archive` 上也优先于 mod.io；`Mods/` 里两个文件夹之间，archive 和 `.xl` 以路径顺序（逐字符比较，大写字母排在小写字母之前）靠前的为准，tweak 以靠后的为准。
- **打开游戏时编译脚本。** `r6/scripts`、`Mods/` 中的 `.reds` 以及插件脚本，在游戏读取脚本之前编译，且只在有改动时才编译。在游戏外实测，有改动时：没有出错的 mod 约 0.7 秒，有 1 个约 1.3 秒，连锁出错（一个 mod 因另一个出错的 mod 而出错）约 2 秒；没有改动时 3–5 ms。一次真实启动中，带那两个有自己界面的 Windows mod、来自 `Mods/` 的 48 个 `.reds`，用时 877 ms。没有改动时，在游戏内也测过：7 ms（2026-10-18 的启动）。
- **脚本有错的 mod 会自动排除。** 如果 `Mods/` 里某个 mod 的 `.reds` 编译不过，只有这个 mod 被排除；其他 mod 和 BWMS 照常加载，游戏照常打开。该 mod 在报告里显示为红色，附错误的文件:行号和处理办法（"修正该文件，或把这个 mod 移出文件夹"）。不会删除或移动任何东西。已于 2026-10-24 在游戏内验证（开发版 build）。BWMS 自身出错，或直接放进 `r6/scripts` 的 mod（Windows 方式）出错时，保持原来的行为：游戏用上一次成功编译的脚本打开，错误会带着文件和行号（文件:行号）写进报告；不需要再运行一次安装程序。这种情况是在游戏外测试的，没有在游戏内测试。
- **报告。** `Mods/BWMS-RELATORIO.txt`，每次打开游戏时重写，用葡萄牙语和英语写成：按 mod 列出加载了什么（archive、`.xl`、tweak、`.reds`）、哪些没有加载及原因、被拒绝的 archive 及原因、archive 之间的资源冲突（谁胜出，以及如何反过来：重命名文件夹或文件）、BWMS 还不应用的 `.xl` 段落，以及 Scripts 部分。每个 mod 都有一行 `luz / light`（绿：完整加载，黄：部分加载，红：不能用）；`[mod.io]` 部分列出 mod.io 的 mod；路径是相对路径，不含你机器上的路径。`.lua` 显示为 BWMS 通过 Mods 文件夹运行的 Lua mod，需要 `.dll` 的部分除外；`.dll` 显示为在 Mac 上不加载的 Windows 插件。
- **用户版中的 widget 生成。** widget 生成时的 controller 覆盖（`Item:Controller`，那两个有自己界面的 Windows mod 都在用）——通过 RTTI 注入外部库依赖——以及带 `inkWidgetTarget` 过滤器的 `InkWidget/Spawn` 事件，现在已进入用户版。覆盖和注入在同步和异步生成中都适用；`InkWidget/Spawn` 事件只来自同步生成。
- **开销更小。** BWMS 在游戏每次脚本调用上增加的开销从 ~33 ns（0.1.4）降到 ~8 ns，只有原来的 1/4 —— 两份源码用同一个测试，在游戏外测得。公开版 dylib 从 2.76 MB 降到 1.99 MB（−28%）。启动时间没有可测量的提升；这也不是 FPS 数据。
- **产品中零第三方代码。** dylib 和 `bwms-helper` 不链接任何第三方 crate。0.1.4 的 dylib 带有第三方的 Metal 绑定、一个 C++ 界面库和一个 PNG 解码器；现在 Metal 桥接是 BWMS 自己的，启动画面的图片由 macOS 自带的图像框架读取。
- **脚本文件使用 BWMS 自己的名字。** 原来带有其他框架名字的 `.reds` 现在叫 `bwms-*` / `ponte-*`。从 0.1.4 更新时，安装程序按一份明确的列表删除旧文件名；文件夹里其他 mod 的文件保留不动。
- **放入 mod 的两种方式；BWMS 的 mod 库已移除。** mod 通过游戏自己的 Mods 菜单（mod.io；安装需要账号和网络）或 `Mods/` 文件夹（Nexus 和其他网站）进入游戏。移除的有：`BWMS/mods/<类别>/`、`nexus/mods/`、用来安装的 `bwms mods` 命令，以及 Mods 选项卡里的开关。从 0.1.2–0.1.4 更新时，安装程序会把库里处于开启状态的每个 mod 以 APFS 克隆方式移到 `Mods/<名字>`（原文件保留）。`Mods/` 里已有同名文件夹时不会覆盖（该 mod 留在原处，报告会提示比较两个文件夹）；处于关闭状态的 mod 留在 `BWMS/mods`，并列在报告里；库放进游戏的文件（`archive/Mac/content/` 里带 BWMS 前缀的 archive、tweak、脚本）按它记录的列表移除。用户的东西一样都不会删除：报告会建议把 `BWMS/` 文件夹移到废纸篓。
- **标记和 Mods 选项卡显示两种方式的 mod。** 右上角的标记和控制台的 Mods 选项卡会显示 `Mods/` 和 mod.io 里的每个 mod，带绿（完整加载）、黄（部分加载）或红（不能用）灯，并显示数量。`Mods/` 与 mod.io 之间（以及 mod.io 与 mod.io 之间）的冲突会写明谁胜出：在 `.archive` 上 `Mods/` 胜出（游戏在同一组里、在它之后挂载 mod.io）。BWMS 只读取 mod.io：从不写入它的文件夹，也不写 `EnabledMods.json`。没有勾选框。以前 `Mods/` 里有 mod 时，标记显示灰色的"0"。
- **`factories` 由游戏应用。** 2.5 自带的 `.xl` 支持会应用 `Mods/` 里 `.xl` 的 `factories` 段落；BWMS 不再为它设钩子或表。如果游戏无法应用（例如 `.csv` 不在任何 archive 里），该 mod 显示为黄色并写明原因。
- **"按任意键"之前的卡死已修复。** BWMS 在视频打开处的钩子没有把一个参数（flags）传给游戏的视频库；在某些启动中，这会卡住游戏的读取线程。已于 2026-10-24 修复。BWMS 的另外 2 个钩子也已修复：一个没有传递引擎更新注册的一个参数，另一个在 8 字节的区段里覆盖了相邻函数的开头。
- **"清除通缉等级" 已修复。** 该作弊请求游戏自己的通缉系统执行游戏本身使用的同一种降级。以前它只写入通缉等级的 facts，通缉等级并不会消失。
- **资源缓存读取已修复。** 读取已加载的资源（`BwmsDepotIsResourceLoaded`、`BwmsDepotLoadedResourceClass` 以及依赖它的函数）时读的是一个只因内存巧合才有效的地址，`Mods/` 文件夹里有 archive 时可能会失败。现在通过引擎的 loader 读取这张映射表。已在游戏内核对（`Mods/` 里有 4 个 archive）。

已在 2.5 上于游戏内验证（开发版 build，2026-10-23）：启动到游戏内并自动读取存档；用 BWMS 测试 mod 通过 `Mods/` 应用 tweak（TweakDB 中 13 个值中 13 个）；`Mods/` 的 `.archive` 由游戏挂载（4 个中 4 个）；作弊选项卡（无敌、隐身、清除通缉）；那两个 Windows mod 的界面显示该 mod 的物品和 controller（4 个中 4 个）。2026-10-24：脚本有错的 mod 自动排除、标记和 Mods 选项卡显示 `Mods/` 与 mod.io 的 mod、`factories` 由游戏应用，以及修复卡死后的完整测试流程。尚未验证：通过 `Mods/` 的来自 Nexus 的真实 `.yaml`/`.tweak` mod、按键传给 mod 的 widget，以及 zip 里的公开版 build 安装在 2.5 上。通过游戏原生途径的 `.xl`：2.5 自带 `.xl` 支持，读取 `.json`；`Mods/` 中 `.xl` 的界面文本（`localization.onscreens`）改由它应用（已用那两个有自己界面的 Windows mod 验证），`factories` 也是。其他段落仍由 BWMS 应用；当 mod.io 的 mod 或游戏的其他 mod 文件夹也带有文本时，文本也由 BWMS 应用。`Mods/BWMS-RELATORIO.txt` 会写明每个段落由谁应用。

## 2.5 的官方 mod（mod.io）与 BWMS

2.5 版游戏通过 mod.io 提供官方 mod，在游戏的 Mods 菜单里。安装需要 CD PROJEKT RED 账号和网络；之后可以离线运行。它们仍留在游戏菜单里；Nexus 和手动安装的 mod 放进 `Mods/`。这就是放入 mod 的两种方式。BWMS 只读取 mod.io 的文件夹（从不写入它，也不写 `EnabledMods.json`），并在标记、Mods 选项卡和报告里显示这些 mod。BWMS 不需要网络，也不需要账号。2.5 版游戏会给在游戏 mod 文件夹里有 mod 时做的存档标上一个图标；BWMS 会把 `Mods/` 里的 archive 放到那里，所以存档会被标记。在 Mac 上只是一个图标；在 Mac 上是否会阻止成就：未测量。

---

## 系统要求

- 运行于 **Apple Silicon**（M1 / M2 / M3 / M4）的 macOS。已在 macOS 27 上测试；macOS 26 未经我们测试（有一份 0.1.4 在 macOS 26.6.2 上崩溃的报告）。
- 通过 [rustup](https://rustup.rs) 安装的 **Rust**（stable），并带有 `aarch64-apple-darwin` target。
- `codesign`（随 macOS 提供）。仅此而已：构建所需的 Mach-O 对齐步骤现在是一个随其余部分一起编译的小型 Rust 程序，因此**不再需要 `python3`**。以前需要，现在不需要了。
- 一份合法的、已安装的 Cyberpunk 2077（来自 Steam 的 macOS 版，2.5）。

构建运行时**不**需要完整的 Xcode 应用或 Homebrew。

一次性添加构建 target：

```sh
rustup target add aarch64-apple-darwin
```

---

## 从源码构建

以下是确切的、可复现的命令。运行时及所有工具仅从本仓库中包含的本地 crate 构建，不依赖任何 crates.io 包——除此之外无需任何其他东西。

### 1. 运行时（产品 dylib）

```sh
cd cp77-console
./build-core.sh
```

`build-core.sh` 用 `cargo`（`publico` profile）编译，重映射构建路径（出于隐私考虑），应用符号表对齐修正（`fix-symtab-align`，由 `src/bin/` 编译），设置干净的 install-name，对结果进行 ad-hoc 签名，并打印对齐检查结果（`stroff mod8=0` = 可加载）。若已安装 nightly 工具链和 `rust-src`，它还会用 `panic=immediate-abort` 重新编译 `std`，dylib 会更小；在 stable 上唯一的区别是体积。若磁盘上有游戏二进制文件（`CP77_DIR`），它会先重新生成 `dados/` 中的地址表；否则使用本仓库中已有的地址表。

**输出：** `target/libcp77_console-publico.dylib`

`cp77-console` crate 不依赖任何 crates.io 包：除标准库外，只使用本仓库中的 `bwms-primitives`，因此无需任何额外设置即可离线构建。

### 2. 数据工具（可选）

数据工具是同一个 Rust crate `bwms`，每个工具是它的一个子命令。用以下命令构建：

```sh
cargo build --release
```

在 `bwms/` 目录内运行：

| 目录               | 作用                                  |
| ------------------ | ------------------------------------- |
| `bwms`             | 统一的命令行前端；`bwms mods checar` 在不动缓存的情况下编译游戏脚本，并指出没有任何注册的 `native func`，`bwms tweak` 读取 / 编辑 `tweakdb.bin`，`bwms archive` 读取 / 提取 `.archive` 容器，`bwms input` 合并按键绑定 / 输入定义 |

`bwms` 使用本地的 `bwms-primitives` crate，它已包含在本仓库中——无需为它做任何外部拉取。

### 3. redscript 脚本（游戏内）

redscript 源码位于 `r6/scripts/blackwall-mods/*.reds`。它们由安装程序在**安装时**用捆绑的 `scc` redscript 编译器编译，之后每次打开游戏时，如果有脚本改动就会再编译一次——对终端用户而言没有单独的手动编译步骤。安装程序会把 `scc` 和 `libscc_lib.dylib` 复制到 `red4ext/`；打开游戏时编译用的就是这份副本。

---

## 安装（终端用户）

适用于只想运行模组（无需进行开发）的玩家：

1. 下载发布版 zip 并解压。
2. 运行 **`INSTALAR.command`**（或在终端中运行 `bwms-install.sh "<game dir>"`）。
3. 从 **Steam（Play）** 启动游戏——而不是从 Finder 启动。

安装程序会向游戏二进制文件添加一个 `LC_LOAD_DYLIB` 条目，并对 `.app` 进行 ad-hoc 重新签名，只带 dylib 所需的 `com.apple.security.cs.*` 放宽项。CDPR 的身份类 entitlements 会被去掉：它们绑定在 CDPR 的证书上，在 ad-hoc 签名下带着它们 macOS 会拒绝启动游戏。它仅使用 macOS 基础工具（`codesign`、`xattr`）：无需密码，不更改 SIP 或 Gatekeeper。

卸载前，请先在游戏里关闭无敌（God Mode）和隐身并存档：游戏本身会把无敌写进存档，而隐身是游戏标记为可保存的效果。

卸载：运行 `extras/DESINSTALAR.command`（在终端中：`bwms-install.sh --restore "<game dir>"`）。它会把原始二进制文件放回去，并对游戏进行 ad-hoc 重新签名；CDPR 的签名只有通过 Steam 的"验证游戏文件的完整性"才能恢复。如果 Steam 已经放回了它自己的二进制文件（游戏更新或"验证完整性"之后），它不会覆盖任何文件，只删除 bwms 的脚本。可选的彻底清理：删除 `red4ext/`、`Mods/BWMS-RELATORIO.txt`，以及 `r6/cache/` 里的 `final.redscripts.bwms-bak`、`final.redscripts.bk` 和 `final.redscripts.ts`。

---

## 仓库结构

```
cp77-console/            The runtime dylib (in-game console, native bridges)
bwms/                    Unified command-line tool (`bwms mods checar`: check the game's scripts; `bwms tweak`: read / edit tweakdb.bin; `bwms archive`: read / extract .archive containers; `bwms input`: merge keybind / input definitions) + `bwms-helper`, the small privilege-free helper used by the installer (`src/bin/`)
bwms-primitives/         Single source of what the runtime and the tools compute the same way (leaf crate): hashes, game addresses, `.yaml`/`.tweak` parser, `Mods/` runtime tables, library migration, pure cores
r6/scripts/blackwall-mods/   redscript sources (compiled at install time) — the
                         same set the released package ships
example-plugin/          Example native plugin (C ABI), for mod authors
                         (the release package ships this same source in `engine/example-plugin/`)
docs/                    Modder API, public changelog
INSTALAR.command         End-user installer (entry point)
bwms-install.sh          Installer script (terminal / scriptable)
```

---

## 许可证

双重许可，可任选其一：

- MIT 许可证（[LICENSE-MIT](LICENSE-MIT)）
- Apache License, Version 2.0（[LICENSE-APACHE](LICENSE-APACHE)）

由你自行选择。

---

## 商标 / 无隶属关系

"Cyberpunk 2077" is a trademark of CD PROJEKT S.A.; this project is not
affiliated with or endorsed by CD PROJEKT.

本项目**不附带任何形式的游戏资产或数据**。你必须拥有一份合法的游戏副本才能使用它。

---

## 备注

- **仅限单人模式。** 不支持反作弊，也无此意图。
- 使用作弊功能前请**备份你的存档**。
- BWMS 是**免费的**。欢迎捐赠，但从不强制。

由 **Blackwall** 编写。

项目主页：`https://github.com/Blackwall-sys/black-wall-mod-system`
