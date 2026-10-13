# PR #42 审查记录

- **PR**：`fix(ocr): 修复 O1/O2/O3/O5 登记缺陷（文本长度门 + 合并空档门 + worker 几何行序），O4 评估后不修`
- **分支**：`docs/sync-current-state` → `dev`（10 个提交：e43ec3e / 0d8120e / 9add817 / 8848078 / d18e4dc / 33e154e / b501045 / 171236e / dfb550f / 6da1b7b）
- **审查日期**：2026-10-13
- **审查者**：15 年资深工程师视角，严重度映射 🔴→P0 / 🟡→P1 / 🟢→P2

## 审查范围

- `src-tauri/src/ocr/merge.rs`（O3 空档门 + O1/O5 输出层 pass）
- `src-tauri/src/ocr/params.rs` / `pipeline.rs` / `mod.rs`（参数贯通与 pass 位置）
- `scripts/ocr_worker.py`（O2 几何行重排）
- `src-tauri/tests/`（common 夹具、bench_pierro、ocr_e2e、lib 单测）
- 前端（`ocrDefaults.ts`、`types/index.ts`、`AsrView.vue`、`CorpusView.vue`）
- `benchmark/OCR_PIPELINE_DEFECTS.md`、`.gitignore`

## 总评

**无 P0。** 三个修复的实现质量高：O3 的门值有四素材探针普查背书（合法对上限 10.731s vs 缺陷间隔 44.733s，取 11.0s 两端都有余量）；O1/O5 的"实质字符"口径直接复用既有 `norm_chars`，避免再造口径；O2 的几何行重排两次否决记录（方向反了 / 下标错位）写进了代码 docstring 与台账，防复发做得很到位。台账判读更正（`e43ec3e` 撤销 21.02s 条误判）与状态行回填均符合本仓库"改了就更新档案"约定。

发现 1 条 P1（基准复现步骤缺失，会让"可复现"承诺落空）与若干 P2（其中 1 条为本 PR 新引入的 clippy 警告）。

## 逐条发现

### P1-1 PR 的"验证方式（可复现）"缺 `bootstrap_ocr.ps1` 步骤，O2 修复对已有 runtime 的复现者不生效

- **位置**：PR 描述「验证方式（可复现）」节；`scripts/bootstrap_ocr.ps1:100`
- **问题**：O2 修复改的是**真源** `scripts/ocr_worker.py`，实际运行的是部署副本 `runtime/worker/ocr_worker.py`——副本只在跑 `bootstrap_ocr.ps1` 时用 `Copy-Item -Force` 覆盖（该脚本无启动时自动同步）。复现者若已装好 runtime（一次性 bootstrap 过），按 PR 给出的两条 `cargo test --ignored` 命令跑，**worker 仍是旧版行拼接逻辑**，会得出"O2 修复无效/基准对不上"的错误结论，且无任何报错提示。台账 O2 节虽写了"真源，经 bootstrap_ocr.ps1 复制进 runtime/worker/"，但那只解释机制，不构成操作指引。
- **建议**：在 PR「验证方式」节开头加一行：`若 runtime 已初始化，先重跑 scripts/bootstrap_ocr.ps1 同步 worker（本次 O2 改的是 scripts/ocr_worker.py，需重新复制）`；后续 PR 凡涉及 `scripts/*_worker.py` 的都照此惯例。

### P2-1 本 PR 新引入 clippy `unused_mut` 警告（pipeline.rs:558）

- **位置**：`src-tauri/src/ocr/pipeline.rs:558`
- **问题**：dev 基线上 `let mut segments = merge_short_fragments_into_next(...)` 的 `mut` 是必要的——随后 `for seg in &mut segments` 原地改它。本 PR 在其后插入 shadowing 绑定 `let mut segments = drop_below_min_text_length(...)`，原绑定从此只被 move 消费，`mut` 变多余 ⇒ 新增 `unused_mut` 警告（本次 worktree 实测 8 条警告，其余 7 条经比对均为既有基线）。
- **修复**：去掉 pass-6 绑定的 `mut`：

```rust
// pipeline.rs:558（第七遍已有自己的 mut 绑定，此绑定只被 move）
let segments = merge_short_fragments_into_next(segments, min_subtitle_sec);
```

