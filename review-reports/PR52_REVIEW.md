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

> **更新（2026-11-17）**：P0-1、P1-1 ~ P1-4、P2-1 ~ P2-23 **全部在本分支修复并复测**，
> 当前 P0/P1/P2 三条线全部清零（23 条 P2 见「处置记录」，每条一提交）。
> 下方总评保留发现当时的判断。

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

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P0-1，提交号 `dc59140`）。
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

### 🟡 P1-1 「侧栏默认折叠」开关写的是实时键，`gsa.sidebarDefault` 从 UI 永远设不上 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P1-1）。下方保留发现时的原始取证，行号对应修复前。

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
    // 顺手清掉手动折叠记录：readSidebarCollapsed 只在没有手动记录时才看默认值，
    // 不清的话「改默认」对已经手动折叠过的用户永远不生效（后果③）
    localStorage.removeItem(SIDEBAR_KEY);
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

### 🟡 P1-2 搬家复制不完整时留下半搬目录，而提示里的「可重试」必然失败 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P1-2）。下方保留发现时的原始取证，行号对应修复前。

- **位置**：`src-tauri/src/ai_runtime/config.rs:897-910`（`create_dir_all` 在 `:900`，对账失败在 `:906-910`）
- **问题**：`:900` 已把 `target` 建出来，`:902` 开始拷；`:906` 对账缺文件时直接
  `return Err("复制不完整：目标目录缺少 {missing}（未改动指针，可重试）")`，**不清理**已拷的部分。
  用户按提示重试时，`:897 if target.is_dir() && dir_entry_count(target) > 0` 立刻回
  「目标目录非空」 —— 唯一出路是手工去资源管理器删掉那半个目录（而且删错就有丢数据风险）。
  这与设计文档自己的口径直接冲突：`docs/SETTINGS_PANEL_SCOPE.md` §3.4「失败回滚」、
  `docs/RUNTIME_BOOTSTRAP_CONTRACT.md:721-722`「失败不写指针（**半搬状态比不搬更糟**）」。
- **修复建议**：只回滚「本次自己拷进去的东西」；指针与旧目录一律不动。
- **修复后代码**（实际落地版；比原建议多覆盖了「目标目录是用户原有的空目录」这一半）：

  ```rust
  // 只有自己新建的目录才敢在失败时整个删掉；用户指定的已存在空目录是别人的目录
  let created_target = !target.is_dir();
  fs::create_dir_all(target).map_err(|e| format!("创建目标目录失败: {e}"))?;

  let rollback = |created: bool| {
      if created {
          let _ = fs::remove_dir_all(target);
          return;
      }
      // 目标目录是用户原有的空目录：目录本身留着，只清掉本次拷进去的内容
      if let Ok(entries) = fs::read_dir(target) {
          for p in entries.flatten().map(|e| e.path()) {
              let _ = if p.is_dir() { fs::remove_dir_all(&p) } else { fs::remove_file(&p) };
          }
      }
  };

  let bytes = match copy_dir_all(&current, target) {
      Ok(n) => n,
      Err(e) => {
          rollback(created_target);
          return Err(format!("{e}（已回滚，旧目录与指针未改动）"));
      }
  };
  if let Some(missing) = src_paths.iter().find(|p| !dst_paths.contains(*p)) {
      rollback(created_target);
      return Err(format!("复制不完整：目标目录缺少 {missing}（已回滚，旧目录与指针未改动）"));
  }
  ```

### 🟡 P1-3 新目录不存在时「删除旧目录」不可达，且报错信息误导 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P1-3）。下方保留发现时的原始取证，行号对应修复前。

- **位置**：`src-tauri/src/ai_runtime/config.rs:934`（`read_location` 的 `is_dir()` 检查在 `:234-246`）
- **问题**：`delete_old_runtime_dir_impl` 第一行用 `read_location(app_data)`，而 `read_location`
  内要求**指针指向的目录当前存在**（`dir.is_dir()`，否则打印「外接盘未挂载？」返回 None）。
  于是「移动硬盘拔了 / 新目录被误删」这一确切场景下，清理旧目录的路径**永远不可达**，
  用户看到的是 `"没有搬家记录（指针文件缺失）"` —— 一句话把两种完全不同的病因混成了一个，
  而且这句话是假的：指针文件好端端在那儿。
  这正是 `docs/MISC_TODO.md` 里记下的真机事故（把 `D:\__GSA_RUMTIME` 打成 `D:\_GSA_RUNTIME`，
  指针悬空+旧目录待删）所需要的那个入口。
- **修复建议**：读指针时改用不校验目录存在性的 `read_location_raw`，把「新目录是否在」只用于提示。
- **修复后代码**（实际落地版）：

  ```rust
  pub(crate) fn delete_old_runtime_dir_impl(app_data: &Path) -> Result<String, String> {
      // 这里要的是「指针文件里记了什么」，而不是「指针指向的新目录此刻在不在」：
      // 用 read_location 的话，新目录缺失（外接盘没插 / 被手动删）时会返回 None，
      // 明明指针文件好好的却报「没有搬家记录（指针文件缺失）」，清理旧目录这条路也永远走不到。
      // 2026-11-07 真机就是这么卡住的（D:\_GSA_RUNTIME 少打一个下划线，旧目录反而删不掉）。
      let mut loc =
          read_location_raw(app_data).ok_or_else(|| "没有搬家记录（指针文件缺失）".to_string())?;
      let previous = loc.previous.clone().ok_or_else(|| "没有待删除的旧目录".to_string())?;
      let old = PathBuf::from(&previous);
      let current = resolve_runtime_dir(Some(app_data.to_path_buf()));
      if normalize_lexical(&old) == normalize_lexical(&current) {
          return Err("旧目录就是当前 runtime 目录，拒绝删除".into());
      }
      // 新目录不在时给一句人话，别让用户以为搬家成功过
      let new_dir_missing = !PathBuf::from(&loc.runtime_dir).is_dir();
      let hint = if new_dir_missing {
          "（注意：指针指向的新目录当前不存在，删完旧目录后只剩悬空指针）"
      } else {
          ""
      };
      if !old.exists() {
          loc.previous = None;
          write_location(app_data, &loc)?;
          return Ok(format!("旧目录已不存在：{previous}（已清理记录）{hint}"));
      }
      fs::remove_dir_all(&old).map_err(|e| {
          format!("删除 {previous} 失败: {e}（可能有文件正被占用，先退出所有转写/OCR 任务再试）")
      })?;
      loc.previous = None;
      write_location(app_data, &loc)?;
      Ok(format!("已删除旧目录：{previous}{hint}"))
  }
  ```

