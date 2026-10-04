<p align="center">
  <a href="https://slate.benchgrid.dev">
    <img src="https://slate.benchgrid.dev/opengraph-image" width="1200" alt="BenchGrid 出品的 Slash 与 SlateOS — A little less human. A lot more possible.">
  </a>
</p>

<h1 align="center">SlateOS</h1>

<p align="center">
  <strong>为人与 AI Agent 共同工作而构建的 Linux 桌面。</strong><br>
  说出任务，延续工作，掌握控制权。
</p>

<p align="center">
  <a href="https://github.com/BenchGrid-dev/slate/actions/workflows/ci.yml"><img src="https://github.com/BenchGrid-dev/slate/actions/workflows/ci.yml/badge.svg" alt="Rust CI"></a>
  <a href="#项目状态"><img src="https://img.shields.io/badge/status-pre--alpha-orange" alt="状态：pre-alpha"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--or--later-blue" alt="许可证：GPL-3.0-or-later"></a>
</p>

<p align="center">
  <a href="#快速开始">快速开始</a> ·
  <a href="docs/architecture.md">架构</a> ·
  <a href="docs/roadmap.md">路线图</a> ·
  <a href="CONTRIBUTING.md">参与贡献</a> ·
  <a href="README.md">English</a>
</p>

---

**SlateOS 是基于 NixOS 构建的开源、Agent 原生 Linux 桌面。** 它将自然语言交互、后台应用操作和任务级控制整合进同一套系统，让 Agent 能够在你使用的环境中，跨越终端、桌面应用和系统配置完成工作。

核心理念很简单：**Agent 应当在桌面上拥有自己的工作位置。** Slate 为它提供独立的 Wayland 输入席位，连接命令行与应用程序，并提供审批、审计和文件恢复机制。你选择 Agent 后端，Slate 提供它工作的系统环境。

本仓库同时包含 Slate 运行时与 SlateOS 桌面配置。你可以先体验对话式 shell，也可以部署完整桌面。

