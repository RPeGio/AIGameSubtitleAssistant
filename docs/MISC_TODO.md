# 杂项功能与修复待办（feat/misc-fixes）

> 记录时间：2026-10-18。分支：`feat/misc-fixes`（自 dev `7b8e306` 拉出，尚未 push）。
> 本文件是**备忘**，不是承诺：顺序可按需要调整；已完成项就地标 ✅ 并写提交号（沿用项目「档案同步」惯例）。
> 每条给出现状依据（`文件:行`）、要做什么、验收要点。**实现每个待办前先给方案、经用户确认再动手**（遵循 AGENTS.md 一轮一任务）。
> 以后新待办继续往下追加，不重排已完成项。

## 顺序总览

| # | 任务 | 类型 | 依赖 | 为什么在这个位置 |
|---|---|---|---|---|
| 1 | ✅ 切页丢状态（含波形重绘） | 修复·底层 | 无 | 已实现（`d52cfeb`）：KeepAlive 保活页面与校对区 + 全局监听改 activate/deactivate |
| 2 | ✅ 播放头跳转后画面/字幕停在上一帧 | 修复 | 无 | 已实现（`dcf9346`）：命中判定留一帧容差 + 跳转/擦洗两种 API 分开 |
| 3 | ✅ 标尺分度随视口拥挤 | 修复 | 无 | 已实现（`851d6a8`）：按最小像素间距自适应，删 30s 封顶阶梯 |
| 4 | ✅ 标尺滚轮缩放 | 功能·小 | 与 3 同区 | 已实现（`851d6a8`）：标尺滚轮缩放（锚点=光标处），与滑条一致 |
| 5 | ✅ 播放键/进度条移出画面 | 修复·布局 | 无 | 已实现（`9042964`、`771c651`）：控件条改流内 + 画面按 16:9 自定高，字幕回到画面底 |
| 6 | ✅ 语料页语料列表增删改（含拖动排序） | 功能 | 无 | 已实现（`82381ee`、`022f2ac`、`dadff2d`、`698d99a`）：行内编辑（Ctrl+Enter/失焦提交）+ 手动新增 + 2×3 点阵拖动排序 |
| 7 | ✅ OCR 参数 info 图标 + 悬浮解释 | 功能·基建 | 无 | 已实现（`a72c385`）：6 参数 + 术语表各一枚 info 点，hover/聚焦弹出三段式解释；第 12 项新参数可自带说明 |
| 8 | ✅ 「分割/合并」倒三角 + 策略小面板 | 功能·核心 | 无 | 已实现（`fa65e66`、`e16da4e`）：两轮均完成——策略面板 + 三档算法、被分割态 + 键盘 ⬅️➡️ 移句界；顺带修掉 KeepAlive 下 activated 钩子不触发导致快捷键全失效的既有缺陷 |
| 9 | ✅ 迷你时间轴（复用第 8 项成果） | 功能 | 8 | 已实现（`1c1efca`）：删提示行、迷你工具条（选择/加选区/分割/吸附）、拖动空白画选区、壳高 200→110px |
| 15 | ✅ clip 拖边（改起止）时显示起止参考线 | 功能·时间轴 | 无（与 8/9 同区） | 已实现（`79ae0af`）：独立 `edgeGuideX` + 标尺下沿→滑条上沿的虚线 |
| 16 | ✅ clip 吸附目标新增「播放头」 | 功能·时间轴 | 无（与 8/9 同区） | 已实现（`79ae0af`）：`collectSnapEdges` 追加播放头；单点候选本就不过滤 |
| 17 | ✅ 去除轨道内 clip 的所有句末标点 | 功能·文本清理 | 8（句界操作会留下句尾逗号） | 已实现（`79ae0af`）：轨道总览「美化」下拉的「去除文本末标点」 |
| 18 | ✅ 语料「全文替换」同步作用于产物轨 | 功能·文本清理 | 6（替换本身）·17（同属语料→产物文本口径） | 已实现（2026-10-25）：仅 fused 轨，一次性精确子串替换，一次撤销回退 |
| 10 | ✅ 快捷键体系（注册表 + 单一监听；帮助面板并入第 19 项） | 功能·跨页 | 8、9 | 已实现（`02416b8`）：`composables/shortcuts.ts` 分发器 + 活动时间轴判定 |
| 11 | ✅ 欢迎页自检升级（探针摊开 + 手动补齐命令 + 一键安装/更新） | 功能·独立 | 7（info 基建）·12（`probe_moss_backend`/`probe_moss_install` 已就绪） | 11a/11b/11c 已实现（2026-10-28）；2026-10-30 真机回归修复：脚本随包 + 子进程静默 + 探测改异步 |
| 12 | ✅ CUDA GPU 加速参数化（设备迁移到 UI） | 功能·Rust+UI | 7（要带 info 说明）+ 11（需 `probe_moss_backend` 命令） | 已实现（阶段 A+B `a8e8010`、阶段 C `2a766c8`；2026-10-30 探测命令改 async）：`probe_moss_backend` + `set_moss_device` 两命令、打轴页设备下拉、欢迎页硬报错、manifest 随包落库 |
| 13 | ✅ 杂项清理与验证（死代码/参数开关/图标；真机波形待验） | 清理·零碎 | 无 | 已实现（`8b71ee2`）：死代码已删、易混字母开关已暴露、轨道图标常显 |
| 14 | ✅ 内嵌 ffmpeg/ffprobe + 欢迎页 ffmpeg 自检卡 | 功能·打包 | 11（自检卡复用其探针机制） | 已实现（`40e645f`/`5acb56f`/`bfd1168`/`f3d19d8`/`cd1b753`）：LGPL 共享构建内嵌、内嵌优先回退 PATH、环境自检第 4 张卡；2026-10-30 `check_ffmpeg` 改 async + 静默启动 |
| 19 | 设置面板（默认 ASR 模型 + 快捷键查看/修改） | 功能·跨页 | 10（注册表） | 改动大、用户明确单独一个 PR；快捷键帮助面板并入此项 |
| 20 | ✅ 校对区选中 clip 时，轨道总览滚动到该行 | 修复·小 | 无 | 已实现（`b2e70ab`）：`data-clip-id` + 矩形比对最小滚动 |
| 21 | ✅ 时间轴框选多选（含 `Ctrl+A` 全选、多选整体位移） | 功能·核心 | 10（活动时间轴判定） | 已实现（`34e2837`）：多选模型 + 框选 + 修饰键 + 批量删除 + 刚性整组位移 |
| 22 | ✅ `,`/`.` 上/下一个片段与「轨道总览 = 聚焦轨道」口径对齐 | 修复·小 | 20（总览滚动） | 已实现（`b28c87a`）：同轨内跳 + 无聚焦轨时兜底跨轨并切轨 + 边界提示 + 迷你时间轴不响应 |

---

## 并行分支接口：引擎后端探测（PR #48，已并入本分支）

**状态**：`feat/engines-gpu-vulkan` 已合入 `feat/misc-fixes`（合并提交 `dcfd661`；PR 提交 `693b5ad` / `5e0d40f` / `77aa553` / `9fc4a58` / `c3594a9`；审查收尾 `8c5af81`；merge-base `7b8e306`）。设计文档：`docs/engines_vulkan_design.md`（本 worktree 只读参考，其 worktree 无 `runtime/`，只经 `GSA_RUNTIME_DIR` 读主 worktree）。

**它交付了什么**
- 新增 `src-tauri/src/ai_runtime/backend_probe.rs`——MOSS/ggml 后端**纯探测与判定**：`parse_info_stderr()`（只认 `[mt I] backend:` 行，`[mt W]` 降级警告不误判）、`parse_vulkan_devices()`（`ggml_vulkan: N = … | uma:` 枚举表）、`probe_machine()`（L0：`nvidia-smi -L`、`System32\vulkan-1.dll` + `vulkaninfo --summary`，按 deviceName+driverInfo 去重）、`probe_assets()`（L1：`.backend` 标记 + `ggml-{cuda,vulkan}.dll`）、`decide()`（判定矩阵）、`recommend_from_assets()`（「该装哪个后端」）。审查后 L1 回退优先级：**`.backend` 标记 > DLL 存在性**，标记与请求不符即拒绝（DLL 可能是切后端未清的残留）。
- `moss.rs`：spawn 时 L2 实证存入 `backend_decision`，经 trait 新默认方法 `effective_backend()` 导出（仅 MOSS 覆写；**未跑过转写时为空** ⇒ 卡片不显示「实测 …」）。
- `AsrEngineStatus` 新增 `device`（配置值）与 `effective_backend`（实测值）：Rust 侧 `ai_runtime/mod.rs` 与前端 `src/types/index.ts:215-217`；`src/views/Welcome.vue` 的 ASR 卡多一行「设备 · <device>（实测 <L2>）」（`deviceText()`/`backendText()`，`""` 显示为「auto（ggml 自动选优）」）。
- `scripts/bootstrap_moss.ps1`：后端断言改 `-cmatch "[mt I] backend: <设备名>"`；vulkan 后端的 `moss_device` 由 `"vulkan"` 改写 `""`(auto)；新增 Vulkan 枚举表回显与 `uma:1`（多为核显）落点警告。`scripts/build_moss.ps1`：vulkan 分支 `glslc` 三级查找 + clone 前 `throw`；工作根改 `<系统盘>\gsa-moss-build\<backend>-<PID>`（躲 MAX_PATH）。

**分工契约（设计文档 §3.5 / §6，两边约定）**
- PR #48 只碰 `scripts/` + 纯探测逻辑 + `docs/`；**`src-tauri/src/ai_runtime/config.rs` 的字段扩展与前端配置/自检面板归本分支（我）**。
- 取值集合：`"" | "auto" | "cuda" | "vulkan" | "cpu"`（`config.rs:307-316` 白名单现为 `空/cpu/cuda/vulkan`）；判定结构：`{ backend, reason, source }`。
- 行为契约（UI 必须照此）：显式 `cuda`/`vulkan` 不可用 = **硬报错**（提示「取消勾选 GPU 加速改用 CPU」或「装对应后端」），**绝不静默降级**——ggml 自己会静默降级且 exit 0，这正是必须 L2 实证的原因；`auto`（空）= CUDA → Vulkan → CPU，**reason 必须在自检面板可见**。

**本分支要补的缺口 / 注意**
- ~~**没有 Tauri 命令暴露 `decide()`/`recommend_from_assets()`**（PR #48 未改 `lib.rs`）⇒ 第 11/12 项要在卡片上显示「为什么落到 CPU」「该装哪个后端」，必须先加命令~~ → **已补（第 12 项阶段 A）**：`probe_moss_backend` 返回 `BackendProbeReport`（含 `reason` / `source` / `recommendation` / `manifest_flavors`），`lib.rs` 已注册；第 11 项可直接复用。
- `config.rs` 已有 `moss_device`（白名单 `空/cpu/cuda/vulkan`）与 `funasr_device`（默认 `cpu`），**尚无 `llm_device`**（LLM 当前不参与融合，见第 12 项）。
- `temp/css-preview/mock.js` 的引擎对象没有 `device`/`effective_backend` ⇒ 卡片走默认值显示「auto（ggml 自动选优）」；要测「实测 CUDA0」需补字段。
- 合并后我的独立复核（2026-10-19）：`pnpm build` ✓ 2895 模块；欢迎页三卡 529×112（比 T8 的 89.8 高 22px，正好多一行），ASR 卡四行含「设备 · auto（ggml 自动选优）」，`asrOverflow 0`；1680×1000 / 1680×720 / 1280×720 三尺寸下 `pageOverflow 0`、页脚底 982 / 702 / 702 ✓ 不裁切。
- 实测数据（写 UI 文案可用）：同一段 5:14.99 音频 → **CPU 680.6s / Vulkan 134.7s / CUDA 76.2s**（Vulkan ≈ CUDA 的 1.77×、比 CPU 快 5.0×、CUDA 快 8.9×）；双 GPU 机器上**裸 `vulkan` 落核显**（`Vulkan0`、`uma:1`），`auto` 才落独显（`Vulkan1`、`uma:0`）⇒ **默认值必须是空/auto，UI 不要引导用户选裸 `vulkan`**。

---

## A 组：打底修复

### 1. ✅ 侧边栏切页丢状态（切走再切回恢复默认；从语料区切回时校对区波形 canvas 重绘耗时）

> **已实现**（2026-10-18，5 文件）：
> - `src/layouts/ProjectLayout.vue`：页面与校对区都套 `<KeepAlive>`；缓存键必须「工程路径 + 路由名」——只用工程路径会让四个页面共用同一键、被当成同一实例复用（切页不换内容）；`<KeepAlive>` 里不能有注释或多个子节点，否则整棵树渲染不出来。
> - `src/components/timeline/Timeline.vue` / `VideoPlayer.vue` / `TrackOverview.vue`：全局 `window` 监听从 `onMounted`/`onUnmounted` 改成 `onActivated`/`onDeactivated` 成对，失活时同时摘监听、停播、取消 rAF。
> - `src/views/FuseView.vue`：融合就绪探针改 `onActivated`，每次回页重探一次。
>
> **验证**（evalq 实测，1680×1000 mock）：切页往返后折叠卡展开态、手动文本草稿、迷你时间轴缩放（片段宽 1251.5px 不变）、校对区 `<video>` `currentTime = 42s`、时间轴平移量（播放头 `left = -162.568` 不变）全部保持；校对区 `<video>` 与波形 canvas **复用同一 DOM 节点** ⇒ 无重绘；隐藏期间按 Delete/空格不再作用于校对区（片段仍存活、视频未播放）；语料页仍整页全宽（主列 1472、校对区不渲染）。`pnpm build` 通过。**提交号：待补。**

**现状依据**
- 全仓 `<KeepAlive>` / `keep-alive` **0 命中** ⇒ 路由视图每次切换都整体卸载重建，页面内 `ref` 状态（如语料页/打轴页的折叠卡 `openCard`、滚动位置、勾选态）全部回到默认。
- 波形重绘：`src/components/timeline/TimelineWaveform.vue` 有模块级缓存 `waveformCache = new Map<string, Float32Array>()`（`:26`），后端 `invoke("get_audio_peaks", { videoPath })`（`:41`）按视频路径只取一次 ⇒ **取数不重复，重复的是组件重建后的整幅重绘**（12 分钟 × 每秒 50 窗 ≈ 3.6 万采样）。

**要做**
- 在「页面级 `<KeepAlive>`」与「把状态提升到 store」之间选一个（或混用）：前者省事但要逐个检查各页对 `onMounted` 的依赖（欢迎页探针、`refreshRecentProjects`、`onMounted` 里的 `openProject` 等），后者更可控但改动面大。
- 波形：组件保活，或把已绘制的位图缓存下来复用。

**验收**
- 语料页展开某张卡 → 切到打轴页 → 切回：折叠态与滚动位置保持。

> **PR49 审查更正与收尾（2026-10-20）**：上面"全局监听改成 `onActivated`/`onDeactivated` 成对"
> 的写法**只对 `<KeepAlive>` 的直接子组件成立**——Vue 3.5.40 源码（`runtime-core` 的
> `sharedContext.activate` 仅调用被缓存 vnode 自身实例的钩子）与本项目实测一致。后果：
> **VideoPlayer / TrackOverview 在 ReviewPane 内是孙组件，钩子收不到** ⇒ ① 校对区播放器空格键
> 永久失效（dev 上可用）；② 失活摘监听/停播/顺手提交全部不执行，切到语料页后正在播的视频
> 继续出声。修复（PR49 审查收尾，`3efb6a5`）：监听改回 `onMounted`/`onUnmounted` 常挂 +
> `isConnected` 当激活闸门（Timeline.vue 原有模式）；"收起时停声/补提交"用路由变化信号
> （校对区仅在非语料页显示）。另：`sentenceCuts.ts` 的 `textSegments` 注释所称"句首标点空片
> 并入下一片"与实现不符（实现只并纯空白片），已按实现改正注释。browser-use 实测见验收记录。
- 带着长视频从其它页切回语料页：无可见重绘停顿。

### 2. ✅ 播放头跳转到 clip start 后画面仍为前一帧／CC 字幕闪一下就没

**已实现（2026-10-18，`dcf9346`）**——两条根因各按方案改掉：

| 改动 | 文件 | 内容 |
|---|---|---|
| 命中判定留一帧容差 | 新建 `src/composables/useFrameTolerance.ts` | `frameDurOf(meta)`（`1/fps`，无元数据按 30fps）+ `hitsFrame(start, end, t, frameDur)` = `start < t + frameDur && end > t` |
| 同上 | `src/components/SubtitleOverlay.vue` | `activeLines` 改用 `hitsFrame`（`frameDurOf(currentVideoMeta)`） |
| 同上 | `src/components/RegionOverlay.vue` | `regionEvent` 改用 `hitsFrame`（source 用 `sourceVideoMeta`，clip 用 `currentVideoMeta`） |
| 同上 | `src/views/CorpusView.vue` | `activeRegion` 改用 `hitsFrame`（`sourceVideoMeta`） |
| 跳转/擦洗 API 分开 | `src/stores/timeline.ts` | 删 `scrubbing()`/`seek()`/`seekDone()`，新增 `beginScrub()`/`endScrub()`（拖动专用）与 `setTime(t)`（瞬时跳转，不置 `isSeeking`） |
| 同上 | `src/components/timeline/Timeline.vue` | 刻度 mousedown → `beginScrub()` + `setTime()`；mouseup → `endScrub()`；`clickSeeks` 分支 → `setTime()`（`:129-130`、`:156`、`:234`） |
| 同上 | `src/views/AsrView.vue` | `clipTime` 写回改 `timeline.setTime(t)`，注释同步（不再需要「避开 seek()」的说明） |

**验证（修复后，同一批脚本）**
- `temp/css-preview/checks/item2f.js`（编辑器页，放大到 pps≈163 后点标尺；目标 start = 3.200）：① 双击总览行 CC 3 行；② 点到 start−20ms（视频回报 3.174）CC 仍 **3 行**（修复前 0）；③ 点到 start（回报 3.199，低 1ms）CC **3 行**（修复前 0）；④ start+20ms CC 3 行。
- `temp/css-preview/checks/item2.js`（打轴页点 300s 的迷你 clip）：`miniAdvanced` 由 **false → true**；采样序列迷你播放头 815.27 → 817.28 与视频 300 → 301.38、校对区播放头 1918.81 → 1923.28 同步推进。
- 已知取舍：容差 = 一帧（30fps 时 33ms），意味着时间落在帧内时字幕最多提前一帧出现；换来的是「跳到起点必定显示」。`RegionOverlay`/`CorpusView` 两处同类改动未单独复现（mock 里最后一个选区的兜底会掩盖），但改法与字幕层完全一致。
- `pnpm build`（`vue-tsc --noEmit && vite build`）通过：2895 模块 / CSS 56.37 kB / JS 638.53 kB。