### 🟡 P1-4 滚轮语义被本 PR 中途改掉（标签列由原生滚动改为显式代滚，丢了触控板惯性） ✅ 已修复

> **状态**：✅ 2026-11-16 已按用户裁决「以改 UI 之前的逻辑为准」恢复（见文末「处置记录」P1-4）。
> **同时更正原报告的一处错判**：下方原先写「`0025211` 删掉了**基线里的**这段 `canScrollTracks`」，
> 实测不成立——`git show 9cae08c:src/components/timeline/Timeline.vue` 里没有 `canScrollTracks`，
> `git log -S canScrollTracks` 只有 `aca4210`（加）与 `0025211`（删）两个提交，即那段是**同一 PR 内
> 自己加又自己删**，不是基线能力回退。基线 `onWheelRoot` 正是恢复后的实现（逐字节一致）。

- **位置**：`src/components/timeline/Timeline.vue:570-617`（`onWheelRoot`）、`:1124-1133`（`min-height:0`）
- **问题**（按更正后的事实重写）：基线（`9cae08c`，含样稿 06 皮肤）的 `onWheelRoot` 约定是
  「clip 区普通滚轮 = 横向平移；Shift+滚轮 = 纵向滚动；**轨道信息标签列不在 JS 里处理**，
  直接放行让浏览器原生滚（标签列位于 `.tl-tracks` 内，原生滚轮会滚到那个 `overflow-y:auto` 容器）」。`0025211` 把标签列改成显式代滚：

  ```ts
  // 0025211（修复前）
  if (t.closest(".tl-label-col")) {
    const el = rootRef.value?.querySelector<HTMLElement>(".tl-tracks") ?? null;
    if (el && el.scrollHeight > el.clientHeight + 1) {
      e.preventDefault();                       // ← 吃掉原生滚动
      el.scrollTop += e.deltaY + e.deltaX;      // ← 每帧按增量手动推，无惯性
    }
    return;
  }
  ```

  后果：①`preventDefault` + 逐帧 `scrollTop` 让触控板/精度的惯性滚动失效，滚动手感与其它区域不一致；
  ②这条路径还依赖 `scrollHeight > clientHeight + 1` 的阈值，边界处会出现「滚不动也不放行」的死区。
  安全网：这条回归之所以能看出来，是同一 PR 的 `min-height:0`（`:1124-1133`）让 `.tl-tracks`
  真的开始溢出——否则旧分支本来也不生效。
- **用户裁决**（2026-11-16）：「之前的滚轮逻辑是：在 clip 区滚轮横向滚动，shift+滚轮纵向滚动，
  在轨道信息标签处滚轮纵向滚动。如果改 UI 前的逻辑与我有冲突，以之前的逻辑为准。」
  ⇒ 即**恢复基线实现**，不要「clip 区普通滚轮纵向滚动」那种改写。
- **修复后代码**（= 基线实现，已逐字节还原，并补上「三处约定」注释头防止再被改）：

  ```ts
  function onWheelRoot(e: WheelEvent) {
    const t = e.target as HTMLElement;
    if (t.closest(".scrollbar-track")) return;
    if (t.closest(".tool-strip") || t.closest(".tl-label-col")) return;  // 标签列放行原生滚动
    if (t.closest(".ruler")) { /* 缩放，锚点取标尺左缘 */ return; }
    if (e.shiftKey) {
      const tracksEl = rootRef.value?.querySelector<HTMLElement>(".tl-tracks") ?? null;
      if (tracksEl && tracksEl.scrollHeight > tracksEl.clientHeight) {
        if (t.closest(".tl-tracks")) return;          // 区内放行原生（保留惯性）
        e.preventDefault();
        tracksEl.scrollTop += e.deltaY + e.deltaX;    // 区外代为滚动
        return;
      }
      e.preventDefault();
      return;
    }
    e.preventDefault();
    timeline.pan(e.deltaY + e.deltaX);                 // clip 区普通滚轮 = 横向平移
  }
  ```

### 🟢 P2-1 `OcrManager::reload()` 换掉了 provider，却没更新 `self.config`，`dev_debug` 改不动 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-1）。

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

### 🟢 P2-2 同一个 patch 接口里，有的字段静默钳位、有的字段硬报错 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-2）。

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

### 🟢 P2-3 `normalize_ocr_device` 没有白名单，非法设备串会被照存 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-3）。

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

### 🟢 P2-4 「重新检查运行时」把用户踢出主界面，且不带任何新信息 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-4）。

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

### 🟢 P2-5 快照加载失败时面板照常渲染「默认值」，会把没装误报成未找到清单 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-5）。

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

### 🟢 P2-6 设置面板打开时全局快捷键仍然生效，Delete 能删掉被遮住的片段 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-6）。

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

### 🟢 P2-7 「删除旧目录」确认按钮忽略 loading，双击会连发两次删除 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-7）。

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

### 🟢 P2-8 settings store 的 `load`/`patch` 没有序号，慢的快照会覆盖新的 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-8）。

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

### 🟢 P2-9 「重新检查运行时」「打开日志目录」没有 loading/disabled ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-9）。

