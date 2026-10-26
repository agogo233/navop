# 内置 AI 的资源实时状态（ResourceContextRefresher）

> 文档状态：Implemented Design
>
> 目标读者：维护 AI 资源池（`ai_chat_view`）与各侧边栏 AI 面板（`terminal_view` / `db_view` /
> `redis_view` / `mongodb_view`）的开发 Agent
>
> 编写日期：2026-10-26

---

## 1. 问题

侧边栏内置 AI 的资源池（`ResourceContext`）**只在面板创建时构建一次**：终端侧边栏早在
`TerminalSidebar::new` 就把 `terminal_ai_resource` 交给 `DefaultAgentChatPanel`，之后不再回写。
于是模型看到的永远是「终端刚打开时」的样子：

- 用户 `cd` 到别的目录、在 `vim` 里编辑文件、选中一段报错，AI 全都不知情；
- 只有主动调用 `terminal.read` 才能看到内容，而模型往往不会主动调用，或要先
  `connections.list_sessions` 绕一圈（SSH 侧边栏的默认目标还是连接而非终端 session）。

模型「感知现场」的成本，本可以为零。

## 2. 设计

分两层，**状态主动推送、内容按需拉取**：

| 层 | 机制 | 位置 |
| --- | --- | --- |
| 状态 | 每轮发送前重建资源池，把终端实时状态投影成资源作用域 | `ai_chat_view::ResourceContextRefresher` + `terminal_view::ai_live_context` |
| 内容 | 完整屏幕内容仍由 `terminal.read` 按需读取 | `public_mcp` 既有实现，未改动 |

### 2.1 每轮重建（地基）

`AgentChatView` 新增可选回调：

```rust
pub type ResourceContextRefresher =
    Arc<dyn Fn(&App) -> Option<ResourceContext> + Send + Sync>;
```

`start_local_submission` 在应用 `@` 提及之前调用它，命中时 `session.set_resources(...)`，
后续 `AgentTask` 每步重建 system prompt 时自然带上最新资源池
（`agent_runtime::tasks::agent` 每步用 `ctx.resources` 重新渲染 `{{resource_info}}`）。
回调在主线程执行，因此可以安全读取 `Entity<Terminal>` 等实体状态；返回 `None` 表示沿用旧池。

资源池由宿主注入（`DefaultAgentChatPanel::set_resource_context_refresher` → `AgentChatView`），
所以任何侧边栏都能复用，不需要改 `agent_runtime` 的 prompt 管道。

### 2.2 终端状态投影

`crates/terminal_view/src/ai_live_context/` 把 `Terminal` 的现场投影成作用域：

| 作用域键 | 取值来源 | 说明 |
| --- | --- | --- |
| `cwd` | `current_working_dir()`（OSC 7） | 用户当前目录 |
| `screen` | `mode().contains(ALT_SCREEN)` + `title()` | 普通 shell 还是全屏程序（vim/less/htop），后者提示「按键会进程序」 |
| `selection` | `selection_text()` | 行数/字符数 + 单行摘要，正文仍走工具 |
| `size` | `rows()` / `cols()` | 排版相关建议需要 |
| `process` | `child_exited()` | 终端进程已退出时提示 |

投影只替换本模块负责的键（`apply_live_scopes`），宿主自己写入的作用域（如连接信息）保留。
选中内容只进摘要、不进正文：正文每步重发会被反复计费，且会把终端输出反复送出。

### 2.3 默认目标

`build_live_resource_context` 把终端放在资源列表首位并设为 `current`，连接等资源只作为
target 候选。这样模型省略 `target` 时命中的是**眼前这台终端**，而不是「连接」——
后者在 SSH 侧边栏里原本指向连接，`terminal.read` 需要先 `list_sessions` 再指定 session。

## 3. 不做什么

- **不把整屏塞进 prompt**：40 行约 600–800 token/轮，还会随对话历史反复发送。
- **不写「每轮必须读屏」的硬规则**：多一轮工具调用（3–5s 延迟），且模型会读没用的内容；
  有了 `screen` / `process` 作用域，它自己判断该不该读。
- **不把状态放进用户消息**：会污染历史。资源池每轮重建、不落历史，天然 ephemeral。
- **不在数据库/Redis/Mongo 侧边栏先行接入**：它们没有「屏幕现场」概念，等有明确需求再复用
  同一个 `ResourceContextRefresher`。

## 4. 验证

- `cargo test -p terminal_view ai_live_context`：投影/摘要/替换/`describe()` 共 9 条单测。
- `cargo test -p ai_chat_view`：454 条既有测试通过（资源池、提及、会话切换无回归）。
- `cargo clippy -p terminal_view -p ai_chat_view --all-targets`：新增代码无告警。
- 手工：终端里 `cd` / 进 `vim` / 选中若干行后提问，观察模型是否在首轮直接引用当前目录与
  全屏状态；`terminal.read` 不传 target 时是否读到当前终端。

## 5. 已知取舍

- **前缀缓存**：资源池位于 system prompt，作用域变化会让前缀失效。但 `{{current_time}}`
  精确到秒、每个 step 都在变，前缀缓存当前并未生效；若要修复缓存，应把时间与资源池一起
  挪到消息尾部，而不是单独处理资源池。
- **选中内容时效**：只在发送瞬间抓取；发送后用户继续选中不会影响本轮。
- **状态读取有锁**：`mode()` / `selection_text()` 需要 `FairMutex`，与解析线程争锁的临界区
  很短，尚未观察到发送卡顿；如遇大量输出导致卡顿，可改为非阻塞尝试并回退到上一次的状态。
