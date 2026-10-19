# 杂项功能与修复待办（feat/misc-fixes）

> 记录时间：2026-10-18。分支：`feat/misc-fixes`（自 dev `7b8e306` 拉出，尚未 push）。
> 本文件是**备忘**，不是承诺：顺序可按需要调整；已完成项就地标 ✅ 并写提交号（沿用项目「档案同步」惯例）。
> 每条给出现状依据（`文件:行`）、要做什么、验收要点。**实现每个待办前先给方案、经用户确认再动手**（遵循 AGENTS.md 一轮一任务）。
> 以后新待办继续往下追加，不重排已完成项。

## 顺序总览

| # | 任务 | 类型 | 依赖 | 为什么在这个位置 |
|---|---|---|---|---|
| 1 | ✅ 切页丢状态（含波形重绘） | 修复·底层 | 无 | 已实现（提交号待补）：KeepAlive 保活页面与校对区 + 全局监听改 activate/deactivate |
| 2 | ✅ 播放头跳转后画面/字幕停在上一帧 | 修复 | 无 | 已实现（提交号待补）：命中判定留一帧容差 + 跳转/擦洗两种 API 分开 |
| 3 | ✅ 标尺分度随视口拥挤 | 修复 | 无 | 已实现（提交号待补）：按最小像素间距自适应，删 30s 封顶阶梯 |
| 4 | ✅ 标尺滚轮缩放 | 功能·小 | 与 3 同区 | 已实现（提交号待补）：标尺滚轮缩放（锚点=光标处），与滑条一致 |
| 5 | ✅ 播放键/进度条移出画面 | 修复·布局 | 无 | 已实现（提交号待补）：控件条改流内 + 画面按 16:9 自定高，字幕回到画面底 |
| 6 | 语料页语料列表增删改 | 功能 | 无 | 页面级独立高频功能 |
| 7 | OCR 参数 info 图标 + 悬浮解释 | 功能·基建 | 无 | 基建先落地，第 12 项新参数可自带说明 |
| 8 | 「分割/合并」倒三角 + 策略小面板 | 功能·核心 | 无 | 时间轴功能扩展第一块，**先与用户对齐策略细节** |
| 9 | 迷你时间轴（复用第 8 项成果） | 功能 | 8 | 用户明确要求：主时间轴功能定稿后再复用 |
| 10 | 快捷键体系（调研 → 注册表 → 帮助面板） | 功能·跨页 | 8、9 | 键位必须绑在定稿交互上，先做必返工 |
| 11 | 欢迎页自检升级（探针 + 静默 bootstrap） | 功能·独立 | 无（可与 8–10 并行） | 独立大件，放中后段不打断时间轴主线 |
| 12 | CUDA GPU 加速参数化 | 功能·Rust+UI | 7（要带 info 说明）；先看 `feat/ocr-cuda` | 需先勘察既有分支，避免重复造 |
| 13 | 杂项清理与验证（死代码/参数开关/波形/图标） | 清理·零碎 | 无 | 随时可插入的零碎项，放最后一批 |
| 14 | 内嵌 ffmpeg/ffprobe + 欢迎页 ffmpeg 自检卡 | 功能·打包 | 11（自检卡复用其探针机制） | 外部依赖一致性；涉及打包策略，宜与 11 一起看 |

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
- 带着长视频从其它页切回语料页：无可见重绘停顿。

### 2. ✅ 播放头跳转到 clip start 后画面仍为前一帧／CC 字幕闪一下就没

**已实现（2026-10-18，提交号待补）**——两条根因各按方案改掉：

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

**已实现（2026-10-19，提交号待补）** —— `src/components/timeline/TimelineRuler.vue` 删掉 pps 分档阶梯，改为按像素密度自适应：
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

