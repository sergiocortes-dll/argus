# Argus — plan

Agentic, team-oriented terminal written in Rust. Cross-platform (Windows, Linux, macOS), GPU-rendered with `winit` + `wgpu`, multi-provider LLM agent, designed with DDD + hexagonal architecture.

## 1. Goals

- Replace Warp as the daily terminal.
- Idle memory < 150 MB with 3 tabs open; startup < 300 ms.
- Work is organized by **teams** (e.g. `tekus`, `personal`): each team has its own git provider, repositories, worktrees, tasks, rules, secrets and specials.
- The agent works inside a team, reads its rules, and never runs or writes anything without permission unless the team's allowlist says so.

## 2. Ubiquitous language

| Term | Meaning |
|---|---|
| **Team** | A workspace: root folder, git connection, repositories, rules, vault, specials, agent settings. |
| **GitConnection** | Provider type (GitHub, Bitbucket, GitLab, Azure DevOps) + credential reference for a team. |
| **Repository** | A remote repo added to a team. Has a read-only base clone in `repositories/`. |
| **Worktree** | A working copy of a repository on a branch, tied to a task. The only place code is written. |
| **Task** | Unit of work (e.g. `TSK-4675`) grouping worktrees, notes and status. |
| **Rules** | Team instructions the agent always loads (`rules/`, `ARGUS.md`). |
| **Vault** | Team secrets. Values live in the OS keyring; the team only stores references. Agent has no access by default. |
| **Specials** | Files that fit no other category but are needed: test fixtures, binaries, scratch data. |
| **Session** | A live shell (tab/pane) bound to a team and a working directory. |
| **Block** | One command with its output, exit code, duration and cwd. |
| **Conversation** | A thread with the agent inside a team, including tool calls and permission decisions. |

## 3. On-disk layout

```
~/.argus/
├─ config.toml              # global: font, theme, keybindings, default LLM profile
├─ teams.toml               # registry: team name -> root path (default ~/.argus/teams/<name>)
└─ teams/
   └─ <team>/
      ├─ team.toml          # git connection, worktree template, agent policy
      ├─ ARGUS.md           # team entry point for the agent
      ├─ rules/
      ├─ repositories/<repo>/          # base clones, read-only
      ├─ worktrees/...                 # per team template
      ├─ tasks/...
      ├─ secrets/                      # references only
      ├─ specials/
      └─ conversations/                # agent history (JSONL)
```

A team root can live elsewhere (e.g. another drive) by changing its path in `teams.toml`.

Example `team.toml`:

```toml
name = "tekus"

[git]
provider = "bitbucket"
workspace = "<workspace>"
credential = "keyring:argus/tekus/bitbucket"
base_branch_default = "develop"

[layout]
worktree = "worktrees/sprint-{sprint}/{repo}/{type}-{task}-{slug}"

[agent]
profile = "anthropic-opus"
rules = ["ARGUS.md", "rules/**/*.md"]
read_only = ["repositories/**"]
deny = ["secrets/**"]
allow_commands = ["git status", "git diff*", "cargo check*"]
```

## 4. Bounded contexts

| Context | Kind | Key aggregates / entities | Responsibility |
|---|---|---|---|
| **Teams** | Core | `Team` (root), `Repository`, `Worktree`, `Task` | Create/import teams, add repositories, create/remove worktrees, enforce invariants (no writes in `repositories/`, one worktree per (task, repo), worktree path follows template). |
| **Agent** | Core | `Conversation` (root), `ToolCall`, `PermissionPolicy` | Agent loop, tools, permission decisions derived from the team policy. |
| **Terminal** | Supporting | `Session` (root), `Block` | Shell lifecycle, blocks, history. |
| **SourceControl** | Supporting (ACL) | `GitConnection`, `RemoteRepository` | Translate provider APIs into one model; ports to list/clone repos and manage worktrees. |
| **Llm** | Generic (ACL) | `ModelProfile` | `LlmProvider` port; Anthropic and OpenAI-compatible adapters. |
| **Secrets** | Generic | `SecretRef` | `SecretStore` port; OS keyring adapter. |

