# PR #52 审查记录

- **PR**：`feat: 设置面板与运行时目录搬家（第19项）+ 真机两轮反馈修复`
- **分支**：`feat/first-run-prep` → `dev`（8 提交，2026-11-02 ~ 11-15）
- **审查日期**：2026-11-16
- **审查者**：15 年资深工程师视角，严重度映射 🔴→P0 / 🟡→P1 / 🟢→P2
- **审查范围**：`git diff 9cae08c e260e6b6`，29 文件（+2075/−108）

## 审查范围

| 提交 | 内容 |
|---|---|
| `cc9a32a` | 设置面板（第19项）：`SettingsModal.vue` + `stores/settings.ts` + `composables/appPrefs.ts` + `ui.rs` |
| `2f97aa2` | 搬家后管理器读不到新目录（`resolve_runtime_dir` 语义修正）+ 缩放改 WebView zoom |
| `aca4210` | 语料行内编辑反复全选、轨道溢出无法滚动、运行中进度行挤压、搬家后设置存不进去 |
| `1e84e3f` | 轨道区真正可滚动（`min-height:0`）、运行中进度与按钮分行、默认解释器路径 |
| `42f78b6` | 尾部动作区整体贴底，运行中多出的进度行向上生长 |
| `0025211` | 滚轮纵向滚动只作用于标签列，clip 区恢复水平平移 |
| `216c66f` | 补记 2026-11-07 真机四处修复与配置校验拆分口径 |
| `e260e6b` | 修掉共用临时目录导致的偶发失败 |

后端核心是 `ai_runtime/config.rs`（+717/−29，现 1600 行）：`RuntimeLocation` 指针文件、
`resolve_runtime_dir` 语义、`validate` 拆成 `validate_schema`/`validate_paths`、
搬家/删旧目录/日志目录/设置快照 5 个新命令；前端是设置面板与偏好抽取。

## 总评

**有 1 条 P0 阻塞合并，4 条 P1 建议本 PR 内处理。**

> **更新（2026-11-16）**：P0-1 已在本分支修复并复测。当前 P0 清零，
> 剩余 4 条 P1 + 23 条 P2 按原优先级跟进。下方总评保留发现当时的判断。

结构性设计是好的：`RuntimeConfig` 的 `download_source/segment_mb/setup_auto` 三字段与面板一一对应；
指针文件先写 `.tmp` 再 `rename`（真原子）；`load_or_default` 只在 `config.json` **不存在**时才兜默认、
坏了仍然报错（这一点踩过 2026-11-02 真机「改任何一项都失败」的坑，注释把因果写透了）；
`validate` 拆成 schema/paths 两半，让「面板能存」与「能跑」解耦，方向正确；
`SettingsSnapshot/SettingsPatch/MoveResult` 与 `src/types/index.ts` 逐字段对齐，无漂移。

但这一 PR 里有一处**典型的 PowerShell 大小写不敏感踩坑**，把上一版本还在工作的镜像兜底
整条断掉了，而且断得无声无息（不报错、不告警，只是候选少一条）——这正是最有价值的一类审查发现。
搬家路径的失败语义也与两份设计文档自己写下的口径相反（「失败回滚」「半搬状态比不搬更糟」）。

## 逐条发现

### 🔴 P0-1 `-Mirror` 参数被同函数局部变量覆盖，镜像兜底从此永远失效 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P0-1，提交号见该条）。
> 下方保留发现时的原始取证，行号对应修复前（`$mirror` 现为 `$mirrorCands`）。

- **位置**：`scripts/bootstrap_runtime.ps1:404`（配合 `:56` 参数、`:405`、`:414`、`:416`）
- **问题**：PowerShell 变量名**大小写不敏感**，`$mirror` 与参数 `$Mirror` 是同一个变量。
  `:404 $mirror = New-Object System.Collections.ArrayList` 直接把 `:56` 的
  `[string]$Mirror = "https://gh-proxy.com/"` 覆盖成空 ArrayList，紧接着 `:405 if ($it.url -and $Mirror)`
  判定为空 ⇒ **镜像候选从不加入**；`:414/@($mirror)`、`:416/@($mirror)` 也都拿到空集合。
  三个 `-Source` 档位全部受影响。
  基线 `9cae08c` 里是没有这个 bug 的（`:392-407` 只有单个 `$list`，`:398 if ($Mirror) {...}`）——
  是本次为引入 `-Source` 三档而重写该函数时引入的回归。
- **实测复现**（用真实 `powershell.exe` 5.1.26100.9818 抽出 BASE/HEAD 的 `Resolve-Url`+`Get-UrlCandidates`，
  喂真实的 `src-tauri/resources/runtime/manifest.json` 条目 `moss`）：

  | 版本/档位 | 候选数 | 内容 |
  |---|---|---|
  | BASE（任意档） | **3** | `[cdn] dl.rpegio.top` · `[cdn] github.com` · `[mirror] gh-proxy.com/...` |
  | HEAD `auto` | 2 | 只剩两条 cdn，**无 mirror** |
  | HEAD `mirror` | 2 | 有 mirror 但**不排第一**（该档位的全部意义消失） |
  | HEAD `direct` | 1 | 只剩 `[cdn] github.com` 一条 |

  函数内实测：`type=[ArrayList] bool=[False] str=[]`，返回后参数恢复为 `https://gh-proxy.com/`。
- **影响**：DL 站与 GitHub 双双不可达、只有 gh-proxy 能通的网络（国内直连 Release 的典型场景）
  **完全无法安装运行时**；而本 PR 新增的「强制镜像」设置项
  （`SettingsModal.vue` → `config.rs:784` → `runtime_install.rs:730 -Source`）是个**静默空操作**，
  用户以为换了源，实际候选表没变。`docs/RUNTIME_BOOTSTRAP_CONTRACT.md:742-744` 承诺的行为代码已不再实现。
- **修复建议**：局部变量改名（`$mirrorCands` / `$directCands`），并在 `scripts/` 侧加一条
  「每个档位的候选数与首项」冒烟断言，防止再次被静默改坏。