- **位置**：`src/components/SettingsModal.vue:272-273`
- **问题**：连点会开多个资源管理器窗口；P2-4 修好后还会并发多次 `runtime_status`。
- **修复建议**：加最小 loading 态。
- **修复后代码**：

  ```vue
  <NButton size="small" :loading="rechecking" :disabled="rechecking" @click="onRecheckRuntime">
    重新检查运行时
  </NButton>
  ```

### 🟢 P2-10 `onMove` 的目录选择在 try 之外；`res.ok` 的 else 分支是死代码 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-10）。

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

### 🟢 P2-11 缩放先落盘后应用，失败时 UI、localStorage 与真实缩放三者不一致 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-11）。

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

### 🟢 P2-12 搬家对账的三处不一致：`.downloads` 深度、读目录失败静默、无大小校验 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-12）。

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

### 🟢 P2-13 搬家前不校验目标盘剩余空间 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-13）。

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

### 🟢 P2-14 设计文档里「禁止目标是 %APPDATA% 子路径」没有实现 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-14）。

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

### 🟢 P2-15 删旧目录完全信任指针文件里的 `previous`，没有落点白名单 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-15）。

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

### 🟢 P2-16 `update_runtime_settings` 在异步线程上做阻塞 I/O 与 powershell 调用 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-16）。

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

### 🟢 P2-17 安装时 `config.json` 损坏会静默退回默认下载源/分段 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-17）。

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

### 🟢 P2-18 前端可选集合比后端允许范围窄，非枚举值会显示成「选不回去」的原始值 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-18）。

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

### 🟢 P2-19 「吸附」在会话内切换不落盘；关掉自动保存后状态条永远显示「未保存」 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-19）。

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

### 🟢 P2-20 `PREF_KEYS` 里 splitStrategy/mergeStrategy 重复定义且无人使用 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-20）。

- **位置**：`src/composables/appPrefs.ts:14-15` vs `src/stores/timeline.ts:43-51`
- **问题**：两处各写一份 `"gsa.timeline.splitStrategy"` 字符串常量，面板/时间轴改任一侧漏改另一侧就是静默不生效。
- **修复建议**：`timeline.ts` 改用 `PREF_KEYS`，删掉本地常量。
- **修复后代码**：

  ```ts
  import { PREF_KEYS } from "../composables/appPrefs";
  const SPLIT_KEY = PREF_KEYS.splitStrategy;
  const MERGE_KEY = PREF_KEYS.mergeStrategy;
  ```

### 🟢 P2-21 死类名、缺 `min-width:0`、store 模块求值期副作用 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-21）。

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

### 🟢 P2-22 两个 PowerShell 脚本的健壮性小口子 ✅ 已修复

> **状态**：✅ 2026-11-16 已在本分支修复（见文末「处置记录」P2-22）。下方保留发现时的原始取证。

- **位置**：`scripts/verify_cdn_mirror.ps1:119`、`scripts/bootstrap_runtime.ps1:767 / :804`
- **问题**：①`$ErrorActionPreference` 的恢复在正常路径上，中途 `throw` 会把全局偏好留在 `Continue`；
  ②`Remove-Item $tmpDir -Recurse -Force` 未用 `-LiteralPath`，也未校验 `$tmpDir` 落在
  `.downloads` 之内（同文件 `:696/:702` 是正确写法）。今天不可利用（7 个 manifest id 都是静态字面量），
  但这是最容易在「以后加一条清单项」时变成真问题的地方。
- **修复建议**：恢复动作放进 `try/finally`；删除加 `-LiteralPath` 与前缀断言。
- **修复后代码**：

  ```powershell
  # verify_cdn_mirror.ps1：采样体整段进 try，还原动作进 finally
  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    # …响应头 / Range 探测 / 采样循环…
  } finally {
    $ErrorActionPreference = $prevEap
  }
  ```

  ```powershell
  # bootstrap_runtime.ps1：新增统一清理入口，删除一律 -LiteralPath + 临时目录断言
  function Remove-TempArtifact([string]$path, [switch]$Recurse) {
    if (-not $path) { return }
    $tempRoot = [System.IO.Path]::GetTempPath().TrimEnd('\', '/')
    $full = [System.IO.Path]::GetFullPath($path)
    if (-not $full.StartsWith($tempRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
      Write-Warning "拒绝清理临时目录之外的路径：$full"
      return
    }
    Remove-Item -LiteralPath $full -Recurse:$Recurse -Force -ErrorAction SilentlyContinue
  }
  ```
  > 注：与本节原建议的「校验落在 `.downloads` 内」相比，实际落的是**落在系统临时目录内** ——
  > 出问题的两条路径（`$tmpZip` / `$tmpDir`）本来就建在 `GetTempPath()`，不在 `.downloads`
  > 之下；按原建议写会把正常清理也拦掉。

### 🟢 P2-23 同一 PR 内文档与代码漂移（5 处） ✅ 已修复

> **状态**：✅ 2026-11-17 已在本分支修复（见文末「处置记录」P2-23）。
> 下方保留发现时的原始取证（行号对应修复前）。

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
- 审查当时未做交互式 GUI 验证（本环境无真机），UI 行为结论来自源码路径推演 + 提交信息里的实测记录采信。
  **补记（2026-11-16，修 P1-1 / P1-4 时补做）**：P1-4 用真 Chrome headless + CDP 派发可信滚轮事件验证
  （9/9，含修复前 8/9 的负向对照）；P1-1 用 vite SSR 把真实 `src/stores/ui.ts` 打进 Node 跑（10/10，
  含修复前 7/10 的负向对照）。方法见「处置记录」P1-1 / P1-4 两条。

## 结论