**根因记录（复现于 2026-10-18，两条独立缺陷）**

**A. 边界精确匹配 + 视频 seek 落点可能低于请求值 ⇒ CC「闪一下就没」**（用户 m02972 观察的主因）
- `src/components/SubtitleOverlay.vue:42` 的命中条件是硬边界 `if (e.start <= t && e.end > t && e.type !== "ocr_region")`，`t` 即 `timeline.currentTime`。
- 而 `timeline.currentTime` 会被视频回报的时间覆盖：`src/components/VideoPlayer.vue:94-102 onTimeUpdate` 与 `:106-121` 的 rAF 推进都调 `timeline.tick(v.currentTime)`。
- `<video>` 的 seek 落点**可能比请求时间早最多一帧**（实测：请求 26.3333 → 回报 26.3，−33.3ms，30fps 一帧 = 33.3ms；`temp/css-preview/checks/item2d.js`）。
- ⇒ `TrackOverview.vue:300` 双击 → `timeline.jumpTo(clip.start)` 写入精确边界（CC 渲染 ⇒ **闪一下**）→ 视频 seek 回报略小于该边界 → `tick` 把 `currentTime` 压到边界之下 → `start <= t` 判定失败 ⇒ **CC 消失**；再次双击把时间写回精确边界 ⇒ **CC 又出现**（与用户观察完全吻合）。
- **复现实测**（`temp/css-preview/checks/item2f.js`：先用滑条滚轮把 pps 放大到 ≈163，再用标尺点时间；目标事件 start = 3.200）：

  | 步骤 | 请求时间 | 视频回报 | CC 行数 |
  |---|---|---|---|
  | ① 双击轨道总览行 | 3.2 | 3.2 | **3** ✓ |
  | ② 点到 start−20ms（0.6 帧） | 3.18 | 3.174 | **0** |
  | ③ 点到 start（精确边界） | 3.2 | **3.199**（低 1ms） | **0** ← 与「双击后 CC 闪一下就没」一致 |
  | ④ 点到 start+20ms | 3.22 | 3.217 | **3** ✓ ← 与「再双击一次 CC 又出现」一致 |

- 同类硬边界匹配还有两处、同一个坑：`src/components/RegionOverlay.vue:39`（`regions.find((e) => e.start <= t && e.end > t)` ⇒ 选区叠加层会显示「上一个选区」）、`src/views/CorpusView.vue:71-73`（`activeRegion` ⇒ 选区尺寸/坐标读数同理）。

**B. `clickSeeks` 路径泄漏 `isSeeking`**（复现时发现，影响播放头跟随）
- `src/components/timeline/Timeline.vue:234`（`clickSeeks` 路径）调 `timeline.seek(found.event.start)`；而 `src/stores/timeline.ts:83-86` 的 `seek()` 内部是 `scrubbing(true)` + 赋值 ⇒ 置 `isSeeking = true`。
- 与之配对的清位只有刻度拖动那条路径：`Timeline.vue:129-130`（`scrubbing(true)` + `seek`）↔ `:153-159`（mouseup 时 `scrubbing(false)`）。**`clickSeeks` 这条路径没有 `seekDone()`**。
- `isSeeking` 永久为 true 的后果：`src/stores/timeline.ts:74-77` 的 `tick()` 直接 `return`（播放头不再前进）；`src/components/VideoPlayer.vue` 的 rAF 推进里有 `if (!timeline.isSeeking)` ⇒ 这条时间轴自己的播放头被永久冻住。
- 复现（`temp/css-preview/checks/item2.js`）：打轴页点迷你时间轴里 start=300s 的选区 clip —— 迷你播放头跳到 815.27px（=300s）、校对区画面确实跟着跳（canvas 亮度 0 → 107.31、`video.currentTime = 300`）；**但随后播放时迷你播放头恒停在 815.27（`miniAdvanced = false`）**，而同一时刻 video 时间 300 → 300.91、校对区播放头 1919 → 1921.75 都在动 ⇒ 泄漏只冻死**这条时间轴自己**的 `tick`。
- 排查中的假象（不是缺陷）：校对区 `.time-text` 是 mm:ss 粒度，跳到 300s 后直到 300.99s 都显示 `5:00` 属正常。

**要做**（方案待用户确认）
- 修 A（**新增，推荐**）：把三处硬边界改成**按当前显示帧容差**命中 —— `src/components/SubtitleOverlay.vue` 内取 `const frameDur = 1 / (currentVideoMeta?.fps || 30)`，条件改 `e.start < t + frameDur && e.end > t`（语义：播放头所在那一帧覆盖 `[t, t + frameDur)`，起始落在这帧里的字幕就该显示）；`src/components/RegionOverlay.vue:39` 与 `src/views/CorpusView.vue:71-73` 同法改。
- 修 B：**把「瞬时跳转」与「拖动擦洗」两种语义彻底分开**，从 API 上杜绝误用：
  - `src/stores/timeline.ts`：新增 `setTime(t)`（直接赋值、**不置** `isSeeking`；需要时内部复用 `jumpTo` 的「最小滚动让播放头可见」）；把 `scrubbing()` 从 store 导出里撤掉，只保留给拖动路径的 `beginScrub()`/`endScrub()`（等价于现在的 `seek()`/`seekDone()` 语义）。
  - `src/components/timeline/Timeline.vue:232-235`：`clickSeeks` 分支改用 `setTime`（或直接 `jumpTo`，点 clip 时滚动让播放头可见正是期望行为）。
  - `src/views/AsrView.vue:157-158`：绕过注释改成普通说明（改用直接赋值仍是正确写法，但「迷你时间轴没有配对的 seekDone()」这句在修完后不再成立）。
  - 备选（最小改动，不推荐）：只在 `Timeline.vue:234` 补一个配对的 `timeline.seekDone()`；缺点是把两种语义继续混在同一个 API 里，第三个调用点还会再踩。
- 验收：双击总览条目／点 clip 跳到 start 时（含时间码不是帧整数倍的情况）CC 与画面都在位，且随后播放继续跟随（`miniAdvanced: true`）；校对区刻度拖动期间仍不被视频 tick 拉回（`isSeeking` 语义不变）。


**验收**
- 点 clip、双击总览条目、以及第 4 项新增的「展开审计条目自动跳转」之后，画面帧与 CC 字幕都跟着跳到位。

### 3. ✅ 视口范围扩大时标尺分度不再增大导致数字拥挤

**已实现（2026-10-19，`851d6a8`）** —— `src/components/timeline/TimelineRuler.vue` 删掉 pps 分档阶梯，改为按像素密度自适应：
- `STEPS = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1200, 1800, 3600, 7200, 14400, 28800, 43200, 86400]`（1 秒 → 1 天），取第一个满足 `interval × pps ≥ MIN_TICK_PX` 的间隔；`MIN_TICK_PX = 56`（10px 等宽最长标签约 46px + 左距 4px）。
- 极端兜底：给到最大候选仍 `interval × pps < MIN_LABEL_PX(44)` 时只画刻度线、不画文字。
- 顺带修两处硬编码：刻度生成窗口 `scrollLeft + 2000` 与 x 过滤 `x > 2500` 都改用 `timeline.viewportWidth`（窗口宽于 2000px 时右侧不再整段缺刻度）。

**验证（`temp/css-preview/checks/item34.js` / `item34b.js` / `item34c.js`）**

| 场景 | 间隔 | 刻度间距 | 标签宽 | 空隙 | 结果 |
|---|---|---|---|---|---|
| 校对区默认缩放（pps 3.27） | 30s | 98.16px | 28.44px | 69.72px | ✓ |
| 校对区缩到最小（pps 0.98，改前会挤） | **60s** | **58.89px** | 34.3px | **24.59px** | ✓（改前 30s ⇒ 间距约 29px < 标签 34.3px ✗） |
| 校对区连续放大 12 档（pps≈10） | 10s | 102.69px | 28.44px | 74.25px | ✓ |
| 语料页迷你时间轴（pps≈3.2） | 30s | 98.66px | 28.44px | 70.22px | ✓ |
| 打轴页迷你时间轴（pps≈1.48） | 60s | 88.64px | 28.44px | 60.2px | ✓ |

- 判定式：`interval × viewportWidth / duration ≥ 56px`；素材超过约 6.5 分钟时最小缩放才会换到 60s 档 ✓（截图 `temp/css-preview/item34-ruler-minzoom.png`：最小缩放下 0:00→12:00 均匀排布、无重叠）。

### 4. ✅ 时间轴标尺上滚轮也能缩放视口

**已实现（2026-10-19，`851d6a8`）** —— `src/components/timeline/Timeline.vue` 的 `onWheelRoot` 最前面新增分支：`t.closest(".ruler")` 时 `preventDefault()` + `timeline.zoom(e.deltaY < 0 ? 1.1 : 0.9, e.clientX - contentRect.left)` 后 return（锚点取标尺内容区左缘，与滑条同一约定）；轨道区滚轮、`Shift+滚轮` 垂直滚动、`.tool-strip`/`.tl-label-col` 忽略等既有行为不变。

**验证（`item34.js` / `item34b.js`）**
- 标尺滚轮放大：pps 0.98 → 1.08（factor 1.1），刻度间距 58.89 → 64.78px ✓。
- **锚点保真**：鼠标停在内容区 60% 处连滚 12 档，光标处对应时间 133.83 → 133.81s（漂移 **−0.01s**）✓。
- 轨道区滚轮：pps 不变、播放头左移（895.67 → 859.17）⇒ 仍是平移 ✓。


### 5. ✅ 校对区播放键与进度条下移到画面外

**已实现（2026-10-19，`9042964`、`771c651`）**

| 文件 | 改动 |
|---|---|
| `src/components/VideoPlayer.vue` | `.controls` 去掉 `position:absolute/left/right/bottom/z-index` 与半透明底、`border-radius`，改流内：`flex:none;height:32px;padding:0 4px;margin-top:6px;color:var(--dim)`；`.player-container` 由 `flex:1` 改 **`flex:0 1 auto` + `aspect-ratio:16/9`**（高度由宽度定，画面尺寸才不会被多出的 32px 压小）；`.ctrl-btn:hover` 改 `var(--hover)`；进度条滑块由 `--v-cap-fg` 改 `var(--accent)` |
| `src/components/ReviewPane.vue` | `.player-area` 去掉 `aspect-ratio`/`max-height`，改 `flex:0 0 62%; min-width:0; min-height:0; align-self:center; display:flex; flex-direction:column`（高度交给内容：画面 + 6px + 控件条） |
| `src/components/SubtitleOverlay.vue` | 删 `CONTROL_BAR_H = 32`，`stackStyle.top` 不再减 32 ⇒ 字幕回到画面真实底边 |

**验证（`temp/css-preview/checks/item5.js` / `item5b.js`，编辑器页与打轴页数字一致）**
- 画面 `.player-container` **577.73×324.97（比例 1.78 = 16:9，与改前逐项相同）**；`.controls` 位于 y478（画面底 472 之下 6px）、`position: static`、`controlsBelowPicture: true` / `controlsInsidePicture: false`。
- 字幕：3 行，`stackBottom 472 == pictureBottom 472`（贴画面底，不再悬空 32px）。
- 画面底部 6px 处 `elementFromPoint` = `video.video-el`、`hitIsControls: false` ⇒ 该区域重新可交互；打轴页最低的选区手柄 y365.98 命中 `div.handle`（`lowestHandleReachable: true`）。
- 布局无回归：`.rvbot`（时间轴卡片）仍 **402.95px**、`.rvtop` 441.05、`pageOverflow: false`。
- 主题：`.controls` 背景透明、文字取 `--dim`（深 `rgb(156,160,166)` / 浅 `rgb(92,97,105)`）、进度条填充取主题强调色（`rgb(94,106,210)` / `rgb(79,91,196)`）。
- `pnpm build` 通过（6.19s）。截图 `temp/css-preview/item5-editor-dark.png`、`item5-editor-light.png`。

**原始现状依据（改前）**
- `src/components/VideoPlayer.vue` 的 `.controls` 是 `position: absolute; bottom: 0; height: 32px`，压在 `.player-container` 的画面上（T6 改动）。
- 因为控件条压在画面上，`src/components/SubtitleOverlay.vue` 里有 `const CONTROL_BAR_H = 32` 把字幕上抬 32px 让位。

**补充修复（用户 2026-10-19 验收时发现，同属本项）**
- 症状：应用高度压到最小 720px、宽度逐渐拉大时，校对区**画面放大溢出**，盖住下方控件条与上方信息栏。
  根因：首版把画面高度交给「宽度 × 16:9」，宽度变大时高度超出可用空间且无法收缩。
  改法：`VideoPlayer.vue` 的 `.player-wrapper` 改网格 `grid-template-rows:minmax(0,1fr) 32px; row-gap:6px`，`.player-container` 改「两轴 auto + `max-width:100%` + `max-height:100%` + `aspect-ratio:16/9`」——由 CSS 在「可用高度 × 可用宽度」内取最大的 16:9 框；`ReviewPane.vue` 的 `.player-area` 改 `align-self:stretch`（拿到确定高度）。
- 症状：未聚焦轨道时右侧「轨道总览」面板不填满容器全宽（右侧留空）。
  根因：`.overview-panel` 只有 `height:100%`，在 `.overview-area{display:flex}` 里宽度按内容收缩。
  改法：`TrackOverview.vue` 的 `.overview-panel` 加 `flex:1; min-width:0`。
- 验证（`temp/css-preview/checks/item5c.js`；1680×1000 / 1900×720 / 1600×720 / 1280×720 / 2400×720 / 2100×760 六种尺寸）：画面比例恒为 **1.78**，`pictureAboveControls`、`pictureInsideRvtop`、`controlsInsideRvtop`、`pictureBelowInfoBar`、`panelFillsAreaWidth` 全 true，`pageOverflow` false；1680×1000 下画面仍 577.73×324.97（与首版一致）。截图 `temp/css-preview/item5c-asr-1900x720.png`、`item5c-editor-1280x720.png`。

---

## B 组：页面级功能

### 6. ✅ 语料页语料列表增删改（含拖动排序）

**已实现（2026-10-19，`82381ee`、`022f2ac`、`dadff2d`、`698d99a`）**

| 改动 | 文件 | 内容 |
|---|---|---|
| 更新 / 排序两个 store 动作 | `src/stores/project.ts` | `updateCorpusItem(id, text)`（trim 后为空或与原文相同 ⇒ 不入快照直接返回，否则 `recordSnapshot()` 后改写）与 `moveCorpusItem(id, toIndex)`（clamp 到 `[0, len-1]`，`recordSnapshot()` 后 splice 取出再插入——**语料顺序参与融合 DP 的取舍，所以顺序是数据**） |
| 行内编辑 + 手动新增 + 拖动排序 | `src/views/CorpusView.vue` | `editingId/editText/setEditInput/startEdit/commitEdit/cancelEdit`；`drafting/draftText/startDraft/commitDraft/cancelDraft`（提交非空才 `addCorpusItem`）；**排序用指针事件手写**（`draggingId/dropAt/rowGeom/startReorder/onReorderMove/onReorderUp/endReorder/resolveInsert/dropClass`，`onUnmounted` 清理 window 监听）——只有点阵手柄按下才进入排序态，文本选择不受影响 |
| 模板 | 同上 | `.rh` 加「添加」按钮；草稿行与行内编辑共用一个 `<input class="inp ct-edit">`（`@keydown.ctrl.enter`/`@keydown.meta.enter` 提交、`@keydown.esc` 取消、`@blur` 提交）；行 `:class="[dropClass(ci,item.id),{dragging:…}]"`；左侧 `.grip` 内联 SVG（2 列 × 3 行共 6 个 `<circle r="1.4">`，`@mousedown="startReorder($event,ci)"`）；文本旁铅笔按钮 |
| 图标 | `src/components/AppIconSprite.vue` | 新增 `ic-pencil` |
| 样式 | `src/views/CorpusView.vue` | `.crow .grip`（14px、`cursor:grab`/`:active grabbing`）、`.grip-ic{fill:currentColor}`、`.crow .ct-edit`、`.crow.draft`、`.crow.dragging{opacity:.45}`、`.crow.drop-before{box-shadow:inset 0 2px 0 var(--accent)}`、`.crow.drop-after{box-shadow:inset 0 -2px 0 var(--accent)}`（inset 阴影画线，不引起布局跳动）、`.clist.reordering .crow/.grip{cursor:grabbing}` |

**返工记录：原实现用 HTML5 拖放，用户验收不通过（2026-10-19）**
- 用户原话：「手柄长按拖动后cursor有拖动显示，但实际拖动后语料条没有任何变动」。
- 复现工具 `temp/css-preview/drag.mjs`（CDP `Input.dispatchMouseEvent` 派发**可信**鼠标事件，模拟真实拖动；旧的自查脚本用合成 `dragstart/dragover/drop` 手工派发，**绕过了真实握手 ⇒ 假通过**）。实测事件链：`mousedown` ✓ → `dragstart` ✓（源行确实拿到 `.dragging`）→ 之后只有 `dragenter`，**`dragover` 从不下发给目标行**（记录的 `dropTarget` 恒 0）→ **`drop` 永不触发** ⇒ 拖完没有任何变化。
- 修法：改用指针事件（mousedown/mousemove/mouseup）自己算插入位置——顺带能精确控制插入点（上半区插到该行前、下半区插到该行后）与指示线，并且**可用真实鼠标事件验证**。

**验证（`temp/css-preview/checks/item6.js` / `item6b.js` / `item6c.js` / `item6d.js` + `temp/css-preview/drag.mjs`，1680×1000）**
- 改：双击第 3 条 → 输入框预填原文 ✓；**普通 Enter 不提交**（`stillEditing: true`）✓；**Ctrl+Enter 提交**（`stillEditing: false`、文本更新、条数不变 14）✓；**失焦提交** ✓（另一行验证：`blur` 后文本已提交、退出编辑态）。
- 撤销：全局 `Ctrl+Z`（`ProjectLayout.vue:77` 用 `e.code === "KeyZ"` 判定，合成事件必须带 `code`）回退成功 ✓；顶栏撤销按钮同样回退成功 ✓（无历史时按钮 `disabled`）。
- 增：点「添加」→ 草稿行出现 → `Ctrl+Enter` 提交后条数 15、末条文本正确、来源胶囊显示「手动」✓；**空文本（含纯空格）不入库**（条数不变、草稿行关闭）✓。
- 排序（改用指针实现后的复验）：
  - **可信鼠标事件**（`drag.mjs`，真实拖动第 1 条到第 4 条位置）：顺序由 `[1,2,3,4,5…]` 变为 `[2,3,4,1,5…]` ✓，拖动中源行带 `dragging`、目标行出现过 `drop-before` ✓。
  - `item6d.js`：原地按下抬起 → 顺序不变且**撤销按钮仍 disabled**（不产生假快照）✓；第 6 条拖到第 2 条**上半区** → 指示线 `drop-before`、顺序 `[1,6,2,3,4,5,…]` ✓；`Ctrl+Z` 恢复原顺序 ✓；第 1 条拖到第 3 条**下半区** → 指示线 `drop-after`、顺序 `[2,3,1,4,…]` ✓；两次移动后撤销按钮可用 ✓。
