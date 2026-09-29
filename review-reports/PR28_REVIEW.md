# PR #28 代码审查记录（project-storage）

- 审查日期：2026-09-09
- 审查范围：PR #28（`project-storage` → `dev`，合并提交 `22e1720`），唯一功能提交 `2aaa847`
- 提交：`2aaa847` feat(project)!: 项目文件改为 `<项目名>.gsa` 自定义格式（魔数头 + JSON）
- 改动文件：`src-tauri/src/project/mod.rs`（+349）、`src/views/Welcome.vue`、`src-tauri/tests/ocr_e2e.rs`（注释）、README（中英）/ development（中英）/ 计划书
- 严重程度映射：🔴 严重 → P0｜🟡 建议 → P1｜🟢 优化 → P2
- 测试基线：`cargo test --lib` 168 通过（前基线 158 + 本 PR 新增 10）

---

## P0（必须修复）

无。核心路径（解析/序列化/原子写/冲突检查）正确且有测试覆盖。

---

## P1（建议修复）

### P1-1 `save_project` 的改名清理逻辑在"正好需要它"的场景下必然失效 → 目录双 `.gsa`、项目打不开

- **位置**：`src-tauri/src/project/mod.rs`（`save_project` 末尾的清理块）
- **问题**：清理逻辑用 `find_project_file` 找"既有项目文件"再删除异名者，但该函数**在有 ≥2 个 `.gsa` 时返回 Err**。而改名场景恰好先写入了新名文件：

  1. 目录中只有 `A.gsa`（或用户手动把文件改名成 `新名.gsa`）；
  2. `save_project` 按新名写入 `B.gsa` → 目录现有 2 个 `.gsa`；
  3. `find_project_file` 看到 2 个 → **Err("存在多个项目文件")**；
  4. `if let Ok(existing)` 分支不成立 → **清理被跳过**；
  5. `save_project` 照常返回 Ok，但目录永久双文件，下次 `open_project` 直接报"存在多个项目文件，无法确定要打开的项目"，需手动删除一个才能恢复。

  即 commit message 声称的"项目改名时自动替换旧文件"**在任何改名场景下都不会发生**。
- **可达性**：当前 UI 无项目改名入口（只有轨道改名），但存在一条用户可达路径——**在资源管理器里手动重命名 `.gsa` 文件**后，下一次自动保存（1 秒防抖）就会写回旧名的 `.gsa`，目录立即双文件、项目打不开。此外未来任何"项目改名"功能都会直接踩中。
- **修复建议**：不依赖 `find_project_file` 的唯一性语义，直接枚举目录内全部 `.gsa` 并删除异名者：

  ```rust
  // 改名场景：目录中原有的其它 .gsa 已被新文件取代，删除之避免双份
  if let Ok(entries) = fs::read_dir(&project_path) {
      for entry in entries.flatten() {
          let p = entry.path();
          let is_gsa = p.extension().and_then(|e| e.to_str())
              .map(|e| e.eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false);
          if p.is_file() && is_gsa && p != project_file {
              let _ = fs::remove_file(&p);
          }
      }
  }
  ```
- **状态**：✅ 已解决（方案 B，根治：与用户确认后放弃"一目录一项目"约束——**项目身份 = .gsa 文件路径**，`save_project` 永远写回 `path` 指向的文件，删异名清理块整体删除，本 bug 类别不复存在；改名只改 JSON 内字段、文件名不变；`open_project` 混合兼容（.gsa 文件直接打开 / 目录取唯一 .gsa，旧最近项目列表无缝可用）；同目录多项目共存成为新能力（同一素材可建多个字幕版本）。回归测试 `test_save_writes_own_file_only_no_cross_cleanup` 覆盖）

---

## P2（优化）

### P2-1 `create_project` 冲突检查对"多 `.gsa`"目录静默放行，可能覆盖既有项目