**P0-1、P1-1 ~ P1-4 已修复（2026-11-16），P2-1 ~ P2-23 已修复（2026-11-17），P0/P1/P2 全部清零，可进入合并流程。**
P0-1 原本必须在本 PR 内修掉（一个变量改名 + 一条冒烟断言），否则「强制镜像」
这个本 PR 自己新加的功能是空操作，且基线里还好的镜像兜底被静默打断。
P1-1 ~ P1-3 属于「用户会撞上且撞上后没有出路」的失败语义问题，已同批修掉；
P1-4 经用户裁决为「以改 UI 之前的逻辑为准」，已把 `onWheelRoot` 恢复为基线实现
（同时更正了原报告「基线能力被删」的错判，实际是标签列从原生滚动被改成显式代滚、丢了惯性）。
P2 共 23 条，已按「每条一提交」全部修在本分支（2026-11-17）。其中 P2-23（文档漂移）
原本只建议随本 PR 修，实际也一并修了——它是「同一 PR 内两份文档互相矛盾」，属于档案自伤，
拖到后面就变成后来者的错误前提。

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
- **P1-1** ✅ **已修复**（2026-11-16，本分支，`src/stores/ui.ts` + `src/components/SettingsModal.vue:162-169`）——
  store 新增独立的 `sidebarDefault = ref(readSidebarDefault())` 并导出；`setSidebarDefaultCollapsed`
  同时更新这个响应式值（弹窗里立刻反映），且**顺手 `localStorage.removeItem(SIDEBAR_KEY)`** ——
  这修的是原报告后果③：`readSidebarCollapsed` 只在没有手动记录时才看默认值，不清记录的话
  「改默认」对已经手动折叠过的用户永远不生效。模板开关改绑 `ui.sidebarDefault` /
  `ui.setSidebarDefaultCollapsed`；InfoDot 补一句「改这里会清掉手动折叠记录，下次启动按此值」。
  复测（Node 里跑真实 `src/stores/ui.ts`，stub `localStorage`/`document`，每例重新 `setActivePinia` 模拟重启，
  `.tmp_p14/run_p11.mjs` 走 vite SSR 打包）：**修复后 10/10 通过**；同一套用例对修复前代码
  （`git stash` 三个文件）是 **7/10**，3 条 FAIL 正对应「`store.sidebarDefault` 不存在 / 改默认不清手动记录 /
  改默认后重启仍用旧手动值」。
- **P1-2** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:900-931`）——
  `created_target = !target.is_dir()` 记录目标目录是不是本次新建；`copy_dir_all` 或对账失败都走
  `rollback(created_target)`：自建目录整个删掉，用户原有的空目录只清空内容（保留目录本身，不替人删）。
  两条失败信息都改成「…（已回滚，旧目录与指针未改动）」，不再承诺一个必然失败的「可重试」。
  复测：新增 `test_move_rolls_back_created_target_when_copy_incomplete`（用子目录里的 `.downloads`
  稳定制造对账缺失）与 `test_move_rolls_back_into_existing_empty_target`；全量 `cargo test --lib`
  **371 passed**；这两个新测试对修复前代码是 **2 failed**（负向对照），断言有区分力。
- **P1-3** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:963-997`）——
  读指针改用 `read_location_raw`（不校验目录存在性），「新目录是否在」只用来在返回串里附带一句
  「（注意：指针指向的新目录当前不存在，删完旧目录后只剩悬空指针）」。清理旧目录不再受新目录是否挂载影响，
  文案也不再谎称「指针文件缺失」。复测：新增 `test_delete_old_dir_works_when_new_dir_missing`
  （指针指向不存在的新目录 + 存在旧目录 ⇒ 旧目录被真删、`previous` 清空；再测旧目录也不存在时的清记录路径）；
  全量 `cargo test --lib` **372 passed**，该测试对修复前代码 **failed**（负向对照）。
- **P1-4** ✅ **已修复**（2026-11-16，本分支，`src/components/timeline/Timeline.vue:570-617`）——
  按用户裁决「以改 UI 之前的逻辑为准」把 `onWheelRoot` **恢复为基线 `9cae08c` 的实现**
  （`git diff` 抽函数体算 SHA256，与基线同为 `7254366D…`，逐字节一致），并在函数头补「三处约定」注释
  （clip 区普通滚轮=横向、Shift=纵向、标签列交原生）。删除了 `0025211` 引入的
  `canScrollTracks` 变量与标签列显式 `scrollTop` 代滚。
  **同时更正原报告错判**：`canScrollTracks` 那段从来不在基线里（`git log -S canScrollTracks`
  只有 `aca4210` 加、`0025211` 删），所以「基线能力被删」的说法不成立；真正的回归是标签列
  从「放行原生滚动」被改成「显式代滚」，代价是丢触控板惯性 —— 见 P1-4 正文的更正说明。
  复测（真 Chrome headless + CDP 派发**可信** `Input.dispatchMouseEvent` 滚轮，harness
  `.tmp_p14/wheel_test.mjs` 从 `.vue` 里抽取真实 `onWheelRoot` 原文去 TS 后注入）：**9/9 通过**
  （clip 普通滚轮只 `pan` 不纵滚且 `defaultPrevented=true`；标签列纵滚 `defaultPrevented=false` 且有位移；
  轨道区外 Shift 由 handler 代滚；标签列连滚累计 >200px）。
  **负向对照**：同一 harness 跑修复前的 `4c51e3b` 版本为 **8/9**，唯一 FAIL 正是
  「标签列普通滚轮不应 `preventDefault`」——证明这些断言有区分力。