**已实现（2026-10-19，提交号待补）** —— `src/components/timeline/Timeline.vue` 的 `onWheelRoot` 最前面新增分支：`t.closest(".ruler")` 时 `preventDefault()` + `timeline.zoom(e.deltaY < 0 ? 1.1 : 0.9, e.clientX - contentRect.left)` 后 return（锚点取标尺内容区左缘，与滑条同一约定）；轨道区滚轮、`Shift+滚轮` 垂直滚动、`.tool-strip`/`.tl-label-col` 忽略等既有行为不变。

**验证（`item34.js` / `item34b.js`）**
- 标尺滚轮放大：pps 0.98 → 1.08（factor 1.1），刻度间距 58.89 → 64.78px ✓。
- **锚点保真**：鼠标停在内容区 60% 处连滚 12 档，光标处对应时间 133.83 → 133.81s（漂移 **−0.01s**）✓。
- 轨道区滚轮：pps 不变、播放头左移（895.67 → 859.17）⇒ 仍是平移 ✓。


### 5. ✅ 校对区播放键与进度条下移到画面外

**已实现（2026-10-19，提交号待补）**

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

### 6. 语料页语料列表增删改

**现状依据**
- store 只有新增与删除：`addCorpusItem(text, source = "paste")`（`src/stores/project.ts:827`）、`removeCorpusItem(id)`（`:839`），**没有更新函数**（`pushCorpusTexts`（`:797`）是批量追加、`writeOcrToCorpus`（`:822`）走 OCR 产物）。
- 列表行在 `src/views/CorpusView.vue` 的 `.clist > .crow`，目前只有序号、文本、来源胶囊、删除按钮。

**要做**
- store 新增 `updateCorpusItem(id, text)`（含 `recordSnapshot()`，与其它编辑一致，保证 Ctrl+Z 可回退）。
- 行内编辑：双击或点编辑按钮进入输入态，Enter 提交 / Esc 取消 / 失焦提交（可与 `TrackOverview.vue` 的既有编辑交互保持一致）。
- 手动新增一行（走 `addCorpusItem`，来源 `paste`）。

**验收**
- 改字后落盘（`save_project`）、撤销可回退、来源胶囊与序号不错位、编辑态不触发行点击跳转。

### 7. OCR 各参数加 info 小图标 + 悬浮详细解释

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

**现状依据**
- 工具按钮：`src/components/timeline/TimelineToolStrip.vue` 的 5 个 `.tool-btn`（24×22，点击即 `setTool(...)`）。
- 分割语义：`src/stores/project.ts:528` 的 `splitEvent(eventId, time)`——`{ ...event }` 继承全部字段（含 `x1/y1/x2/y2`）、`EPS = 0.05` 防 sliver、`recordSnapshot()`。
- 合并语义：`src/stores/project.ts:1303` 的 `mergeTwo(idA, idB, requireAdjacent)`——**文本拼接**（`first.text + "\n" + second.text`）并删除后者，明确拒绝 `ocr_region`（`:1310`）；`mergeAdjacent`（`:1328`）同理。

**要做**
- 按钮**稍稍放大** + 右下角倒三角；**已选中状态下再次点击**才展开小面板（首次点击仍是切工具）。
- 分割面板三档：`按断句标点分割`（默认）、`按空格分割`、`保留原文分割`（现状）。
- 合并面板三档：`保留后者文本`（默认）、`保留前者文本`、`前后文本堆叠`（现状）。
- 策略存 `timeline` store（或按项目惯例持久化到 `gsa.*`），`splitEvent` / `mergeTwo` 接受策略参数。

**待与用户讨论（实现前）**
- 断句标点集合（中英标点表）、找不到标点时如何回退（按空格？整体切成两段？拒绝？）；
- 「合并」三档对时间轴与 `fuse_meta` 的影响（尤其被丢弃一侧的 `source_text` / 候选信息）；
- 策略是否需要跨会话记住。

**验收**
- 两工具 × 三档策略各自生效；**默认档与现状逐字一致**（回归）；每次操作都能 Ctrl+Z 回退。

### 9. 迷你时间轴（复用第 8 项成果）

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

---

## D 组：跨页面体验与外部一致性