- **位置**：`src-tauri/src/project/mod.rs`（`create_project` 开头）
- **问题**：`if let Ok(existing) = find_project_file(...)` 在目录有 ≥2 个 `.gsa`（已损坏状态）时返回 Err → 检查被跳过 → 照常创建。若新项目名 sanitize 后与损坏目录中某个既有 `.gsa` 同名，**原子写会直接用空项目覆盖那个文件**（数据丢失）。前提是目录已处于损坏状态，故定级 P2；与 P1-1 同修时可顺带解决（改为枚举判定：目录内存在任何 `.gsa` 即拒绝并列出）。
- **状态**：✅ 已解决（方案 B：冲突检查改为仅 `sanitize(项目名).gsa` 已存在时拒绝——多项目目录成为合法状态，不再有"损坏目录"误判面；同名文件的保护从"整个目录"收窄到"自己的文件"，覆盖行为明确且经 `test_create_allows_sibling_projects_same_name_rejected` 验证）

### P2-2 原子写缺 `sync_all`（fsync），断电时序不保证

- **位置**：`src-tauri/src/project/mod.rs`（`write_project_file`）
- **问题**：`fs::write` + `fs::rename` 之间无 `File::sync_all`。进程崩溃时临时文件策略已能保护目标文件；但**断电/系统崩溃**时 NTFS 不保证元数据落盘顺序，rename 后的目标文件可能是 0 字节或半截。桌面字幕工具可接受（项目另有视频/导出物），记录备查；追求极致时在 rename 前 `sync_all`。
- **状态**：⬜ 记录备查

### P2-3 并发保存的临时文件名碰撞

- **位置**：`src-tauri/src/project/mod.rs`（`write_project_file` 的固定 `.gsa.tmp` 路径）
- **问题**：`save_project` 是同步 Tauri 命令（线程池执行），自动保存（1s 防抖）与手动 Ctrl+S 同时 in-flight 时，两个保存共用同一 `*.gsa.tmp`：后完成者 rename 时发现 tmp 已被前者消费 → NotFound → 报"无法写入项目文件"（临时文件清理失败被吞，目标文件不受影响）。窗口极窄、后果为一次保存失败提示，记录备查；如需消除可给 tmp 加进程内序号或 uuid 后缀。
- **状态**：⬜ 记录备查

---

## 审查通过的部分（无需改动）

- **魔数头解析**：`parse_project` 容忍 UTF-8 BOM 与 CRLF；`v` 版本号 `u32` 解析，更高版本明确拒绝并提示升级（有测试）；纯 JSON（旧 `project.json`）不再被接受（有测试）。`split_once('\n')` 的单行 JSON 边界正确报"缺少文件头"。
- **`sanitize_project_name`**：非法字符/控制符替换、首尾空白与结尾点清理、保留设备名（含 `nul.txt` 形态，针对首点前的 stem 判断）、按字符数（非字节）截断、清空回退 "project"，测试覆盖全面。保留名追加 `_` 不可能超长（保留名 ≤4 字符而截断阈值 100）。
- **原子写**：tmp 与目标同目录（rename 保持原子性）；rename 失败时清理 tmp；tmp 扩展名 `.tmp` 不会被 `find_project_file` 误认（有测试验证无残留）。
- **冲突检查常规路径**：单 `.gsa` 目录创建被正确拒绝；`open_project` 找 0/1/多 文件三态明确报错（有测试），扩展名不区分大小写。
- **前端**：`Welcome.vue` 用与后端一致的非法字符集前置拦截并给出错误提示；后端 sanitize 仍保留（纵深防御）。文件名由后端 sanitize 决定，前端不重复实现命名逻辑，职责清晰。
- **文档**：README（中英）、development（中英）、计划书同步更新；`ocr_e2e` 仅注释调整；BREAKING CHANGE（旧 `project.json` 不再兼容）在提交信息中明确声明。
- **测试**：10 个新单测命名与断言质量好（含边界与错误路径）；`cargo test --lib` 168 全过。

## 修复优先级建议

1. **P1-1（改名清理失效 → 目录双文件打不开）**：唯一需要改代码的项，约 10 行；建议连同 P2-1 一起把"枚举 + 删除异名者"的模式用于 `save_project`，把"枚举 + 拒绝"用于 `create_project`。
2. **P2-2 / P2-3**：记录备查，可顺手加 `sync_all` 与 tmp 随机后缀（各 1-2 行）。