- **P2-1** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/mod.rs:317-411`）——
  `OcrManager.config` 改为 `RwLock<RuntimeConfig>`，`config()` 返回**拷贝**而非引用（引用不可能既安全又不过期），
  `reload()` 在换 provider 的同时 `*self.config.write() = config`。新增
  `test_ocr_reload_refreshes_config_too`（改盘上 `dev_debug=true` → `reload()` → `config().dev_debug` 为真），
  全量 `cargo test --lib` **373 passed**。
- **P2-2** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:680-717`）——
  新增 `normalize_moss_threads`（0–256）/ `normalize_moss_timeout_minutes`（0–1440），
  与 `segment_mb`/`download_source` 统一成「超范围报错、错误串带原值与允许区间」，
  `update_runtime_settings` 里两处 `.min(...)` 改成 `?` 传播。新增
  `test_normalize_threads_and_timeout_reject_instead_of_clamping`；契约 §8.10 的字段表同步改口
  （明确「不静默钳位」）。全量 `cargo test --lib` **374 passed**。
- **P2-3** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:713-745 / :445-449`）——
  `normalize_ocr_device` 改为返回 `Result` 并做白名单：空 / `cpu` / `gpu:<n>`（同时放行 `npu`/`xpu`，
  与 `scripts/ocr_worker.py:_resolve_device` 的口径一致——worker 明确把 npu/xpu 交给 paddle 判断），
  挡下 `gpu:abc`/`cuda`/`gup:0`/`gpu:-1` 等；`validate_schema` 也复用同一函数，手改 `config.json`
  写坏设备串会被挡在保存/启动校验处。新增 `test_normalize_ocr_device_whitelist`；
  全量 `cargo test --lib` **375 passed**。
- **P2-4** ✅ **已修复**（2026-11-16，本分支，`src/components/SettingsModal.vue:120-140`）——
  `onRecheckRuntime` 改为先 `await setup.refresh()`（真的重探 `runtime_status`），只有仍然 `needs`
  才 `show=false` + `reopen()` 回准备页；`failed` 走 `message.error`，`ready` 走
  `message.success("运行时状态已更新")` 并留在当前界面。这样「重新检查」不再把用户踢出主界面，
  也不会展示旧快照。`npx vue-tsc --noEmit` EXIT=0。
- **P2-5** ✅ **已修复**（2026-11-16，本分支，`src/components/SettingsModal.vue:154-162 / :204-378 / :458-470`）——
  模板在 `settings.loading` 之后新增 `v-else-if="settings.error"` 的错误行（整行红字 + 「重试」按钮，
  直接再调 `settings.load()`），并把 **B 运行时 / C 引擎** 两组整体包进 `<template v-if="snap">`：
  快照拿不到时不再渲染「默认值」，`未找到 manifest.json` 这句也不会在「未知」状态下出现。
  新增 `.set-error` 样式（红字 + 行内按钮）。`npx vue-tsc --noEmit` EXIT=0。
- **P2-6** ✅ **已修复**（2026-11-16，本分支，`src/composables/shortcuts.ts:109-127 / :130`、`src/components/SettingsModal.vue:24-42`）——
  `shortcuts.ts` 新增 `suppressDepth` 计数 + `suppressShortcuts()`（返回幂等释放函数，嵌套弹窗先关的
  那个不会提前放开），`dispatchShortcut` 开头 `if (suppressDepth > 0) return false;`；
  `SettingsModal.vue` 把两个 `watch(show)` 合成一个：打开时 `releaseShortcuts = suppressShortcuts()`
  并 `void settings.load()`，关闭时释放，`onUnmounted` 兜底释放。
  验证：`.tmp_p2\p26_entry.ts` + `run_p26.mjs`（vite SSR 打包后 node 跑）**13/13 PASS**
  —— 覆盖「无抑制时 Delete 命中」「抑制期间 Delete 与 Ctrl+Z 都短路且处理器不执行」
  「释放后恢复」「重复释放不多减」「嵌套两层只放一层仍抑制」；
  反向对照（同一 harness 指向 `git show HEAD:src/composables/shortcuts.ts`）
  输出「修复前：Delete 命中=true，处理器调用=1 次」。`npx vue-tsc --noEmit` EXIT=0。
- **P2-7** ✅ **已修复**（2026-11-16，本分支，`src/components/SettingsModal.vue:118-127 / :236-243`）——
  `onDeleteOld` 开头加 `if (settings.saving) return;`（并发的第二个请求直接丢掉，不再拿「没有搬家记录」的 Err
  去弹红色失败）；`NPopconfirm` 补 `:positive-button-props="{ loading: settings.saving, disabled: settings.saving }"`
  ——naive-ui 的 `:loading` 只作用于 trigger NButton，确认按钮必须走 `positiveButtonProps`（已核对
  `node_modules/naive-ui/es/popconfirm/src/Popconfirm.mjs:21-22` 确有该 prop）。`npx vue-tsc --noEmit` EXIT=0。
- **P2-8** ✅ **已修复**（2026-11-16，本分支，`src/stores/settings.ts:31-77`）——
  加 `let seq = 0;`，`load()`/`patch()` 各自 `const mine = ++seq;`，回来时 `if (mine !== seq) return;`
  丢弃过期结果；`loading`/`saving`/`error` 的复位也 gate 在 `mine === seq` 上（过期请求的失败
  不污染 `error`）。`patch` 开头自增，意在作废任何还在飞的 `load`——patch 的返回值才是最新真相。
  验证：`.tmp_p2\p28_entry.ts` + `run_p28.mjs`（vite SSR + `@tauri-apps/api/core` 桩，可控延时）
  **6/6 PASS**（慢 load 不覆盖后发 patch、先发慢 load 不覆盖后发快 load、过期失败不写 error、loading 归位）；
  反向对照（同一 harness 指向 `git show HEAD:src/stores/settings.ts`）输出
  `最终快照 runtime_dir=STALE_LOAD`，确认断言有区分力。`npx vue-tsc --noEmit` EXIT=0。
- **P2-9** ✅ **已修复**（2026-11-16，本分支，`src/components/SettingsModal.vue:130-161 / :340-356`）——
  新增 `const rechecking = ref(false)` / `const openingLogs = ref(false)`；两个 handler 开头
  `if (xxx.value) return;`，`try/finally` 复位；模板两个 NButton 补 `:loading` + `:disabled`。
  核对：`npx vue-tsc --noEmit` EXIT=0；已用 `Select-String` 确认两个按钮都带 `:loading` 与 `:disabled`
  且 handler 都有并发闸（本条无真机 UI 自动化，属静态 + 类型核对）。
- **P2-10** ✅ **已修复**（2026-11-16，本分支，`src/components/SettingsModal.vue:107-118`）——
  `open({ directory: true })` 移进 `try`：对话框插件抛错时走 `message.error("移动失败：…")`，
  不再变成未处理 rejection（用户看到「点了没反应」，与「自己取消」无法区分）；
  同时删掉 `if (res.ok) … else message.error(res.message)` 的死分支——后端
  `move_runtime_dir_impl` 只在成功时构造 `MoveResult`，失败一律 `Err`（`config.rs:877-930`），
  改为 `message.success((await settings.moveRuntimeDir(target)).message)`，并在注释里写明
  「后端失败一律走 Err，故无需判 ok」。`npx vue-tsc --noEmit` EXIT=0。
- **P2-11** ✅ **已修复**（2026-11-16，本分支，`src/composables/appPrefs.ts:39-62`、`src/stores/ui.ts:65-69`、`src/components/SettingsModal.vue:172-175`）——
  `applyZoom` 改为返回 `Promise<boolean>`（`invoke` 成功 true，异常 catch 后 false）；
  `setZoom(zoom)` 只在 `applyZoom` 成功后才 `localStorage.setItem`；
  `ui.setZoom` 改成 `async`：`if (await persistZoom(next)) zoom.value = next;`——
  三层同时改，才能保证「界面没变 ⇒ 内存状态与 localStorage 都没变」。
  验证：`.tmp_p2\p211_entry.ts` + `run_p211.mjs`（vite SSR + invoke 桩）**8/8 PASS**
  （失败不写盘/不改状态、成功才写、重启读回、失败不污染已有值）；
  反向对照（同一 harness 指向 `git show HEAD:` 的旧 appPrefs/ui）输出
  `invoke 失败后 zoom=1.25，localStorage["gsa.zoom"]="1.25"`，确认断言有区分力。
  `npx vue-tsc --noEmit` EXIT=0。
- **P2-12** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:869-951`、`:1010-1045`）——
  `copy_dir_all` 增加 `root` 与 `&mut Vec<(String, u64)>` 参数，复制时顺手记下每个文件的
  相对路径与字节数；`rel_paths` 改成返回 `Result<BTreeMap<String, u64>, String>`：
  ①`.downloads` 在任意层级都跳过（与复制口径统一，此前复制跳任意层、对账只跳根层）；
  ②读不动的目录用 `map_err` 传播，不再 `else { return }` 静默当成「没有文件」；
  ③顺带记录字节数。搬家对账改成「路径 + 大小」双向比较，并直接复用复制时记下的清单——
  整树遍历从 3 次（复制 + 两次 rel_paths）降为 2 次（复制 + 目标侧一次）。
  验证：`cargo test --lib` **378 passed / 0 failed**（新增 3 条：
  `test_rel_paths_skips_downloads_at_any_depth_and_records_size`、
  `test_rel_paths_propagates_unreadable_dir`、`test_move_succeeds_with_nested_downloads`）；
  `test_move_succeeds_with_nested_downloads` 正是修复前必然失败的用例——
  此前 P1-2 的反向验证就是靠「子目录里的 `.downloads` 能让搬家对账失败」成立的，
  口径统一后那个套路不再产生缺失，故 P1-2 的两个测试改用「独占打开源文件」制造复制失败
  （不依赖磁盘满/拔盘这类偶发条件）。
