<p align="center">
  <h1 align="center">SlateOS</h1>
  <p align="center"><b>用说的来用的 Linux 桌面。</b></p>
  <p align="center">
    <a href="#当前状态">状态：pre-alpha，已能端到端跑通</a> ·
    <a href="#试一试">试一试</a> ·
    <a href="docs/architecture.md">架构</a> ·
    <a href="docs/roadmap.md">路线图</a> ·
    <a href="CONTRIBUTING.md">贡献</a> ·
    <a href="README.md">English</a>
  </p>
</p>

---

SlateOS 是一个 Linux 发行版，在这里用电脑的正常方式是把你想要的说出来。shell 是一段对话。背后的 agent 是你已经在付费的 Claude Code 或 Codex 订阅，不是 API key，也不是跑在笔记本上的本地模型。它有自己的鼠标和键盘，可以在你继续用电脑的同时操作你的应用；它做的每一步都有审计，都可以撤销。

它和"Linux 上开个聊天窗口"有三点根本不同：

- **你的订阅就是大脑。** SlateOS 启动官方的 `claude` 或 `codex` 二进制，只通过它们公开的扩展点集成（hooks、MCP、skills、permission 工具）。不调模型 API，不用买 token，不越过订阅条款。
- **agent 有自己的 seat。** 在 Wayland 上它拿到第二套指针和键盘、自己的焦点、自己的剪贴板。你在这边打字，它可以在后台操作 Firefox、文件管理器或设置页。真的需要借用*你的*鼠标时，顶栏会闪，按 Esc 就收回。
- **撤销是一个动词。** 每个任务开始前先做文件系统快照。可逆操作不问直接做，破坏性操作停下来问，`/undo` 把东西放回去。所有动作都在审计日志里。

bash 和 zsh 都还在，一个按键就到。只是你打开它们的次数会变少。

## 用起来是什么样

按 `Mod+s`（或点顶栏的 Slate 按钮），右上角落下一个输入框，一次只显示这一轮问答：

```
◆  把 Downloads 里的 report.pdf 用 Firefox 打开，缩放到适合宽度

   把 Downloads 里的 report.pdf 用 Firefox 打开，缩放到适合宽度
   已在 Firefox 打开 ~/Downloads/report.pdf 并设为适合宽度。
   ▸ desktop_key ctrl+0 → "report.pdf — Mozilla Firefox"      11.2s · 4 turns
```

这期间你可以接着干自己的事：agent 的点击走它自己的 seat。如果它必须借你的键鼠（GTK4 应用只认第一个 seat），顶栏显示闪烁的 **controlling**，按 Esc 拿回来。

同一个会话在任何终端里也能用，那里的 shell 就是 `slash`：

```
~ ❯ 这里什么东西占了这么多磁盘？
  ▸ Bash: du -sh * | sort -h | tail
  target/ 占了 4.1G，其余都在 50M 以下。
  ✓ 6.2s, 2 turns

~ ❯ !git status                # ! 在你真正的 bash/zsh 里跑一行，agent 看得到输出
~ ❯ /undo                      # 回滚上一个任务的改动
~ ❯ /auto on                   # 本会话跳过审批（仍有审计，仍可撤销）
~ ❯ /agent codex               # 切换后端
```

## 它是怎么拼起来的

```
┌──────────────────────────────────────────────────────────────────────┐
│  你                                                                  │
│   ├─ 终端里的 slash            ─┐  同一个会话，两个视图               │
│   └─ Slate 输入框（Mod+s）     ─┘  （背后是 slash --serve）           │
├──────────────────────────────────────────────────────────────────────┤
│  slash — shell                                                       │
│   裸文本 → agent · /cmd → slash 或 agent · !cmd → bash/zsh           │
│   渲染 agent 的事件流；不持有任何模型凭据                             │
├──────────────────────────────────────────────────────────────────────┤
│  Agent 后端（自带）：Claude Code · Codex                              │
│   只通过无头模式、hooks、MCP、skills、permission-prompt 工具驱动      │
├──────────────────────────────────────────────────────────────────────┤
│  slated — 守护进程                                                   │
│   审批分级 · 审计日志 · btrfs 快照与撤销 · 记忆                       │
├──────────────────────────────────────────────────────────────────────┤
│  slate-desktop — 后台 computer use                                   │
│   agent seat · 虚拟指针和键盘 · 按窗口截图                            │
│   窗口管理 · 输入前验证焦点 · 接管指示                                │
├──────────────────────────────────────────────────────────────────────┤
│  sway（wlroots）· NixOS                                              │
└──────────────────────────────────────────────────────────────────────┘
```

完整设计和未决问题见 [docs/architecture.md](docs/architecture.md)，已定的决策在 [docs/decisions](docs/decisions)。

## 原则

1. **自带 agent。** SlateOS 从不调用模型 API。它运行官方的 `claude` 或 `codex` 二进制，只用它们的扩展面。后端可插拔。
2. **agent 有自己的 seat。** 不是你的鼠标、不是你的焦点、不是你的剪贴板。共存是 compositor 层面的保证，不是一个 UX 约定。
3. **优先走无聊的路。** CLI、D-Bus、配置文件优先于 GUI；OS Skills 教 agent 系统每个部分"无聊但正确"的做法。
4. **撤销是一等动词。** 先快照；可逆操作不问直接做，破坏性操作停下来问。
5. **一切可审计。** 每个动作都记录：做了什么、拿的是哪一级审批。
6. **逃生舱永远开着。** `!` 给你真正的 shell，`chsh` 给你回原来的登录 shell。Slate 没有任何部分是底层 Linux 的依赖。

