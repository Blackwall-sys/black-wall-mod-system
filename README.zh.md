[English](README.md) · [Português](README.pt.md) · [简体中文](README.zh.md)

# BWMS — Black Wall Mod System

**面向 macOS / Apple Silicon 的原生 Cyberpunk 2077 模组运行时。**

100% Rust + redscript。没有虚拟机，没有 Wine，没有 Windows 串流——模组直接运行在游戏的原生 macOS 版本内部。

> **状态：BETA 0.1.3** —— 早期但真实。本 README 如实说明今天哪些功能可用、哪些还在路线图上。仅限单人模式。

---

## 它能做什么

BWMS 是一个原生运行时，外加一套针对 Apple Silicon 版游戏的数据工具。截至 0.1.3：

- **游戏内控制台 + ImGui 覆盖层** —— 通过基于 Metal 的 ImGui 覆盖层在游戏之上渲染的开发者控制台。它是一个**原生命令控制台，不是 Lua 解释器**：cargo 的 `lua` feature **默认关闭**，因此发布的 build 不内嵌 LuaJIT，`lua_stub.rs::run_code` 是一个空操作（no-op）—— 它**不**加载 CET 的 Lua mod。命令：`heal`、`level`、`money`、`give`、`remove`、`godmode`、`help`。它还会把两条粘贴的 CET 风格代码（`Game.AddMoney(N)`、`Game.AddToInventory(...)`）翻译成原生调用。
- **作弊功能（Cheats）** —— 无敌模式、负重容量、伤害与资源开关、欧元（eddies）、属性、专长、载具，以及类似的单人便利功能（约 15 个），作为原生 redscript 动作暴露在内置的 **设置 > Mods** 面板里（BWMS 自己的 redscript UI —— **不是** PC 上的 NativeSettings 框架）。
- **三档跳过启动** —— 一个选择器（关闭 / 到主菜单 / 直接进入游戏，零输入），带有启动加载画面和真实进度条；从下一次启动开始生效。
- **实时 TweakDB 编辑** —— 在运行中的 TweakDB 中读取和编辑记录（伤害、属性、flats），无需重新打包归档文件。
- **面向模组作者的反射（Reflection）** —— 通过引擎的 RTTI，按名称读写字段、调用方法，作用于游戏的实时对象。
- **归档工具** —— 读取并提取 `.archive` 容器。散装（loose）`.archive` 视觉 mod 已经能在游戏内加载。
- **模组管理器** —— 以事务化方式安装、列出和移除模组。

商店一致性：**Steam 和 GOG 已测试可用**；Epic 使用相同的 Mac build，但尚未测试。框架支持（Codeware、ArchiveXL）**尚未实现** —— 依赖它们的 mod 无法加载。

这是 beta 软件。请预期会有粗糙之处。使用作弊功能前请务必备份你的存档（见底部免责声明）。

---

## 系统要求

- 运行于 **Apple Silicon**（M1 / M2 / M3 / M4）的 macOS。
- 通过 [rustup](https://rustup.rs) 安装的 **Rust**（stable），并带有 `aarch64-apple-darwin` target。
- `codesign`（随 macOS 提供）和 `python3`（`build.sh` 的路径重映射步骤会用到）。注意：较新的 macOS 版本**不**预装 `python3`；若缺失，请安装 Apple 的命令行工具（`xcode-select --install`）或 python.org 版本。
- 一份合法的、已安装的 Cyberpunk 2077（macOS 版 —— Steam 或 GOG 已测试；Epic 尚未测试）。

构建运行时**不**需要完整的 Xcode 应用或 Homebrew。

一次性添加构建 target：

```sh
rustup target add aarch64-apple-darwin
```

---

## 从源码构建

以下是确切的、可复现的命令。运行时及所有工具均从 crates.io 依赖项，加上本仓库中包含的本地 crate 构建——除此之外无需任何其他东西。

### 1. 运行时（产品 dylib）

```sh
cd cp77-console
./build.sh
```

`build.sh` 以 release 模式用 `cargo` 编译，重映射构建路径（出于隐私考虑），strip 二进制文件，将 install-name 设为 `@rpath`，对结果进行 ad-hoc 签名，并通过用 `dlopen` 加载来验证它。

**输出：** `target/release/libcp77_console.dylib`

`cp77-console` crate 仅依赖 crates.io 包（`metal`、`imgui`、`foreign-types` 等），因此它无需任何额外设置即可独立构建。

### 2. 数据工具（可选）

每个工具都是一个标准的 Rust crate。用以下命令构建其中任意一个：

```sh
cargo build --release
```

在该工具自身的目录内运行：

| 目录               | 作用                                  |
| ------------------ | ------------------------------------- |
| `archive-tool`     | 读取 / 提取 `.archive` 容器           |
| `tweakdb-tool`     | 读取 / 编辑 `tweakdb.bin`             |
| `input-loader`     | 合并按键绑定 / 输入定义               |
| `mac-mod-manager`  | 安装 / 列出 / 移除模组                |
| `bwms`             | 统一的命令行前端                      |

`bwms` 和 `mac-mod-manager` 使用本地的 `bwms-core` crate，它已包含在本仓库中——无需为它做任何外部拉取。

### 3. redscript 脚本（游戏内）

redscript 源码位于 `r6/scripts/blackwall-mods/*.reds`。它们由安装程序在**安装时**用捆绑的 `scc` redscript 编译器编译——对终端用户而言没有单独的手动编译步骤。

---

## 安装（终端用户）

适用于只想运行模组（无需进行开发）的玩家：

1. 下载发布版 zip 并解压。
2. 运行 **`INSTALAR.command`**（或在终端中运行 `bwms-install.sh "<game dir>"`）。
3. 从 **Steam（Play）** 启动游戏——而不是从 Finder 启动。

安装程序会向游戏二进制文件添加一个 `LC_LOAD_DYLIB` 条目，并对 `.app` 进行 ad-hoc 重新签名，同时**保留 CDPR 原有的 entitlements**。它仅使用 macOS 基础工具（`codesign`、`xattr`）：无需密码，不更改 SIP 或 Gatekeeper，并且完全可逆。

卸载：

```sh
INSTALAR.command --restore
```

或运行 `extras/DESINSTALAR.command`。

---

## 仓库结构

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