- **P2-13** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:965-987`、`:1015-1016`）——
  新增 `ensure_enough_space(need, free) -> Result<(), String>`（纯函数，便于单测）与 `fmt_bytes`；
  搬家在 `create_dir_all` **之前**用 `rel_paths(&current)?.values().sum()` 算要写的字节数，
  再与 `runtime_install::query_free_bytes(target)` 比对，留 10% 余量，不足直接报错并点名两个数字。
  探测失败（`free_bytes = None`，如非 Windows 盘符路径）**不拦**——宁可让复制自己失败回滚，
  也不因探测不到就把路堵死（与 `precheck_runtime_disk` 的既有口径一致）。
  验证：`cargo test --lib` **379 passed / 0 failed**，新增
  `test_ensure_enough_space_leaves_margin_and_never_blocks_on_unknown`（None 放行 /
  刚好够但无余量拦 / 1099 拦而 1100 过 / 0 字节不误拦 / 报错带 GB 字样）。
  契约 §8.10 的搬家规则同步改口（空间预检、任意层级 `.downloads`、按「路径 + 大小」对账、回滚）。
- **P2-14** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:1006-1017`）——
  在「盘符根」检查之后补 `if tgt_norm.starts_with(&normalize_lexical(app_data))` →
  `目标目录不能放在应用数据目录内部: …（会与日志/指针文件互相干扰，请选盘上另一个位置）`。
  选择**实现文档承诺**而不是改文档口径：文档那条理由（防递归复制）之外还有实际风险——
  runtime 搬进 app_data 后与日志、`runtime-location.json` 指针同树，且删旧目录时波及面更大。
  验证：`cargo test --lib` **380 passed / 0 failed**，新增 `test_move_rejects_target_inside_app_data`
  （`app_data\rt-inside` 被拒且不建目录；`app_data` 之外的正常目标照常搬成功）。