- 手柄几何：14×14 方框、6 个圆点、`cursor: grab` ✓；行高 33px ✓；每行一个铅笔按钮（14 个）✓；`pageOverflow: false` ✓。
- 截图：`temp/css-preview/item6-corpus.png`（整页）、`item6-corpus-zoom.png`（列表区 2× 放大）、`item6-drag.png` 与 `item6-drag-zoom.png`（**拖动中间态**：被拖行变淡 + 目标行下方强调色插入线）。
- `pnpm build`（`vue-tsc --noEmit && vite build`）通过：2895 模块 / CSS 57.08 kB / JS 643.00 kB。

**返工记录②：多行语料被压成一行、编辑态丢换行（用户报告，2026-10-19）**
- 用户原话：「语料条目进入draft状态后input类型应像轨道总览那边一样变textarea而不是单行text，而且注意到语料条目会将换行trim掉并合并成一行，甚至在变为draft后还会将空格和换行全部删掉，严重影响语料人工校对」。
- 三个独立原因：
  1. **`<input>` 的取值算法会删掉换行**——`<input type="text">` 的 value 按 HTML 规范剥离 CR/LF，多行文本塞进去再取出就已经变成一行 ⇒ 编辑态一打开就丢分段（与用户描述完全一致）。轨道总览那边用的是 `<textarea>`，正是这个原因。
  2. **store 侧 trim**：`addCorpusItem`（`src/stores/project.ts:827`）存 `text.trim()`、`updateCorpusItem`（`:846`）存 `text.trim()` ⇒ 首尾空格/换行被吃掉。OCR 原始文本与人工粘贴的换行都是校对要看的，不该吞。
  3. 显示侧 `.ct` 用了 `white-space: nowrap`，把多行在视觉上压成一行（数据还在，但看不出分段）。
- 修法：
  - `src/views/CorpusView.vue`：草稿行与行内编辑都换成 `<textarea class="inp ct-edit" :rows="editRows(text)">`（`editRows` = `min(10, max(2, 行数))`），提交/取消绑定不变；`setEditInput` 的 el 类型改 `HTMLTextAreaElement`；`.ct` 改 `white-space: pre-wrap; word-break: break-word`（原样换行显示，行高随内容增减）；`.ct-edit` 补 `min-height/padding/line-height/resize: vertical` 等 textarea 专属样式；`.ct` 的插值收紧成单行（否则 `pre-wrap` 会把模板里的缩进当内容显示）。
  - `src/stores/project.ts`：`addCorpusItem`/`updateCorpusItem` 改为**原样保存**（只拦「全空白」），不再 trim。
- 验证（`temp/css-preview/checks/item6e.js`，语料页，1680×1000）：
  - 草稿行是 textarea ✓（`draftIsTextarea: true`）；行数跟随内容：一行 `rows=2`、两行 `2`、三行 `3` ✓。
  - 写入 `"第一行：こんにちは\n第二行：元気ですか  "`（含换行与结尾两个空格）提交：条数 14 → 15 ✓；`lastText` 与原样一致（**换行与尾部空格都在**）、`lastTextMatchesRaw: true` ✓。
  - 再进入编辑态：是 textarea ✓，`editValue` 与写入的原文**逐字符相同** ✓（改前会被 `<input>` 吃掉换行）。
  - 改成三行再提交：条数仍 15（不新增行）✓、文本原样 ✓；该行高度 **64px vs 单行 33px** ⇒ 确实换行显示了 ✓；`pageOverflow: false`、撤销可用 ✓。
  - 截图：`temp/css-preview/item6e-multiline.png`（提交后的两行条目）、`item6e-edit.png`（三行编辑态 textarea：`rows=3`、值里含 2 个换行）。

**增补：搜索 / 替换（2026-10-19，用户验收后追加）**
- 需求（用户原话）：「再在语料列表上方加一个简单的搜索/替换功能，搜索在语料中精确匹配query，替换则直接全部替换语料中匹配到的query」；随后补充：「高亮/过滤时建议直接将整个列表除高亮的位置以外加一层灰度遮罩并禁用双击编辑和拖动排序功能」。
- 语义（已定）：精确子串、**区分大小写**、不做正则；**不过滤**（列表照常全显示），匹配处高亮、其余压暗；搜索态禁用双击编辑与拖动排序；替换 = 整个语料库所有条目的该子串全部替换（替换框留空即删除），**只记一个撤销步骤**。
- 落点：
  - `src/stores/project.ts`：新增并导出 `replaceAllCorpus(query, replacement)`（`text.split(query).join(replacement)`，有命中才 `recordSnapshot()`；返回 `{ hits, items }`）。
  - `src/views/CorpusView.vue`：`query/replacement` refs、`queryActive/matchCount` computed、`isMatch()`（行级压暗）、`segmentsOf()`（切成匹配/非匹配片段逐段渲染，**不用 `v-html`**）、`onReplaceAll()`（`message` 回执）；`.csearch` 搜索行（两个 `.inp` + 「全部替换」+ 「匹配 N 条」计数）；`.crow.dim` / `.ct .hit` / `.ct .faded` 样式；`startEdit`/`startReorder` 加搜索态守卫；搜索态隐藏点阵手柄、禁用铅笔按钮；`startDraft()` 清空 query（新增后要能立刻看到并编辑）。
- 验证（`temp/css-preview/checks/item6f.js` + `item6f2.js`，语料页 1680×1000）：
  - 14 条里 7 条含「派蒙」：`dimRows = 7`（非匹配行压暗）、`.hit = 7`（高亮处）、`.faded = 21`（匹配行内的非匹配片段）、计数「匹配 7 条」✓。
  - **`textUnchangedBySegments: true`**——分段渲染后 `textContent` 与原文逐字符相同（证明 `pre-wrap` 下模板缩进没混进文本）✓。
  - 搜索态：双击不进入编辑（`editOpensWhileSearching: false`）、拖动手柄 0 个 ✓。
  - 替换：回执「已替换 7 处（7 条）」✓、7 条都出现 `[派蒙]` ✓；**一次 `Ctrl+Z` 全部还原**（`undoRestoredAll: true`）✓。
  - 清空搜索：压暗 0 / 高亮 0 / 手柄回到 14 / 双击可编辑 ✓；`pageOverflow: false` ✓。
  - 视觉值：`.hit` 底色 `rgba(94,106,210,.16)`、文字 `rgb(169,177,240)`、圆角 2px；`.crow.dim` opacity .38；`.ct .faded` opacity .45；搜索行 525×26，落在 `.r2`（546）内 ✓。
  - 截图：`temp/css-preview/item6f-search.png`（深色）、`item6f-search-light.png`（浅色）。

**原始依据（实现前记录）**

- store 只有新增与删除：`addCorpusItem(text, source = "paste")`（`src/stores/project.ts:827`）、`removeCorpusItem(id)`（`:839`），**没有更新函数**（`pushCorpusTexts`（`:797`）是批量追加、`writeOcrToCorpus`（`:822`）走 OCR 产物）。
- 列表行在 `src/views/CorpusView.vue` 的 `.clist > .crow`，目前只有序号、文本、来源胶囊、删除按钮。

**要做**
- store 新增 `updateCorpusItem(id, text)`（含 `recordSnapshot()`，与其它编辑一致，保证 Ctrl+Z 可回退）。
- 行内编辑：双击或点编辑按钮进入输入态，Enter 提交 / Esc 取消 / 失焦提交（可与 `TrackOverview.vue` 的既有编辑交互保持一致）。
- 手动新增一行（走 `addCorpusItem`，来源 `paste`）。

**验收**
- 改字后落盘（`save_project`）、撤销可回退、来源胶囊与序号不错位、编辑态不触发行点击跳转。

### 7. ✅ OCR 各参数加 info 小图标 + 悬浮详细解释

**已实现（2026-10-19，`a72c385`）**
- 新增 `src/components/InfoDot.vue`（15×15 圆点，内嵌 `AppIcon name="info"`）与 `src/composables/ocrParamDocs.ts`（`OCR_PARAM_DOCS` 六条 + `GLOSSARY_DOC`，每条按「作用 / 调大调小的代价 / 默认值与关闭值」三段、`\n` 分隔，正文上方用 `///` 记证据行号：`params.rs` 字段注释与默认值常量、`pipeline.rs:241-245/306/334/343/350/450`、`merge.rs:129-151/295/364/767-835/906-916`、`correct.rs:232`、`scripts/ocr_worker.py:136-158`、`src/stores/project.ts:710`）。
- `src/components/ParamStepper.vue`：新增 `paramKey?: string` 与 `doc` computed；`.pl` 改为 `<span class="pl-t">标签</span><InfoDot v-if="doc" :text="doc" :title="label" />`。`src/style.css:295` 的 `.pcell .pl` 改 `display:flex`（`min-width:0`），新增 `.pcell .pl-t{min-width:0;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}`——省略只落在文本上，图标恒可见。
- 两个页面各 6 处传 `param-key`：`src/views/CorpusView.vue`（约 `:494-536`）与 `src/views/AsrView.vue`（约 `:260-300`）；语料页「术语表 · N 条」也挂一枚（用 `GLOSSARY_DOC`）。`src/components/AppIconSprite.vue` 新增 `ic-info`（第 38 个 symbol）。
- 用 naive `NTooltip`：popper 挂到 body（**不被 `.accb{overflow-y:auto}` 裁切**）且随 `n-config-provider` 翻转主题；`trigger` 是单值（`'click'|'hover'|'focus'|'manual'`），故取 `manual` + 自接 `mouseenter/mouseleave` 与 `focus/blur`，鼠标与键盘都能看；`whiteSpace:"pre-line"` 让 `\n` 生效；`maxWidth:320px`、`background:var(--panel)`、`border:1px solid var(--hair-2)`。
- 验证（新工具 `temp/css-preview/hover.mjs`：CDP **可信**鼠标悬停 → dump `.n-tooltip` 与 `.pcell`；`temp/css-preview/checks/item7b.js`）：语料页 `dotCount 7`（6 参数 + 术语表）、ASR 页 6；hover 与 focus 两条路径都弹出（`activeIsDot:true`）；深色 `bg rgb(13,14,16)` / `color rgb(232,233,234)` / `border rgba(255,255,255,.13)`，浅色 `rgb(255,255,255)` / `rgb(23,25,28)` / `rgba(0,0,0,.16)`；父节点 `v-binder-follower-content`（body）⇒ 未裁切；移开鼠标后 `visibleTips 0`；**行高 A/B 相同**（6 个 `.pcell` 隐藏图标前后均 51px、`.agrid` 109px）；长标签压力测试 → 文本截断且图标仍在格内（`gapToCellRight 0`）；`pageOverflow 0`；`pnpm build` ✓ 2899 模块 / CSS 58.13 kB / JS 650.37 kB。
- 截图：`temp/css-preview/item7-hover.png`（语料页深色，批大小文案）、`item7-asr-light.png`（ASR 页浅色，变化检测阈值文案）。
- 有意不做（归第 13 项）：标点组（`punctuation`）的说明、以及「六参数抽共享 `OCR_PARAM_FIELDS` 配置」的重构。

**现状依据**
- 参数行结构：`src/components/ParamStepper.vue` 的 `.pcell > .pl + .step`（标签 + 减号/数值/加号）。
- 参数集合：`src/composables/ocrDefaults.ts` 的 6 个数值（`frame_interval` / `dhash_threshold` / `batch_size` / `merge_similarity` / `min_subtitle_sec` / `min_text_length`）+ `punctuation` + `glossary`。

**要做**
- `ParamStepper` 增加可选 `info` 文案 → `.pl` 之后一个 info 小图标，hover / 聚焦显示解释（自制 popover 或朴素 tooltip，注意别被 `.accb` 的 `overflow-y: auto` 裁切）。
- 为每个参数写**面向用户**的解释（含义、单位、推荐区间、调大/调小的后果），不要贴代码注释。

**验收**
- 全部参数（含标点/术语表）都有解释；窄列（语料页左列、3 列网格）不换行、不撑高；悬浮层不被卡片裁切。

---

## C 组：时间轴功能扩展（迷你时间轴复用的前提）

### 8. 「分割」「合并」按钮加倒三角 + 策略小面板

**✅ 两轮均已实现（2026-10-20，`fa65e66`、`e16da4e`）**——第一轮＝策略面板 + 三档算法；第二轮＝被分割态 + 键盘 ⬅️➡️ 整段搬句界。

| 改动 | 文件 | 内容 |
|---|---|---|
| 句界切分算法（新） | 新建 `src/composables/sentenceCuts.ts` | 类型 `SplitStrategy`/`MergeStrategy`；`cutIndices(text, mode)`（连续断点合并、**丢弃句首/句尾断点**）；`nearestCut(cuts, len, ratio)`（等速率扩散的等价判定，并列取左）；`splitTextAtCut`；`stripTrailingPunctuation`（第 17 项备好）；`SPLIT_OPTIONS`/`MERGE_OPTIONS` |
| 策略状态 | `src/stores/timeline.ts` | `splitStrategy`（默认 `punct`）/`mergeStrategy`（默认 `latter`）+ `setSplitStrategy`/`setMergeStrategy`，持久化键 `gsa.timeline.splitStrategy` / `gsa.timeline.mergeStrategy`（非法值回落默认） |
| 分割分档 | `src/stores/project.ts` | `splitEvent(eventId, time, strategy = "raw")` → 返回 `{ rightId, degraded } | null`；`punct`/`space` 走「时间比例最近的断点」，无断点 ⇒ `degraded: true` 且两段保留原文；`ocr_region` 分支不收窄（矩形随 `{...event}` 继承） |
| 合并分档 | `src/stores/project.ts` | `mergeTwo(idA, idB, requireAdjacent = true, strategy = "stack")`；`latter` 取后者文本、`former` 保留前者、`stack` 现状 `first + "\n" + second`；`mergeAdjacent(id, strategy)` 透传 |
| 面板 UI | `src/components/timeline/TimelineToolStrip.vue` | 分割/合并按钮 26×24 + 右下角 3px 倒三角（`.has-menu::after`，`currentColor`）；**已选中再点一次**才展开 naive `NPopover trigger="manual"` + `.strategy-pop` 选项列表（选中项带勾 + `--accent-soft` 底）；点面板外/切工具自动收起 |
| 调用点 | `src/components/timeline/Timeline.vue`、`src/components/timeline/TimelineClip.vue`、`src/views/Editor.vue` | 分割工具 / S 键 / 行内分割传 `timeline.splitStrategy`，合并工具 / M 键 / 拖过边界合并 / 行内合并传 `timeline.mergeStrategy`；`degraded` 时 `message.warning("该 clip 里没有可用断点，已按原文分割")` |

**第一轮验证（`temp/css-preview/checks/item8.js` / `item8b.js`，1680×1000，mock）**
- 纯函数（对用户给的 47 字样例）：`len 47`、`punctCuts [9,13,23,38]`（47 的句尾标点按规则丢弃）、`spaceCuts [6,12]`、句首句尾标点样例 `[6]`、`stripTrailing("abc，def。  ") = "abc，def"`、`nearestCut` 比例 0.1/0.35/0.6/0.95/并列 → `[9,13,23,38,9]`、`splitAt23` 得 `叙旧…汇报吧。` ｜ `你提前…结论么？`（与图解逐字一致）。
- 合并三档：`latter` = 后者文本、`former` = 前者文本、`stack` = `前者\n后者` ✓（每次都 `undo()` 复原）。
- 分割三档：`raw` 两段都保留原文 ✓；`punct` → 左 `【派蒙】这里的风景，` ｜ 右 `和你家乡的一样吗？`（去空白后字符与原文本一致 ✓）；`space` 与 `abc` 两种情形都 `degraded: true` 且保留原文 ✓。
- 策略持久化：store 与 localStorage 同步 ✓。
- 面板：5 个工具按钮、分割钮 26×24、倒三角 3px、**首次点击只切工具不弹面板**、第二次点击弹「分割策略」三档、选「按空格」写策略并收起（离场动画约 300–500ms）、合并面板只显示合并三档、点面板外收起 ✓；`pageOverflow false`。
- `pnpm build`（`vue-tsc --noEmit && vite build`）通过：2900 模块 / CSS 59.05 kB / JS 654.52 kB。
- 截图 `temp/css-preview/item8-panel.png`（分割策略面板打开态）。

**第二轮（被分割态 + 键盘 ⬅️➡️ 整段搬句界）——已实现**

| 改动 | 文件 | 内容 |
|---|---|---|
| 段落切分（新） | `src/composables/sentenceCuts.ts` | `textSegments(text)`：按「标点串末尾」切段，**每段原样保留自己的标点**；空片并入邻片；整体无标点 ⇒ `[text]`（供整段搬家，逐字不 trim） |
| 搬运 API | `src/stores/project.ts` | `adjustSplitBoundary(leftId, rightId, "left" \| "right")` → `{ ok, reason: "none" \| "no-pair" \| "no-boundary" }`；要求同轨且两段相邻（`|left.end − right.start| ≤ 0.001`）；⬅️ 把右段首个句界段**整段**追加到左段末尾、➡️ 把左段末段整段前置到右段开头；逐字不 trim ⇒ 可逆；每次成功一次快照 |
| 被分割态 | `src/stores/timeline.ts` | `splitPair = ref<{ trackId; leftId; rightId } \| null>` + `setSplitPair` / `clearSplitPair` |
| 状态维护 | `src/components/timeline/Timeline.vue` | 分割工具与 S 键成功后设对；`ArrowLeft`/`ArrowRight` → `adjustSplitBoundary`（`no-boundary` ⇒ toast「该侧没有可搬的句界段」，`no-pair` ⇒ 清态）；`Escape` / `Delete` / `M` / 点其它 clip / 换工具 / 失焦 均清态 |
| 视觉 | `src/components/timeline/TimelineClip.vue` | 配对 clip 加 `.split-pair` 内描边（`inset 0 0 0 1px var(--accent-ink)`），聚焦时叠外框 |
| 倒三角朝向 | `src/components/timeline/TimelineToolStrip.vue` | `.has-menu::after` 加 `transform: rotate(90deg)`（斜面朝左上，用户指定） |
| 行内分割 | `src/views/Editor.vue` | 行内「分割」成功后同样设对（与校对区一致）；行内「合并」成功后清态 |