### P2-2 `drop_below_min_text_length` 调用点注释把 0/1 语义写反

- **位置**：`src-tauri/src/ocr/pipeline.rs:538`
- **问题**：注释写 `0/1 = 只丢纯符号段/关闭`，按斜杠对位即 0→只丢纯符号段、1→关闭，与实际（`merge.rs:907`：0 = 关闭，1 = 只丢纯符号段）恰好相反。`params.rs` / 前端文案都是对的，仅此一处笔误，但对先读调用点的人是误导。
- **修复**：

```rust
// 文本长度门（O1/O5）：0 = 关闭，1 = 只丢纯符号段，见 `drop_below_min_text_length`
```

### P2-3 截图语料入口（`run_ocr_images`）不受 `min_text_length` 门控制，两个语料 OCR 入口行为不一致

- **位置**：`src-tauri/src/ocr/commands.rs:80-128`（`run_ocr_images` → `lines_from_results`，不经过 `ocr_pass`）
- **问题**：语料侧有两条 OCR 入口——视频管线（走 `ocr_pass`，受新门约束）与用户选截图（`run_ocr_images` 直出 `lines_from_results`，**不过门**）。粗略框选的截图同样可能产出 `口`/`+` 一类纯符号行，两入口过滤行为不一致且无提示。截图源通常是用户逐张挑的剧情文本，垃圾概率低，故不要求本 PR 改行为；但至少应把口径写明，避免后来者以为"语料侧一定过滤"。
- **建议**：在 `params.rs` 的 `DEFAULT_MIN_TEXT_LENGTH` 注释补一句作用域说明：`该门只作用于视频 OCR 管线（ocr_pass）；run_ocr_images 截图语料入口不过此门`。

### P2-4 `test_merge_similar_adjacent_gap_within_window_by_interval` 的断言没有真正验证 `interval` 缩放项

- **位置**：`src-tauri/src/ocr/tests.rs:1786-1793`
- **问题**：测试名与注释称"空档门随网格缩放：interval=2.0 时 1.5s 的空档仍在窗内"，但门 = `SIMILAR_MAX_GAP_SEC.max(interval)` = `11.0.max(2.0)` = 11.0——1.5s 空档在**纯绝对门**下也合并，断言结果与不含 `.max(interval)` 的实现无差别。真正需要守的是"interval 超过 11.0 时门跟着变大"这一分支（当前产品中 interval 最大 0.5~2.0，该分支暂无真实路径，但既然为它写了测试就应测到它）。
- **建议**：把用例改成 interval=12.0、空档 11.5：

```rust
let segs = vec![/* 空档 1.0→12.5，gap 11.5 */];
let out = merge_similar_adjacent(segs, 0.3, 12.0);
assert_eq!(out.len(), 1); // .max(interval) 生效：12.0 > 11.5 → 仍合并
```

（不阻塞合并——现状实现正确，只是测试没测到目标分支。）

### P2-5（观察项，不要求修）`_lines_geometric` 逐链比较只用 `cur[-1]`，极端下高框可能"搭桥"跨行

- **位置**：`scripts/ocr_worker.py` `_lines_geometric` 行分组循环
- **说明**：行分组比较的是新框与**当前行最后一个加入的框**，而非当前行的聚合外接框。若出现一个纵向贯穿两行的高框夹在中间，理论上可经链式比较把两行物理行并进一行。但本 PR 的判据经四帧实测标定（真同行重叠 68~75px vs 相邻行 0~5px，0.4×行高门在中间），且 game subtitle 场景几乎不存在跨两行对话的单一 rec 框——**现状正确性由实测背书，勿动**；此处登记只为将来若出现"整帧多行连成一句"的回归时，第一个该查的边界就是它。

### P2-6（观察项，不要求修）`.gitignore` 新增 `*.pyc` 但仓库仍跟踪一个 pyc

- **位置**：`.gitignore:61-62` vs `src-tauri/tests/fuse_lab/__pycache__/__init__.cpython-312.pyc`（仍被跟踪）
- **说明**：`6da1b7b` 刻意"还原为基线版本、不做内容变更"（属他人在研目录），处置正确；但结果是"ignore 规则存在 + pyc 仍被跟踪"的并存状态（git 对已跟踪文件不适用 ignore）。将来该目录再发生字节码变化时会重新出现"修改的 pyc"噪音，届时应把该 pyc 也一并 `git rm --cached` + 提交。