- **修复后代码**：

  ```powershell
  function Get-UrlCandidates($it) {
    $cdn = New-Object System.Collections.ArrayList
    foreach ($u in @($it.urls)) {
      if ($u) { [void]$cdn.Add([pscustomobject]@{ Url = [string]$u; Kind = "cdn" }) }
    }
    # 局部变量禁止再用 $mirror / $Mirror 之类的名字：PowerShell 变量名大小写不敏感，
    # 会直接覆盖同名参数（PR#52 回归：镜像候选永远为空）
    $mirrorCands = New-Object System.Collections.ArrayList
    if ($it.url -and $Mirror) {
      [void]$mirrorCands.Add([pscustomobject]@{ Url = (Resolve-Url ([string]$it.url)); Kind = "mirror" })
    }
    $directCands = New-Object System.Collections.ArrayList
    if ($it.url) { [void]$directCands.Add([pscustomobject]@{ Url = [string]$it.url; Kind = "direct" }) }

    $list = switch ($Source) {
      "mirror" { @($mirrorCands) + @($cdn) + @($directCands) }
      "direct" { @(@($cdn) | Where-Object { $_.Url -match "://github\.com/" }) + @($directCands) }
      default  { @($cdn) + @($mirrorCands) + @($directCands) }
    }
    ...
  }
  ```

  ```powershell
  # 冒烟断言（scripts/ 下任意检查脚本里都可挂）
  foreach ($src in @("auto", "mirror", "direct")) {
    $c = Get-UrlCandidates $item -Source $src
    if (@($c | Where-Object Kind -eq "mirror").Count -eq 0) { throw "档位 $src 没有镜像候选（参数被覆盖？）" }
  }
  ```

### 🟡 P1-1 「侧栏默认折叠」开关写的是实时键，`gsa.sidebarDefault` 从 UI 永远设不上

- **位置**：`src/components/SettingsModal.vue:159-171`、`src/stores/ui.ts:21-25 / 57-70`
- **问题**：开关标签是「侧栏默认折叠」，InfoDot 明写「下次启动时的初始状态；**本次**已展开/折叠的状态
  不受影响」，但模板绑的是 `:value="ui.sidebarCollapsed"` / `@update:value="ui.setSidebarCollapsed"` ——
  即实时键 `gsa.sidebar`。真正的写入函数 `setSidebarDefaultCollapsed`（`ui.ts:67-70`）**全仓库零调用方**
  （只在 store 的定义与 return 里出现），`readSidebarDefault` 也仅被 `readSidebarCollapsed` 的
  `raw === null` 分支消费。后果三重：①点开关**立即**折叠/展开侧栏，与自己 InfoDot 的承诺相反；
  ②`gsa.sidebarDefault` 永远存不进；③用户只要手动折叠过一次，`gsa.sidebar` 就落盘了，
  之后「默认」设置永久失效（`ui.ts:22-24` 只在 raw 为 null 时才读默认值）。
  这是「设置项存在但功能不存在」，比缺功能更糟。
- **修复建议**：store 暴露独立的 `sidebarDefault` 响应式值，开关只写默认值；顺带在弹窗打开时同步一次。
- **修复后代码**：

  ```ts
  // src/stores/ui.ts
  const sidebarDefault = ref(readSidebarDefault());          // 新增
  function setSidebarDefaultCollapsed(collapsed: boolean) {
    sidebarDefault.value = collapsed;                        // 弹窗里立刻反映
    setSidebarDefault(collapsed);                            // 只写 gsa.sidebarDefault
  }
  return { ..., sidebarDefault, setSidebarDefaultCollapsed };
  ```

  ```vue
  <!-- src/components/SettingsModal.vue:165-169 -->
  <NSwitch
    :value="ui.sidebarDefault"
    size="small"
    @update:value="ui.setSidebarDefaultCollapsed"
  />
  ```

### 🟡 P1-2 搬家复制不完整时留下半搬目录，而提示里的「可重试」必然失败

- **位置**：`src-tauri/src/ai_runtime/config.rs:897-910`（`create_dir_all` 在 `:900`，对账失败在 `:906-910`）
- **问题**：`:900` 已把 `target` 建出来，`:902` 开始拷；`:906` 对账缺文件时直接
  `return Err("复制不完整：目标目录缺少 {missing}（未改动指针，可重试）")`，**不清理**已拷的部分。
  用户按提示重试时，`:897 if target.is_dir() && dir_entry_count(target) > 0` 立刻回
  「目标目录非空」 —— 唯一出路是手工去资源管理器删掉那半个目录（而且删错就有丢数据风险）。
  这与设计文档自己的口径直接冲突：`docs/SETTINGS_PANEL_SCOPE.md` §3.4「失败回滚」、
  `docs/RUNTIME_BOOTSTRAP_CONTRACT.md:721-722`「失败不写指针（**半搬状态比不搬更糟**）」。
- **修复建议**：只在「本次是自己新建的空目录」这种前提下回滚；指针与旧目录一律不动。
- **修复后代码**：

  ```rust
  let created = !target.is_dir();                    // 记录是不是我们建的
  fs::create_dir_all(target).map_err(|e| format!("创建目标目录失败: {e}"))?;
  let bytes = match copy_dir_all(&current, target) {
      Ok(n) => n,
      Err(e) => {
          if created { let _ = fs::remove_dir_all(target); }   // 回滚，别留半搬状态
          return Err(format!("复制失败: {e}（已回滚，旧目录与指针未改动）"));
      }
  };
  if let Some(missing) = src_paths.iter().find(|p| !dst_paths.contains(*p)) {
      if created { let _ = fs::remove_dir_all(target); }
      return Err(format!("复制不完整：目标目录缺少 {missing}（已回滚，旧目录与指针未改动）"));
  }
  ```

### 🟡 P1-3 新目录不存在时「删除旧目录」不可达，且报错信息误导

- **位置**：`src-tauri/src/ai_runtime/config.rs:934`（`read_location` 的 `is_dir()` 检查在 `:234-246`）
- **问题**：`delete_old_runtime_dir_impl` 第一行用 `read_location(app_data)`，而 `read_location`
  内要求**指针指向的目录当前存在**（`dir.is_dir()`，否则打印「外接盘未挂载？」返回 None）。
  于是「移动硬盘拔了 / 新目录被误删」这一确切场景下，清理旧目录的路径**永远不可达**，
  用户看到的是 `"没有搬家记录（指针文件缺失）"` —— 一句话把两种完全不同的病因混成了一个，
  而且这句话是假的：指针文件好端端在那儿。
  这正是 `docs/MISC_TODO.md` 里记下的真机事故（把 `D:\__GSA_RUMTIME` 打成 `D:\_GSA_RUNTIME`，
  指针悬空+旧目录待删）所需要的那个入口。