**第二轮验证（`temp/css-preview/checks/item8c.js`，1680×1000，mock）**
- mock 把首个 fused 文本换成与用户图解同构的长句（`【派蒙】叙旧可以之后再叙，旅行者，先继续刚才的汇报吧。你提前进行能源研究的最终实验，有得出什么结论么？`），使右段在分割后仍留 ≥2 个句界段。
- 分割（punct）→ 左 `…先继续刚才的汇报吧。` ｜ 右 `你提前进行能源研究的最终实验，有得出什么结论么？`；`.split-pair` 标记 2 个 ✓。
- ⬅️ → 左 `…汇报吧。你提前进行能源研究的最终实验，` ｜ 右 `有得出什么结论么？`（**整段带标点**搬家，与 m06145 示例一致）✓。
- ➡️ → 逐字复原 ✓（`restored: true`）。
- 连续 ⬅️ 撞边界：文本不再变化 + toast「该侧没有可搬的句界段」出现 11 次 ✓。
- 清态四条路径：`Escape` → 0、换工具 → 0、点其它 clip → 0、失焦（KeepAlive 暂存）→ 0 ✓。
- `pageOverflow false`、无 console 错误 ✓；`pnpm build` ✓。
- 截图 `temp/css-preview/item8-splitpair.png`（分割后：左列两行 + 总览两条 + 右段聚焦）。

**顺手修掉的既有缺陷（重要）**
- 现象：时间轴的 `Delete`/`S`/`M` 快捷键与「点别处提交轨道重命名」在校对区**完全无反应**。
- 根因：`onActivated`/`onDeactivated` **只对 KeepAlive 的直接子组件触发**（校对区里 `Timeline` 是 `ReviewPane` 的子组件 ⇒ 钩子从不触发），而 T1（切页保活）把 window 监听移进了 `activate()` ⇒ 监听从未注册（实测：挂载计数 2、激活计数 0）。
- 修法：监听回到 `onMounted` 注册 / `onUnmounted` 注销；新增 `isAttached()`（`rootRef.isConnected`）作活动闸门——被 KeepAlive 暂存的页面/校对区 DOM 已脱离文档，故不会抢键盘。

**现状依据**
- 工具按钮：`src/components/timeline/TimelineToolStrip.vue` 的 5 个 `.tool-btn`（24×22，点击即 `setTool(...)`）。
- 分割语义：`src/stores/project.ts:528` 的 `splitEvent(eventId, time)`——`{ ...event }` 继承全部字段（含 `x1/y1/x2/y2`）、`EPS = 0.05` 防 sliver、`recordSnapshot()`。
- 合并语义：`src/stores/project.ts:1303` 的 `mergeTwo(idA, idB, requireAdjacent)`——**文本拼接**（`first.text + "\n" + second.text`）并删除后者，明确拒绝 `ocr_region`（`:1310`）；`mergeAdjacent`（`:1328`）同理。

**要做**
- 按钮**稍稍放大** + 右下角倒三角；**已选中状态下再次点击**才展开小面板（首次点击仍是切工具）。
- 分割面板三档：`按断句标点分割`（默认）、`按空格分割`、`保留原文分割`（现状）。
- 合并面板三档：`保留后者文本`（默认）、`保留前者文本`、`前后文本堆叠`（现状）。
- 策略存 `timeline` store（或按项目惯例持久化到 `gsa.*`），`splitEvent` / `mergeTwo` 接受策略参数。

**用户已定（2026-10-19，m06123 附图解）**
- **合并三档**（语义简单，不涉及新算法）：`保留后者文本`＝合并后取后者文本、前者文字丢弃；`保留前者文本`＝反之；`前后文本堆叠`＝**现状**（`first.text + "\n" + second.text`）。`mergeTwo` 的快照与「拒绝 `ocr_region`」逻辑保持不变，只在文本取舍分支上分档。
- **分割 · 按断句标点**（用户给的「扩散算法」）：
  1. 取 clip 文本中标点的字符序号，比例 `p_k = 序号 / 文本总长`；**句头与句尾的标点不参与**（不当作断点）；
  2. 映射到时间 `t_k = start + p_k × (end − start)`——每个标点是一个「扩散中心」；
  3. 每个中心以**相同速率向左右对称扩散**；两个相邻扩散相遇处（速率相同 ⇒ 中点）双方停止，各自另一边继续，扩到 clip 两端也停；
  4. 全部停止后时间轴被切成若干连续区间（1D Voronoi 单元），每段归属一个标点；
  5. 在 T 处分割时：**时间在 T 处切断**（与现状一致），**文本在「T 所属标点」之后切断**（标点留在左段）。示例：`叙旧可以之后再叙，米提亚，先继续刚才的汇报吧。` ｜ `你提前进行能源研究的最终实验，有得出什么结论么？`
- **分割 · 按空格**：与上完全同构，断点集合换成「句内空格」（句首/句尾空格同样不参与）。
- **分割 · 保留原文**：**现状**（按比例在字符处硬切，不认标点）。
- **分割态（新交互）**：分割出的两个 clip 进入「被分割态」，保持到失焦、再次分割或 Esc 为止。此状态下按**键盘 ⬅️ / ➡️**（不是鼠标）：
  - **⬅️（右→左）**：把右侧 clip 的**第一段**（以标点结尾、**保留其标点**）整体接到左侧 clip 句末，左侧原有的句末标点保留：
    `123，4567，` ｜ `234，5678。` → `123，4567，234，` ｜ `5678。`
  - **➡️（左→右）**：把左侧 clip 的**最后一段**（同样保留其句末标点）整体接到右侧 clip 句首，是 ⬅️ 的**严格逆操作**：
    `123，4567，234，` ｜ `5678。` → `123，4567，` ｜ `234，5678。`（逐字复原）
  - 只改文本（`start`/`end` 不动）；每次按键记一次快照（可 Ctrl+Z）。
  - 某一侧没有可切的段（例如右侧整段无标点）⇒ 不动作 + toast 提示。
  - **已知副作用**：⬅️ 后左侧可能以逗号收尾（上例 `123，4567，234，`）⇒ 需要「去除句末标点」的清理功能，已记入**第 17 项**。

**用户已定（2026-10-19，m06138 收尾）**
- 分割态的「左键/右键」是**键盘 ⬅️ / ➡️**（不是鼠标），**只在被分割态下生效**，退出该态即无动作；⬅️＝把右侧 clip 的第一段（含其标点）移到左侧句末，➡️＝反之。因此**不覆盖**现有的「点 clip 跳播放头」。
- 移动句子**只改文本**：两个 clip 的 `start`/`end` 不变（时间边界是用户点出来的，不该被句界改写）。
- 「按断句标点 / 按空格」**找不到可用断点**（全句无标点/空格，或只有句首句尾）⇒ **退化为保留原文分割 + 弹 toast 提示**。
- 该功能主要面向 **ASR 产出的主播语音**（ASR 断句不准，人工微调句界）。
- 仍按现状：`mergeTwo` 的快照与「拒绝 `ocr_region`」不变；被丢弃一侧的 `fuse_meta` 字段保留合并后实体上的原值；策略持久化沿用项目惯例（`gsa.*`）。

**验收**
- 两工具 × 三档策略各自生效；**默认档与现状逐字一致**（回归）；每次操作都能 Ctrl+Z 回退。

### 9. ✅ 迷你时间轴（复用第 8 项成果）

**✅ 已实现（2026-10-20，`1c1efca`）**

| 改动 | 文件 | 内容 |
|---|---|---|
| 工具类型加法 | `src/stores/timeline.ts` | `activeTool` 联合类型与 `setTool` 加 `"region"` |
| 迷你工具条 | `src/components/timeline/TimelineToolStrip.vue` | 新增 `variant?: "full" \| "mini"`（默认 full）；mini 只渲染 选择 / 加选区 / 分割（无策略面板）/ 吸附，去掉合并与分段互换；full 保持现状（分割▾/合并▾/互换/吸附） |
| 加选区工具 | `src/components/timeline/Timeline.vue` | `create-track-region` emit；`REGION_MIN_DRAG_PX = 3`；`regionDraft`（起点锚 + 实时区间 + 轨道矩形）+ `regionDraftStyle`；`startRegionDraft`（只允许 `ocr_region` 轨）、`onRegionDraftMove`（按锚点排序并 clamp 0..duration）、`onRegionDraftUp`（≥3px → emit + **自动切回选择**；否则当点击 seek）、`deactivate()` 里 `stopRegionDraft()`；`.region-draft` 半透明草稿块 |
| 工具条变体透传 | `src/components/timeline/Timeline.vue` | 新增 `toolVariant?: "full" \| "mini"` prop，`<TimelineToolStrip :variant="props.toolVariant" />` |
| 接线 | `src/components/SourceTimeline.vue` | 删 `.hint` 模板块与样式；`:show-tool-strip="true"` + `tool-variant="mini"`；`@create-track-region` → `addOcrRegionEvent` + `setTime(start)` + `emit("update:time")`；`.tl-shell` 200 → **110px** |
| 图标 | `src/components/AppIconSprite.vue` | 新增 `ic-region`（四角括号 + 内部虚线框） |

**验证（`temp/css-preview/checks/item9.js`，两页分别跑，1680×1000）**
- 语料页 `/project/demo/corpus` 与打轴页 `/project/demo/asr` 结果一致：`.hint` = **0**（提示行已删）、工具条 **4 个按钮**、`.tl-shell` 高 **110px**、选区轨 1 条、`pageOverflow false`。
- 加选区：点工具后 `regionToolActive true`；在轨道空白处拖 100px → 草稿 `.region-draft` 出现且宽 **100px**；松手后选区 2 → **3 条**，工具**自动切回选择** ✓。
- 分割：切到分割工具点第一个 clip → 2 → **4 条**（在已有 3 条基础上再分一段）✓。
- `pnpm build` 通过：2904 模块 / CSS 59.46 kB / JS 660.62 kB。
- 截图：`temp/css-preview/item9-corpus.png`、`item9-asr.png`（工具条在卡片顶部、壳高明显收紧、下方空白留在卡内——即用户要求的「腾出的 90px 留给卡内」）。

> 与「要做」中原文的一处偏差：原文写「创建后停留在该工具便于连画」，用户 m06763 明确改为「**可以自动切回选择**」，实现按用户口径（松手即回到选择工具）。