- **P2-15** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:985-1000`、`:1142-1152`）——
  新增 `looks_like_runtime(dir)`：目录里有 `config.json`，**或**目录名含 `runtime`
  （用户自选的搬家目标可能还没落 config.json，故不能只认 config.json）；
  `delete_old_runtime_dir_impl` 在「不等于当前目录」之外再加这道闸，拒绝时
  `… 不像 runtime 目录（没有 config.json，目录名也不含 runtime），拒绝删除`。
  定位说明：这不是完备防护，只求挡住「手改指针文件指向 `C:\Windows`」这类事故。
  验证：`cargo test --lib` **381 passed / 0 failed**，新增
  `test_delete_refuses_dir_that_does_not_look_like_runtime`（伪造的普通目录被拒不误删，
  名字带 `runtime` 的目录放行并删掉）。
- **P2-16** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/config.rs:820-885`）——
  `update_runtime_settings` 原先在 async 线程上做完「读配置 → 校验 → 写盘」再拼快照，
  而拼快照里的 `find_manifest`（读清单）与 `query_free_bytes`（spawn 一次 powershell
  `Get-PSDrive`）都是阻塞调用，慢盘/杀软扫盘时会把同一 runtime 的其它命令一起拖住。
  改法：写盘段整体 `spawn_blocking`，返回 `(config, runtime_dir, ocr_device_before)`；
  快照段再单独 `spawn_blocking`；两处都 `map_err` 出人话（`保存设置失败: …` / `读取设置失败: …`）。
  与 `runtime_settings`（`:739-752` 本来就是 spawn_blocking）口径一致。
  验证：`cargo test --lib` **381 passed / 0 failed**，`cargo check --lib` 零告警；
  签名/行为不变（仍是 `Result<SettingsSnapshot, String>`），前端无需改动。
- **P2-17** ✅ **已修复**（2026-11-16，本分支，`src-tauri/src/ai_runtime/runtime_install.rs:724-733`）——
  `run_install` 用 `RuntimeConfig::load(&runtime_dir).unwrap_or_default()`：config.json 损坏时
  静默退回默认下载源/分段，用户明明改过设置却没有任何线索（装完才发现「设置没生效」）。
  改成 `match`：Err 时先 `eprintln!("[ai_runtime] 读取配置失败，本次按默认参数安装: {e}")`
  再用 `default()`。行为不变（安装仍继续，不因配置损坏而卡住安装入口），但原因不再被吞。
  验证：`cargo test --lib` **381 passed / 0 failed**，`cargo check --lib` 零告警。
- **P2-18** ✅ **已修复**（2026-11-16，本分支，`src/components/SettingsModal.vue:30-94`、`:330 / :391 / :407 / :420`）——
  前端候选表远窄于后端：`segment_mb` 只列 0/16/64/256（后端 0–1024）、`moss_threads` ≤32（后端 0–256）、
  `moss_timeout_minutes` ≤120（后端 0–1440）、`ocr_device` 只有 ""/cpu/gpu:0（后端还收 npu/xpu）。
  手改 config.json 或旧版本留下的 32 / gpu:1 这类值，下拉框显示为空，用户切走一次就再也选不回来。
  改法：新增 `withCustom(opts, cur)`——当前值不在候选表里时动态补一项「自定义（<值>）」；
  四个下拉框改用 `withCustom(...)`，并把两处内联 `.map()` 字面量提成模块级
  `MOSS_THREAD_OPTIONS` / `MOSS_TIMEOUT_OPTIONS`（顺带止住每次 patch 重建数组）。
  验证：`npx vue-tsc --noEmit` EXIT=0；抽 `withCustom` 真实实现到 node 跑 8 条断言 **8/8 PASS**
  （枚举值不添加、null/undefined 不添加、0 不被当空值、非枚举数值/字符串各补一项且标签为「自定义（…）」）。
- **P2-19** ✅ **已修复**（2026-11-16，本分支，`src/stores/timeline.ts:243-250`、
  `src/stores/project.ts:242-244 / :277-281`、`src/components/SaveStatusIndicator.vue:20-21`）——
  两个小毛病同源：「状态灯说的不是真话」。
  ① `toggleSnap()` 只改内存，不写 `gsa.timeline.snap`；而设置面板的「默认开启吸附」写的是
     同一个键 ⇒ 本会话切换重启即丢，面板还显示旧值。改为顺带 `setSnapDefault(snapEnabled.value)`。
  ② 关掉自动保存后 `scheduleSave()` 早退却仍置 `saveState = "pending"`，于是每次改动状态灯都
     亮「未保存」且永不复位（其实根本没有定时器要跑）。新增中性态 `"manual"`，状态灯映射为
     「自动保存已关」（灰点），与 `pending`（黄点、马上就写盘）区分开。
  验证：`npx vue-tsc --noEmit` EXIT=0（`saveState` 的字面量联合类型与 `statusMeta` 的 switch
  同步扩展，漏写分支会被穷尽性检查抓到）。
- **P2-20** ✅ **已修复**（2026-11-16，本分支，`src/stores/timeline.ts:6 / :43-46`）——
  `gsa.timeline.splitStrategy` / `gsa.timeline.mergeStrategy` 这两个键在`appPrefs.ts:14-15`（`PREF_KEYS`）与 `timeline.ts:43-44` 各写了一份字符串，而 `PREF_KEYS` 的那两个导出无人使用。
  改一边漏一边就会静默变成两个互不相干的偏好。改为 `const SPLIT_KEY = PREF_KEYS.splitStrategy;`
  / `const MERGE_KEY = PREF_KEYS.mergeStrategy;`（`readStrategy` 的兜底逻辑与 `SPLIT_VALUES`/`MERGE_VALUES` 校验不动）。
  验证：`npx vue-tsc --noEmit` EXIT=0；`grep` 确认 `timeline.ts` 内已无字面量键名，键值本身未变（仍写 `gsa.timeline.*`，老数据继续可读）。