### 10. 收集主流视频编辑软件通用快捷键并应用

**现状依据（散落三处）**
- `src/layouts/ProjectLayout.vue`：Ctrl+S 保存 / Ctrl+Z 撤销 / Ctrl+Shift+Z、Ctrl+Y 重做。
- `src/components/timeline/Timeline.vue:378-411`：Delete 删除聚焦 clip、S 在播放头处分割、M 合并相邻（**每个 Timeline 实例各绑一次**）。
- `src/components/VideoPlayer.vue` 的 `handleKeydown`（播放器自身的按键）。

**要做**
- 先产出调研对照表：主流 NLE（Premiere Pro / DaVinci Resolve / Vegas / Subtitle Edit / Aegisub）的通用键位 → 与我们功能集的「对齐 / 保留 / 冲突」三栏；
- 收敛成一份**快捷键注册表**（集中定义、冲突检测、可选帮助面板），再逐个替换现有散落绑定。

**为什么放这里**：第 8、9 项会改分割/合并语义与迷你时间轴交互，先定键位必返工。

**待与用户讨论**：`Space` 播放/暂停、`J/K/L` 变速、`I/O` 入出点、方向键逐帧与现有 `S`/`M`/`Delete` 的冲突取舍。

### 11. 欢迎页模型自检升级（探针 + 未就绪静默 bootstrap）

**现状依据**
- 现在只做探针：`src/views/Welcome.vue` 的 `probeOcr()`（`invoke("check_ocr_runtime")`）、`probeAsr()`（`projectStore.getAsrEngines()` 取 `moss`）、`probeEmbed()`（`invoke("check_embed_runtime")`），三者不 await、未就绪只显示状态、不阻塞。

**要做**
- 探测到未就绪时**自动静默跑 bootstrap**（覆盖首次启动完全没有任何环境的情况）；
- 自检**不阻塞**其它操作，只在真正运行该引擎时才阻拦；
- 自检卡上显示进度与失败原因。

**依赖/前置**：需要 Rust 侧配合（bootstrap 脚本/清单、进度事件）；先勘察仓库现有 `bootstrap_*.ps1` 与模型清单，产出方案后再动手。

**顺序说明**：与 C 组无依赖，可与 8–10 并行；放中后段是为了不打断时间轴主线。

### 12. CUDA GPU 加速参数化

**现状依据**
- MOSS 目前是自动判断（欢迎页卡文案「ggml，有 CUDA/Vulkan 即 GPU」），界面无可配项；配置在 `src-tauri/src/ai_runtime/config.rs`（`moss_binary` / `moss_model` / `embed_model` / `model_dir` 等）。
- **仓库已存在 `feat/ocr-cuda` 分支**——先勘察它做过什么，避免重复造。

**要做**
- 把设备选择（自动 / CUDA / Vulkan / CPU）与相关参数暴露成可配置项，前端可调、后端落盘；
- 新增参数**必须同时带 info 说明**（复用第 7 项的基建）。

### 13. 杂项清理与验证

- `src/components/SourceVideoPreview.vue` 内建的 `.preview-header`（「更换剧情录屏」/「更换切片视频」按钮）**是死代码**——`src/views/CorpusView.vue` 与 `src/views/AsrView.vue` 两处调用点都传 `:show-header="false"`，可整块删除（顺带删相关样式与 props 分支）。
- OCR 参数里后端已有、界面未暴露的开关（如 `src/composables/ocrDefaults.ts` 的 `punctuation.fix_misread_letters`）→ 评估补 UI（当初 T3 因「不想造假控件」未做）。
- 波形行 `.wf-row`（48px）**从没在有真实波形数据时目视验证过**（mock 无音频）→ 用带音频素材看一眼。
- 轨道操作图标目前 hover 才显示（样稿是常显淡色）→ 按观感定夺，属风格取舍。

### 14. 内嵌 ffmpeg/ffprobe + 欢迎页 ffmpeg 自检卡

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