## 已核对无误的点（抽查清单）

| 检查项 | 结论 |
|---|---|
| `norm_chars` 口径与"实质字符"宣称一致（Unicode 字母数字含 CJK、剔标点空白，`Vec<char>::len()`） | ✓ `merge.rs:394` |
| `drop_below_min_text_length` 位置：全部合并 pass 之后、标点归一化之前；产品默认 1 只丢 0 实质字符段，`嗯`/`a` 等单字实词不受影响 | ✓ `pipeline.rs:558-559` |
| O3 门判据 `seg.start - last.end <= max(11.0, interval)`，与 `merge_contained_adjacent` 同款缩放；正例 0.97s 对保留、缺陷 44.7s 对不并 | ✓ `merge.rs:878-879` |
| `merge_similar_adjacent` 签名变更的所有调用点（pipeline + tests）同步 | ✓ `pipeline.rs:539` |
| worker `_extract_items` 置信度过滤**带框索引**（先对齐后过滤，避开台账教训②）；`_lines_geometric` 任一框缺失几何即回落 rec 顺序（兼容路径不劣化）；`rec_boxes` 形状异常（非 Nx4）同样回落 | ✓ |
| 行间保留 `\n` 不拍平（D9/D11/D12 姓名框结构依赖） | ✓ `_lines_geometric` 返回 `"\n".join(out)` |
| 前端持久化：旧 localStorage 存档缺 `min_text_length` → `loadOcrParams` 逐字段合并自动补默认 1 | ✓ `ocrDefaults.ts` |
| 基准夹具：四案例 `hardsub_min_text_length=2` 与"真值文件无 ≤1 条目"的零误杀论证一致；语料基准走产品默认 1（`M`/`X` 按用户标定保留）；env 覆盖与既有 `GSA_BENCH_HARDSUB_MIN_SUBTITLE_SEC` 同款 | ✓ `common/mod.rs` |
| 台账状态行：O1/O2/O3/O5 依据提交号与实际提交一一对应；O4 处置理由登记完整；21.02s 判读更正证据链（真值 20.43s + 日志逐帧 conf + 产出侧 #58/#59 独立条目）自洽 | ✓ |
| e43ec3e 误入库 pyc 的处置（新增者 untrack、他人在研目录者还原基线 + ignore 防复发） | ✓ |

## 验证（本审查独立执行）

- 在 `../AIGSA_pr42_review` worktree（`origin/docs/sync-current-state` @ 6da1b7b）运行：
  - `cargo test --lib`：**261 passed, 0 failed**（与 PR 描述一致）
  - `cargo clippy --lib`：8 条警告，**1 条为本 PR 新引入**（P2-1），其余 7 条与 dev 基线相同
- `#[ignore]` GPU 基准（嵌字 71.5/71.4/71.7/58.9、语料 100.0/97.1/99.6/99.5）依据 PR 描述与台账记录采信，未重跑（需 GPU + 长时长）。

## 结论

**建议合并前处理 P1-1（PR 描述补 bootstrap 步骤）与 P2-1/P2-2（一行注释 + 一处 mut），其余按需。** 三处改动均不影响行为，可与后续提交一并带上。

## 处置记录（2026-10-13，用户裁决）

- **P1-1 ✅**：台账 O2 状态行补 runtime/worker 副本同步提示（本次提交）；PR #42 描述「验证方式」节已同步插入该提示（`gh pr edit`，不落提交）。
- **P2-1 ✅ / P2-2 ✅**：`pipeline.rs` 已修（去 mut + 注释改正），本次提交。
- **P2-3 ✅**：`params.rs::DEFAULT_MIN_TEXT_LENGTH` 注释补作用域说明，本次提交。
- **P2-4 ✅**：测试改为 interval=12.0（真正守住 `.max(interval)` 分支，另补产品默认网格对照断言），`cargo test --lib` 261 过、clippy 恢复 6 条基线警告，本次提交。
- **P2-5 / P2-6**：按观察项保留，不处置。
- 处置提交号待用户手动提交后回填（本仓库 Git 约定：不代提交）。
