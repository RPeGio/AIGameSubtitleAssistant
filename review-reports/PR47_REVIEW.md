# PR #47 审查记录

- **PR**：`refactor(style): 重做项目前端样式与布局，优化部分操作体验`
- **分支**：`style/CSS-optimization` → `dev`（15 提交，2026-10-17 ~ 10-18）
- **审查日期**：2026-10-18
- **审查者**：15 年资深工程师视角，严重度映射 🔴→P0 / 🟡→P1 / 🟢→P2

## 审查范围

32 文件（+3799/−2072），纯前端：设计令牌与明暗主题（`style.css` 令牌层 + `stores/ui.ts`）、
三段式外壳（AppSidebar/AppToolbar/AppStatusBar/ProjectLayout）、四页重写
（Welcome/CorpusView/AsrView/FuseView）、时间轴皮肤、共用播放头与音频互斥、
OCR 选区迁移（RegionOverlay 抽件）、吸附/迷你时间轴修复、user-select 策略。
**Rust 后端零改动。**

## 总评

**无 P0。** 这类"样式重构夹带行为修复"的 PR 最怕两件事：行为悄悄丢失、行为悄悄变形。
逐一核对结果：**没有发现行为丢失**——Editor 页的撤销/重做/导出工具条完整迁入全局 AppToolbar
（cleanupFocus 语义保留）；OCR 六参数面板 + 术语表 + 语料 diff 审批流在两页重写后绑定完好；
FuseView 的 unmatch 掩码开关、审计四动作、就绪探测全部保留。行为修复本身质量高：
共用播放头的音频互斥（claimPlayback/让权 watch）、RegionOverlay 的 contentRect 几何换算
（修 letterbox 系统性偏移，注释把因果写透）、迷你时间轴时长改从工程元数据派生（修"预览
删掉后 duration 恒 0"的自伤回归）都有真实验证记录。

发现 2 条 P2 与若干观察项，无阻塞合并项。

## 逐条发现

### P2-1（PR43 遗留，本审查中已顺手修）`check_fuse_audit.mjs` 的自述转译命令与 import 路径不一致

- **位置**：`scripts/check_fuse_audit.mjs:3-5,19`
- **问题**：脚本头注释给出的转译命令**照抄必失败**——`fuseAudit.ts` 的 `import type ... from "../types"`
  会把 `src/types/index.ts` 拉进编译程序，rootDir 推断上浮到 `src/`，tsc 实际产出
  `temp/stores/fuseAudit.js`，而脚本的 import 写的是 `../temp/fuseAudit.js` ⇒
  复现者直接 `ERR_MODULE_NOT_FOUND`（本审查真实踩中，T4 工作树当时是手工摆过 temp 才过的）。
- **处置（已在分支就地修复）**：import 改为 `../temp/stores/fuseAudit.js` + 头注释说明 rootDir
  上浮原因。实测：转译 → `node scripts/check_fuse_audit.mjs` → **30 断言全过**。

### P2-2 共用播放头的"同一文件"前提未被代码固定，依赖运行时巧合

- **位置**：`src/stores/timeline.ts`（`activePlayer`）、`VideoPlayer.vue`（PLAYER_ID="review"）、
  `SourceVideoPreview.vue`（`syncPlayback`）
- **问题**：互斥机制防止的是"**同一支切片视频**两处同时出声/双解码"。校对区播放器的 src 取
  `currentVideoMeta`（ReviewPane.vue:93），打轴页预览也取 `currentVideoMeta`——当前**恰好同文件**，
  所以共用一份 `currentTime` 是对的。但 `currentVideoMeta` 是"当前工作视频"这个**可变**概念
  （用户在语料页可切到剧情录屏，`loadVideoMeta` 会改写它并持久化），而 `VideoPlayer` 的身份
  硬编码为 "review"。若某天校对区改为常显切片、或工程 primary video 指向录屏，共用播放头
  会在**两支不同时长的视频**间共享时间轴（duration 各写各的、currentTime 超出短视频时长被
  浏览器钳制），表现为"校对区一加载打轴页预览就跳时间"，很难排查。
- **建议**：不加代码（当前行为正确），但建议在 `VideoPlayer.vue` 的 PLAYER_ID 注释处补一句
  **前提声明**："共享播放头成立的前提 = 校对区与打轴页预览加载同一支视频
  （同一 currentVideoMeta）；两者若变成不同文件，共用 currentTime 即为错误"，
  并在计划文档 6.2 的迷你时间轴待办区登记该前提。

### P2-3（观察项，不要求修）`ParamStepper` 约束与旧输入框不一致 + 硬编码上限散落

- **位置**：`src/components/ParamStepper.vue` + 两页用法
- **说明**：新步进器带硬编码 `min/max`（`min_subtitle_sec` max=5、`frame_interval` min=0.1、
  `batch_size` max=128 等）。这些界与旧 `NInputNumber` 的界未必逐项相同（旧界我未逐项比对），
  超出范围的**历史持久化参数**（localStorage 里用户手调过的值）不会被钳——只在下一次
  步进/编辑时钳制，属良性。留意点仅一个：`min_subtitle_sec` 的 5 上限是否覆盖未来素材
  （vesna 案例 p5=0.43 走下沿没问题；上限方向暂时无素材需求）。

### P2-4（观察项，不要求修）PR 描述为空

- **说明**：15 个提交的提交信息写得很完整（根因/验证/否决记录齐全），但 PR body 一行没有。
  本仓库的 PR 描述一直是"验收门 + 部署注意"的载体（PR42/43 均如此）；合并前建议把提交信息
  汇总成 body，尤其"验证方式"与"环境自检探针依赖 check_* 命令"两点。

## 已核对无误的点（抽查清单）