- **修复建议**：读指针时改用不校验目录存在性的 `read_location_raw`，把「新目录是否在」只用于提示。
- **修复后代码**：

  ```rust
  pub(crate) fn delete_old_runtime_dir_impl(app_data: &Path) -> Result<String, String> {
      // 这里要的是「记录里写了什么」，不是「那个目录还在不在」：
      // 新目录所在盘拔掉时，用户更需要能清掉旧目录
      let mut loc = read_location_raw(app_data)
          .ok_or_else(|| "没有搬家记录（指针文件缺失）".to_string())?;
      let new_dir_missing = !PathBuf::from(&loc.runtime_dir).is_dir();
      let previous = loc.previous.clone().ok_or_else(|| "没有待删除的旧目录".to_string())?;
      // …（后略，逻辑不变；结束时若 new_dir_missing 可在返回串里附带一句提示）
  }
  ```

### 🟡 P1-4 clip 区普通滚轮不再纵向滚动（能力被删，只因另一处修复才变得可见）

- **位置**：`src/components/timeline/Timeline.vue:575-617`（`onWheelRoot`）、`:1124-1133`（`min-height:0`）
- **问题**：`0025211` 删掉了基线里的这段：

  ```ts
  if (canScrollTracks && t.closest(".tl-tracks")) {
    if (e.shiftKey) { /* pan */ }
    if (Math.abs(e.deltaY) >= Math.abs(e.deltaX)) { return; }   // ← 纵向交还原生滚动
  }
  ```

  之后 `.tl-tracks` 上的**普通滚轮纵向滚动消失，只剩 Shift+滚轮**。要注意这条回归的隐蔽性：
  基线 `.timeline-col` **没有** `min-height:0`，`.tl-tracks` 实际上从不溢出，那条分支本来也是死的；
  正是本 PR 的 `min-height:0` 修复让轨道「真的能滚了」，同时也让这条丢失的能力第一次可被感知。
  提交信息写的是「clip 区仍是水平平移」，属于**有意的设计取舍**，但对用户是能力回退
  （大工程纵览时 Shift 是隐式手势）。
- **修复建议**：二选一并写清：①恢复 `.tl-tracks` 上的原生纵向滚动（推荐，横向平移交给 Shift+滚轮和
  滚动条，符合轨道编辑器惯例）；②保留现状，但必须在 `docs/MISC_TODO.md` 或面板快捷键表里显式登记
  「纵向滚动需按 Shift」，否则下一个人只会当成 bug 又改回去。
- **修复后代码**（方案①）：

  ```ts
  if (t.closest(".tl-tracks")) {
    if (e.shiftKey) {                       // Shift+滚轮 = 水平平移
      e.preventDefault();
      timeline.pan(e.deltaY + e.deltaX);
      return;
    }
    // 普通滚轮：不拦截，交给 .tl-tracks 的 overflow-y:auto 原生滚动
    return;
  }
  ```

### 🟢 P2-1 `OcrManager::reload()` 换掉了 provider，却没更新 `self.config`，`dev_debug` 改不动

- **位置**：`src-tauri/src/ai_runtime/mod.rs:384-410`、`:341-343`；消费方 `src-tauri/src/ocr/pipeline.rs:240`、`src-tauri/src/asr/mod.rs:90`
- **问题**：`reload()` 从磁盘重读配置构造新 provider，但**没有** `self.config = config`
  （对照 `AsrManager::reload_engine` `mod.rs:941-962`，它是 `*self.config.write() = config`）。
  于是面板里改 `dev_debug`（或 `ocr_device`）→ provider 重建了 → 但
  `manager.config().dev_debug` 仍是旧值，OCR/ASR 流水线继续按老开关跑，**要到重启才生效**，
  与面板「改这里立即生效」的承诺不符。
- **修复建议**：`reload` 用 `Mutex<RwLock>` 或直接把 config 也放进锁里一起替换。
- **修复后代码**：

  ```rust
  pub fn reload(&self) -> Result<(), String> {
      let config = RuntimeConfig::load(&self.runtime_dir).unwrap_or_else(|_| self.config.clone());
      let provider = build_provider(&config)?;
      *self.provider.lock().unwrap_or_else(|p| p.into_inner()) = provider;
      // 关键：config 必须一起换，否则 config().dev_debug 永远是旧值
      self.apply_config(config);          // 内部用 RwLock<RuntimeConfig> 替换
      Ok(())
  }
  ```

### 🟢 P2-2 同一个 patch 接口里，有的字段静默钳位、有的字段硬报错

- **位置**：`src-tauri/src/ai_runtime/config.rs:775`、`:778`（钳位） vs `:784`、`:787`（报错）
- **问题**：`moss_threads = v.min(256)`、`moss_timeout_minutes = v.min(24*60)` 静默截断，
  用户填 999 也不知道被改成 256；而 `download_source`、`segment_mb` 超范围直接 `Err`。
  同一个命令两种契约，前端也无法统一提示。
- **修复建议**：统一为「报错 + 有用文案」，或统一为「钳位 + 返回值里带上实际生效值」。
- **修复后代码**：

  ```rust
  // 统一成显式校验：超范围就告诉用户，不静默改
  fn normalize_threads(raw: u32) -> Result<u32, String> {
      if raw == 0 || raw > 256 {
          return Err(format!("moss_threads 取值非法: {raw}（1–256）"));
      }
      Ok(raw)
  }
  ```

### 🟢 P2-3 `normalize_ocr_device` 没有白名单，非法设备串会被照存

- **位置**：`src-tauri/src/ai_runtime/config.rs:689-691`
- **问题**：只做 `trim().to_ascii_lowercase()`；对照 `validate_schema` 里 `moss_device`（`:426-434`）
  与 `funasr_device`（`:439-444`）都有白名单。手改 `config.json` 填 `gpu:7` 会存盘成功，
  直到 OCR 真正跑起来才静默退回 CPU——用户以为在用 GPU。
- **修复建议**：允许空/`cpu`/`gpu:<n>` 形态，其余报错。
- **修复后代码**：

  ```rust
  fn normalize_ocr_device(raw: &str) -> Result<String, String> {
      let v = raw.trim().to_ascii_lowercase();
      if v.is_empty() || v == "cpu" {
          return Ok(v);
      }
      match v.strip_prefix("gpu:") {
          Some(n) if n.parse::<u32>().is_ok() => Ok(v),
          _ => Err(format!("ocr_device 取值非法: {raw}（可选 cpu / gpu:0 / 空）")),
      }
  }
  ```

### 🟢 P2-4 「重新检查运行时」把用户踢出主界面，且不带任何新信息

