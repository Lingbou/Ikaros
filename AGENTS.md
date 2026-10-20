# AGENTS.md — Ikaros 开发与协作规范

本文件为所有参与 **Ikaros** 开发的人类与 AI Agent（Codex / Claude 等外部会话）设定强制性工作流、工程约束与架构原则。在本仓库创建 Issue、写代码、提 PR、改文档，一律以本文件为准。

快速索引：项目现状与实现细节见 `README.md`（它是现状快照，具体行为以代码为准）。

---

## 零、 核心开发哲学

1. **破坏性重构优先（Zero Backward Compatibility Burden）**
   项目处于 Pre-1.0 Alpha。协议、数据库 schema、内部接口一律不留向后兼容包袱：没有迁移层、没有旧格式 shim、没有废弃 API 的过渡保留。发现更优设计就直接改、直接提升版本、直接删除旧路径。
2. **显式重置优于静默兼容**
   协议 / schema 变更时同步更新版本常量与生成物；开发数据按 README §10.4、§14.2 的方式显式重置。永远不静默接受、改写或删除用户数据。
3. **一切以功能交付为中心**
   当前阶段不启用 CI，也不设强制门禁与必跑脚本；流程存在的意义是让功能更快、更干净地落地。验证按需进行（见第五节），不为了流程而流程。
4. **零垃圾留存（Zero Garbage Left Behind）**
   所有临时产物、实验脚本、参考仓库克隆收敛于 `.temp/`；每次任务交付后彻底清理 worktree 与本地临时分支，保持主工作区绝对整洁。

---

## 一、 标准任务开发工作流

先 Issue，后代码。功能、修复、重构都走完整闭环：

```text
[创建 Issue] ➔ [.temp/worktrees 建 Worktree] ➔ [开发] ➔ [提交(-s -S)并推送] ➔ [创建 PR 并合并] ➔ [同步 main] ➔ [彻底清理]
```

### 步骤 1：创建 Issue
```powershell
# 远程与 gh 一律使用 SSH；gh 未在 PATH 时用全路径调用
gh issue create --title "<type>(<scope>): <清晰描述>" --body "<背景、目标、验收方式>"
# 记录返回的 Issue 编号，例如 #15
```

### 步骤 2：在 `.temp/worktrees/` 下创建隔离 Worktree
严禁在主工作区切分支开发。统一在 `.temp/worktrees/` 下开辟独立 worktree：
```powershell
# 位于主仓库根目录、main 分支
git checkout main
git pull --ff-only origin main

$BRANCH = "feat/issue-<编号>-<简短名>"   # 例：feat/issue-12-runtime-packaging
git worktree add ".temp/worktrees/$BRANCH" -b $BRANCH
cd ".temp/worktrees/$BRANCH"
```

### 步骤 3：在 Worktree 中开发并提交
专注把功能做对；提交前按需自测（当前不设强制门禁）。所有提交必须签名并带签收：
```powershell
git add .
git commit -s -S -m "<type>(<scope>): <subject>"   # -s 签收，-S SSH 签名
git push -u origin $BRANCH
```

### 步骤 4：创建 Pull Request
```powershell
gh pr create --title "<type>(<scope>): <subject>" --body "Closes #<编号>`n`n<变更说明与影响；做了验证就附一句结论>"
```

### 步骤 5：合并后同步主分支
```powershell
cd <主仓库根目录>
git checkout main
git pull --ff-only origin main
```

### 步骤 6：清理 Worktree 与本地临时分支（强制）
```powershell
git worktree remove --force ".temp/worktrees/$BRANCH"
git branch -D $BRANCH
git worktree list   # 确认列表干净
```

---

## 二、 分支与提交规范

### 1. 分支命名
- 格式：`<type>/issue-<编号>-<简短描述>`；
- 常用前缀：`feat/`、`fix/`、`refactor/`、`chore/`、`docs/`；
- 使用通用语义化前缀，不使用工具专属前缀（如 `codex/`）。

### 2. Commit Message 与签名
严格遵循 **Conventional Commits**：`<type>(<scope>): <subject>`。

- `feat` 新功能 ／ `fix` 修缺陷 ／ `refactor` 重构不改变行为 ／ `docs` 文档 ／ `chore` 构建与依赖 ／ `perf` 性能 ／ `test` 测试 ／ `style` 纯格式；
- `scope` 用实际改动模块：`runtime`、`desktop`、`agent`、`provider`、`process`、`tools`、`storage`、`protocol`、`memory`、`skills`、`server` 等；
- 破坏性变更在冒号前加 `!`（如 `refactor(runtime)!: adopt codex-style context budgeting`），并在 body 说明影响与数据重置方式；
- **所有提交必须使用 `git commit -s -S`**：`-s` 添加 `Signed-off-by`，`-S` 做 SSH 签名；依赖每台机器上的 `gpg.format=ssh` 与 `user.signingkey` 配置，公钥需在 GitHub 注册为 Signing Key 才会显示 Verified；
- 一个提交只做一件事，禁止把无关重构或格式化混入功能提交。

### 3. 远程与网络
- 仓库远程一律使用 SSH：`git@github.com:Lingbou/Ikaros.git`，禁止改成 https 地址；
- `gh` 的 git 协议保持 SSH：`gh config set git_protocol ssh --host github.com`。

---

## 三、 目录与环境隔离规范

