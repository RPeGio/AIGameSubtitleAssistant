# PR #43 审查记录

- **PR**：`feat(fuse): T4 向量召回融合路线落地 —— Python 对齐核心 / Rust 接线 / 前端审计表`
- **分支**：`feat/fuse-t4-vector-dp` → `dev`（5 提交：04ba47d / 8b8433a / fb323d1 / f533cad / 1d0fb10）
- **审查日期**：2026-10-13（此 PR 为 BREAKING CHANGE：LLM 融合路径删除）
- **审查者**：15 年资深工程师视角，严重度映射 🔴→P0 / 🟡→P1 / 🟢→P2

## 审查范围

26 文件（+2751/−1200）：`scripts/fuse_core.py`（对齐核心单源）、`embed_worker.py`（产品子进程）、
`fuse_worker_parity.py`（对拍门）、`fuse_calib.py`/`fuse_lab/__init__.py`（改为导入 fuse_core）、
`ai_runtime/embed.rs`/`config.rs`/`mod.rs`、`fuse/mod.rs`（重写）、`project/mod.rs` 持久化契约、
前端（`FuseView.vue`/`fuseAudit.ts`/`project.ts`/`types`/`check_fuse_audit.mjs`）、
删除 `tests/bench_fusion.rs`、四个档案（plan / FUSE 台账 / F14 / 标定）与 `bootstrap_embed.ps1`。

## 总评

**无 P0。架构决策可圈点**：对齐算法抽成 `fuse_core.py` 单一真源（产品 worker 与实验台共同导入，
漂移面归零）；对拍门（8/8 零差异）让"实验台验证过的算法 = 产品行为"有了**机制保证**而不只是口头承诺；
Rust 侧 `map_worker_segments` 对越界 `ocr_index` **报错而非静默降级**——明确吸取了 F13 时代"假绿"的教训。
F14「按说话人分块对齐」拟议方案经合成场景**数据否决**且记录了否决依据（比做成无效方案再回滚便宜得多）。

文件级抽查未发现算法/契约错误。发现 1 条 P1（`embed_model`/`embed_tokenizer` 配置键有校验无消费——
配置了模型/分词器路径，worker 却从不接收）与若干 P2。

## 逐条发现

### P1-1 `embed_model` / `embed_tokenizer` 配置键被校验、注册就绪探测消费，但从未传给 worker

- **位置**：`src-tauri/src/fuse/mod.rs:179-192`（`EmbedRequest` 只有 corpus/segments/unmatch_filter/top_k）、
  `scripts/embed_worker.py:81-82`（worker 自己按 `inp.get("model")`/`inp.get("onnx")` 或内置默认取值）
- **问题**：`RuntimeConfig` 的四个 `embed_*` 键走"整组必配或整组为空"校验，`EmbedManager::status` 也检
  四键齐；但真跑融合时 `embed_worker` 与 `embed_deps_dir` 被 Rust 使用，**`embed_model`/`embed_tokenizer`
  从未进入请求体**——worker 恒以内置默认 `multilingual-e5-small` + `model_qint8_avx512_vnni.onnx` 运行
  （环境变量 `GSA_EMBED_ONNS`/`GSA_EMBED_ONNX` 除外）。bootstrap 脚本也只写死同一套路径。
  当前无害（用户只会配同一套），但契约上这是"配置存在 ⇔ 生效"的假承诺：将来换编码模型时
  用户/脚本改 `embed_model` 不会有任何效果，而校验通过、就绪探测就绪，等于换不了。
- **修复（二选一，建议 a）**：
  a) `EmbedRequest` 补 `model`/`tokenizer`（或 `onnx`）字段，`fuse_pipeline` 从 config 传入——worker 已支持这两个可选键：

```rust
// fuse/mod.rs：EmbedRequest 增加（worker 端 inp.get 已兼容）
model: config.embed_model.rsplit('/').next().filter(|s| !s.is_empty())
    .map(|s| s.to_string()),  // worker 只吃模型目录名
onnx: config.embed_model.rsplit('/').next().map(|s| s.to_string()),  // 具体键位按语义拆分
```

  （注意 worker 的 `model` 语义是 models/embed 下**目录名**，`onnx` 是 onnx 文件名——config 里两个合一字段，
  传参前需在 Rust 侧拆分或把 config 拆成两键；若嫌复杂，b 方案：保留现状但在 `RuntimeConfig.embed_model`
  的 doc 注释明确写"**当前仅作配置占位，worker 恒用内置默认**，换模型需改 `fuse_core.py`/bootstrap"。）

