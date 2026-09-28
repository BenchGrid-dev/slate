<p align="center">
  <h1 align="center">Slate</h1>
  <p align="center"><b>一个 AI 原生的 Linux：人和 agent 共用同一张桌面。</b></p>
  <p align="center">
    <a href="#当前状态">状态：设计阶段</a> ·
    <a href="docs/architecture.md">架构</a> ·
    <a href="docs/roadmap.md">路线图</a> ·
    <a href="CONTRIBUTING.md">贡献</a> ·
    <a href="README.md">English</a>
  </p>
</p>

---

Slate 是一个 Linux 发行版加 agent 运行时，只围绕一个想法：**你不该再需要 shell 才能用电脑，但 agent 应该拥有一个比你用过的都好的 shell。**

你用自然语言跟机器说话。agent 去干活，大脑是你已经在用的 Claude Code 或 Codex 订阅。它有自己的光标、自己的键盘焦点、自己的剪贴板，所以它可以在后台操作桌面上的任何应用，而你在前台继续做你的事。每个动作都有审计，每次改动都有快照，"撤销"永远有效。

zsh 和 bash 还在。只是你会慢慢不再打开它们。

## 为什么现在还没有这种东西

零件都有了：

- **阿里云 Linux Agentic Edition** 把默认 shell 换成了自然语言、加了机器可读的 OS Skills，但它面向云服务器，不是你坐在前面的桌面。
- **Omarchy** 是 agent 优先的桌面发行版，但它的 agent 住在终端窗口里，跟你抢鼠标。
- **macOS 上的 Codex** 给了 agent 自己的光标、能在后台操作应用。它靠的是 SkyLight 私有 API，而且只有 Mac 能用。
- **豆包手机**和其他系统级手机 agent 证明了用户愿意把跨应用的长任务交给 agent，但那是一个你永远盯着它干活的平台。

没有人把这些拼成一张人和多个 agent 真正共存的桌面。而在 Wayland 上，把这件事做对所需的原语（独立 seat、按窗口截图、虚拟输入、临时 seat）已经全是标准协议。Slate 就是把它们组装起来的项目。

## 用起来是什么样

```
❯ 上周那封机票确认邮件，把日期加到日历，pdf 转发给 Alice

  ▸ 在 Thunderbird 找到 "Booking confirmation – SFO→NRT"（9 月 18 日）
  ▸ 在 GNOME Calendar 创建 2 个事件（10 月 3 日出发，10 月 17 日返回）
  ▸ Thunderbird：新建邮件 → alice@… → 附加 confirmation.pdf
  ⏸ 发送邮件给 alice@example.com？  [y] 发送  [n] 取消  [v] 看草稿
```

这期间 Thunderbird 和日历是被 agent 的 seat 驱动的。你的鼠标一动不动。你可以继续在编辑器里打字，也可以切过去看幽灵光标在干活，或者说一句**停**、**我来**。

```
❯ !git status                 # ! 在你真正的 zsh 里跑一行
❯ /agent codex                # / 是给 slash 自己或 agent 后端的控制命令
❯ /undo                       # 回滚上一个任务对文件系统的改动
```

## 一屏看懂架构

```
┌──────────────────────────────────────────────────────────────────────┐
│  你                                                                  │
│   ├─ slash（终端）      ─┐   同一个会话，两个视图                     │
│   └─ slash（桌面面板）  ─┘                                           │
├──────────────────────────────────────────────────────────────────────┤
│  slash — shell / 前端                                                │
│   • 路由：裸文本 → agent，/cmd → 控制，!cmd → zsh                    │
│   • 渲染后端事件流，先给答案，stdout 折叠                            │
│   • 不拥有 agent 循环，不碰模型 token                                │
├──────────────────────────────────────────────────────────────────────┤
│  Agent 后端（自带）                                                  │
│   claude（Claude Code）  │  codex  │  …                              │
│   只通过官方扩展面驱动：                                             │
│   无头模式 · hooks · MCP · skills · permission-prompt 工具           │
├──────────────────────────────────────────────────────────────────────┤
│  slated — 守护进程（OS 层的部分）                                    │
│   身份 ─ 审批代理 ─ 审计日志 ─ 快照/撤销 ─ 记忆                      │
│   skills 注册表 ─ 会话上下文 ─ 给后端用的 MCP server                 │
├──────────────────────────────────────────────────────────────────────┤
│  slate-desktop — Linux 上的后台 computer use                         │
│   agent seat（ext-transient-seat）· 虚拟指针/键盘                    │
│   按窗口截图 · AT-SPI2 树 · 幽灵光标 · headless 输出                 │
├──────────────────────────────────────────────────────────────────────┤
│  Wayland compositor（wlroots 系，按需打补丁）· Linux                 │
└──────────────────────────────────────────────────────────────────────┘
```

完整细节和未决问题见 [docs/architecture.md](docs/architecture.md)。