### 1. `.temp/` 目录
- 路径：`$PROJECT_ROOT/.temp/`，已加入 `.gitignore`，**永不提交**；
- `.temp/worktrees/`：开发用 Git Worktree，合并后立即删除，不留孤立目录；
- `.temp/` 其余目录：参考仓库克隆（airi、codex、openclaw 等）与实验脚本，仅用于阅读借鉴，不参与交付，不作为改动来源；
- 定期清理，保持 `.temp/` 无长期残留。

### 2. 运行数据与密钥
- 运行数据默认 `~/.ikaros`（Windows：`%USERPROFILE%\.ikaros`），由 Runtime 独占管理；
- 做实验用 `IKAROS_HOME` 指向临时目录，避免污染日常数据；
- 密钥只放本地 key 文件或 `/secrets/`（已忽略），不得进入命令行、Issue、PR、日志或提交。

---

## 四、 架构设计与代码底线

1. **技术栈锁定**
   - Desktop：Electron + TypeScript（strict）+ React 19 + Tailwind CSS v4 + Zustand + Radix UI，构建链 electron-vite / Vite / Vitest；
   - Runtime：Python 3.12–3.13，依赖保持最小面（当前仅 httpx / websockets / PyYAML + 标准库），存储 SQLite；
   - 新增依赖必须有明确理由，并保证 Windows + Linux 双平台可用。
2. **进程边界不可穿透**
   渲染进程是纯呈现层：只能通过 preload 暴露的 `window.ikaros` API 使用能力，不直接触碰 WebSocket、Electron IPC 原生对象或文件系统。新增 IPC 通道必须在 main 侧校验参数，安全策略集中在 `src/main/security.ts`。
3. **协议唯一事实源**
   `runtime/src/ikaros_runtime/protocol/spec.py` 是协议（方法 / 事件 / 工具 ID / 版本号）的唯一定义处。生成物 `runtime/protocol/runtime-protocol.json`、`runtime/protocol/golden-trace.json`、`ikaros/desktop/src/shared/generated/runtimeProtocol.ts` 一律由生成器产出，手工编辑会被 `--check` 拒绝。协议变更必须同步 golden trace 与两端契约测试。
4. **存储与恢复语义**
   `state.db` 的 events 表是 append-only Journal，Thread/Turn/Run/Item 等查询表都是可重建投影；schema 变更 = 提升版本常量 + 更新测试 + 文档说明重置方式（开发期无迁移路径）。用户数据只增不删，除非用户显式重置。
5. **执行模型保持有序**
   单 Runtime 进程、单事件循环、单调度 worker、每 Run 工具串行、每 Branch 串行队列。并发 / 并行化属于架构级变更：先设计、再补测试、并验证崩溃恢复与事件重放语义。
6. **上下文管理保持 codex 风格**
   上下文预算、自动压缩（compaction）与有界完成检查的语义不得削弱；相关改动同步更新 README §6.6–6.7 与测试。
7. **数据归属清晰**
   会话、执行、Memory、Provider 配置归 Runtime；主题、语言、草稿等 UI 偏好归 Desktop。不跨域存储、不双写。
8. **凭据绝不外泄**
   凭据的精确值匹配 + ≥8 字符嵌入检测规则不得放宽；凭据不进入 Journal、事件、日志、列表响应、异常与 UI。live smoke 的密钥扫描输出必须 `PASS`。
9. **界面文案双语同步**
   固定短语在 `ikaros/desktop/src/renderer/i18n.ts` 同时维护 en 与 zh-CN；i18n 测试校验键集合一致。用户与模型内容不做翻译。
10. **文档与代码同步**
   协议、schema、关键常量、环境变量变化时，同步更新 README 对应章节（§6.2 / §6.18 / §7 / §8 / §13）与核对日期。

---

## 五、 验证（按需，不强制）

项目当前不启用 CI，也不要求必跑的门禁或脚本；验证服务于本次改动本身。需要自测时可用：

Runtime（主仓库根目录）：
```powershell
uv run --project runtime ruff check runtime
uv run --project runtime mypy runtime/src runtime/tests
uv run --project runtime pytest runtime/tests
uv run --project runtime python -m ikaros_runtime.protocol.generate --check
```

Desktop（`ikaros/desktop/`）：
```powershell
pnpm check   # typecheck + vitest + 生产构建
```

- 真实 Provider 的 live smoke 属可选验收（需 key 文件，见 README §10.2）；
- 涉及打包改动时可用 `pnpm package:dir` 冒烟，正式产物用 `pnpm package:win` / `pnpm package:linux`。

---

## 六、 常见变更路径速查

| 变更 | 步骤 |
| --- | --- |
| 协议（方法 / 事件 / 工具 ID） | 改 `protocol/spec.py` → 运行 `protocol.generate` → 更新 golden trace → 跑两端测试 |
| 数据库 schema | 改 `storage/schema.py` 或 `memory/schema.py` → 提升版本常量 → 更新测试与文档 |
| 新增 / 修改工具 | 改 `tools/` 实现与注册装配；新工具 ID 走协议变更流程 |
| 界面文案 | `renderer/i18n.ts` 同步 en + zh-CN |
| Provider 行为 | 改 `providers/openai_compatible/` 并更新 `test_openai_compatible.py` |

更细的步骤说明以 README §10.4 为准。

---

## 七、 任务完成标准

一个任务只有同时满足以下条件才算完成：

- 代码、文档随同一个 PR 交付并已合并；
- 主分支已同步，worktree 与本地临时分支已清理；
- 所有提交均为 `-s -S` 签名提交；
- 若改变了对外行为或关键常量，README 已同步更新。