### P2-1 `run_embed_worker` 的固定临时目录按进程 PID 命名，并发调用会互相覆盖输入/输出

- **位置**：`src-tauri/src/ai_runtime/embed.rs:135-140`
- **问题**：`temp_dir().join(format!("gsa_embed_fuse_{}", std::process::id()))` 对同一进程恒定不变。
  当前唯一调用链 `run_fuse` 受前端 `fuseRunning` 门控，单窗口语义下安全；但风险面是
  **同一进程内两次并发 `run_embed_worker`**（未来若加"批量工程"或双命令入口）互写对方的
  `input.json`/`output.json`：若两请求 corpus/段数不同，段数校验能兜住；语料条数恰好相同则
  **静默换文本**——没有任何报错。
- **修复**（一行级）：调用每次生成唯一子目录，并在读完后清理：

```rust
let tmp = std::env::temp_dir().join(format!(
    "gsa_embed_fuse_{}_{}", std::process::id(), std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
));
```

（防御性：加一句跑前 `let _ = std::fs::remove_file(&out_path);` 亦可消除陈旧输出被读的残余可能。）

### P2-2 worker 子进程无超时，onnxruntime 卡死则融合命令永久阻塞

- **位置**：`src-tauri/src/ai_runtime/embed.rs:177`（`child.wait()` 无限时）
- **问题**：`embed_worker` 的取舍注释明确"不做常驻、不做取消"（一次性 2s 级子进程，合理），
  但连**兜底超时**也没有：CPU 上 onnxruntime 偶发卡死（线程池死锁/磁盘慢）时 `run_fuse`
  永久 pending，前端只显示"融合运行中..."，用户唯一出路是重启应用。FunASR 有
  `funasr_timeout_minutes`，嵌入侧没有对应项。
- **修复**：`child.wait()` 改 `child.try_wait()` 轮询（如 120s 兜底，超时 kill + 读 stderr 报错），
  或补一个默认开启的 `embed_timeout_secs` 配置键（0=不限，与 funasr 同风格）。

### P2-3 三个档案的未来日期戳（2026-10-15 > 审查日 2026-10-13）

- **位置**：`GameSubtitleAssistant_Plan.md` T4 进度块（"F14 已实证收口（2026-10-15）"）、
  `benchmark/FUSE_PIPELINE_DEFECTS.md` F14 实验收口行、`src-tauri/src/fuse/mod.rs:14` 头注释
- **问题**：同一个未来日期在三个档案/代码注释重复出现。今天是 2026-10-13，实验不可能是
  10-15 做的。档案的价值在于可追溯，未来日期戳会让后来者怀疑整条实验记录的时间线。
- **修复**：统一改为实验实际执行日期；`fuse/mod.rs:14` 的"登记于 2026-10-15 待适配"同时
  与 §F14 现状矛盾——该节内容写的是"已实证收口、不做算法侧修复"，头注释的"不要启用
  向量融合"的红线表述也**过强**（残留面仅审计表展示、台词内容不受影响， 且四动作裁决
  已上线），建议同步措辞。

### P2-4 `map_worker_meta` 把候选下标与命中下标用同一 `.max(0)` 折叠，但两者 0 语义不同源

- **位置**：`src-tauri/src/fuse/mod.rs:136-154`
- **问题**：命中下标 `out.ocr_index`（-1=不配 → `max(0)`=0 合理）与候选下标 `out.candidates[].ocr_index`
  （worker **从不发 -1**，被掩码列已剔除；`max(0)` 属死代码）做了同款折叠。`usize` 无负值，
  `i64→usize` 直接 `as` 在 Rust 是截断转换——它**不报错**，将来 worker 若真发出异常负值，
  两个位置都会被无声折成 0（0 在候选里读作"#0"，排查时多绕一层）。越界正数已有显式
  Err 防护（`map_worker_segments`），负值也应同样显式。
- **修复**：候选映射改为 `else if idx < 0 { return Err(...) }` 走同一报错路径（或 `fieldset`
  校验后 `usize::try_from(...).map_err(...)`）。不阻塞合并——当前 worker 无法产生 -1 候选。
- **说明**：`FuseMeta.ocr_index`（0=不配、命中为 1-based）与 `FuseCandidate.ocr_index`（恒 1-based）
  的双基数约定**已完整对齐到前端**（`fuseAudit.ts` 的 `>=1` 判据、`corpusTexts[oi-1]` 取文本、
  `refreshReused` 计数、`buildAuditRows` 越界防空串），前端核对无误——此条仅涉 Rust 侧防御。