| 检查项 | 结论 |
|---|---|
| Editor.vue 减掉的撤销/重做/导出：完整迁入全局 `AppToolbar.vue`（cleanupFocus 逻辑逐行保留；`export_track_subtitle` 调用迁到 AppToolbar:68，TrackOverview 的另一处为 dev 既有） | ✓ |
| 音频互斥闭环：`claimPlayback`（play 时自报）→ 两侧 `watch(activePlayer)` 让权暂停；语料页 `syncPlayback=false` 不参与（独立时间轴，符合"语料页仍绑剧情录屏"设计） | ✓ |
| 共用播放头双向同步：VideoPlayer `watch(timeline.currentTime)` 外部跳转回写视频（isSeeking + 0.02s 防回环），rAF 采样解决 4Hz timeupdate 抖动；AsrView `clipTime` computed get/set 直连 store 且注释了为何不用 `timeline.seek()`（isSeeking 死锁） | ✓ |
| OCR 选区：`RegionOverlay` 归一化坐标 × `useVideoContentRect` 画面矩形（修 letterbox 百分比偏移）；move 整体平移钳位、resize 按手柄钳位（MIN=0.02 防翻转）；转写页选区只在校对区画一次（`showClipRegion` = asr 页 + 焦点在 main）；压暗遮罩以**选区矩形**为界（16:9 铺满容器时画面矩形四周≈0 的坑已注明） | ✓ |
| 选区轨过滤 `type + video` 与 dev 旧实现相同（无 page 过滤不是本 PR 引入的回归） | ✓ 对比 dev `SourceVideoPreview:53` |
| 吸附修复：`snapEvents` = 本时间轴实际渲染事件（`tracks()` 按 trackFilter 过滤），两页迷你时间轴不再跨源互吸、不吸字幕轨边界；自身原位边界保留 | ✓ `Timeline.vue:108-112` |
| 迷你时间轴时长：`clipDuration`/`sourceDuration` 改由 `currentVideoMeta`/`sourceVideoMeta` 派生（computed，唯一来源），预览改 `:duration` 单向——修掉"预览删掉后恒 0 → 滚动/缩放/双击加选区全失效"的同 PR 自伤 | ✓ |
| OCR 参数面板：两页重写后 `usePersistedOcrParams` 六参数（frame_interval/dhash/batch/merge_similarity/min_subtitle_sec/min_text_length）+ 术语表 + 语料 diff 审批流绑定齐全；`ParamStepper` 有 NaN 防护（`Number.isFinite`）与 min/max 钳位 | ✓ |
| 主题：令牌层 `:root, [data-theme="dark"]` 默认深色 + `[data-theme="light"]` 覆盖；`main.ts` 挂载前写 `<html data-theme>` 防首帧闪色（pinia 在 `app.use(createPinia())` 后 active，store 调用合法）；naive-ui 侧 `darkTheme ? : null` + themeOverrides 同步换色；残留硬编码色均为令牌定义本身/彩色底上的白字/白勾（两主题下均成立） | ✓ |
| user-select：`html,body` none + 表单控件与 `.selectable` 放行；`.selectable` 已标注在语料文本、文件路径、总览字幕等内容位置（CorpusView 5 处、ReviewPane 1 处）；FuseView 审计行 0 处——审计文本目前不可复制，见观察项说明 | ✓（可选改进） |
| vite dev watch 忽略 `.tmpdir` 原子写目录（EBUSY 退出的根因注释清楚）；AppIcon sprite 统一图标体系（TimelineToolStrip 仅图标替换，工具逻辑未动） | ✓ |
| Welcome 环境自检三卡：只探针不阻塞（`check_ocr/check_asr/check_embed_runtime` 均为 dev 既有命令，dev 已含 PR43 合并） | ✓ |
| `FuseView` 行为保留：unmatch 掩码开关、`canFuse` 三就绪门、审计四动作与全语料弹窗（v-model 数值绑定改自绘 checkbox/按钮，语义等价） | ✓ |

## 验证（本审查独立执行）

- `pnpm run build`（vue-tsc + vite）：**通过**（34.4s）
- `node scripts/check_fuse_audit.mjs`（转译修正后）：**30 断言全过**
- `cargo test` 侧不受影响（本 PR 未触碰 Rust）——抽验未跑全套，理由：diff 中无 src-tauri 文件
- 手工 GUI 验证（主题切换、拖选区、共用播放头出声互斥）依据提交信息中的实测记录采信，本环境无法做交互验证

## 结论

**建议合并。** P2-1 已在本分支顺手修复（转译路径对齐，30 断言复跑通过）；P2-2 建议合并前
在 `VideoPlayer.vue` 补一行前提声明注释（或合并后随计划待办处理）；其余观察项随用随记。

## 处置记录（2026-10-18，用户裁决）

- **P2-1 ✅**：`check_fuse_audit.mjs` import 改 `../temp/stores/fuseAudit.js` + 头注释说明 rootDir
  上浮原因；30 断言复跑通过。
- **P2-2 ✅**：`VideoPlayer.vue` 的 `PLAYER_ID` 注释补前提声明（共用播放头成立前提 = 校对区与
  打轴页预览同载 `currentVideoMeta`；前提被破坏时的两个出路：独立播放头 / 时长来源绑定）。
- **P2-3 ✅（用户裁决，不改）**：`min_subtitle_sec` 上限 5 目前够用；且用户已**人工确认**
  ParamStepper 的 min/max 与旧 NInputNumber 逐项相同——约束差异的疑虑撤销。
- **P2-4 ✅**：PR #47 描述已补（汇总各提交信息 + 验收门 + 部署/前提注意）。
- 处置提交号待用户手动提交后回填。