- **P2-21** ✅ **已修复**（2026-11-16，本分支，`src/views/AsrView.vue:530 / :536 / :642-645`、
  `src/stores/ui.ts:71-90`、`src/main.ts:13-15`）——三条互不相干的小口子：
  ① `.arow-acts` 是全仓零规则的死类名（两处模板拼了它，没有一条 CSS 命中），直接去掉，
     布局全由 `.arow` + `.astack` 决定；② `.arow-msg .enote` 补 `min-width: 0` —— flex 项默认
     `min-width: auto`，长路径/长报错会按最长不可断词撑宽父级，把 `.accb` 的 `overflow-x: auto`
     撑出一条横向滚动条；③ store 求值期副作用（写 `data-theme` + `void applyZoom(...)`）收进
     显式 `initUiShell()`：原先任何 import 到 ui store 的路径（包括 Node 里跑 store 逻辑的测试）
     都会连带触发一次 IPC 与 DOM 写入。调用点只有一处 —— `src/main.ts` 的 `useUiStore().initUiShell()`，
     时序与注释原意一致（挂载前写主题，避免首帧闪色）。
  验证：`npx vue-tsc --noEmit` EXIT=0；`pnpm build` EXIT=0（js 713.05 kB / gzip 221.90 kB）；
  Node harness 6/6 PASS（"import 不发 IPC"、"initUiShell 恰好发一次 set_ui_zoom"、
  "写入 data-theme"），同一 harness 对修复前的 `ui.ts` 为 2/4（失败项：实例化即发 IPC、无 initUiShell）。
- **P2-22** ✅ **已修复**（2026-11-16，本分支，`scripts/verify_cdn_mirror.ps1:47-125`、
  `scripts/bootstrap_runtime.ps1:878-894 / :958 / :995-996`）——
  ① `verify_cdn_mirror.ps1` 的 `$ErrorActionPreference` 还原原先排在采样循环之后，中途
     `throw` / `Ctrl+C` / 参数非法都会把全局偏好永久留在 `Continue`（诊断脚本后续行为全变）；
     现把采样体整段包进 `try { … } finally { $ErrorActionPreference = $prevEap }`（块内不重排缩进，
     diff 保持最小）。
  ② `bootstrap_runtime.ps1` 的两处 `Remove-Item $tmpDir -Recurse -Force` 未用 `-LiteralPath`
     （`[ ] * ?` 会被当通配符）、也未校验路径在系统临时目录内 —— 今天不可利用（7 个 manifest id
     都是静态字面量），但这是「以后加一条清单项」最容易变成真问题的地方。
     新增 `Remove-TempArtifact($path, -Recurse)`：`-LiteralPath` + `GetFullPath` 后断言落在
     `GetTempPath()` 之内，越界只 `Write-Warning` 不删；`Expand-Archive` 前与 `finally` 两处改用它。
  验证：两脚本 `PARSE_ERRORS=0`、`BOM=EF BB BF`、CRLF-only（`bootstrap_runtime.ps1` 1124 行 /
  `verify_cdn_mirror.ps1` 147 行，LF-only 均 0）。`Remove-TempArtifact` 单测：临时目录内真删（True）、
  仓库内路径被拒且保留（True）。EAP 用例：脚本内 `throw` 后外层 `$ErrorActionPreference` 仍为 `Stop`。
  端到端：`verify_cdn_mirror.ps1` 对真实 ModelScope 直链跑通（206 / TTFB 0.36s / 1.85 MB/s，EXIT=0）；
  `bootstrap_runtime.ps1 -Items moss -Flavor cpu` 装成功（`{"ok":true}`，11 个文件），
  `%TEMP%` 下 `gsa_rt_*` 残留 0 个。
- **P2-23** ✅ 已修复（同一 PR 内文档漂移，9 处口径全部按落地版校正）
  - `docs/SETTINGS_PANEL_SCOPE.md`：`gsa.theme` 的键定义点由 `ui.ts:6` 改为
    `appPrefs.ts:6`（`ui.ts:14` 只是取别名）；「侧栏默认折叠」明确写成
    `gsa.sidebarDefault`（`appPrefs.ts:9`）并与 `gsa.sidebar`（`ui.ts:15`）区分；
    `GSA_OCR_DEVICE` 行号 `paddle.rs:187` → `:185`；`MAX_HISTORY` 的
    `project.ts:307` → `:315`；吸附默认开关由「未持久化」改为「已持久化
    `gsa.timeline.snap`（`appPrefs.ts:13`，`timeline.ts:67` 读初值、`:249` 写回）」；
    自动保存延迟由「硬编码 1000 ms」改为「已可配置 `gsa.autosaveDelay`
    （`appPrefs.ts:19` 可选 `[300,1000,3000]`，`project.ts:284` 消费）」；
    快捷键动作数「22」→「**21** 条 / 27 键位」；§3.2 调用点由「7 处」改为
    「**13 处（已完成）**」并换成实测行号（`lib.rs:39`、`config.rs:562/590/798/
    827/1023/1157/1224`、`backend_probe.rs:807`、`installed_state.rs:337/363`、
    `runtime_install.rs:498`、`setup.rs:261`）；§3.3–3.5 三条（移动流程 / 失败回滚 /
    禁止目标）按 P1-2、P2-12、P2-13、P2-14 的落地版重写；§4 命令表
    `move_runtime_dir(target, delete_source: bool)` 改为
    `move_runtime_dir(target: string)` 并注明「没有 `delete_source` 参数，旧目录由
    `delete_old_runtime_dir()` 单独删」。
  - `docs/RUNTIME_BOOTSTRAP_CONTRACT.md`：参数表补上漏掉的两行 `-Source`
    （`auto`/`mirror`/`direct` 三档）与 `-RuntimeDir`（默认 `<脚本目录>/../runtime`，
    GUI 每次都显式传，`runtime_install.rs:711-726`）。
  - 复测：`git diff --stat` = 2 files / 14 insertions / 12 deletions；
    `Select-String` 复查每条替换后的行号与数字与代码一致（本报告自身引用的
    行号亦已按当前文件核对）。