**现状依据**
- `src/components/SourceTimeline.vue`：提示行在 `:87-89`（文案里只有 `Ctrl+滚轮缩放` 失实），`Timeline` 传 `:show-tool-strip="false"`（`:93`），外壳 `.tl-shell { height: 200px }`（`:124-127`）。
- 可交互区域只有 20px 刻度 + 48px 车道；`.tl-tracks`（`src/components/timeline/Timeline.vue:484`）**没有绑定任何鼠标事件** ⇒ 下面约 130px 是死区。
- 等价能力现状：拖动调起止 / 整体位移 / 吸附 / **分割**（`Timeline.vue:167-179` → `splitEvent`）/**删除**（`Timeline.vue:383-389` 的 Delete）/ S 键在播放头处分割，**都已具备**；只有「在空白处拖动画出选区」需要新增（现在只有双击 ±1.5s：`Timeline.vue:195-200` → `SourceTimeline.onDblClickTrack` → `projectStore.addOcrRegionEvent`）。

**用户已定的决策（2026-10-18）**
- 提示行**整行删除**；
- 工具条放**顶部**（占原提示行位置，与编辑器时间轴同款同位），并把壳高收到 ≈110px（只留刻度 + 车道，不留空白）——即"顶部 + 去掉空白"方案；
- **选区不做合并**（`src/stores/project.ts:1310` 的 `mergeTwo` 继续拒绝 `ocr_region`，store 保持现状）；
- 交互功能**等主时间轴（第 8 项）定稿后再复用**，本项排在其后。

**要做**
- 删 `.hint` 行与样式 → `:show-tool-strip="true"`，`TimelineToolStrip` 加 `variant`（mini 版按钮集合：选择 / 加选区 / 分割 / 吸附，去「分段互换」——迷你轴只有一条选区控制轨，互换无意义）；
- 新增「拖动空白画出选区」（工具值如 `activeTool = "region"`，`src/stores/timeline.ts:48` 的联合类型加值是加法）：按下起点、拖动实时预览、松开 `emit("create-region", { trackId, start, end })` → `addOcrRegionEvent`，位移 < 3px 视为误触；创建后停留在该工具便于连画；
- 壳高收敛到只容刻度 + 车道；若仍有余量，再评估把 `.tl-tracks` 空白区也接上同一套鼠标事件。

**验收**
- 语料页与打轴页两处都测：提示行消失、工具条在顶部、壳高收敛、拖动画出的选区起止与鼠标位置一致、分割可用、双击 ±1.5s 仍可用、滚轮平移与刻度拖动无回归、现有 `.mini-timeline` 1px 边框与页面无重边。

### 15. ✅ clip 拖边（改起止）时显示起止参考线

**✅ 已实现（2026-10-20，`79ae0af`）**
- `src/stores/timeline.ts`：新增 `edgeGuideX = ref<number | null>(null)` + `setEdgeGuide(x)`（与 `snapGuideX` 并列、互不影响 ⇒ 两条线可同时显示）；已加入 return。
- `src/components/timeline/TimelineClip.vue`：`onWindowMouseMove` 在吸附块之后、`updateEventTime` 之前调 `timeline.setEdgeGuide(d.mode === "move" ? null : (d.mode === "l" ? start : end) * timeline.pixelsPerSecond)`；`onWindowMouseUp` 与 `setSnapGuide(null)` 一起置 null。
- `src/components/timeline/Timeline.vue`：`.snap-guide` 之前新增 `.edge-guide` 覆盖层——`left: contentLeft + edgeGuideX - scrollLeft`、`top: calc(playheadTop + 20px)`（标尺下沿）、`bottom: 8px`（滑条容器上沿），`repeating-linear-gradient` 虚线 + `opacity .85` + `z-index 14`（playhead 15 之下、轨道内容之上）。
- 实测（`temp/css-preview/checks/item15.js`，编辑器页 1680×1000）：拖右边缘时 `.edge-guide` 1 条，`top 610.05` = 标尺下沿、`bottom 953` = 滑条上沿（高 342.95）**精确对齐**；松手后 0 条；拖左边缘同样显示且可与吸附线并存；整体位移不显示；`pageOverflow false`。
- 视觉证据：`temp/css-preview/item15-edge-only.png`（吸附关闭、拖边到 ≈26.4s：播放头实线在 1002px、拖边虚线在 1023px，两者可辨）与 `item15-guides-zoom.png`（5 倍放大对照：左＝实线吸附与虚线拖边重合，右＝只有虚线拖边）。

**现状依据（实现前）**
- 拖动逻辑在 `src/components/timeline/TimelineClip.vue` 的 `onWindowMouseMove`（模式 `"l"`/`"r"` 改单边、`"move"` 整体位移；`dt = (dx + (scrollLeft - origScrollLeft)) / pixelsPerSecond`）；**改边期间没有任何落点视觉参考**。
- 现有唯一的参考线是吸附提示线：`src/stores/timeline.ts` 的 `snapGuideX`（`setSnapGuide`）＋ `src/components/timeline/Timeline.vue:678-681` 的 `.snap-guide`（`v-if="timeline.snapGuideX !== null"`，`left: contentLeft + snapGuideX - scrollLeft`）——它只在吸附命中时出现，且是**贯穿整个轨道区**的长线。
- 参考线只能在 `Timeline.vue` 的覆盖层里画：clip 位于 `.tl-track-body` 内（`overflow:hidden`，仅分段互换时 `visible`），子元素无法跨出本条轨道行。

**要做**（用户 m06103）
- 拖动 clip 的**左/右边缘**（改 start/end）期间，在对应边缘的时间位置持续显示一条 1px 垂直细线：**上端自标尺线下部起，下端到水平滑条所在容器上部止**，随鼠标实时更新，松开即消失。
- **整体位移（mode `"move"`）不显示**。
- 与吸附提示线**区分开**（长度不同，可再配合颜色/透明度）：两者可能同时出现（拖边时恰好吸附到某个边界）⇒ **不要复用 `snapGuideX` 一个字段**，新增独立字段（如 `edgeGuideX: number | null` + `setEdgeGuide`，或在 store 里把两条线建模成 `{ x, kind }` 列表）。
- 定位口径与 `.snap-guide` 一致：`left = contentLeft + 时间 × pixelsPerSecond - scrollLeft`。

**验收**
- 拖左边缘 / 右边缘各一条线，位置与 `start` / `end` 的当前值精确一致（可用 `evalq` 比对线 `left` 与 `clip` 边缘像素）；随鼠标移动实时更新；松开后消失；整体位移时不出现；与吸附线同时出现时两条都能看见且可区分；编辑器时间轴与（第 9 项之后）迷你时间轴都成立；无布局跳动。

### 16. ✅ clip 吸附目标新增「播放头」

**✅ 已实现（2026-10-20，`79ae0af`）**
- `src/components/timeline/TimelineClip.vue` 的 `collectSnapEdges()` 末尾追加播放头：`const playhead = timeline.currentTime; if (playhead > 0 && playhead <= timeline.duration) edges.push(playhead);`——随 `d.snapEdges` 在拖动开始时快照一次，拖动中播放头移动不会让目标漂移。
- **复核「当年为什么没成功」**：`snapToEdges` 的 `e - s >= MIN_DUR` 校验作用于**吸附后的结果跨度**（clip 时长），候选本身只是数字 ⇒ **单点候选不会被过滤**，`snapToEdges` 无需改动（下面「要做」里那条担心不成立）。
- 实测（同一脚本）：播放头设到 21.96s，把宽 clip 左边缘拖到 8px 窗口内（目标 22.7s）→ clip 起点**精确落在 21.96s**（= 播放头实际时间）、`.snap-guide` 同时出现；把磁铁关掉后同样落点停在 23.79s 且**无吸附线** ⇒ 吸附确由播放头候选驱动。
- 验收：暂停时拖边 <8px 精确落在 `currentTime` ✓；播放中拖动不漂移 ✓；与同轴事件边界吸附并存（最近者胜）✓；`Ctrl+Z` 可回退（沿用拖动开始时的快照）✓。

**现状依据（实现前）**
- 吸附候选取自 `src/components/timeline/TimelineClip.vue` 的 `collectSnapEdges()`（`:104-114`，拖动开始时的快照：自身 `origStart/origEnd` + `props.snapEvents` 各事件起止），`props.snapEvents` 由 `src/components/timeline/Timeline.vue:107-110` 的 `snapEvents` computed 提供（= 当前**显示**的轨道的事件集合，跨源吸附就是在这里修掉的）。**`timeline.currentTime`（播放头）不在候选里**。
- 命中判定在 `snapToEdges(mode, start, end, edges, pps, minStart, maxEnd)`（`:119-157`）：`let bestDist = SNAP_THRESHOLD_PX`（8px），并且要求 `s >= minStart && e <= maxEnd && e - s >= MIN_DUR`（`MIN_DUR = 0.05`）。**播放头是单点候选（start == end），会被 `e - s >= MIN_DUR` 这一条时长校验直接过滤掉**——这很可能就是「之前实现过但没成功」的原因（待实现时先复核这段代码确认）。

**要做**（用户 m06103）
- 把播放头作为**单点吸附目标**加入候选，且让 `snapToEdges` 支持单点（单点只需落在 `[minStart, maxEnd]` 内、按最近距离参与比较，跳过时长校验）。
- 播放中播放头在移动 ⇒ 候选在**拖动开始时取一次快照**（与现有 `d.snapEdges` 同时刻），避免拖到一半目标漂移。
- 吸附命中时给出可辨识的反馈（播放头位置本来就有一条线，建议用颜色/加粗或另一种提示区分「吸到了播放头」）。

**待与用户讨论（实现前）**
- 播放中拖动 clip 是否也允许吸附播放头（还是只在暂停时）；
- 命中阈值是否沿用 8px。

**验收**
- 暂停时把 clip 左/右边缘拖到播放头附近（<8px）→ 精确落在 `timeline.currentTime`；播放中拖动时目标不漂移（用拖动开始时的播放头时间）；与现有「同轴事件边界」吸附不冲突；`Ctrl+Z` 可回退到拖动前。

### 17. ✅ 去除轨道内 clip 的所有句末标点

**✅ 已实现（2026-10-20，`79ae0af`）**
- `src/stores/project.ts`：新增并导出 `stripTrackTrailingPunctuation(trackId: string): number`——先收集待改条目、**改动前** `recordSnapshot()`（一次撤销还原整批）、跳过 `ocr_region` 与无 `text` 事件，返回改动条数。
- `src/components/AppIconSprite.vue`：新增 `ic-sparkle` 图标。
- `src/components/TrackOverview.vue`：新增「文本美化」下拉（`BEAUTIFY_ACTIONS` + `onBeautify`，`NPopselect` 包 `.btn.ghost.sm`），位置在**「导出」左边**，仅文本轨显示；回执「已去除 N 条的末尾标点」/「没有需要去除的末尾标点」。
- 实测（`temp/css-preview/checks/item17.js`）：表头按钮 `["美化","导出"]`；下拉选项 `["去除文本末标点"]`；执行后 11 条变化（如 `…似てる？`→`…似てる`）；回执「已去除 11 条的末尾标点」；**一次 Ctrl+Z 整批还原**；`pageOverflow false`。

**起因（用户 m06145）**
- 第 8 项的「被分割态」用 ⬅️ 把右侧一段移到左侧后，左侧可能以逗号收尾（例 `123，4567，234，`）。需要一条批量清理：把**当前轨道内所有 clip 文本的句末标点去掉**。

**现状依据**
- 单条改文本走 `src/stores/project.ts` 的 `updateEventText(id, text)`；批量变换没有现成 API，需要新增一个「整条轨道遍历 + 一次快照」的 store action。
- 标点表可复用第 8 项的 `src/composables/sentenceCuts.ts`。

**要做**
- `src/composables/sentenceCuts.ts` 增加 `stripTrailingPunctuation(text)`（只剥句末连续的标点/空白，句中标点不动）。
- `src/stores/project.ts` 增加 `stripTrackTrailingPunctuation(trackId)`：遍历该轨道所有**文本事件**逐条处理，**有变化才 `recordSnapshot()`**（一次 Ctrl+Z 还原整批），返回 `{ changed }`。
- 入口待定：轨道总览头部（与「导出」并列、作用于当前聚焦轨道）或时间轴轨道标签的菜单。**建议前者**，与现有聚焦轨道的操作集中在一起。

**验收**
- 只影响目标轨道与文本事件（`ocr_region` 不动）；句中标点不受影响；一次 Ctrl+Z 全部还原；无变化时不产生快照。

### 18. ✅ 语料「全文替换」同步作用于产物轨（PR49 审查登记的功能候选）

**已实现（2026-10-25）**

| 子项 | 处理 | 位置 |
|---|---|---|
| 同步范围 | **仅 `type === "fused"` 的产物轨**；转写轨（`ocr_text`/`embed_ocr`/`asr`）与 `manual` 手打轨一律不动 | `src/stores/project.ts`（`replaceAllCorpus`） |
| 匹配口径 | 精确子串（与语料替换、搜索高亮同口径）；先算出新文本再统一落盘，不就地改 | 同上（`patched: Map<eventId, text>`） |
| 撤销 | 只在**两边合计命中 > 0** 时 `recordSnapshot()`；快照本就含 `tracks + corpus`，**一次 Ctrl+Z 语料与产物一起还原** | 同上（`snapshot()`） |
| 返回值 | 扩为 `{ hits, items, trackHits, trackItems }`，语料处数与产物轨处数分开报 | 同上 |
| 提示文案 | 替换成功提示按实际命中分项：「已替换：语料 3 处（2 条），产物轨 2 处（1 条）」 | `src/views/CorpusView.vue`（`onReplaceAll`） |
| 审计表 | **不动 `fuse_meta`**——审计表「不配 → 回退转写原文」依赖 `fuse_meta.source_text` | 无改动 |

**决策（用户 m00901）**：同步范围**仅 fused 轨**；**不加开关**（不做「同时替换产物轨」勾选框）；语料区与产物两边的文本**不保证同步、也不需要同步**——这次只是一次性文本搬运，不建回链、不做持续对齐。

**验证**：`pnpm build`（含 `vue-tsc --noEmit`）通过。**未经真机 UI 手测**（需在有 fused 产物的工程上替换一次并 Ctrl+Z）。

**未做（明确留口）**：`manual` 轨不同步（手打文本不是语料搬运）；转写轨不同步（那是待替换的原文）；不做"语料改了产物自动跟着变"的持续同步。

**起因（PR49 审查观察项，2026-10-20）**
- 第 6 项实现的语料全文替换（`replaceAllCorpus`）只改 `project.corpus`、**不动轨道事件文本**。
  已融合产物（fused 轨）的文本是语料的逐字搬运快照——语料改名后产物轨仍是旧文本，
  用户可能预期"替换语料时把产物轨里的同款文本一并替换"。

**现状依据**
- `src/stores/project.ts` `replaceAllCorpus(query, replacement)`：仅遍历 `project.corpus`；轨道侧更新文本的唯一入口是 `updateEventText(id, text)`（单条）。
- 产物文本与语料条目**没有回链**（fused 事件没有记录"文本来自语料第几条"）——按文本内容匹配（与搜索语义一致）即可，无需建回链。

**设计要点（实现前与用户确认，已按此执行）**
- 作用范围：建议仅 fused 产物轨——转写轨（ocr_text/embed_ocr/asr）原文属"待替换的转写文本"，不该被语料反向污染；
- 匹配口径：精确子串（与语料替换一致）；一次快照、可整体回退；
- 与审计表的交互：改 fused 文本不影响 `fuse_meta.source_text`（它记的是融合时刻转写原文）。

**验收**
- 替换后 fused 轨中包含该子串的条目同步更新，其余轨不动；一次 Ctrl+Z 还原；无命中不产生快照；审计表「不配 → 回退转写原文」不受影响。

---

## D 组：跨页面体验与外部一致性

### 10. ✅ 快捷键体系（注册表 + 单一监听；帮助面板移到第 19 项）

**已实现（2026-10-21，`02416b8`）**

| 改动 | 文件 | 内容 |
|---|---|---|
| 注册表 + 分发器 | 新建 `src/composables/shortcuts.ts` | `SHORTCUTS`（21 个动作、按组分类）、`shortcutKey()` 规范成 `Ctrl+Shift+Z` / `Space` / `ArrowLeft`、`onShortcut(id, run, active?)` 注册、`dispatchShortcut(e)` 分发并返回是否命中、`markHoveredTimeline()/isActiveTimeline()` 活动时间轴、`isEditableTarget()`；注册表自身键位冲突在启动时 `console.warn` |
| 唯一 window 监听 | `src/layouts/ProjectLayout.vue` | 原 `onGlobalKeydown`（Ctrl+S/Z/Y 硬编码）改为 `dispatchShortcut`，并登记 save/undo/redo 三个动作 |
| 时间轴动作下沉 | `src/components/timeline/Timeline.vue` | 删掉自己挂在 window 的 keydown（**修掉「两条时间轴同时响应」的既有缺陷**），改为登记 toolSelect/toolSplit/toggleSnap/frameStep/secondStep/docStartEnd/prevNextClip/zoom/zoomFit/splitAtPlayhead/mergeNext/delete/exitSplitPair；新增 props `timelineRole`（primary/mini）与 `videoSource`（逐帧按哪支视频的 fps）；`@mouseenter/@mouseleave` 标记活动时间轴；⬅️➡️ 在分割态搬句界、否则逐帧（`1/fps`） |
| 播放动作 | `src/components/VideoPlayer.vue` | 登记 playPause/reverse/stopPlay/forward；**倒放用 rAF 按真实耗时递减 currentTime**（浏览器不支持负 `playbackRate`），到 0 自停；K 暂停并复位方向 |
| 缩放至全部 | `src/stores/timeline.ts` | 新增 `zoomToFit()`（`pps = 视口宽/时长`、`scrollLeft = 0`） |
| 迷你时间轴角色 | `src/components/SourceTimeline.vue` | 传 `timeline-role="mini"` 与 `video-source`（语料页按剧情录屏 fps 逐帧） |

**验证（`temp/css-preview/checks/item10.js`、`item10b.js`、`item10c.js`；合成 keydown，真实敲键待用户确认）**
- 工具：`C` → 分割工具（含策略面板标题）、`V` → 选择工具、`N` 吸附开/关（`.tool-btn.active` 翻转）。
- 跳转：`Home` → 0；`End` → 743.481；`→` 逐帧 0.11px（= `1/30 × pps`）、`Shift+→` 3.38px（= `1 × pps`）；`,`/`.` 跳到上/下一个 clip 起点（3.2s）。
- 缩放：同一个 clip 22.58 → 27.32（两次 `=`）→ 24.59（一次 `-`）→ 6.77（`\` 缩放至全部）。
- 编辑：clip 内部 `S` 分割 41 → 42 且 `.split-pair` = 2、`Esc` 归零；`M` 41 → 40；`Delete` → 39；均在聚焦 clip 上生效。
- 播放：`Space` 500ms 前进 0.313s、再按暂停；`J` 450ms 后退 0.45s（≈1×）、`K` 后 Δ = 0；`L` 恢复播放。
- 输入框内按 `C` 不切工具（`isEditableTarget` 生效）。
- **活动时间轴**：悬停迷你时间轴按 `N` → 只有迷你开关翻转；悬停主时间轴按 `N` → 只有主时间轴翻转；两者都移出后按 `N` → 主时间轴（默认响应者）翻转。
- `pnpm build` ✓ 2901 模块。

**未做（用户明确留待第 19 项）**：帮助面板与「快捷键查看/修改」UI；注册表已预留 `group`/`label` 字段供其直接消费。

**现状依据（散落三处）**
- `src/layouts/ProjectLayout.vue`：Ctrl+S 保存 / Ctrl+Z 撤销 / Ctrl+Shift+Z、Ctrl+Y 重做。
- `src/components/timeline/Timeline.vue:378-411`：Delete 删除聚焦 clip、S 在播放头处分割、M 合并相邻（**每个 Timeline 实例各绑一次**）。
- `src/components/VideoPlayer.vue` 的 `handleKeydown`（播放器自身的按键）。

**调研结论（2026-10-21）**
- Premiere：`Space` 播放、`←/→` 一帧、`Shift+←/→` 五帧、`Home/End` 首尾、`V/C` 选择/剃刀、`S` 吸附、`Ctrl+K` 播放头切开、`+/-` 缩放、`?` 帮助。
- Resolve：`J/K/L` 倒放/停/正放、`A/B` 选择/刀片、`N` 吸附、`Ctrl+B` 播放头切开。
- Aegisub（字幕向）：`s` 播放选区、`q/w/e/d` 前后 500ms、`g` 提交进下一行、`z/x` 换行、`a/f` 滚动。
- 依据：[Aegisub 计时手册](https://aeg-dev.github.io/AegiSite/docs/3.0/Timing/)、[Premiere 默认快捷键](https://helpx.adobe.com/premiere/desktop/get-started/keyboard-shortcuts/default-keyboard-shortcuts.html)、[Resolve 快捷键表](https://www.editorskeys.com/blogs/news/davinci-resolve-keyboard-shortcuts-pdf-cheat-sheet)。

**用户已定的决策（2026-10-21，m06929）**
- **吸附开关用 `N`**（避让 `S`）；`S` 保持「播放头处分割」（本应用是字幕工作站，分割为最高频操作，属**有意偏离** Premiere）。
- **`J/K/L` 只做单速**：`J` 1× 倒放、`L` 1× 正放、`K` 停止并恢复正放；**不做连按加速**（用户认为没必要）。
- **`⬅️/➡️` 不在分割态时退化为逐帧步进**（分割态仍为搬句界）。
- **帮助面板与它的快捷键暂时只记待办**——由第 19 项「设置面板」承载（快捷键查看 + 修改）。
- **`Ctrl+A` 暂不做**，随第 21 项多选一起实现（理由见第 21 项）。

**要做**
- 新建 `src/composables/shortcuts.ts`：`ShortcutDef { id, group, label, keys[], scope: "global" | "timeline" | "player" }` + `formatKey(e)`（规范成 `Ctrl+Shift+Z` / `Space` / `Shift+ArrowLeft`）+ `SHORTCUTS` 注册表 + `SHORTCUT_GROUPS`；**键位只在这里定义**（将来设置面板与帮助面板同源读取）。
- 单一监听：`src/layouts/ProjectLayout.vue` 现有 `onGlobalKeydown` 扩成唯一入口 —— 先放行输入框（INPUT/TEXTAREA/contenteditable），再 `dispatchShortcut(e)`，命中则 `preventDefault()`；各组件通过注册函数挂自己的处理器，不再各自 `window.addEventListener("keydown")`。
- **活动时间轴**：`timeline` 作用域的处理器按实例注册，分发时只调到「最近一次鼠标进入的时间轴」（`mouseenter` 标记；无标记时默认真正的校对区时间轴，即注入的独立 store 之外的那个）。**顺带修既有缺陷**：现在校对区 `Timeline` 与迷你时间轴 `Timeline` 都往 window 挂 keydown，打轴页按 `Delete`/`S`/`M` 会同时作用到两条时间轴。
- 键位清单：`Space` 播放暂停、`J/K/L` 单速倒放/停/正放、`←/→` 逐帧（分割态为搬句界）、`Shift+←/→` ±1 秒、`Home/End` 片头/片尾、`,`/`.` 上/下一个 clip、`=`/`-` 缩放、`\` 缩放至全部、`N` 吸附、`V`/`C` 选择/分割工具、`S` 分割、`M` 合并、`Delete` 删除、`Esc` 退出分割态、`Ctrl+Z`/`Ctrl+Y`(`Ctrl+Shift+Z`)/`Ctrl+S`。
- 全局监听一律按 PR49 审查的写法：挂 `onMounted` + `isConnected`/注入判定当激活闸门（`onActivated` 只对 `<KeepAlive>` 直接子组件触发）。

**验收**
- 每条键位用 CDP 可信 `keydown` 实测（含输入框内不触发、KeepAlive 暂存页不响应、两条时间轴互不串扰）；
- `pnpm build` 通过；倒放实测时间在减少、`K` 后恢复正放；逐帧步进 = `1/fps`。


### 11. ✅ 欢迎页模型自检升级（探针 + 手动补齐命令 + 一键安装/更新）

**现状依据**
- 现在只做探针：`src/views/Welcome.vue` 的 `probeOcr()`（`invoke("check_ocr_runtime")`）、`probeAsr()`（`projectStore.getAsrEngines()` 取 `moss`）、`probeEmbed()`（`invoke("check_embed_runtime")`），三者不 await、未就绪只显示状态、不阻塞。
- 合并 PR #48 后，ASR 卡已多一行「设备 · <config>（实测 <L2>）」。（原文还写「判定与建议前端够不到、没有对应命令」——**已过时**，见下方「前置状态」。）

**前置状态（2026-10-28 核对，含 P5/P6 补丁已套用）**

| 依赖 | 状态 |
|---|---|
| `probe_moss_backend`（判定 / `reason` / `source` / `recommendation` / `manifest_flavors` / `cuda_compute_cap` / `cuda_runnable` / `gpus` / `error`） | ✅ 就绪（第 12 项 + P5 `351cd5a`） |
| `set_moss_device`（落盘，下次启动生效） | ✅ 就绪（`a8e8010`） |
| `probe_moss_install`（已装运行时 vs 清单，**八态**） | ✅ 命令就绪（P6 `80eda1d`）；数据源 `runtime/.installed.json` 自 `7d3f8d5` 起由 `scripts/bootstrap_runtime.ps1` 写入（`install_runtime` 跑完即落盘），未跑过安装时仍返回 `not_installed` / `unknown`，属预期。**2026-10-30 起多一态 `model_missing`**：装了二进制却缺 gguf（模型单独成条目，见下一行），不再谎报 `ok` |
| `models-moss`（gguf 模型条目：清单 + Release 资产） | ✅ 已发布（2026-10-30）：`models-moss.zip` 626,397,892 B / sha256 `809f8999…9a8cb1`，挂在 Release `moss-runtime-v1` 上；清单第 4 条 `dest=models/moss`、**不带 flavor**（三种后端共用一份 q5_k）。二进制与模型缺一不可 ⇒ `bootstrap_runtime.ps1` 的 `-Items` 缺省改为 `moss,models-moss` |
| `check_asr_engines.effective_backend`（L2 实测） | ✅ 已有（PR#48） |
| `scripts/bootstrap_moss.ps1 -Backend <cpu\|cuda\|vulkan>`（现存脚本，可作指引） | ✅ 可用；注意默认 `cpu`，要 GPU 必须显式传 `-Backend`，也支持 `-BinaryUrl` + `-BinarySha256` 指向发布包 |
| `scripts/bootstrap_runtime.ps1`（契约 §2 新入口 + §3 JSONL 进度） | ✅ 已交付（`7d3f8d5`，分支 `feat/runtime-bootstrap`）：stdout 纯 JSONL（`start`/`progress`/`item_skip`/`item_done`/`item_fail`/`error`/`done`，末行必 `done`），退出码 0/1/2/3/4，多条目 + 多 flavor，`-Flavor auto` 走 L0 探测，下载前磁盘预检，镜像失败回退直连，SHA256 校验通过才解压，写 `runtime/.installed.json`（已装且哈希一致 → `item_skip`，`-Force` 重装）。**2026-10-30 补：下载器改「多源候选 + 64 MB 分段 + 断点续传」**（`-SegmentMB`/`-NoResume`/`-MaxAttempts`，缓存 `<runtime>\.downloads\<id>-<sha8>\`，半截 `.curlpart` 下次并入段文件；不支持 Range 的源退回单流且不续传，避免 `curl -C -` 重复追加）——契约 §4.5 |
| `deps-ocr` / `deps-ocr-gpu`（OCR 运行时两条目）+ CUDA 运行库（pip 现拉） | ✅ 已发布（2026-10-30）：`ocr-runtime-v1` Release 上两个资产——`ocr-runtime-cpu.zip` 309,803,561 B（解压 911,518,754，`flavor=cpu`）、`ocr-runtime-cuda-paddle.zip` 502,533,301 B（解压 996,251,695，`flavor=cuda` + `optional`）——**2026-10-30 重打重传**：旧包把冒烟日志 `smoke.stderr.log` 与构建机路径一起发了出去，现已在 `Invoke-Smoke`/`New-Package` 两处清掉并 `--clobber` 覆盖。**`deps-ocr-cuda-libs` 条目已删除**：CUDA/cuDNN 运行库改由安装器用 pip 从 PyPI 镜像现拉（8 个 `nvidia-*` 轮子，`--require-hashes`；缺省华为云镜像 10.3 MB/s），理由 ① CUDA EULA 1.2.2/1.1.2 禁止把 SDK 当独立产品公开分发 ② `nvidia/` 1.86 GB 贴死 GitHub 单资产 2 GiB 上限。实测落地 `deps_gpu/nvidia` 2,568.4 MB / 258 文件。两条目都带 `unpacked_size`，磁盘预检按 `Σ下载 + Σ解压 + 最大单包解压 + 100 MB`（+ CUDA 运行库 4.34 GB）算 |
| `deps-embed`（向量融合运行时条目：清单 + Release 资产） | ✅ 已发布（2026-10-30）：`embed-runtime.zip` 157,386,205 B / 解压 345,868,793 / sha256 `1771e9e6…6a1884`，挂在 Release `embed-runtime-v1`；清单第 7 条 `dest="."`、**不带 flavor**（26 个包是纯 CPU 的 onnxruntime，没有 CUDA/DML 变体）。包内 `deps_embed/` + `models/embed/multilingual-e5-small/` + `worker/{embed_worker.py,fuse_core.py}`；**不内嵌 Python**，复用 `deps-ocr` 的 `python/python.exe`（省 47 MB）⇒ 安装前置 = `deps-ocr` 已装。Rust 侧新增 `embed_paths`（空配置回退标准布局）+ 命令 `probe_embed_install` |
| Rust 侧「起子进程 + 读 JSONL + 进度事件 + 取消 + 磁盘预检」 | ✅ 已交付（`7d3f8d5`）：`src-tauri/src/ai_runtime/runtime_install.rs`，命令 `precheck_runtime_disk` / `install_runtime` / `cancel_runtime_install`，进度走 `runtime-install-progress` 事件，取消用 `taskkill /T /F` 杀进程树，同一时刻只允许一个安装任务 |

**规划（2026-10-28 拆三步，11a/11b 不阻塞）**

- **11a 探针与指引（可立即做：纯前端 + 现有命令，零跨会话依赖）**
  - 欢迎页 ASR 卡三行：①设备（`probe_moss_backend.configured_device`，附「去打轴页改」入口）②为什么 / 该装哪个（`reason` / `source` / `recommendation(_reason)` / `cuda_compute_cap` + `cuda_runnable`）③运行时状态（`probe_moss_install.state` + `message`）；
  - OCR / Embed 卡同口径：未就绪时把 `check_ocr_runtime` / `check_embed_runtime` 的失败原文摊到卡面（现在只有状态点 + `title` 悬停看）；
  - 仍然「自检不阻塞」，只在真正运行引擎时硬拦。
- **11b 命令指引层（可立即做）**：未就绪时给一条**可复制**的命令（`scripts/bootstrap_moss.ps1 -Backend vulkan` 这一代）；若要判断脚本是否存在，加一个只读命令 `probe_bootstrap_scripts`（返回脚本存在与路径），不让前端猜路径。
- **11c 静默 bootstrap（阻塞已解除，2026-10-28）**：执行体 `scripts/bootstrap_runtime.ps1` 与 Rust 侧基础设施（spawn / JSONL 进度事件 / 取消 / 磁盘预检）已由分发侧随 `7d3f8d5` 交付。本分支剩下的是**接线**：首次启动或用户点「安装」时先调 `precheck_runtime_disk`，再调 `install_runtime`（`items` / `flavor` / `force`），监听 `runtime-install-progress` 渲染进度，取消按钮调 `cancel_runtime_install`。**遗留一条**：打包形态下脚本尚未随 app 发布（`bundle.resources` 只有 manifest，`bootstrap_candidates` 已留 `resources/scripts/bootstrap_runtime.ps1` 位置），需分发侧补 `tauri.conf.json` 的资源映射。（2026-10-30 核对：`feat/misc-fixes` 的 `bd8ee43` + `scripts/sync_bundle_resources.mjs` 已在那边补齐 `resources/scripts/*` 与构建期同步，本分支尚未合入 ⇒ 合并后此条关闭。）

**11a / 11b 已实现（2026-10-28；11c 接线随后并入 installer 侧交付）**

| 子项 | 处理 | 位置 |
|---|---|---|
| ASR 卡三行 | ①设备（配置值 + 实测后端）②为什么 / 该装哪个：显式指定不可用 → 红字硬报错；**装的是 CPU 版但本机能上 GPU → 黄字「建议改用 cuda 版运行时：<理由>」**（比对的是 `probe_moss_install.installed.flavor`，不是 `requested`，避免 auto 下天天误报）③运行时状态：`probe_moss_install.message`，需要动作的状态（`update_available` / `file_missing` / `flavor_not_in_manifest`）标黄 | `src/views/Welcome.vue`（`probeAsr` / `setAsrInstall` / `INSTALL_ACTION_STATES`） |
| OCR / Embed 卡 | 未就绪时把 `check_*_runtime` 的失败原文直接摊到卡面（原先只藏在 `title` 悬浮里）；`pnpm build` 前 ASR 卡的原因已由上面两行覆盖 ⇒ 不再重复显示引擎 message | 同上 |
| 手动补齐命令（仅 OCR / e5-small） | 未就绪时给一行等宽命令，**点击复制**（`navigator.clipboard`，失败降级提示手动选中）：`scripts/bootstrap_ocr.ps1` / `scripts/bootstrap_embed.ps1`。MOSS 在 11c 之后改用安装按钮，卡面不再并列给 `scripts/bootstrap_moss.ps1`（两个 CTA 打架，见 11c 决策） | 同上（`cmdOf` / `showCmd` / `copyCmd`、样式 `.ecard .cmd`） |
| store / 类型 | 新增 `probeMossInstall()` 与 `MossInstallStatus` / `MossInstalledItem` | `src/stores/project.ts`、`src/types/index.ts` |

| 顺手修正 | OCR / e5-small 卡此前也会显示「设备 · auto（ggml 自动选优）」——`ggml` 是 MOSS 专属，`deviceText()` 对任何 `device===""` 的卡都返回该文案（`setProbe` 的 device 默认空）。改成只给 ASR 卡渲染设备行 | `Welcome.vue` 设备行加 `card.key === 'asr'` 守卫 |

**验证**：`pnpm build`（含 `vue-tsc --noEmit`）EXIT=0；**已用 headless Chrome + Tauri mock 实测**（`temp/css-preview/checks/item11.js`，1680×1000）：
- 正常态（MOSS 就绪 + 运行时 `update_available`）：MOSS 卡多出黄字「运行时是旧版…」+ 命令 `scripts/bootstrap_moss.ps1 -Backend vulkan`（`update_available` 正确地取**已装** flavor）；e5-small 未就绪卡多出失败原文 + `scripts/bootstrap_embed.ps1`；`pageOverflow=false`、`.env` 高 167px。
- 最拥挤态（MOSS 未就绪 + 显式 cuda 不可用）：红字硬报错、`MOSS 运行时未安装`、命令、`推理设备可在打轴页…设置` 五行并存，三张卡等高不溢出。
- 命令点击 → 弹「复制失败，请手动选中命令」（headless 无剪贴板权限，即降级分支按设计工作；成功分支未在无头环境验证）。
- 亮色主题同步核对。
- 截图：`temp/css-preview/item11-env-cards.png`（暗/正常）、`item11-env-notready.png`（暗/最拥挤）、`item11-env-light.png`（亮/正常）。

**11c 已实现（2026-10-28，一键安装/更新）：前置由 `7d3f8d5` 交付**（`scripts/bootstrap_runtime.ps1` 406 行 + `src-tauri/src/ai_runtime/runtime_install.rs` 1100 行：`precheck_runtime_disk` / `install_runtime` / `cancel_runtime_install` 三命令 + `runtime-install-progress` 事件）。

| 子项 | 处理 | 位置 |
|---|---|---|
| 入口 | MOSS 卡在「未就绪」或「运行时需更新」时给按钮（`canInstall` / `installLabel`：未就绪 →「安装运行时」，否则「更新运行时」），旁注「按本机能力选后端」 | `src/views/Welcome.vue` |
| 磁盘预检 | 点按钮先 `precheck_runtime_disk`（Rust 侧按 cuda 包 622 MB × 展开系数 3 估 ≈1.97 GB）；不通过只 toast、不启动安装 | 同上 `startInstall` |
| 进度 | 安装态放 store（切页面不丢）：`installRunning` / `installPhase` / `installPercent` / `installDetail`；`listen(RUNTIME_INSTALL_EVENT)` 在 store 顶层注册、`onScopeDispose` 退订；卡面渲染 3px 进度条 +「下载 42%」+「取消」，`detail`（脚本原文短诊断）单行省略、全文进 `title` | `src/stores/project.ts`、`Welcome.vue` |
| 取消 | 「取消」→ `cancel_runtime_install()`；返回 `false` 时提示「没有正在运行的安装任务」 | 同上 `cancelInstall` |
| 完成后 | 无论成功/取消都重跑 `probeAsr()`（引擎状态、设备判定、实测后端、安装状态全部重取） | 同上 `startInstall` |
| 类型 | `RUNTIME_INSTALL_EVENT` / `RuntimeInstallEvent` / `RuntimeInstallSummary` / `RuntimeDiskPrecheck`（snake_case 对齐 Rust） | `src/types/index.ts` |

**11c 决策（含对该项原标题「静默 bootstrap」的有意偏离）**：
- **不做首启静默下载**：cuda 包 622 MB，静默在后台拉对大流量/家用带宽不可接受，失败时用户什么也看不到；改成「显式按钮 + 先报磁盘」。11a 的自检卡已经把「缺什么、该装哪个后端」说清，静默的那点便利不值这个代价。形态可逆。
- **按钮取代 MOSS 的手动命令**：两个并列 CTA（按钮 + 命令）互相打架，且命令在安装版里根本不存在（没有 `scripts/`）；OCR / e5-small 没有清单条目，仍只给命令。
- **`flavor` 传 `auto`**：脚本 `Resolve-AutoFlavor`（cuda 算力 ≥7.5 → cuda → vulkan → cpu）与 Rust `recommend_from_assets` 口径一致，与卡面「建议改用 cuda 版运行时」不矛盾。
- **进度状态放 store**：安装几分钟，用户切走页面不能丢进度，与既有 OCR/ASR/LLM 进度监听同一写法。

**11c 验证**：`pnpm build`（含 `vue-tsc --noEmit`）EXIT=0（2901 modules）；headless Chrome + Tauri mock 实测（`temp/css-preview/checks/item11c.js`，1680×1000；mock 新增 `plugin:event|listen` 捕获与 `window.__emitEvent(name, payload)`，并新增 `precheck_runtime_disk` / `install_runtime` / `cancel_runtime_install` 三个 case，`?asrmock=notready` 切到未安装态）：
- 未就绪态卡面顺序：设备 → 红字硬报错 → 运行时未安装 → 「安装运行时」按钮 + 旁注 → 「推理设备可在打轴页…设置」，`pageOverflow=false`；
- 点击后 500 ms：`.pbar` 出现（30%）、行文案「下载 30%」+「取消」、安装按钮消失、`.ell` 显示 `moss-vulkan.zip`，状态点变琥珀 + 「安装中…」；
- 1.8 s 后：toast「安装完成：新增 1 项，跳过 0 项」，按钮回到「安装运行时」（mock 仍返回未安装态）；
- 就绪 + `update_available`：按钮文案为「更新运行时」，黄字仍显示运行时提示；
- 截图：`temp/css-preview/item11c-install.png`（未就绪）、`item11c-progress.png`（安装中）、`item11c-update.png`（可更新）。
- **未验证**：真实 installer 子进程（本机无外网，cuda 包 622 MB）、取消对真实进程树的 `taskkill /T /F` 效果——需联网后在有网环境下走一次真装。

**真机回归修复（2026-10-30，MSI 装机反馈；提交 `bd8ee43`）**：上一版安装包在真机上暴露三个问题——「探测阻塞整个应用（连拖窗口都卡）」「命令行窗口疯狂闪」「一键安装必然失败」，根因与修法：

| 缺陷 | 根因 | 修法 |
|---|---|---|
| 探测冻结 UI | `probe_moss_backend` / `check_ffmpeg` / `get_video_metadata` 是**同步** `#[tauri::command]`，跑在主线程上；探测要起 `moss info`（加载几百 MB 模型）+ `nvidia-smi` ×2 + `vulkaninfo` ×2 | 三个命令改 `async` + `tauri::async_runtime::spawn_blocking`；`MossProvider` 构造时**不再同步 probe**（`probed: AtomicBool` + `ensure_probed()`，`describe()` 未探测时返回「正在探测 MOSS 后端」），`AsrManager::new` 起后台线程逐个 `warm_up()` |
| 控制台窗口乱闪 | 28 处 `Command::new(...)`（powershell / nvidia-smi / vulkaninfo / moss / python / ffmpeg）都没设 `CREATE_NO_WINDOW`，GUI 进程起控制台子进程必弹窗 | 新增 `src-tauri/src/subprocess.rs`（`hidden()` / `quiet()` 统一加 `CREATE_NO_WINDOW`），全仓库替换；新增两条单测钉住「静默但仍能取到 stdout」 |
| 打包后一键安装必失败 | `bootstrap_runtime.ps1` 从未进 `bundle.resources` ⇒ `bootstrap_candidates()` 全落空 | 新增 `scripts/sync_bundle_resources.mjs`（构建前置步骤，拷脚本到 `src-tauri/resources/scripts/`），`package.json` 的 `build` 串进去，`bundle.resources` 加 `resources/scripts/*`；已验证 NSIS/MSI 内确实含 `resources\scripts\bootstrap_runtime.ps1` |
| 失败了看不到原因 | 安装子进程无可见控制台后成了黑盒 | 每次安装写 `runtime/logs/install.log`（脚本路径 / flavor / 退出码 / stderr 尾部 4000 字符），失败时 `InstallSummary.message` 追加「（日志：<路径>）」 |

- `bundle.resources` 现在是三条：`resources/runtime/manifest.json`、`resources/ffmpeg/*`、`resources/scripts/*`。**不能把 `src-tauri/resources/scripts/` 整个 gitignore**：目录空掉时 Tauri 会报 `glob pattern resources/scripts/* path not found or didn't match any files.`，故 `.gitignore` 只忽略 `*.ps1`，另提交一份 `src-tauri/resources/scripts/README.md` 常驻。
- 验证：`cargo test --lib` 334 passed、`cargo check --lib` 0 warning、`pnpm tauri build` EXIT=0、`7z l` 确认三处资源都在包里。**真机复测（重装 MSI）由用户执行**。

**未做**：`probe_bootstrap_scripts`（并入安装器后不需要前端判断脚本存在——`install_runtime` 找不到脚本会直接 Err）；首启静默安装（见 11c 决策，不做）。

**依赖/前置**：已全部满足（命令齐、契约齐、执行体已并入）。

- bootstrap 脚本**已在 `scripts/`**（不在根目录）：`scripts/bootstrap_moss.ps1`（模型/二进制获取与就绪自检）、`scripts/build_moss.ps1`（自构建，含 vulkan 前置检查）。

**要做**
- 探测到未就绪时**自动静默跑 bootstrap**（覆盖首次启动完全没有任何环境的情况）——需先定形态：Rust 侧起子进程跑 `scripts/bootstrap_*.ps1`，还是只给「跑哪条命令」的指引文案（前者要进度事件与取消，后者成本低得多，倾向先做指引）；
- 自检**不阻塞**其它操作，只在真正运行该引擎时才阻拦；
- 自检卡上显示进度与失败原因，并给出「该装哪个后端 / 为什么落到 CPU」的指引（需新增 `probe_moss_backend` 命令，见接口一节）。

**依赖/前置**：Rust 侧配合（bootstrap 脚本调用或清单、进度事件）+ 「并行分支接口」一节的判定函数与命令缺口。

**顺序说明**：与 C 组无依赖，可与 8–10 并行；放中后段是为了不打断时间轴主线。

### 12. ✅ CUDA GPU 加速参数化（2026-10-22 阶段 A+B → 2026-10-24 阶段 C 接通 → 2026-10-29 FunASR 设备 + 即时生效；提交 `a8e8010` / `2a766c8` / `1fa8a77`）

**已实现**

| 子项 | 处理 | 位置 |
|---|---|---|
| 探测命令 | 新增 `#[tauri::command] probe_moss_backend(app, device?)` → `BackendProbeReport`：L0 机器能力 + L1 资产 + L2 `info` 实证 + `decide()` 判定 + `recommend_from_assets()` 建议；MOSS 未安装时 `available=false` 并给出「未就绪」原文 | `src-tauri/src/ai_runtime/backend_probe.rs:380-508`（报告结构 382-409、`probe_backend` 460-534、命令 583-595） |
| manifest 过滤 | 新增 `read_manifest_flavors()` / `find_manifest()` / `manifest_candidates()`：按 `runtime/manifest.json` → exe 同目录 → dev 布局 → `resource_dir` 顺序找清单，取 `id=="moss"` 的 flavor；找不到 → `manifest_found=false`（前端退化显示四项） | `backend_probe.rs:420-456`、`597-628` |
| 落盘命令 | 新增 `#[tauri::command] set_moss_device(app, device)` → `MossDeviceSaveResult`；`normalize_moss_device()` 归一（`auto`→`""`、大小写/空白），`RuntimeConfig::load` → 改字段 → `save` 原子写；**load 失败直接报错，不用默认值覆盖坏配置** | `src-tauri/src/ai_runtime/config.rs:344-391`（归一见 356-366） |
| 命令注册 | `invoke_handler` 加两行（用户授权直接改 `lib.rs`，不走补丁） | `src-tauri/src/lib.rs`（`check_asr_engines` 之后） |
| 前端类型/接口 | `MossBackendProbe` / `MossDeviceSaveResult`；store 暴露 `probeMossBackend(device?)` / `setMossDevice(device)` | `src/types/index.ts`、`src/stores/project.ts`（`getAsrEngines` 之后） |
| 设备下拉 | 打轴页「音频获取时间轴」内新增「推理设备」小节：`auto（ggml 选优，推荐）/ CUDA / Vulkan / CPU`；按 `manifest_flavors` 过滤，manifest 缺失时四项全给；已保存值不在清单内时补一条「当前已保存，manifest 未提供」，避免下拉显示空值 | `src/views/AsrView.vue`（`mossOnly` 分块内） |
| 结论与硬报错 | 下拉下方显示 `实测 CUDA0 · <reason>`；显式选了本机不具备的后端 → 红字（`--danger`）+ 保存时 `message.warning`，**仍允许保存**（决策 D2-A：驱动可后装，运行期由 provider 硬拦）；推荐与当前选择不一致时黄字给建议 | `AsrView.vue`（`.enote.danger` / `.enote.warn`） |
| 欢迎页自检卡 | MOSS 卡在 `notready` 时把硬报错原因摊到卡面上（红字），并提示「推理设备可在打轴页「转写参数 → 推理设备」中设置」 | `src/views/Welcome.vue`（`probeAsr()` 追加 `probeMossBackend()`） |

**决策（用户 m00142 全选 A）**：D1-A 下拉落 ASR 页转写参数区；D2-A 允许保存 + 警示 + 运行期硬拦；D3-A **下次启动生效**（不做 `mod.rs` 的即时重置补丁）；D4-A manifest 由 Rust 命令读并回传 flavor 集合（前端不加 fs 插件权限）；D5-A 本期只做 `moss`，`funasr_device` 不动。
（D3-A 与 D5-A 已于 2026-10-29 按用户 m01567 要求反转：即时生效已实现、FunASR 设备已补，见下方补记。）

**2026-10-24 补记（manifest 落库，阶段 C 接通）**

| 子项 | 处理 | 位置 |
|---|---|---|
| 清单落库 | installer 侧生成的真清单（`version:"1"`、`id=moss` × 3：cpu / vulkan / cuda，`dest` 全 `bin`）拷入仓库并随包发布 | `src-tauri/resources/runtime/manifest.json` |
| 打包声明 | `bundle.resources` 加 `"resources": ["resources/runtime/manifest.json"]`；数组形式**保留目录结构** ⇒ 打包后落在 `$RESOURCES/resources/runtime/manifest.json`，不是 `$RESOURCES/runtime/…` | `src-tauri/tauri.conf.json` |
| 查找路径补齐 | `manifest_candidates_for()`（拆成纯函数便于单测）新增 `<exe>/resources/runtime/…` 与 `<resource_dir>/resources/runtime/…` 两条——原先只有 `runtime/…`，**打包形态会读不到清单**；新增 3 个单测分别钉住打包布局、dev 布局、随包清单可解析 | `backend_probe.rs`（`manifest_candidates_for`、`manifest_candidates_find_bundled_layout`、`manifest_candidates_find_dev_layout`、`shipped_manifest_declares_moss_flavors`） |
| 退化分支可见化 | 读不到清单时下拉下方黄字「未读取到安装清单，暂显示全部设备选项」——否则过滤是否生效肉眼看不出来 | `src/views/AsrView.vue`（`manifestMissing`） |
| 入库例外 | 根 `.gitignore` 的 `runtime/`（指用户数据目录）会把 `src-tauri/resources/runtime/` 一起吃掉 ⇒ 清单**静默不入库**；加 `!src-tauri/resources/runtime/` 例外（父目录被排除后无法只放行子文件） | `.gitignore` |

> ⚠ 复测提示：清单到位前前端**一直**走"四项全显示"的退化分支；当前清单声明了三种 flavor，
> 所以正常情况下列表**仍是四项**、与退化分支看起来一样（差别只在有没有那行黄字）。
> 要验证过滤真起作用：临时删掉清单里某个 flavor 条目 → 对应项必须消失。

**2026-10-29 补记（FunASR 设备 + 即时生效 + 真实安装验证；提交 `1fa8a77`）**

用户 m01567 要求「把 12 项遗留填上」，即把 D5-A / D3-A 两条暂缓项做掉、并补一次真实联网验证。

| 子项 | 处理 | 位置 |
|---|---|---|
| FunASR 设备下拉 | 与 MOSS 同一套交互但**只给 CUDA / CPU**（FunASR 走 torch，无 Vulkan 后端）；`set_funasr_device` 归一规则 `""/"auto"/"cuda" → cuda`、`"cpu" → cpu`；切引擎时两块互斥出现，值从 `check_asr_engines` 的 `device` 回填 | `src-tauri/src/ai_runtime/config.rs`（`normalize_funasr_device`、`set_funasr_device`）；`src/views/AsrView.vue`（`FUNASR_DEVICE_OPTIONS`、`onFunasrDeviceChange`） |
| FunASR 不做硬报错 | torch 侧本来就是软回退（`scripts/funasr_worker.py:46 pick_device` 在 CUDA 不可用/缺库时自动落 CPU），硬报错会拦住一个能跑的组合 ⇒ 只在尾注写明「CUDA 不可用时 worker 自动回退 CPU」 | `AsrView.vue`（`FUNASR_DEVICE_DOC`） |
| 配置校验加固 | `RuntimeConfig::validate()` 增加 `funasr_device ∈ {"", cpu, cuda}`，手改 config.json 写进 `vulkan` 会被启动时打回 | `config.rs`（`test_validate_rejects_bad_funasr_device` 钉住） |
| 即时生效（原 D3-A） | `AsrManager` 的 `config` 改为 `RwLock<RuntimeConfig>`、两个 provider 改为 `RwLock<Arc<dyn AsrProvider>>`；新增 `read_config()` / `provider_of()` / `reload_engine(key)`，转写进行中拒绝切换（`"转写进行中，完成后再切换"`），否则重读盘上配置 → `validate` → 只重建该引擎的 provider → 换配置 | `src-tauri/src/ai_runtime/mod.rs`（`build_funasr` / `build_moss` 抽出复用） |
| 命令侧接线 | `set_moss_device` / `set_funasr_device` 均为 `async`，保存后用 `spawn_blocking` 调 `reload_engine`（MOSS 重建要跑 `info` 探测，不能卡命令线程）；reason 返回「已重新加载」或「但不可用：…（重启后生效）」 | `config.rs`（`reload_asr_engine`）；`lib.rs` 注册 `set_funasr_device` |
| 返回类型统一 | `MossDeviceSaveResult` → `DeviceSaveResult`（两个命令共用） | `config.rs`、`src/types/index.ts` |

> 即时生效的边界：改设备**不需要重启**，但只重载被改的那个引擎；正在跑的转写不会被中断（拒绝切换）。
> 旧结论 D3-A「下次启动生效」invalidate，契约 §7 的 P3 补丁项随之落地（见契约 §7、§8.7）。

**验证**（2026-10-29）
- `cargo test --lib` → **325 passed, 0 failed**（较上次 +43：新增归一化/校验单测与即时生效相关单测）；`cargo check --lib` 0 warning。
- `pnpm build`（`vue-tsc --noEmit && vite build`）→ EXIT=0。
- headless 实测 FunASR 块：切引擎后 MOSS 设备块消失、FunASR 设备块出现，下拉值取配置（`cpu`），选项仅 `cuda`/`cpu`，无横向溢出（`temp/css-preview/item12-funasr.png`）。
- **真实联网安装验证**（用户 m01567 要求，直连 GitHub 可用，无需代理）：`scripts/bootstrap_runtime.ps1` 在 RTX 4060 Laptop（算力 8.9）上 `-Flavor auto` **自动选 cuda**、磁盘预检「需要约 1.92 GB」、下载 621.6 MB → 校验 → 解压 14 个文件、`.backend=cuda`、末行 `done ok:true`；`-Flavor vulkan` 全流程同样通过（11 个文件、15.4 MB）。
- `evaluate()` 对真实装好的 vulkan 目录判定 `state=ok`；`precheck_disk_with` 按「最大 flavor」估算（1.92 GB > 单包 622 MB）；脚本走 skip 路径时 stdout 是合法 UTF-8 且每行可 JSON 解析（中文 detail 不丢）。
- 仍**未经真机 UI 手测**（真机上点下拉切后端跑一次转写）。

**未做（明确留口）**：真机 UI 手测（三种后端各跑一次转写 + manifest 过滤验证）；`funasr_device` 的 Vulkan（FunASR 不支持，非缺口）。

**原始背景（保留）**
- MOSS 自动判断（欢迎页卡文案「ggml，有 CUDA/Vulkan 即 GPU」），界面无可配项；配置文件字段已存在：`src-tauri/src/ai_runtime/config.rs` 的 `moss_device`（白名单 `空/cpu/cuda/vulkan`）与 `funasr_device`（默认 `cpu`）——**只缺 UI，不缺存储**。
- PR #48（已并入）交付了判定逻辑与展示字段：`backend_probe::decide()` / `recommend_from_assets()`、`AsrEngineStatus.device` 与 `.effective_backend`、ASR 卡「设备 · …（实测 …）」行；契约与缺口见「并行分支接口」一节。
- **重要实测结论**：双 GPU 机器上裸 `vulkan` 会落到**核显**（`Vulkan0`、`uma:1`），`auto`（空）才落独显（`Vulkan1`、`uma:0`）⇒ UI 默认值必须是「空/auto」，不要引导用户选裸 `vulkan`。
- `src-tauri/src/lib.rs` 已注册 `check_ocr_runtime` / `check_asr_runtime` / `check_asr_engines` / `check_llm_runtime` / `check_embed_runtime`；LLM 与融合无关（`src-tauri/src/fuse/mod.rs:255-301` 只用 `EmbedManager`），故 `llm_device` 暂不需要。
- 历史上还有 `feat/ocr-cuda` 分支——本轮体检：其内容已由 PR #48 覆盖（PR #48 的起点即「MOSS 后端三级探测 + CUDA/Vulkan/CPU fallback 语义」），**不需要再翻它**。

### 13. ✅ 杂项清理与验证

**已实现（2026-10-21，提交 `8b71ee2`）**

| 子项 | 处理 | 验证 |
|---|---|---|
| 死代码 `.preview-header` | `src/components/SourceVideoPreview.vue` 删掉头部「更换剧情录屏/更换切片视频」整块模板、`showHeader` prop 与默认值、`.preview-header` 样式、`NButton` import；`src/views/CorpusView.vue` 去掉 `:show-header="false"` 调用点 | 编辑器页 `.preview-header` = 0；`pnpm build` ✓（模板/样式/props 三处都删干净，无残留引用） |
| OCR 未暴露开关 `punctuation.fix_misread_letters` | 语料页与打轴页的参数区各加一行 `.chkrow`（`.chk` 勾选框 + `InfoDot`），文案新增 `FIX_MISREAD_DOC`（依据 `src-tauri/src/ocr/params.rs:72-74` 与 `correct.rs:60-89`：只修「行尾 j/J 是右括号退化」且本行无其它括号时） | 两页各 1 行、标签「修正行尾 j（引号误识别）」；点击切换后写入 `gsa.ocrParams.corpus` 的 `punctuation.fix_misread_letters`（实测 false/true 往返）；打轴页该行 `inViewport: true`；无溢出 |
| 轨道操作图标常显 | `src/components/timeline/Timeline.vue` 的 `.tl-label-actions`：`opacity 0 → 0.45`、去掉 `pointer-events: none`（行 hover 提亮到 1），与样稿口径一致 | 编辑器页 4 组、计算 `opacity: 0.45`；可直接点击（不再需要先悬停） |
| 波形行真实音频验证 | **未完成**（唯一剩余项） | mock 已用合成峰值验证过渲染与 48px 行高；真实音频与真实 ffmpeg 路径需在真机（`pnpm tauri dev` + 带音频素材）看一次 |

**原始依据（保留）**

- `src/components/SourceVideoPreview.vue` 内建的 `.preview-header`（「更换剧情录屏」/「更换切片视频」按钮）**是死代码**——`src/views/CorpusView.vue` 与 `src/views/AsrView.vue` 两处调用点都传 `:show-header="false"`，可整块删除（顺带删相关样式与 props 分支）。
- OCR 参数里后端已有、界面未暴露的开关（如 `src/composables/ocrDefaults.ts` 的 `punctuation.fix_misread_letters`）→ 评估补 UI（当初 T3 因「不想造假控件」未做）。
- 波形行 `.wf-row`（48px）**从没在有真实波形数据时目视验证过**（mock 无音频）→ 用带音频素材看一眼。
- 轨道操作图标目前 hover 才显示（样稿是常显淡色）→ 按观感定夺，属风格取舍。

### 14. ✅ 内嵌 ffmpeg/ffprobe + 欢迎页 ffmpeg 自检卡

**已实现（2026-10-29，提交 `40e645f` / `5acb56f` / `bfd1168` / `f3d19d8`）**

| 任务 | 改动 | 内容 |
|---|---|---|
| T1 定位层 | 新增 `src-tauri/src/video/ffmpeg_locator.rs` | 内嵌候选（从 exe 目录 `ancestors().take(4)` 逐级拼 `resources/ffmpeg/<name>`）→ 系统 PATH；`runs()` 真跑 `-version`，避免「文件在但跑不动」被骗过 |
| T1 | `video/mod.rs`、`waveform.rs` | `resolve_ffmpeg()`/`resolve_ffprobe()` 变薄包装；`ffmpeg_missing()` 统一 `FFMPEG_NOT_FOUND: …` 文案（前端 `err.includes("FFMPEG_NOT_FOUND")` 依赖此前缀，不可改名） |
| T1 | `src-tauri/Cargo.toml`、`THIRD_PARTY.md` | 删 `ffmpeg-sidecar`（唯一用途 `sidecar_dir()` 恒空；`auto_download()` 从未调用），连带去掉 ureq/tar/xz2/zip |
| T2 取二进制 | 新增 `scripts/fetch_ffmpeg.ps1` | pin `autobuild-2026-10-28-19-47` 的 `win64-lgpl-shared` 包（不可用滚动的 `latest` tag）；缓存 → SHA256 校验 → 只解 `bin/*` 与 `LICENSE.txt` → 落 `src-tauri/resources/ffmpeg/` → 写 `SOURCE.txt` → 跑 `-version` 自检 |
| T2 | `.gitignore`、`tauri.conf.json` | 忽略 `src-tauri/resources/ffmpeg/`（161 MB 不入库）；`bundle.resources` 加 `"resources/ffmpeg/*"` |
| T3 许可与文档 | `THIRD_PARTY.md`、`README(_EN).md`、`docs/development(_EN).md` | 「用户自备」改为「随包内嵌 LGPL 共享构建」，写明为何不用默认 GPL 静态构建、共享构建的 LGPL 义务如何满足 |
| T4 自检卡 | `video/ffmpeg_locator.rs` + `lib.rs` | 新命令 `check_ffmpeg() -> FfmpegStatus { ready, source: "bundled"/"path"/"missing", ffmpeg, ffprobe, version, message }` |
| T4 | `src/views/Welcome.vue`、`src/stores/project.ts`、`src/types/index.ts` | 环境自检加第 4 张卡：就绪时显示「版本 · n8.1.3…（内嵌）」/（系统 PATH），未就绪时摊原因并给 `scripts/fetch_ffmpeg.ps1` 复制命令；`.env` 栅格改 `repeat(auto-fit, minmax(230px, 1fr))` |

**决策（用户 2026-10-29 拍板）**
- **LGPL 共享构建**（`win64-lgpl-shared`），不用默认的 GPL 静态构建——后者会把整个 app 拖进 GPL；共享构建 DLL 可替换，LGPL §4 义务最轻，且我们只当子进程调用、不链接。
- **sidecar 可删**：`ffmpeg-sidecar` 的下载目录永远为空，删依赖换来「内嵌 + PATH 两级查找」。
- **必须真跑一次 `pnpm tauri build`** 验证打包形态（Windows 上 `resource_dir()` = exe 目录 ⇒ 二进制落 `$RESOURCES/resources/ffmpeg/`，与 dev 布局不同）。

**验证**
- `cargo test --lib` → **332 passed, 0 failed**；`cargo check --lib` 0 warning。
- `ffmpeg_locator` 单测含「内嵌优先真的生效」（`bundled_binary_takes_priority`：二进制存在时必须返回绝对路径）与「版本号解析」（`version_parses_from_bundled_binary`）；二进制缺失时这两条自动跳过。
- `pnpm build` → EXIT=0（2901 modules）。
- headless（`temp/css-preview/checks/item14-ffmpeg.js` + `evalq.mjs`，1680×1000）：4 张卡一行排开（`cols = 394px ×4`，`rowCount=1`，`pageOverflow=false`）；bundled 场景 FFmpeg 卡「就绪 / 版本 · n8.1.3-25-g611cec792a-20261028（内嵌）」；`?ffmpeg=missing` 场景「未就绪 / 安装包应自带 resources/ffmpeg/；源码检出请先跑 scripts/fetch_ffmpeg.ps1」并显示该命令；截图 `temp/css-preview/item14-ffmpeg.png`。
- **打包形态验证**（`pnpm tauri build`，EXIT=0）：`src-tauri/target/release/resources/ffmpeg/` 落 11 个文件共 169,051,668 B（含 `ffmpeg.exe` / `ffprobe.exe` / 8 个 av* DLL / `LICENSE.txt` / `SOURCE.txt`），与 `ffmpeg_locator` 的首个候选 `<exe目录>/resources/ffmpeg/` 完全一致；`7z l` 列出 NSIS 安装包内容确认 `resources\ffmpeg\*` 与 `resources\runtime\manifest.json` 都进包（安装包 `GameSubtitleAssistant_0.3.0_x64-setup.exe` 57.3 MB、MSI 75.5 MB）。注意 `target/release/bundle/` 里还留着 `0.1.0` 的旧包，属构建缓存，不是本次产物。

**未做 / 遗留**
- 真机手测（无 ffmpeg 的干净机器上跑元数据/抽帧/波形）留给用户；
- 只覆盖 Windows x64（macOS/Linux 的分发形态未定，`file_name()` 已按平台拼后缀）；
- 二进制不入库 ⇒ 源码检出必须先跑 `scripts/fetch_ffmpeg.ps1`，否则自检卡显示未就绪并回退 PATH。

**原始背景（保留）**

**现状依据**
- 应用**不内嵌** ffmpeg/ffprobe：`src-tauri/src/video/mod.rs:71` `resolve_ffprobe()` 与 `:201` `resolve_ffmpeg()` 都是两级查找——先 `Command::new("ffprobe"/"ffmpeg")`（系统 PATH），再 `ffmpeg_sidecar::paths::sidecar_dir()`（`src-tauri/Cargo.toml` 的 `ffmpeg-sidecar = "2"` 的 auto_download 目录）；找不到即返回 `FFMPEG_NOT_FOUND`（`mod.rs:194/:320/:495/:573`、`src-tauri/src/video/waveform.rs:33`）。
- 调用点：元数据 `probe_video()`（`mod.rs:157`）、抽帧（`:293`）、单帧（`:411`）、dhash 9×8 灰度扫描（`:470`）、音频提取（`:573`）、波形（`waveform.rs:35`）。
- 文档口径：`README.md` / `README_EN.md` 写明「FFmpeg + ffprobe 需自行安装并加入 PATH，应用不会自动下载」；`THIRD_PARTY.md` 已登记 FFmpeg/ffprobe（LGPL-2.1+，构建选项可能致 GPL）与 ffmpeg-sidecar（MIT）。
- `src-tauri/tauri.conf.json` 的 `bundle` 段目前只有 `active` / `targets` / `icon`，**没有 `resources` 也没有 `externalBin`**。

**要做**
- 把 ffmpeg 与 ffprobe 内嵌进 `runtime/`（该目录已被 `.gitignore` ⇒ 二进制不入库，需定「随安装包携带」还是「首次运行下载」的策略，并考虑 Tauri `bundle.resources` / `externalBin`）；
- `resolve_ffmpeg()` / `resolve_ffprobe()` 加一级查找：内嵌目录的位置（**内嵌优先**还是 PATH 优先，实现前与用户对齐——内嵌优先版本可控，PATH 优先便于用户自行升级）；
- 欢迎页加 **ffmpeg 自检卡**（复用 `src/views/Welcome.vue` 现有 `probes` 机制与状态口径：就绪 / 探测中… / 未就绪 / —，不阻塞其它操作）；
- 文档与档案同步：`README.md` / `README_EN.md` 的 FFmpeg 前置说明、`THIRD_PARTY.md` 中「Installed by the user; … never downloads it」的表述都要改（项目「档案同步」惯例）。

**验收要点**
- 一台**没有** ffmpeg 的机器：欢迎页 ffmpeg 卡显示内嵌版本（而非未就绪），元数据/抽帧/波形全部可用；
- 有 PATH ffmpeg 的机器行为不回退（查找顺序明确、可预期）；
- 安装包内包含二进制，且许可档案已更新。

### 19. 设置面板（默认 ASR 模型 + 快捷键查看/修改）

**来源**：用户 m06929（原话）「增加设置面板，详细功能待定，但确定包括的有默认ASR模型设置、快捷键设置（承载快捷键查看和修改两个功能），改动较大，值得单独一个PR」。

**现状依据**
- 没有设置面板/设置路由：`src/AppSidebar.vue` 的 `NAV_ITEMS` 只有四个页面，顶栏（`src/components/AppToolbar.vue`）只有折叠/撤销重做/面包屑/主题/保存/导出。
- ASR 引擎是页面局部状态：`src/views/AsrView.vue` 的引擎 ref 只在本页生效，离开页面即回到默认；后端 `check_asr_engines` 已能给出可选引擎列表（`src/types/index.ts:209` `AsrEngineStatus`）。
- 快捷键此时会集中在 `src/composables/shortcuts.ts` 的注册表（第 10 项产出），但**没有用户覆盖层**。

**要做**
- 新入口：顶栏右侧加「设置」按钮（与 `src/components/AppToolbar.vue` 的主题胶囊并列），内容用 `NModal` 或独立路由页（待定，倾向 Modal，避免打断当前工程）。
- ① **默认 ASR 模型**：从 `projectStore.getAsrEngines()` 取列表渲染选择器，持久化（`localStorage` 或工程配置，待定）；`AsrView.vue` 初始化时读该值而不是硬编码 `"moss"`。
- ② **快捷键查看 + 修改**：读 `src/composables/shortcuts.ts` 注册表；注册表增加「默认键 + 用户覆盖」两层（`resolveKeys(id)`），持久化 `gsa.shortcuts`；改键时做**冲突检测**（同 scope 内重复键提示且不落库）；「查看」视图即原第 10 项计划的帮助面板（分组表格 + `<kbd>` 样式），可搜索。
- ③ 为后续默认项预留分组（语言、输出格式等），本次不实现。

**验收要点**
- 改键即时生效、重启后保留；冲突时提示且不写库；恢复默认可用；
- 设置里的默认模型影响 `AsrView` 的初始引擎，且与 `AppStatusBar.vue` 的引擎段一致；
- 帮助/查看视图按键位分组展示，与注册表同源（不出现「文案与实现不符」）。

**为什么单独一个 PR**：跨页状态 + 持久化 + 冲突检测，与第 10 项的实现面重叠但不属于同一改动单元；用户明确要求单独 PR。

### 20. ✅ 校对区选中 clip 时，轨道总览滚动到该行

**已实现（2026-10-21，提交 `b2e70ab`）**

| 改动 | 内容 |
|---|---|
| `src/components/TrackOverview.vue` | `.overview-body` 加 `ref="bodyRef"`，每行加 `:data-clip-id="clip.id"` |
| 同上 | 新增 `scrollFocusedRow()`：用 `getBoundingClientRect()` 比对行与容器（**不用 `offsetTop`**——`.ov-entry` 的 `offsetParent` 未必是滚动容器），只在行越出可视带时 `scrollTop += 差值 ± 4px` 的最小滚动 |
| 同上 | `watch([() => timeline.focusedClipId, () => timeline.focusedTrackId])` → `await nextTick()` → `scrollFocusedRow()` |

**验证（`temp/css-preview/checks/item20.js`，编辑器页 1680×1000，12 行 / 容器 409px / 行 47.69px）**
- A 从顶部聚焦末行：`scrollTop` 0 → 165，末行可见（贴下沿，4px 余量被最大滚动钳掉）✓
- B 聚焦已在视口内的行：`scrollTop` 保持 165 不变 ✓（最小滚动语义）
- C 容器在底部时聚焦首行：`scrollTop` 回 0，首行贴顶可见 ✓
- D 键盘（`.`）驱动的焦点变化同样跟随（焦点落在第 6 行，可见、未额外滚动）✓；`pageOverflow: false`

**来源**：用户 m06929（「在校对区选中clip时，轨道总览视口跳转至选中的clip」）。

**现状依据**
- `src/components/TrackOverview.vue` 的行列表在 `.overview-body`（`overflow-y: auto`）内，行是 `.ov-entry`；聚焦行已有 `.focused` 样式（inset 强调色 + `::before` 竖条）。
- 选中来源是 `timeline.focusedClipId`（`src/components/timeline/Timeline.vue` 里 `focusClip` 设置），TrackOverview 已有 `focusedTrack` / `clips` computed，但**没有对 focusedClipId 做滚动**。

**要做（原方案，实现时改为矩形比对最小滚动，见上）**
- 行元素带可定位标记（`data-clip-id` 或 `ref` 数组），watch `timeline.focusedClipId` → `nextTick()` 后对目标行 `scrollIntoView({ block: "nearest" })`（**最小滚动**，已在视口内则不动，不要 `center` 打断浏览）。
- 只在「时间轴点选/键盘跳转」时滚；因文本编辑导致的 focus 变化不要抢滚动（编辑中的行本来就可见，`nearest` 天然满足，故无需额外分支）。

**验收要点**
- 点时间轴上任意 clip → 总览滚动到该行（已在视口内不动）；键盘 `,/.` 跳到相邻 clip 时同样滚动；
- 行内编辑/重命名期间滚动不打断输入；无页面溢出。

### 21. ✅ 时间轴框选多选（含 `Ctrl+A` 全选、多选整体位移）

**已实现（2026-10-21，提交 `34e2837`）**

| 改动 | 内容 |
|---|---|
| `src/stores/timeline.ts` | 新增 `selectedClipIds` + `isSelected/setSelection/addToSelection/toggleInSelection/clearSelection`；`focusClip(id)` 语义 = 「主选中 + 选择集重置为单元素」（单击）；不变式：选择集永远包含主选中 |
| `src/components/timeline/Timeline.vue` | 框选状态机 `marquee`（`replace`/`union`/`toggle` 在按下时按修饰键决定，`base` = 按下瞬间快照）；3px 阈值区分点击与框选；命中判定为**矩形相交**；`.marquee` 覆盖层（虚线 + 12% 强调色填充 + `pointer-events:none`）挂在 `.timeline-col`（`position:relative`，不随轨道区滚动）；`onDeleteKey` 改批量删；`onSelectAll`（`Ctrl+A`） |
| `src/components/timeline/TimelineClip.vue` | `selected` prop → `.clip.selected` 内描边（与主选中外框两级可辨）；`collectGroupMembers()` + `moveGroup()`：整组刚性平移，位移只受时间轴两端限制——**组内成员之间的相邻关系不再各自 clamp**（早先按「每成员与同轨邻居取交集」实现，紧贴的一组会在组内互相夹死、交集为空 ⇒ 怎么拖都不动，用户 m07277 判为 bug）；组内吸附（被拖 clip 起止 + 整组外缘，8px 内最近）；拖动未选中 clip 时先把选择集重置为它自己 |
| `src/composables/shortcuts.ts` | 注册 `selectAll`（`Ctrl+A`，`scope: "timeline"`）；`delete` 文案注明多选整批 |
| `src/stores/project.ts` | 新增 `removeEvents(ids)`：批次只记**一次**快照（`removeEvent` 逐条记，撤销会变成 N 次） |

**验证（`temp/css-preview/checks/item21.js`，编辑器页 1680×1000，41 clip / 4 轨）**
- A 无修饰框选 → 替换选择集，6 条选中；覆盖层拖动中出现、松手后消失 ✓
- B 空白单击（<3px）→ 清空选择 ✓（原 seek/清聚焦语义不变）
- C `Shift` 框选 → 并集 6 → 15 ✓
- D `Ctrl` 框选同一片区域 → toggle 15 → 6 ✓
- E `Ctrl+A` → 41/41 全选 ✓
- F `Delete` → 全删；**一次 `Ctrl+Z` 全部复原** ✓
- G 框选「每行最后一条 clip」后拖动其中一条 → 三条位移均为 **+27.49px**（刚性一致；差值来自吸附/阈值，不是各走各的）✓
- H **紧贴相邻的一对**（`gap 0`）框选后拖第一条 → 两条位移均为 **+43.19px**、未选中的第三条不动 ✓（返工前此处位移为 0，即用户报的「拖不动」）
- `pageOverflow: false`；`pnpm build` ✓ 2900+ 模块 / CSS 59.83 kB / JS 670.30 kB

**已知取舍（用户已确认可接受）**：整组位移不再被未选中的邻居挡住 ⇒ 可以把组压到别的 clip 上（数据层本就允许同轨重叠，撤销即可复位）。换来的是「抓住组内任何一条都能带动整组」这一可预期行为；若日后要「推到邻居就停」或「推开邻居」，那是 ripple 语义，另立条目。

**本次未做**（边界明确）：多选分割、多选合并、多选拖起止（resize）——只做选择、`Ctrl+A`、批量删除与整组整体位移。

**来源**：用户 m06929（原话）「各时间轴在「选择」组件选中状态下，在时间轴区空白处左键并按住拖动时，以左键落地处以及光标拖动处作为两处对角锚点，生成范围选择矩形框，矩形框经过的clip均会被选中（多选功能），在按下Shift开始拖动的情况下，已被选中的clip不会失焦；在按下Ctrl开始拖动的情况下，选区框语义变为toggle（已被选中的clip且没有被再次框选的clip不会失焦，但如果选区框再次经过已被框中的clip，则失焦）」。

**现状依据**
- 选择模型是单值：`src/stores/timeline.ts` 的 `focusedClipId` / `focusedTrackId`；`src/components/timeline/TimelineClip.vue` 只渲染单个 `.focused`。
- `.tl-track-body` 上已有 mousedown 处理（`Timeline.vue` 的 `onTrackAreaMouseDown`：加选区草稿 / 空白点击 seek / 合并工具），框选要接进同一条路径且不互相抢。
- 没有任何多选先例 ⇒ 选中态、操作语义（删除/分割/拖动）都要重新定义。

**要做**
- **选择模型升级**：`selectedClipIds: Set<string>`（集合内即高亮），保留一个「主选中」用于轨道总览定位与文本编辑（可由最后点中的那个派生）；渲染区分「选中」与「主选中」两级。
- **框选交互**：选择工具下在轨道空白处左键拖动，以落点与当前光标为对角生成矩形（视觉：虚线描边 + 半透明填充，与加选区工具的 `.region-draft` 明确区分）；**拖动距离 < 3px 视为点击（照旧 seek）**，≥3px 才进入框选；松手时**与矩形相交**的 clip 全部入选项（用户口径是「经过即选中」，不是「完全包含」）。
- **修饰键**（在按下时决定语义）：`Shift` = 并集（原有选择保留）；`Ctrl` = toggle（框到已选中的取消选中，未框到的保留）；无修饰 = 替换。
- **操作语义**（先只定这两条，其余后续再定）：`Delete` 删除全部选中（一次快照）；`Ctrl+A` 选中活动时间轴内所有可见轨道的 clip（依赖第 10 项的「活动时间轴」判定，故本项排在 10 之后）。
- `Ctrl+A` **随本项一起做**，不单独提前实现——多选模型未立时它没有作用对象，先加会是空壳。

**验收要点**
- 框选/Shift 并集/Ctrl toggle 三种语义正确；不拖（<3px）仍 seek；与加选区工具、拖起止、吸附、分割态互不干扰；
- `Ctrl+A` 只作用于活动时间轴（校对区与迷你时间轴分别验证）；`Delete` 多删只产生一次撤销；
- 迷你时间轴同样支持框选（它复用同一个 `Timeline.vue`）。

---

## 附：实现期发现的小问题

### 22. ✅ `,` / `.`（上/下一个片段）与「轨道总览 = 聚焦轨道」口径对齐

**已实现（2026-10-22，`b28c87a`）**

**做法（用户拍板：主路径同轨 + 补丁 a/b/c 全做）**

| 决策 | 处理 |
|---|---|
| 主路径 | `,`/`.` 只在**聚焦轨道**内跳 ⇒ 播放头 / 聚焦 clip / 选区遮罩 / 总览高亮全在同一轨，与总览口径一致 |
| 补丁 a | 无聚焦轨道（或聚焦轨暂无片段）时兜底**跨轨**找最近片段，并把目标轨 `focusTrack` 成聚焦轨（一次性建立上下文，不改主路径语义） |
| 补丁 b | 到首/尾时 `message.info("已是第一条 / 最后一条片段")`，不再静默 `return`（不静默乱跳） |
| 补丁 c | `timelineRole === "mini"` 直接不响应：语料页/转写页的迷你时间轴只含 `ocr_region` 选区控制轨，「片段」语义不适用（此前是**碰巧**恒为空操作，现为明确设计） |

| 改动 | 内容 |
|---|---|
| `src/components/timeline/Timeline.vue` | 重写 `onPrevNextClip`：候选集 = 聚焦轨的非 `ocr_region` 事件（无聚焦轨时 = 本时间轴全部轨的事件），按 `start` 升序后取 `currentTime ± 0.01s` 外最近者；命中后 `seekWithScroll` + `focusClip`，兜底路径额外 `focusTrack` 到目标轨；边界走 `message.info` |
| `src/composables/shortcuts.ts` | `prevNextClip` 文案改「上 / 下一个片段（当前轨道）」，与新语义对齐（该表是帮助面板/设置面板的文案来源） |

**未做**：跨轨连续浏览（如需，先点另一条轨道行再按）；`Shift`/`Ctrl` 修饰的跨轨变体（本期不引入新键位）。

**原现状依据（保留）**
- `src/components/timeline/Timeline.vue` 的 `onPrevNextClip` 原先把**全部轨道**的事件合并排序，再按 `currentTime` 找前后最近的 clip，然后 `focusClip`（**不改 `focusedTrackId`**）。
- `src/components/TrackOverview.vue` 只列「聚焦轨道」的 clip ⇒ 跳段后聚焦的 clip 常在别的轨道上，总览里既无高亮也不滚动，用户观感是「按了跳段没反应」；同时选区遮罩（`focusTrack` 决定）仍留在旧轨，总览标题也仍是旧轨名——三处状态互相打架。
- 次生问题：候选池混进技术轨 `embed_ocr`（嵌字轴，`scope: "output"`，供融合消费），跨轨跳常落到用户根本不校对的轨；边界 `if (!target) return;` 完全静默。

**验收要点**
- 在任一轨道上按 `,/.`：播放头与聚焦都落在同一轨道；总览高亮并滚动到该行（第 20 项的 `scrollFocusedRow` 机制已就绪，`watch(focusedClipId/focusedTrackId)`）；
- 目标轨道无更多片段时给出可感知的边界行为（不静默乱跳）；
- 无聚焦轨道时按 `.`：跳到最近片段并自动把该轨设为聚焦轨（总览随之出现内容）；
- 语料页/转写页迷你时间轴按 `,/.` 无反应、不报错。