Contexts talk through **domain events**, never by calling each other's internals:
`TeamCreated`, `RepositoryAdded`, `WorktreeCreated`, `CommandFinished`, `ToolCallRequested`, `ToolCallApproved`, `ToolCallDenied`.

Rendering and VT emulation are infrastructure, not domain: plain, fast code with no aggregates.

## 5. Workspace structure

```
argus/
├─ crates/
│  ├─ shared-kernel/          # ids (TeamId, RepoId, TaskId), event base, common errors
│  ├─ teams-domain/           # no I/O, no async
│  ├─ teams-application/      # use cases + ports
│  ├─ teams-infra/            # TOML persistence, git CLI adapter
│  ├─ agent-domain/
│  ├─ agent-application/
│  ├─ agent-infra/            # real tools (fs, pty), JSONL persistence
│  ├─ terminal-domain/
│  ├─ terminal-infra/         # portable-pty, alacritty_terminal, OSC 133
│  ├─ source-control/         # GitProvider port + github/bitbucket adapters
│  ├─ llm/                    # LlmProvider port + anthropic/openai_compat adapters
│  ├─ secrets/                # SecretStore port + keyring adapter
│  ├─ render/                 # wgpu + glyphon: rects, text, cursor, glyph atlas
│  ├─ ui/                     # layout: tabs, splits, team switcher, agent panel
│  └─ app/                    # composition root, winit event loop
├─ shell-integration/         # argus.ps1, argus.bash, argus.zsh, argus.fish
└─ .github/workflows/         # CI matrix: windows, ubuntu, macos
```

Hard rule: `*-domain` crates depend only on `shared-kernel` and pure crates (`thiserror`, `serde` derive at most). No `tokio`, `wgpu`, `reqwest` or filesystem.

Threading: `winit` event loop on the main thread; PTYs and the agent on a background `tokio` runtime; everything reaches the UI as an `AppEvent` through `EventLoopProxy`.

## 6. Key technical choices

| Area | Choice |
|---|---|
| Window / GPU | `winit` + `wgpu` |
| Text | `glyphon` (`cosmic-text`): shaping, font fallback, glyph atlas |
| PTY | `portable-pty` (ConPTY on Windows) |
| VT emulation | `alacritty_terminal` |
| Command blocks | OSC 133 shell integration (+ OSC 7 for cwd) |
| Git operations | `git` CLI through an adapter (respects user credential helpers and config) |
| Git provider APIs | `reqwest`; Bitbucket Cloud and GitHub first, GitLab and Azure DevOps later |
| Secrets | `keyring` (Credential Manager / Keychain / Secret Service) |
| Config | `serde` + `toml`, hot reload with `notify`, paths with `directories` |
| LLM | Own `LlmProvider` trait; adapters: Anthropic Messages API (default `claude-opus-5-5`) and OpenAI-compatible Chat Completions (Ollama, LM Studio, OpenRouter, OpenAI) |
| Clipboard | `arboard` |

## 7. Phases and backlog

### Phase 0 — Spike (validate the stack)
- F0-1 Cargo workspace, CI matrix (fmt, clippy, test) on 3 OSes.
- F0-2 `winit` window + `wgpu` surface, clear color, resize.
- F0-3 `glyphon` monospace text grid with configurable font.
- F0-4 `portable-pty`: spawn the OS default shell, reader thread -> `EventLoopProxy`.
- F0-5 PTY -> `alacritty_terminal::Term` -> cell rendering (fg/bg, bold, cursor).
- F0-6 Keyboard -> VT sequences; PTY resize follows the window.
- F0-7 Baseline RAM and startup measurements on 3 OSes.

Exit: `vim`, `htop` and `claude` run correctly on 3 OSes.