- **位置**：`src/components/SettingsModal.vue:121-124`、`src/stores/setup.ts:100-104`、`src/App.vue:30`
- **问题**：`onRecheckRuntime` 只做 `setup.reopen(); show.value = false;`，而 `reopen()` 只是把
  `stage` 置 `"needs"`，**不调用** `runtime_status`，`status/items/missing_required` 全是旧值。
  `stage="needs"` 又让 `passable`（`setup.ts:24`）变 false，`App.vue:30 <router-view v-if="setup.passable">`
  直接把整个主界面卸载 —— 用户点一下「重新检查」，界面消失、被丢回准备页、看到的还是旧数据。
- **修复建议**：先 `await refresh()`，再按结果决定是否 `reopen()`。
- **修复后代码**：

  ```ts
  async function onRecheckRuntime() {
    await setup.refresh();                       // 真正重新探测
    if (setup.stage === "needs") {
      show.value = false;                        // 只在确实缺东西时才回准备页
      setup.reopen();
    } else {
      message.success("运行时状态已更新");
    }
  }
  ```

### 🟢 P2-5 快照加载失败时面板照常渲染「默认值」，会把没装误报成未找到清单

- **位置**：`src/stores/settings.ts:31-42`（写 `error`） vs `src/components/SettingsModal.vue:143 / :192-193 / :279-281`
- **问题**：`load()` 失败只写 `error.value` + `console.error`，而模板只消费 `settings.loading`。
  `snap` 为 null 时：`runtime_dir` 显示「—」、`segment_mb` 回落显示 64、`setup_auto` 显示「开启」，
  最糟的是 `:279-281` 的 `snap?.manifest_found ? ... : "未找到 manifest.json"` —— **把「未知」渲染成
  「未找到」**，足以引导用户去重装运行时。
- **修复建议**：错误态显式渲染 + 重试按钮；关键分组 gate 在 `snap` 上。
- **修复后代码**：

  ```vue
  <p v-if="settings.loading" class="set-loading">正在读取运行时设置…</p>
  <div v-else-if="settings.error" class="set-row">
    <span class="sv warn-line">读取设置失败：{{ settings.error }}</span>
    <NButton size="small" @click="settings.load()">重试</NButton>
  </div>
  <template v-else-if="snap">
    <!-- B 运行时 / C 引擎 两组放进来 -->
  </template>
  ```

### 🟢 P2-6 设置面板打开时全局快捷键仍然生效，Delete 能删掉被遮住的片段

- **位置**：`src/layouts/ProjectLayout.vue:75-78`、`src/composables/shortcuts.ts:105-128`、`src/components/timeline/Timeline.vue:843-845`
- **问题**：`isEditableTarget`（`shortcuts.ts:124-128`）只排除 INPUT/TEXTAREA/contentEditable，
  焦点落在 NSwitch/NButton 这类 `div` 上时一律放行；`isActiveTimeline`（`:105-107`）在
  `hoveredTimeline` 为 null 时回落到 main 时间轴，而 NModal 是 portal，`Timeline.isAttached()` 仍为 true。
  于是面板开着时：Delete 删片段、Ctrl+Z/Y 撤销整个工程、Space 连带切换视频播放、
  Esc 既关面板又执行 `exitSplitPair`。vueuc 的 focus-trap 只处理 Escape，不 `stopPropagation`。
- **修复建议**：加一个抑制计数，弹窗打开期间短路 `dispatchShortcut`。
- **修复后代码**：

  ```ts
  // src/composables/shortcuts.ts
  let suppressDepth = 0;
  export function suppressShortcuts(): () => void {
    suppressDepth++;
    return () => { suppressDepth = Math.max(0, suppressDepth - 1); };
  }
  export function dispatchShortcut(e: KeyboardEvent): boolean {
    if (suppressDepth > 0) return false;          // 弹窗/模态期间整段失效
    ...
  }
  ```

  ```ts
  // SettingsModal.vue
  let release: (() => void) | null = null;
  watch(show, (on) => {
    if (on) { release = suppressShortcuts(); void settings.load(); }
    else { release?.(); release = null; }
  });
  onUnmounted(() => release?.());
  ```

### 🟢 P2-7 「删除旧目录」确认按钮忽略 loading，双击会连发两次删除

- **位置**：`src/components/SettingsModal.vue:210-220`、`src/stores/settings.ts:74-83`
- **问题**：`:loading` 只作用于 NPopconfirm 的 trigger（`:215`），确认按钮不吃这个 prop；
  双击确认 ⇒ 第二个请求拿到 `Err("没有搬家记录（指针文件缺失）")`（删完后 `previous` 已清），
  用户先看到成功再看到红色失败。且 `saving` 是单个 bool，第一个 `finally` 就把按钮解锁了。
- **修复建议**：入口处做重入保护 + 把 loading 透传给确认按钮。
- **修复后代码**：

  ```ts
  async function onDeleteOld() {
    if (settings.saving) return;                 // 重入保护
    ...
  }
  ```

  ```vue
  <NPopconfirm
    :positive-button-props="{ loading: settings.saving, disabled: settings.saving }"
    @positive-click="onDeleteOld"
  >
  ```

### 🟢 P2-8 settings store 的 `load`/`patch` 没有序号，慢的快照会覆盖新的

- **位置**：`src/stores/settings.ts:31-61`
- **问题**：`runtime_settings` 走 `spawn_blocking` 且内含一次 `powershell Get-PSDrive`
  （`config.rs:717`），完全可能比随后一次 `update_runtime_settings` 更晚返回；
  晚到的旧快照会把新值刷回去，表现为「改了又跳回来」。
- **修复建议**：请求序号比较后丢弃过期响应。
- **修复后代码**：

  ```ts
  let seq = 0;
  async function load() {
    const mine = ++seq;
    const s = await invoke<RuntimeSettingsSnapshot>("runtime_settings");
    if (mine !== seq) return;                    // 有更新的请求，丢弃这次
    snapshot.value = s;
  }
  async function patch(p: RuntimeSettingsPatch) {
    seq++;                                       // 让在途 load 作废
    ...
  }
  ```

### 🟢 P2-9 「重新检查运行时」「打开日志目录」没有 loading/disabled

- **位置**：`src/components/SettingsModal.vue:272-273`
- **问题**：连点会开多个资源管理器窗口；P2-4 修好后还会并发多次 `runtime_status`。
- **修复建议**：加最小 loading 态。
- **修复后代码**：

  ```vue
  <NButton size="small" :loading="rechecking" :disabled="rechecking" @click="onRecheckRuntime">
    重新检查运行时
  </NButton>
  ```

### 🟢 P2-10 `onMove` 的目录选择在 try 之外；`res.ok` 的 else 分支是死代码