## 原则

1. **自带 agent。** Slate 从不调用模型 API。它启动官方的 `claude` 或 `codex` 二进制，通过 hooks、MCP 和 skills 集成。你的订阅还是你的，并且在条款允许范围内。后端可插拔。
2. **agent 有自己的 seat。** 不是你的鼠标、不是你的焦点、不是你的剪贴板。共存是 compositor 层面的保证，不是一个 UX 约定。
3. **优先走无聊的路。** CLI、D-Bus、配置文件优先于无障碍树，无障碍树优先于截图。OS Skills 教 agent 系统每个部分"无聊但正确"的做法。
4. **撤销是一等动词。** 每个任务开始前先快照。可逆操作不问直接做，破坏性操作停下来问。
5. **一切可审计。** agent 的每个动作都记录：它看到了什么、做了什么、拿的是哪一级审批。
6. **逃生舱永远开着。** `!` 给你真正的 zsh，`chsh` 给你回到过去的生活。Slate 没有任何部分是底层 Linux 的依赖。

## 组件

| Crate | 是什么 | 状态 |
|---|---|---|
| `crates/slash` | shell。终端视图和桌面面板视图共享同一个会话。 | v0：Claude Code 和 Codex 都能用，见 [crates/slash](crates/slash) |
| `crates/slated` | 守护进程。审批、分级策略、审计、快照与撤销。身份、记忆、skills 待做。 | v0：审批、审计、撤销可用 |
| `crates/slate` | `slated` 的 CLI，也是 Claude Code 调用的 hook 和 MCP 入口。 | v0 |
| `crates/slate-desktop` | 后台 computer use：agent seat、按窗口截图、输入，通过 MCP 暴露。 | v0：sway 上可用，见 [crates/slate-desktop](crates/slate-desktop) |
| `crates/slate-proto` | 跨进程边界的共享类型。 | 占位 |
| `skills/` | OS Skills：给机器读的系统手册。 | 仅示例 |
| `distro/` | Slate OS 镜像构建。基础发行版尚未决定。 | 空 |

## 当前状态

**Pre-alpha，但是真的能跑。** 截至 2026-09-27，下面这些都在开发机（NixOS 26.05，sway 1.12）上跑通，并且用 Claude Code 和 Codex 两个后端做过端到端验证：

- **slash**：自然语言 shell，`/` 和 `!` 前缀，`!` 命令跑在 pty 里所以 agent 能看到输出，Claude Code（stream-json、会话续接）和 Codex（exec --json）后端，流式输出。
- **slated**：三级策略（Observe / Reversible / Confirm）加 `policy.toml`，审批通过 Claude Code 的 permission tool 送到终端前的人，审计日志，无需特权的 btrfs 快照和 `/undo`，记忆（`remember` / `recall` / `forget`）。
- **slate-desktop**：Wayland 上的 agent seat，自己的指针和键盘，按窗口截图，Unicode 输入，两个后端都能用的 MCP 工具。已验证：agent 通过自己的 seat 操作终端窗口并从截图读回结果。已知缺口：GTK4 应用只监听第一个 seat（ADR 0007）。
- **桌面（Phase 1 + 2）**：`services.slate.desktop.enable` 提供一套常规的 sway 桌面：顶栏、启动器、通知、设置应用、右上角浮动的 **Slate 面板**（Mod+s：对话、流式回答、审批按钮、agent 状态）、常驻的 agent seat daemon、借用键鼠时闪烁的 "controlling" 指示和 Esc 收回、输入前验证焦点。
- **NixOS 模块**：`services.slate.enable` 装好全部组件，把 slash 注册为登录 shell，slated 作为用户服务运行。

还没做的：桌面壳层、幽灵光标和人工接管（需要 compositor 补丁）、无障碍树输入、agent 身份隔离、安装器。见 [docs/roadmap.md](docs/roadmap.md)。

## 贡献

Slate 还在设计阶段，所以现在最有用的贡献是论证、原型和 skills，而不是打磨。最需要帮助的方向：

- **Wayland / wlroots 内部** —— 多 seat、transient seat、toplevel 截图、compositor 打补丁
- **Linux 无障碍** —— AT-SPI2，让 Chromium / Electron / Flatpak 应用暴露控件树
- **Claude Code 和 Codex 的扩展面** —— hooks、MCP、无头模式、permission 工具
- **Btrfs / NixOS** —— 守护进程的快照和回滚策略
- **发行版构建** —— Arch 还是 NixOS 做底座、镜像流水线、安装器
- **写 OS Skills** —— Linux 桌面上每个常见任务的"无聊但正确"的做法

见 [CONTRIBUTING.md](CONTRIBUTING.md)。标了 `good first issue` 和 `rfc` 的 issue 是入口，设计改动走 [docs/rfcs](docs/rfcs)。

## 许可证

GPL-3.0-or-later，见 [LICENSE](LICENSE)。