> **项目状态：** Pre-alpha · 当前版本 **0.0.18**。核心流程已在开发环境中端到端跑通，硬件与应用兼容性仍在扩展；live/安装镜像已可从 flake 构建，正在测试。当前能力边界见[项目状态](#项目状态)。

## 为什么是 SlateOS

### 共享桌面，独立输入

Agent 通过独立的 Wayland seat 获得自己的指针、键盘和焦点，并且有自己的后台屏幕：为任务打开的窗口放在一块你看不见的第二块输出上，直到你要求查看；与你当前操作相关的任务则在你的屏幕上进行。在兼容应用中，它可以继续操作，而你在另一个窗口里正常输入。按窗口截图和输入前的焦点验证，让它能够检查操作结果。

对于必须使用用户席位的应用，Slate 提供显式的回退路径：顶栏显示 **controlling**，按 **Esc** 即可收回控制权。应用支持情况与不同后端的审批差异见[下文](#项目状态)。

### 你选择的 Agent，系统级的集成

Slate 启动官方 **Claude Code** 或 **Codex** CLI，通过其无头运行接口和可用扩展点集成。认证与模型访问由所选后端负责，Slate 本身不直接调用模型 API。你可以在 shell 或「设置 → AI」中切换后端。

### 同时理解意图与命令的 shell

`slash` 在同一个界面中接收自然语言任务、内置指令和普通 shell 命令。bash 或 zsh 仍然可用，手动命令的输出也会成为下一轮 Agent 的上下文。桌面输入框通过 `slash --serve` 使用同一套运行时。

### 与任务执行配套的控制机制

`slated` 提供任务记录、可配置的审批分级、持久记忆，以及基于 btrfs 的文件恢复。OS Skills 描述常见系统任务的可靠执行方式，优先使用 CLI、D-Bus 和配置文件，再考虑 GUI 操作。当前的控制能力因后端而异，详见[执行与恢复](#执行与恢复)。

## 使用方式

按 **Super+s** 打开桌面输入框，或在终端中启动 `slash`。你可以这样描述任务：

```text
当前目录里什么最占磁盘空间？
用 Firefox 打开 Downloads 里的报告。
把 Firefox 和终端并排放置。
把音量设为 30%。
```

在终端中，通过输入形式区分操作：

| 输入 | 行为 |
| --- | --- |
| 普通文本 | 将任务发送给当前 Agent。 |
| `!git status` | 在底层 shell 中执行命令，并保留输出作为上下文。 |
| `/help` | 查看 shell 指令。 |
| `/agent codex` | 切换至 Codex 后端；`/agent claude` 切回 Claude Code。 |
| `/model` | 查看或切换后端模型。 |
| `/audit` | 查看 `slated` 的近期审计记录。 |
| `/undo --preview` | 预览最近一个可撤销任务的文件恢复计划。 |
| `/undo` | 执行该恢复计划。 |

桌面输入框实时显示任务进展，将受支持的审批请求呈现为按钮，并在不抢占键盘焦点的情况下展示结果。「设置」（**Super+,**）提供显示、声音、网络、记忆和 Agent 偏好配置。

## 快速开始

### 从源码体验 shell

体验对话界面需要 Linux 或 macOS、稳定版 Rust 工具链、C 工具链与链接器，以及已安装、已登录且位于 `PATH` 中的 Claude Code 或 Codex CLI。

```sh
git clone https://github.com/BenchGrid-dev/slate.git
cd slate
cargo build --workspace
./target/debug/slash
```

默认后端为 Claude Code。如果使用 Codex，在第一个任务前输入 `/agent codex`。输入 `/help` 查看功能，输入 `/exit` 退出。

这种方式不会修改登录 shell 或桌面配置。配套的 `slated` 可用时，`slash` 会启动它。桌面控制需要兼容的 Linux Wayland 会话；文件恢复需要[下文](#执行与恢复)所述的 btrfs 配置。

后端选项与 shell 行为见 [slash 参考文档](crates/slash/README.md)。在安装了 Nix 的 Linux 系统上，`nix develop` 可进入仓库提供的开发环境，`nix build` 可构建运行时软件包。

### 部署 SlateOS 桌面

完整桌面通过 **NixOS flake 模块**分发，提供 `x86_64-linux` 与 `aarch64-linux` 软件包输出。请使用已有的 NixOS 机器或虚拟机。

在系统现有的 `flake.nix` 中添加 Slate 输入：

```nix
inputs.slate.url = "github:BenchGrid-dev/slate";
```

将 `slate` 加入 `outputs` 参数，并在现有 `nixosSystem` 配置的 `modules` 列表中添加以下内容，保留原有硬件与系统模块：

```nix
slate.nixosModules.default
{
  services.slate = {
    enable = true;
    desktop.enable = true;
    loginShellUsers = [ "alice" ]; # 替换为已有用户名。
  };
}
```

按原有 NixOS 流程重新构建系统，然后登录 sway 会话。这会启用 SlateOS 桌面配置，安装运行时与默认的 Claude Code 后端，启动用户服务，并将指定用户的登录 shell 设为 `slash`。在「设置 → AI」中登录后端，即可按 **Super+s** 开始使用。

该模块会在每次登录和每次升级后，为 Claude Code 链接随附的 OS Skills（`slate-skills` 用户服务），并根据当前机器的工具和发行版选择适用的 Skills。不使用该模块时，可在 `slash` 中手动安装：

```text
!slate skills install
```

面向 Codex 的自动 Skill 安装尚未实现。

完整模块选项、Agent 安装、快照根目录和系统配置路径见 [NixOS 部署指南](distro/README.md)。

## 系统架构

Slate 将用户界面、Agent 后端、执行策略与桌面控制拆分为相互协作的进程：

```text
Terminal                         Desktop prompt
   │                                   │
   └────────── slash / slash --serve ───┘
                         │
                Claude Code or Codex
                         │
              Backend integration points
                  ╱               ╲
      slate hooks / MCP       slate-desktop MCP
               │                      │
            slated             Wayland agent seat
      Policy · audit · undo    Capture · input · windows
               │                      │
        User-owned btrfs          sway / wlroots

          NixOS module · desktop profile · OS Skills
```

`slash` 也直接与 `slated` 通信，用于管理任务生命周期、记忆与审批界面。终端和桌面复用同一套运行时及守护进程服务，但各自建立独立的对话会话。基于 hook 的策略集成目前已在 Claude Code 后端实现。

| 组件 | 职责 |
| --- | --- |
| [`slash`](crates/slash) | 对话式 shell、后端适配、事件展示，以及供桌面客户端使用的 JSON-lines 接口。 |
| [`slated`](crates/slated) | 用户级守护进程，负责策略、审批、任务记录、审计日志、快照与记忆。 |
| [`slate`](crates/slate) | 管理 CLI、Agent hooks、MCP 工具与 OS Skills 安装。 |
| [`slate-desktop`](crates/slate-desktop) | Wayland 席位管理、窗口截图、输入、窗口管理与桌面 MCP 工具。 |
| [`slate-proto`](crates/slate-proto) | 本地进程通信共享的协议类型。 |
| [`skills/base`](skills/base) | 十三类任务指南：音量、亮度、显示、窗口与应用、网络、服务、系统配置、撤销、办公文档、邮件与日历、PDF、媒体播放、文本编辑。 |
| [`distro`](distro) | NixOS 模块、sway 配置、顶栏、启动器、通知、应用套件，以及 Python/GTK4 编写的输入框和设置应用。 |

运行时使用 **Rust** 编写。本地服务通过 Unix socket 通信，面向 Agent 的工具使用 **MCP**。设计依据与未决问题见[架构文档](docs/architecture.md)和[架构决策记录](docs/decisions/README.md)。

## 执行与恢复

当 Claude Code 与 `slated` 连接时，工具调用按策略分为三级：

| 级别 | 默认行为 |
| --- | --- |
| **Observe · 观察** | 以读取为主的操作直接执行，无需审批。 |
| **Reversible · 可逆** | 尝试创建任务快照，然后允许执行。 |
| **Confirm · 确认** | 执行前请求审批，已获批准或显式跳过审批的情况除外。 |

规则可在 `~/.config/slate/policy.toml` 中自定义；未知工具默认归为 Confirm。Claude Code hooks 记录工具检查和执行结果。Codex 当前使用其配置的沙箱，`slated` 记录任务级信息；本仓库尚未为该后端提供等价的逐工具审批、审计或自动快照能力。

**Undo 恢复的是已记录任务范围内的本地文件。** 它依赖用户拥有的 btrfs 子卷及成功创建的快照，默认根目录为用户主目录，也可通过 `SLATE_SNAPSHOT_ROOT` 指定。它不能撤回远程操作、消息或任意应用状态。恢复时会比较任务范围内的目录与快照，因此这些目录中的后续编辑也可能出现在恢复计划中；请先使用 `/undo --preview`。

快照不可用时，任务仍可执行。目前的策略层不构成隔离边界；独立 Agent 身份和更强的沙箱隔离仍在路线图中。`/auto on` 会显式跳过当前会话中的 Slate 审批提示。

实现细节见[策略参考](docs/policy.md)和[快照设计](docs/decisions/0006-privilege-free-snapshots.md)。

## 项目状态

当前开发基线为 **NixOS 26.05 + sway 1.12**。仓库包含单元测试与真实桌面环境下的端到端测试；自动化 CI 执行 Rust 格式检查、Clippy、构建和单元测试。桌面与 Agent 集成测试需要配置好的机器，单独运行。

| 领域 | 当前已实现 | 下一步 |
| --- | --- | --- |
| Agent 交互 | Claude Code 与 Codex 适配；终端和桌面输入框；运行中的 shell 内会话续接。 | 跨输入框会话的对话历史。 |
| 桌面控制 | 独立 Agent 席位及其后台屏幕、无障碍树元素与动作（AT-SPI2）、按窗口截图、Unicode 输入、焦点验证、窗口排列及用户席位回退。 | 独立可见光标、合成器席位过滤和更多应用覆盖（Qt、Chromium、Flatpak）。 |
| 任务控制 | Claude Code 审批与工具审计、任务记录、记忆、btrfs 快照、限定范围的撤销，以及经用户授权的 root（sudo 通过桌面对话框向用户索要密码）。 | Codex 审批集成、Agent 身份隔离、面向图形应用提权请求的 polkit agent。 |
| 发行版 | NixOS 模块、桌面配置、应用套件、设置应用、SlateOS 系统命令，以及 live/安装镜像（`nix build .#iso`）和 `slateos-install`。 | 真机镜像测试、图形安装器、首次启动的 agent 登录。 |

**应用兼容性是当前工作的重点。** 在 Agent 席位先于应用创建的条件下，foot、Firefox 和 GTK3/Thunar 已通过验证。已测试的 GTK4/libadwaita 应用需要回退至用户席位。Qt、Chromium/Electron 和 Flatpak 的覆盖仍在探索中。当前桌面集成面向 sway 及其所需的 Wayland 协议，不能直接替换为 GNOME、KDE 或 X11。

接下来的里程碑围绕兼容性、执行边界、无障碍能力和安装体验展开。详见[完整路线图](docs/roadmap.md)、[工具包兼容性记录](crates/slate-desktop/README.md#toolkit-compatibility)和[更新日志](CHANGELOG.md)。

## 参与贡献

SlateOS 涉及系统编程、桌面设计、Agent 集成和实际的 Linux 使用经验。以下方向尤其需要贡献：

- **兼容性：** 测试真实硬件、显示缩放和应用工具包，提供环境信息与可复现的问题描述。
- **桌面基础设施：** 改进 Wayland 输入、合成器集成，扩大 AT-SPI2 覆盖（Qt、Chromium/Electron、Flatpak）。
- **Agent 集成：** 扩展后端支持、审批流程和任务上下文。
- **OS Skills：** 记录日常任务的可靠执行方式，包括验证与恢复步骤，无需 Rust 经验。
- **产品与发行：** 改进输入框、设置、NixOS 打包、首次使用体验与文档。

从[贡献指南](CONTRIBUTING.md)和 [Issues](https://github.com/BenchGrid-dev/slate/issues) 开始。重大设计变更通过 [RFC 流程](docs/rfcs/README.md)讨论。

运行与 CI 一致的 Rust 检查：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
```

涉及 shell、守护进程或桌面的改动，还需要在配置好的桌面环境中运行[端到端测试](tests/e2e/README.md)。社区参与遵循[行为准则](CODE_OF_CONDUCT.md)。

## 文档导航

| 文档 | 内容 |
| --- | --- |
| [NixOS 部署](distro/README.md) | 安装、模块选项、桌面配置与系统工具。 |
| [Shell 参考](crates/slash/README.md) | 后端配置、shell 命令与会话行为。 |
| [桌面参考](crates/slate-desktop/README.md) | MCP 工具、Wayland 要求与工具包兼容性。 |
| [策略参考](docs/policy.md) | 审批分级与规则覆盖。 |
| [OS Skills](skills/README.md) | Skill 格式、安装与贡献规范。 |
| [架构](docs/architecture.md) · [决策记录](docs/decisions/README.md) | 系统设计及其依据。 |
| [路线图](docs/roadmap.md) · [更新日志](CHANGELOG.md) | 后续工作与版本历史。 |

## 许可证

Slate 使用 **GPL-3.0-or-later** 许可证，详见 [LICENSE](LICENSE)。