- **位置**：`src/components/SettingsModal.vue:92-102`；`src-tauri/src/ai_runtime/config.rs:919-929`
- **问题**：①`open({directory:true})` 写在 `try` 外，插件异常会变成未捕获拒绝，与「用户取消」无法区分；
  ②后端只在成功时构造 `MoveResult{ ok: true, ... }`，失败一律 `Err`，所以
  `if (res.ok) ... else message.error(...)` 的 else 永不执行 —— 读代码的人会误以为存在 `ok:false` 路径。
- **修复建议**：`open` 移进 try；删掉死分支（或在后端真的返回 `ok:false`）。
- **修复后代码**：

  ```ts
  async function onMove() {
    try {
      const picked = await open({ directory: true, title: "选择新的 runtime 目录" });
      if (typeof picked !== "string" || !picked) return;   // 用户取消
      const res = await settings.moveRuntimeDir(picked);
      message.success(res.message);                        // 失败走 catch
    } catch (e) {
      message.error(`移动失败：${e}`);
    }
  }
  ```

### 🟢 P2-11 缩放先落盘后应用，失败时 UI、localStorage 与真实缩放三者不一致

- **位置**：`src/composables/appPrefs.ts:44-56`、`src/stores/ui.ts:62-65`
- **问题**：`setZoom` 先写 localStorage 再 `invoke`，`applyZoom` 内部把异常吞成 `console.warn`。
  主窗口拿不到时：下拉框显示 1.1×、localStorage 存着 1.1、WebView 实际还是 1.0。
- **修复建议**：先应用，成功后再持久化。
- **修复后代码**：

  ```ts
  export async function applyZoom(zoom: number): Promise<boolean> {
    try { await invoke("set_ui_zoom", { scale: zoom }); return true; }
    catch (e) { console.warn("设置界面缩放失败:", e); return false; }
  }
  // 调用侧
  async function setZoom(next: number) {
    if (!(await applyZoom(next))) return;        // 失败就不改 UI、不落盘
    zoom.value = next;
    setZoomPref(next);
  }
  ```

### 🟢 P2-12 搬家对账的三处不一致：`.downloads` 深度、读目录失败静默、无大小校验

- **位置**：`src-tauri/src/ai_runtime/config.rs:818-842`（`copy_dir_all`）、`:851-874`（`rel_paths`）
- **问题**：
  ①`copy_dir_all` 在**任意层级**跳过名为 `.downloads` 的目录，`rel_paths` 只在 `dir == root` 时跳过
  （目前 `.downloads` 只出现在根，暂无实害，但这是埋着的雷）；
  ②`rel_paths` 用 `let Ok(entries) = fs::read_dir(dir) else { return };` **吞掉读目录失败**，
  而它正是用来对账的 —— 源侧读不动某目录时，对账会「通过」，恰恰漏掉了没拷成的部分；
  ③对账只比相对路径**集合**，不比大小/sha，而 `docs/SETTINGS_PANEL_SCOPE.md` §3.3 明确要求
  「逐文件比对大小（大文件可只比大小 + 抽查 sha）」；
  ④整棵树被遍历 3 次（拷 1 次 + 两次 `rel_paths`），几 GB 的运行时是纯浪费。
- **修复建议**：统一跳过规则、读失败向上传播、对账改成「拷贝时顺带收集 (rel, size)」一次遍历完成。
- **修复后代码**：

  ```rust
  // 拷贝和对账共用同一份清单，一次遍历、读失败不静默
  fn copy_dir_all(src: &Path, dst: &Path, out: &mut Vec<(String, u64)>) -> Result<u64, String> {
      let mut bytes = 0u64;
      for entry in fs::read_dir(src).map_err(|e| format!("读取 {} 失败: {e}", src.display()))? {
          let entry = entry.map_err(|e| e.to_string())?;
          let name = entry.file_name();
          if name == ".downloads" { continue; }        // 任意层级统一跳过
          let p = entry.path();
          let rel = p.strip_prefix(src).unwrap_or(&p).to_string_lossy().to_string();
          if p.is_dir() {
              fs::create_dir_all(dst.join(&name)).map_err(|e| e.to_string())?;
              bytes += copy_dir_all(&p, &dst.join(&name), out)?;
          } else {
              fs::copy(&p, dst.join(&name)).map_err(|e| e.to_string())?;
              let len = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
              out.push((rel, len));
              bytes += len;
          }
      }
      Ok(bytes)
  }
  ```

### 🟢 P2-13 搬家前不校验目标盘剩余空间

- **位置**：`src-tauri/src/ai_runtime/config.rs:877-900`；`free_bytes` 仅在 `:717` 用于展示
- **问题**：面板上「可用 xx GB」只是显示。源目录 9 GB、目标盘只剩 4 GB 时，会拷到一半失败，
  叠加 P1-2 就是「留下半个目录 + 无法重试」。这类失败最好在动手之前就拦住。
- **修复建议**：拷前用已有的 `query_free_bytes` 对比源目录体积，留 10% 余量。
- **修复后代码**：

  ```rust
  let need = dir_size(&current)?;                                  // 已有的遍历顺手算
  if let Some(free) = super::runtime_install::query_free_bytes(target) {
      if free < need + need / 10 {
          return Err(format!(
              "目标盘可用 {:.1} GB，搬过去约需 {:.1} GB（含 10% 余量），请换一个位置",
              free as f64 / GB, (need + need / 10) as f64 / GB
          ));
      }
  }
  ```

### 🟢 P2-14 设计文档里「禁止目标是 %APPDATA% 子路径」没有实现

- **位置**：`docs/SETTINGS_PANEL_SCOPE.md` §3.5 vs `src-tauri/src/ai_runtime/config.rs:882-899`
- **问题**：实现只挡了「当前 runtime 内部/上级/盘符根/非空」四种，允许把运行时搬进
  `%APPDATA%\...\runtime` 之外的同盘任意位置，包括与自身递归相关的落点（如 app_data 下另一个子目录）。
  文档承诺的护栏不存在——文档比代码严格，属于「承诺未兑现」。
- **修复建议**：补一条包含性检查，或把文档改成与实现一致并写明原因。
- **修复后代码**：

  ```rust
  if normalize_lexical(target).starts_with(normalize_lexical(app_data)) {
      return Err(format!(
          "目标目录不能放在应用数据目录内部: {}（会与日志/指针文件互相干扰）",
          app_data.display()
      ));
  }
  ```

### 🟢 P2-15 删旧目录完全信任指针文件里的 `previous`，没有落点白名单