### P2-5（观察项，不要求修）PR 验收表与 plan 记录的测试数互不一致

- **位置**：PR 描述验收表（"cargo test 252 + 14 + 14"）vs `GameSubtitleAssistant_Plan.md`（"250+15+15"、
  "287/0"）vs 实测（**252 lib + 15 + 15** 全绿，另有 6 个单测试二进制的 1 ignored 项全绿）
- **说明**：三处数字来自不同提交时点，历史演进正常；验收表与 plan 最好在合并前按实测
  统一一次（避免"数字对不上"引发的追溯成本，本项目档案纪律一直很高）。

### P2-6（观察项，不要求修）`fuse_audit` 裁决后语料若再被编辑，`corpusText` 快照与实时语料可不一致

- **位置**：`src/stores/fuseAudit.ts`（`buildAuditRows` 以当下 corpus 建行）与 `FuseView.vue:314`
  （`row.corpusText || corpusAt(...)` 的双源取值）
- **说明**：融合后用户回到语料页增/删条目 → 既有事件的 `ocr_index` 不再指向原行（下游有
  越界防护与 store 替换校验兜底，不会崩）；但"审计表显示的语料行"与"融合当时真正命中的行"
  可分叉。这是**产品级待办**（审计语义要与"语料版本化"配套），本 PR 已尽合理兜底，建议
  在 F 台账登记为已知语义边界即可。

## 已核对无误的点（抽查清单）

| 检查项 | 结论 |
|---|---|
| `fuse_core` DP 四转移不变量：A 用**前缀最大**且 j'<k（纳入顺序先试后纳）、C 用**后缀最大**且 j'>k（先试后纳）严格区间成立；B/D O(1)；复杂度 O(n·m)（A/C 各一趟线性最大值） | ✓ `fuse_core.py:284-319` |
| 自检断言保留：`repeat_penalty=reset_penalty=inf` 与严格递增 `align_fast` 的**逐段等价**护栏随核心迁出后仍在（实测本机全过） | ✓ `fuse_calib --selfcheck` |
| `fuse_calib.py` / `fuse_lab/__init__.py` 是**忠实搬迁**而非复制（判别器双 `bench_fusion_*.py` 依赖的 `load_embedder`/`similarity_matrix`/`split_header` 经 re-export 仍可导入） | ✓ |
| `mask_empty_body` == `语料 body_norm 空` 且 `段 !content_free` → 列置 NEG；`mask_should_unmatched` 行置 NEG；worker 调用顺序与注释要求一致（行掩码在列掩码+masks 之前传递）；两掩码**不还原**其它格 | ✓ `embed_worker.py:93-98` |
| `map_worker_segments` 越界报错不静默、`-1` = 未配路径正确；`FuseResult.metas` 与 `segments` 等长（前端 `writeFusedSegments` 按下标对齐后连事件+meta 一起 sort，不破坏对位） | ✓ `fuse/mod.rs:106-133`、`project.ts` |
| `RuntimeConfig.validate` 四键整组规则 + 防穿越；测试覆盖（半配/穿越/全组） | ✓ `config.rs` |
| `fuse_e2e`（moon）真实走产品子进程 `run_embed_worker` 管线；`bootstrap_embed.ps1` 部署两脚本为 worker（**部署路径正确**、`check_embed_runtime` 提示 bootstrap） | ✓ |
| 审计四动作 store 侧：**克隆上应用 → 成功才 `recordSnapshot` + 写回**（失败零副作用，与既有撤销快照纪律一致）；`refreshReused` 修改判后重算（避免展示撒谎） | ✓ `project.ts` |
| `applyFusedDecision` 三分支战：`keep` 仅标记 `reviewed`；`replace` 校验下标 revista bounds、继承该行The candidate实测分（无候选则 0）并置 `transition=forward/masked=false`；`unmatch` 无 `source_text`（R2 时代旧数据）显式拒绝而非静默回退带原文 | ✓ |
| `check_fuse_audit.mjs` 30 断言的纯逻辑自检覆盖这些分支（`node` 实测通过） | ✓ |
| `FuseMeta` serde: `source_text`/`reviewed` 带 default（R2 时代 `.gsa` 兼容，roundtrip/legacy 测试均在）；`FusedEvent.fuse_meta` default 且 skip_serializing_if None | ✓ `project/mod.rs` |
| 删除 `bench_fusion.rs`：其判别器方法论在 `fuse_lab/bench_fusion_*.py` 已有 Python 载体，台账 F13 节注明复跑方式更换并把退役事实写回能力基准目录（不是静默消失） | ✓ |
| `run_ocr_images` / OCR / ASR 路径不受接线破坏：`run_fuse` 沿用 `llm.rs` 的 `LLM_PROGRESS_EVENT` 既有事件，`normalizeFuseError` 错误文案迁移到向量语义 | ✓ |