## 组件

| 路径 | 是什么 | 状态（0.0.10） |
|---|---|---|
| `crates/slash` | shell：终端视图，以及给桌面输入框用的 `--serve` 模式。Claude Code 和 Codex 后端。 | 可用，见 [crates/slash](crates/slash) |
| `crates/slated` | 守护进程：审批分级和策略文件、审计日志、btrfs 快照与撤销、记忆。 | 可用；agent 身份隔离未开始 |
| `crates/slate` | 守护进程的 CLI，以及 agent 调用的 hook 和 MCP 入口。安装 OS Skills。 | 可用 |
| `crates/slate-desktop` | 常驻的 agent seat、按窗口截图、输入、窗口管理、MCP server。 | sway 上可用，见 [crates/slate-desktop](crates/slate-desktop) |
| `crates/slate-proto` | 各进程共享的协议类型。 | 可用 |
| `skills/base` | OS Skills：音量、亮度、Wi-Fi、systemd、显示设置、窗口、撤销、SlateOS 系统改动。 | 8 个 |
| `distro/` | NixOS 模块和 flake、桌面配置（sway、顶栏、启动器、通知、主题）、设置应用、Slate 输入框。 | 任何 NixOS 都能装；还没有 ISO |

## 当前状态

**Pre-alpha，但已经能端到端跑通。** 截至 2026-09-28（0.0.10），下面这些都在开发机（NixOS 26.05，sway 1.12）上用 Claude Code 和 Codex 两个后端跑通，并由 `tests/e2e/` 的端到端套件覆盖：

- **slash**：自然语言 shell，`/` 和 `!` 前缀；`!` 跑在 pty 里所以 agent 看得到输出；Claude Code（stream-json、会话续接）和 Codex（exec --json）后端；流式回答；`/auto`、`/undo`、`/remember`、`/audit`、`/model`、`/agent`。
- **slated**：Observe / Reversible / Confirm 三级加 `policy.toml`，审批经 agent 的 permission 工具送到你面前，审计日志，无需特权的 btrfs 快照和 `/undo`，记忆。
- **slate-desktop**：Wayland 上常驻的 agent seat，自己的指针和键盘，按窗口截图，Unicode 输入，窗口排布，输入前验证焦点并报告输入落到了哪个窗口，对不认额外 seat 的工具包提供用户 seat 兜底，以及闪烁的 "controlling" 指示和 Esc 收回。
- **桌面**：常规的 sway 桌面（顶栏、启动器、通知、深色主题），右上角的 Slate 输入框（一次一轮问答、审批是按钮、结果回来时不抢你的键盘），设置应用（显示与 HiDPI、声音、网络、记忆，以及 AI 页：后端、模型、登录、verbose、跳过审批）。
- **NixOS 上的 SlateOS**：`services.slate.enable` 装好全部组件，slash 设为登录 shell，守护进程作为用户服务运行，系统对外身份为 SlateOS。

还没有的：需要 root 的系统改动（agent 没法向你要密码）、幽灵光标和按应用过滤 seat（需要 compositor 补丁）、无障碍树作为输入路径、agent 身份隔离、可安装镜像。见 [docs/roadmap.md](docs/roadmap.md)。

## 试一试

需要一台 NixOS 机器（虚拟机也行）和一个 Claude Code 或 Codex 的登录。加上 flake 模块：

```nix
{
  inputs.slate.url = "github:BenchGrid-dev/slate";
  outputs = { nixpkgs, slate, ... }: {
    nixosConfigurations.mybox = nixpkgs.lib.nixosSystem {
      modules = [
        slate.nixosModules.default
        {
          services.slate.enable = true;
          services.slate.loginShellUsers = [ "alice" ];   # alice 的 shell 变成 slash
          services.slate.desktop.enable = true;           # SlateOS 桌面
          services.slate.desktop.autologinUser = "alice";
        }
      ];
    };
  };
}
```

重建、登录，登录一次 agent（`claude auth login` 或 `codex login`，也可以在 设置 → AI 里点），然后按 `Mod+s` 或开一个终端。详细选项和安装器将来要做的事见 [distro/README.md](distro/README.md)。

## 贡献

机制已经跑通了，现在缺的是更多硬件、更多应用、更多人来用。有用的贡献：

- **跑起来然后报告。** 不同 GPU、HiDPI、工具包（Qt、Chromium/Electron、Flatpak），agent seat 能不能碰到它们。
- **OS Skills。** Linux 桌面上每个常见任务"无聊但正确"的做法。不需要写 Rust。见 [skills/README.md](skills/README.md)。
- **compositor。** 按应用过滤 seat、给 agent seat 画幽灵光标（sway 补丁，见 ADR 0007）。
- **无障碍。** 把 AT-SPI2 做成输入路径，让 agent 操作有名字的控件而不是像素。
- **桌面壳层。** 输入框和设置应用现在是 Python/GTK4，设计稳定后计划用 Rust 重写。
- **agent 扩展面。** Claude Code 和 Codex 的 hooks、MCP、无头模式、permission 工具，以及系统改动的 root/polkit 方案。

见 [CONTRIBUTING.md](CONTRIBUTING.md)。设计改动走 [docs/rfcs](docs/rfcs)。

## 许可证

GPL-3.0-or-later，见 [LICENSE](LICENSE)。