- **位置**：`src-tauri/src/ai_runtime/config.rs:934-949`
- **问题**：唯一的护栏是 `normalize_lexical(&old) != normalize_lexical(&current)`（`:941`），
  随后 `fs::remove_dir_all(&old)`（`:949`）。手动把 `runtime-location.json` 的 `previous` 改成
  `C:\Windows` 或某个工程目录，点「删除旧目录」就是不可撤销的全盘删除，
  界面上的「此操作不可撤销」是唯一提示。
- **修复建议**：只允许删「像 runtime 的目录」（含 `config.json` 或本身是某次搬家的来源），
  并要求 `previous` 与 `current` 同属一类根（例如同盘）。
- **修复后代码**：

  ```rust
  // 只删确认是 runtime 的目录：至少要有 config.json，或目录名是 runtime
  let looks_like_runtime = old.join("config.json").is_file()
      || old.file_name().map(|n| n.to_string_lossy().to_lowercase().contains("runtime")).unwrap_or(false);
  if !looks_like_runtime {
      return Err(format!("{previous} 看起来不是 runtime 目录，拒绝删除（请手动确认）"));
  }
  ```

### 🟢 P2-16 `update_runtime_settings` 在异步线程上做阻塞 I/O 与 powershell 调用

- **位置**：`src-tauri/src/ai_runtime/config.rs:814` vs 同文件 `:739-752`（`runtime_settings` 用了 `spawn_blocking`）
- **问题**：返回前要 `find_manifest` + `query_free_bytes`（`powershell -Command Get-PSDrive`），
  这些都在 async 命令体里同步执行，会占住 tokio worker；同一个仓库的只读分支却规规矩矩用了
  `spawn_blocking`。每次改设置都要付一次进程启动成本。
- **修复建议**：把快照构造一并挪进 `spawn_blocking`。
- **修复后代码**：

  ```rust
  let snap = tauri::async_runtime::spawn_blocking(move || {
      settings_snapshot(&app2, &runtime_dir, &app_data, &config)
  }).await.map_err(|e| format!("读取设置快照失败: {e}"))?;
  Ok(snap)
  ```

### 🟢 P2-17 安装时 `config.json` 损坏会静默退回默认下载源/分段

- **位置**：`src-tauri/src/ai_runtime/runtime_install.rs:724`
- **问题**：`RuntimeConfig::load(&runtime_dir).unwrap_or_default()` —— 配置坏了不报错，
  用户设的 `download_source`/`segment_mb` 被静默换成默认值，安装行为与面板显示不一致。
  同文件其它地方（`config.rs:336-341`）刻意区分了「缺失」与「损坏」，这里把两者又合回去了。
- **修复建议**：损坏就明确告知，或按「缺失」语义处理并在日志里落一条。
- **修复后代码**：

  ```rust
  let cfg = match super::RuntimeConfig::load(&runtime_dir) {
      Ok(c) => c,
      Err(e) => {
          eprintln!("[ai_runtime] 读取配置失败，本次按默认参数安装: {e}");
          super::RuntimeConfig::default()
      }
  };
  ```

### 🟢 P2-18 前端可选集合比后端允许范围窄，非枚举值会显示成「选不回去」的原始值

- **位置**：`src/components/SettingsModal.vue:41-52`、`:313`、`:326`
- **问题**：面板只给 `segment_mb ∈ {0,16,64,256}`、`ocr_device ∈ {"",cpu,gpu:0}`、
  `moss_threads ≤ 32`、`moss_timeout ≤ 120`；后端接受 `segment_mb ≤ 1024`、任意 `gpu:n`、
  `threads ≤ 256`、`timeout ≤ 1440`。手改配置文件或换机器后，面板显示一个不在列表里的值，
  用户一旦切换就再也选不回来。另外 `:313/:326` 是内联 `.map()` 字面量，每次 patch 重建数组，
  与模块级 `SEGMENT_OPTIONS` 风格不一致。
- **修复建议**：当前值不在候选里时动态补一项「自定义 <值>」。
- **修复后代码**：

  ```ts
  const MOSS_THREAD_OPTIONS = [0, 4, 8, 12, 16, 24, 32].map(v => ({ label: `${v}`, value: v }));
  function withCustom(opts, cur) {
    return opts.some(o => o.value === cur) ? opts : [...opts, { label: `自定义 ${cur}`, value: cur }];
  }
  ```

### 🟢 P2-19 「吸附」在会话内切换不落盘；关掉自动保存后状态条永远显示「未保存」

- **位置**：`src/stores/timeline.ts:243-246`、`src/stores/project.ts:274-283`
- **问题**：①`toggleSnap` 只改内存值，`readSnapDefault()` 仅决定初值，用户这次关掉吸附、重启又回来；
  而同文件的 `setSplitStrategy/setMergeStrategy` 都写盘，行为不一致。
  ②自动保存关掉时 `scheduleSave` 早退并置 `"pending"`，之后再无任何路径把它改回 `"saved"`，
  状态条长期停在「未保存」——用户分不清「确实没存」还是「只是没开自动存」。
- **修复建议**：`toggleSnap` 顺带写偏好；关自动保存时状态条显示中性的「自动保存已关闭」。
- **修复后代码**：

  ```ts
  function toggleSnap() {
    snapEnabled.value = !snapEnabled.value;
    snapGuideX.value = null;
    setSnapDefault(snapEnabled.value);        // 与 splitStrategy/mergeStrategy 对齐
  }
  ```

  ```ts
  // project.ts:scheduleSave
  if (!readAutosaveEnabled()) { saveState.value = "manual"; return; }  // 新增中性态
  ```

### 🟢 P2-20 `PREF_KEYS` 里 splitStrategy/mergeStrategy 重复定义且无人使用

- **位置**：`src/composables/appPrefs.ts:14-15` vs `src/stores/timeline.ts:43-51`
- **问题**：两处各写一份 `"gsa.timeline.splitStrategy"` 字符串常量，面板/时间轴改任一侧漏改另一侧就是静默不生效。
- **修复建议**：`timeline.ts` 改用 `PREF_KEYS`，删掉本地常量。
- **修复后代码**：

  ```ts
  import { PREF_KEYS } from "../composables/appPrefs";
  const SPLIT_KEY = PREF_KEYS.splitStrategy;
  const MERGE_KEY = PREF_KEYS.mergeStrategy;
  ```

### 🟢 P2-21 死类名、缺 `min-width:0`、store 模块求值期副作用