## 验证（本审查独立执行，全部通过）

- `cargo test`：**252 lib + 15 bench_corpus + 15 bench_hardsub 全绿**（另有 6 个二进制的 `#[ignored]` 单测待跑位处不受影响）
- `cargo test --test fuse_e2e -- --ignored`：**moon 15/15 段全对**（真实子进程、产品路径）
- `runtime/python/python.exe scripts/fuse_calib.py --selfcheck`：**两项 DP 等价全过**
- `runtime/python/python.exe scripts/fuse_worker_parity.py`：**8/8 口径逐段零差异**
- `node scripts/check_fuse_audit.mjs`：**30 断言全过**
- `pnpm run build`（vue-tsc + vite）：通过
- `cargo clippy --lib`：6 条警告与 main 基线相同，**无新增**
- 注：硬门 210/236 / 226/236 评测依据 PR 台账与本地 parity 复算（ poorer per-case分数未单跑——4 案例系统包括其他不是本验收路径）。

## 结论

**建议合并**：架构与工程纪律均在水准之上，验收门全部独立复现。P1-1（embed_model/tokenizer 死配置）
建议随本 PR 或合并后立即处理——它不伤害当前行为，但属于"配置契约不兑现"的陈腐技术债；
P2-1（并发浅险）/P2-2（无超时）是 embed 客户端的两条低成本加固，P2-3（未来日期戳）合并前顺手改。

## 处置记录（2026-10-13，用户裁决：P1-1 按 A 方案；P2 全修；两条观察项修正/同步）

- **P1-1 ✅（A 方案）**：`EmbedRequest` 补 `model`/`onnx`（skip_serializing_if None），`fuse_pipeline`
  经 `split_embed_model_config` 从 `RuntimeConfig::embed_model`（`models/embed/<目录>/<文件>.onnx`）拆出
  两键传入；空串/ bare 文件名回落 worker 内置默认。新增单测 `test_split_embed_model_config`。
- **P2-1 ✅**：临时目录改为 `pid+纳秒` 唯一并**用完即清**（成功/失败/解析失败/超时路径均清）。
- **P2-2 ✅**：`child.wait()` 改 `try_wait()` 轮询 + 120s 兜底 kill（一次成功端到端 ~2s），超时报错含终止说明。
- **P2-3 ✅**：全部 `2026-10-15`/`2026-10-XX` 落为实际执行日 2026-10-13（plan两处 / FUSE 台账三处 /
  OCR 台账 D12 注 / 标定 §8.3 两处 / `fuse/mod.rs` 头注释）；`fuse/mod.rs` 头注释同步 F14 收口现状
  （撤"发布版不要启用向量融合"的过强红线，改为"残留面=审计表展示、人工裁决"）。
- **P2-4 ✅**：`map_worker_meta` 改 Result，候选下标 `<1` 或 `>n` 显式 Err（与命中下标越界防护同款）；
  新增单测 `test_map_worker_meta_rejects_bad_candidates`。
- **P2-5 ✅**：plan T4 块新增"收口快照"（252 lib + 15 + 15，各落地时点数字注明）；PR 描述验收表改为
  `252 lib + 15 + 15`。
- **P2-6 ✅（登记）**：F 台账新增 **F15（语义边界，非缺陷）**——语料融合后再编辑使审计行指向偏移，
  现有兜底（bounds 校验/实时取值/越界空串）安全，处置登记为产品级待办（需语料版本化配套）。
- **顺手**：`fuse_calib.py` / `fuse_worker_parity.py` 头注释补 `PYTHONPATH=runtime/deps_embed` 直接运行说明
  （对拍门是 PR 验收项，复现者需要）。
- **验证（收尾提交实测）**：`cargo test` 254 lib（+2 新单测）+ 15 + 15 全绿；`fuse_e2e --ignored` moon 15/15
  （新 model/onnx 传参走真实产品子进程路径）；`cargo clippy --lib` 6 条与主基线相同，无新增。
- 处置提交号待用户手动提交后回填。