### Phase 1 — Usable terminal
- F1-1 Damage tracking and frame pacing (no idle CPU).
- F1-2 Scrollback, mouse selection (char/word/line).
- F1-3 Clipboard, bracketed paste.
- F1-4 Tabs and splits (layout tree).
- F1-5 `~/.argus/config.toml`: font, theme, shell, keybindings, hot reload.
- F1-6 Scrollback search.
- F1-7 IME, DPI scaling, monitor changes.
- F1-8 Hyperlinks (OSC 8 + URL detection).

### Phase 2 — Teams (local)
- F2-1 `teams-domain` with TDD: `Team`, `Repository`, `Worktree`, `Task`, invariants, events.
- F2-2 `teams.toml` registry and `team.toml` persistence.
- F2-3 Use cases: `CreateTeam` (scaffolds the layout), `ListTeams`, `SwitchTeam`.
- F2-4 Team switcher in the UI; new sessions open at the team root; active team shown in the status bar.

### Phase 3 — Repositories and worktrees
- F3-1 Git CLI adapter (clone, fetch, worktree add/remove/list).
- F3-2 `AddRepository` by URL -> base clone in `repositories/`, marked read-only.
- F3-3 `CreateWorktree` / `RemoveWorktree` following the team template.
- F3-4 Tasks and worktrees view; open a worktree in a new tab.

### Phase 4 — Git providers
- F4-1 `GitProvider` port: `list_repositories(filter)`, `default_branch`, `clone_url`.
- F4-2 Bitbucket Cloud adapter (API token).
- F4-3 GitHub adapter (token; OAuth device flow later).
- F4-4 Credentials in keyring per team.
- F4-5 UI: connect provider -> searchable remote repo list -> select -> clone.

### Phase 5 — Blocks
- F5-1 OSC 133 scripts for pwsh, bash, zsh, fish; auto-injected on spawn.
- F5-2 `Block` model: command, output range, exit code, duration, cwd.
- F5-3 Block UI: separators, error marker, jump between blocks, copy command/output.
- F5-4 Graceful fallback without shell integration.

### Phase 6 — Agent v1
- F6-1 `llm`: internal types (`Message`, `ContentBlock`, `ToolCall`, `ToolResult`, `StreamEvent`) + `LlmProvider::stream`.
- F6-2 Anthropic adapter: SSE, tool use, effort, prompt caching, refusal / max_tokens handling.
- F6-3 OpenAI-compatible adapter: SSE, `tool_calls`; tested against Ollama.
- F6-4 Agent loop: parallel tool calls, step limit, cancel with Esc.
- F6-5 Tools: `run_command`, `read_file`, `edit_file`, `write_file`, `list_dir`, `search`.
- F6-6 `PermissionPolicy` from `team.toml`: read/write/execute classes, `read_only`, `deny`, `allow_commands`; inline approval with diff preview.
- F6-7 Context: team rules, OS, shell, cwd, `git status`, last N blocks (truncated). Tool output and file contents are data, never instructions.
- F6-8 UI: agent input (`Ctrl+Space`), streaming answer as a block, "explain / fix this block" on failed blocks.
- F6-9 Model profiles in config; API keys in keyring.

### Phase 7 — Polish and packaging
- F7-1 Conversation history per team.
- F7-2 RAM/CPU budget checked in CI.
- F7-3 Packaging: MSI/winget, `.dmg`, AppImage/deb.

### Phase 8 — Extras
MCP client (`rmcp`), saved workflows, AI autocomplete, shareable themes, GitLab / Azure DevOps adapters.

## 8. Risks

- Text rendering (shaping, ligatures, emoji) is the biggest time sink: rely on `glyphon`/`cosmic-text`.
- ConPTY quirks on resize and some sequences: `portable-pty` covers most of them.
- Agent safety: never auto-execute outside the allowlist; `secrets/` is denied by default; untrusted content stays data.
- DDD ceremony: apply it to Teams and Agent (real business rules); keep render/VT as plain infrastructure.

## 9. References to study (not copy)

Alacritty (VT engine), WezTerm (pty, multiplexing), Rio (Rust + wgpu terminal), Zed (`gpui`), `iced_term` / `egui_term` (widgets over `alacritty_terminal`).