- **位置**：`src/views/AsrView.vue:530 / :536`（`.arow-acts` 全仓库无 CSS 规则）、
  `:642-645`（`.arow-msg .enote` 无 `min-width:0`）、`src/stores/ui.ts:72-73`
- **问题**：①`.arow-acts` 是纯装饰性类名，看着像有布局规则，实际没有（两个模板命中，零 CSS 命中）；
  ②`.enote` 作为 flex item 缺 `min-width:0`，超长 Windows 路径会在 `.accb`（`style.css:329-337`
  `overflow-y:auto` 使 `overflow-x` 计算为 auto）里撑出横向滚动条；
  ③`ui.ts` 在 store 定义体里直接 `document.documentElement.dataset.theme = ...` 和
  `void applyZoom(zoom.value)`，既绑死 DOM 环境，又在默认缩放时也白跑一次 IPC。
- **修复建议**：删掉死类名或补规则；补 `min-width:0`；把副作用收进显式 `initUiShell()`。
- **修复后代码**：

  ```css
  .arow-msg .enote { line-height: 1.5; word-break: break-word; min-width: 0; }
  ```

  ```ts
  // app 入口
  onMounted(() => { ui.initUiShell(); });   // 内部才写 dataset.theme / applyZoom
  ```

### 🟢 P2-22 两个 PowerShell 脚本的健壮性小口子

- **位置**：`scripts/verify_cdn_mirror.ps1:119`、`scripts/bootstrap_runtime.ps1:767 / :804`
- **问题**：①`$ErrorActionPreference` 的恢复在正常路径上，中途 `throw` 会把全局偏好留在 `Continue`；
  ②`Remove-Item $tmpDir -Recurse -Force` 未用 `-LiteralPath`，也未校验 `$tmpDir` 落在
  `.downloads` 之内（同文件 `:696/:702` 是正确写法）。今天不可利用（7 个 manifest id 都是静态字面量），
  但这是最容易在「以后加一条清单项」时变成真问题的地方。
- **修复建议**：恢复动作放进 `try/finally`；删除加 `-LiteralPath` 与前缀断言。
- **修复后代码**：

  ```powershell
  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    # …curl 采样…
  } finally {
    $ErrorActionPreference = $prevEap
  }
  ```

  ```powershell
  if (-not $tmpDir.StartsWith($runtime, [StringComparison]::OrdinalIgnoreCase)) {
    throw "临时目录越界: $tmpDir"
  }
  Remove-Item -LiteralPath $tmpDir -Recurse -Force -ErrorAction SilentlyContinue
  ```

### 🟢 P2-23 同一 PR 内文档与代码漂移（5 处）

- **位置/问题**：
  1. `docs/SETTINGS_PANEL_SCOPE.md:79` 写「22 个动作」，`src/composables/shortcuts.ts:23-47` 实为 **21** 条；
     同一 PR 的 `docs/MISC_TODO.md` 写的是「5 组 21 行 27 键位」—— **两份文档自相矛盾**。
  2. 同文件 `:71` 写「吸附默认开关…未持久化」，实际 `src/composables/appPrefs.ts:13` 有
     `snap: "gsa.timeline.snap"` 且面板会持久化。
  3. 同文件 `:75` 写「自动保存延迟 硬编码 1000 ms（project.ts:273）」，实际 `project.ts:282` 用
     `readAutosaveDelay()`，可选 `[300,1000,3000]`。
  4. 同文件 `:26/:27/:74/:48` 的行号引用全部错位（`ui.ts:6` 实际是 import 项、
     `MAX_HISTORY` 在 `project.ts:313`、`GSA_OCR_DEVICE` 在 `paddle.rs:185`）。
  5. `docs/RUNTIME_BOOTSTRAP_CONTRACT.md:40-51` 的参数表**漏了本 PR 新增的 `-Source`**，
     而 `:738-744` 又在讲这一档；`docs/SETTINGS_PANEL_SCOPE.md` §4 仍写
     `move_runtime_dir(target, delete_source: bool)`，实现里没有 `delete_source`。
- **修复建议**：按本仓库「改了就在同提交更新点名它的档案」的规矩，把行号与数字对齐，
  参数表补 `-Source`，§4 表改成实现签名；否则下一个按文档核对的人会重复误判。
- **修复后代码**（文档示例）：

  ```markdown
  | `-Source <auto\|mirror\|direct>` | 默认 `auto` | 候选源顺序：auto=清单→镜像→直链；mirror=镜像优先；direct=只用 GitHub 直链；见 §8.10 |
  ```

## 已核对无误的点（抽查清单）

| 检查项 | 结论 |
|---|---|
| `cargo test --lib`：**368 passed / 0 failed**（本审查独立复跑，1.04s）——PR 描述的 368 属实 | ✓ |
| `pnpm run build`（vue-tsc + vite）：**EXIT=0**，2910 modules，css 65.62 kB / js 711.50 kB（gzip 221.35 kB） | ✓ |
| `resolve_runtime_dir` 语义改为「入参 = app_data 本身」，12 处调用点全部同步（config 7 + backend_probe 1 + installed_state 2 + runtime_install 1 + setup 1） | ✓ 逐处核对，无残留 `join("runtime")` |
| 指针/config 写入均为 `.tmp` + `fs::rename`，断电不会留半个 JSON | ✓ `config.rs:249-255 / :322-330` |
| `load_or_default`：仅 `config.json` **不存在**时兜默认，损坏仍 `Err`（有单测） | ✓ `config.rs:336-341`、`:1482` |
| `validate` 拆成 `validate_schema`（形状）+ `validate_paths`（文件存在），面板可存、启动才查路径 | ✓ `config.rs:351-354 / :361 / :454` |
| `SettingsSnapshot`/`SettingsPatch`/`MoveResult` 与 `src/types/index.ts` 逐字段一致（含 `free_bytes: number \| null`） | ✓ |
| 搬家跳过 `.downloads` 有单测断言（`assert!(!dst.join(".downloads").exists())`） | ✓ `config.rs:1556` |
| 搬家的四种非法落点（同目录/内部/上级/盘符根）+ 非空目标均有拒绝分支，`test_move_rejects_non_empty_target` 覆盖 | ✓ |
| 单实例安装守卫（`running.swap(true, SeqCst)`）+ `RunningGuard` RAII 复位 | ✓ `runtime_install.rs:367-404 / :649-654` |
| `set_ui_zoom` 契约与前端调用一致（`scale` 参数名、`clamp(0.5,2.0)`），窗口 label 默认 `main` 正确 | ✓ |
| 设置面板快捷键表由 `SHORTCUTS`+`SHORTCUT_GROUPS` 派生，21 条动作 / 27 键位，无硬编码键位 | ✓ 无漂移可能 |
| `InfoDot` 的 `:focus-visible` 判据 + NModal `:auto-focus="false"` 修掉「弹窗一开就冒 tooltip」 | ✓ `InfoDot.vue:28-36` |
| `CorpusView` 行内编辑去重（`if (next === editInputEl) return;`）—— 修掉每次 patch 都全选，且 O(1) 不需 IME 处理 | ✓ `CorpusView.vue:211-221` |
| 轨道滚动修复点在正确的元素上：`.timeline-col{min-height:0}` + `.tl-row{min-height:0}` + `.tl-tracks{min-height:0}`，祖先链已全部有 `min-width:0` | ✓ `Timeline.vue:1124-1159` |
| 滚轮监听 `{passive:false}` 注册、`onUnmounted` 移除，无泄漏 | ✓ `Timeline.vue:823 / :852-854` |
| `sentenceCuts.ts` 的 split/merge 取值域与面板 `SPLIT_OPTIONS/MERGE_OPTIONS` 一致 | ✓ |
| `SetupGate` 的 `auto_start ?? true` 兜底与后端 `RuntimeStatus.auto_start` 默认 true 一致 | ✓ |
| 安全面：全仓无 `v-html`/`innerHTML`/`eval`/`new Function`，无 SQL，localStorage 只存非敏感偏好，搬家目标仅来自原生目录选择器 | ✓ |
| 两个 `.ps1` 均为 UTF-8 **带 BOM** + CRLF（`bootstrap_runtime.ps1` 46374B / CRLF 933 / 孤立 LF 0；`verify_cdn_mirror.ps1` 6938B / CRLF 140 / LF 0） | ✓ PR 声明属实 |
| `bootstrap_runtime.ps1` 无 PS7 专有语法（无 `??`、无 `-Parallel`），`$Source` 归一化在 `:74-76` | ✓ |
| `Invoke-CurlSegment` 用 `ProcessStartInfo.Arguments` + `UseShellExecute=$false`，无 `Invoke-Expression`，无命令注入面 | ✓ `:453-523` |
| `e260e6b` 修的临时目录重名：`config.rs` 内 33 个 `temp_dir()` 名字已无重复 | ✓ |
| `-Source` 的 `direct` 档过滤 `://github\.com/`、`mirror` 档不再混入 ESA 地址，语义与注释一致（**但被 P0-1 架空**） | ✓ 逻辑本身对 |

## 验证（本审查独立执行）

- `cargo test --lib`（`src-tauri/`）：**368 passed, 0 failed, 0 ignored**
- `pnpm run build`：**EXIT=0**（vue-tsc 无错 + vite 产物正常，仅 chunk >500kB 的常规告警）
- P0-1 用真实 `powershell.exe`（5.1.26100.9818）复现：从 BASE/HEAD 抽真实函数体 + 真实
  `manifest.json` 条目运行，得到 3 / 2 / 2 / 1 条候选的对比（见 P0-1 表格）
- 文档漂移逐条按「文件:行号」点开核对（见 P2-23）
- 未做交互式 GUI 验证（本环境无真机），UI 行为结论来自源码路径推演 + 提交信息里的实测记录采信

## 结论

**P0-1 已修复（2026-11-16），当前不建议直接合并的唯一硬阻塞已解除；建议修完 P1-1 ~ P1-3 再合并。**
P0-1 原本必须在本 PR 内修掉（一个变量改名 + 一条冒烟断言），否则「强制镜像」
这个本 PR 自己新加的功能是空操作，且基线里还好的镜像兜底被静默打断。
P1-1 ~ P1-3 属于「用户会撞上且撞上后没有出路」的失败语义问题，建议同批修；
P1-4 是设计取舍，至少要显式登记为既定行为。
P2 共 23 条，绝大部分可以合并后按档案规矩逐条跟进，但 **P2-23（文档漂移）建议随本 PR 一起修**——
它是「同一 PR 内两份文档互相矛盾」，属于档案自伤，拖到后面就变成后来者的错误前提。

## 处置记录（待用户裁决后回填）

> 按 `AGENTS.md` 的档案同步规矩：**修复提交与本文档的状态行必须同一提交更新**，
> 状态行要写依据（提交号 / PR 号），不要只写「已修复」。

- **P0-1** ✅ **已修复**（2026-11-16，本分支，`scripts/bootstrap_runtime.ps1:399-420`）——
  `Get-UrlCandidates` 内三个局部 ArrayList 全部改名（`$mirror` → `$mirrorCands`、
  `$direct` → `$directCands`），并在函数头写下「禁止与同名参数撞名」的注释说明因果
  （PowerShell 变量名大小写不敏感，`$mirror = New-Object ArrayList` 直接覆盖 `[string]$Mirror`）。
  复测（真实 `powershell.exe` 5.1，逐字抽取 `Resolve-Url`+`Get-UrlCandidates`，喂真实
  `manifest.json` 的 `moss/cpu` 条目）：

  | 清单形状 | `-Source auto` | `-Source mirror` | `-Source direct` |
  |---|---|---|---|
  | 当前（2 条 `urls`） | **3**（cdn×2 + mirror） | **3**，mirror 排第一 | 1（仅 github） |
  | 合并 #53 后（3 条 `urls`） | **4**（ModelScope + cdn×2 + mirror） | **4**，mirror 排第一 | 1（仅 github） |

  对比修复前：`auto` 2 条（无 mirror）、`mirror` 2 条且 mirror 不排第一、`direct` 1 条。
  函数返回后参数仍为 `type=String value=https://gh-proxy.com/`（修复前函数内是 `ArrayList`/空）。
  这同时解开了 #53 的跨 PR 阻塞：合并后 `$ranked` 末尾的镜像兜底不再是死代码
- **P1-1** ⚠ 未修复（`SettingsModal.vue:165-169` 绑实时键，`setSidebarDefaultCollapsed` 无调用方）
- **P1-2** ⚠ 未修复（`config.rs:906-910` 对账失败不回滚，重试撞「目标目录非空」）
- **P1-3** ⚠ 未修复（`config.rs:934` 用 `read_location`，新目录缺失时不可达）
- **P1-4** ⚠ 未修复（`Timeline.vue` clip 区普通滚轮不再纵向滚动，待裁决「改回」还是「登记为既定行为」）
- **P2-1 ~ P2-23** ⚠ 未修复（见上，建议合并后按严重度跟进；其中 P2-23 文档漂移建议随本 PR 修）
