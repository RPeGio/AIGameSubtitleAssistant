# 设置面板（第 19 项）可调条目清单

> 状态：**已实现（2026-11-02，分支 `feat/first-run-prep`）** —— 实现摘要见 `docs/MISC_TODO.md` 第 19 项，命令契约见 `docs/RUNTIME_BOOTSTRAP_CONTRACT.md` §8.10
> 日期：2026-11-02
> 依据：用户 2026-11-02 决策 —— ① UI 用 **NModal 弹层**（不新增路由页）；② runtime 默认仍存 `%APPDATA%`（现状），设置里提供「移动整体 runtime 目录」，**需重启后生效**。

---

## 0. 两条已定决策

| 决策 | 结论 | 理由 |
|---|---|---|
| UI 形态 | **NModal 弹层** | 设置是低频、跨页面共用的；独立路由页会打断「打轴中改一下线程数」的动线 |
| runtime 位置 | 默认 `%APPDATA%`，可整体搬走，**重启生效** | 运行中的进程把 `runtime/bin/*.dll`、`python.exe` 全锁着，热搬必然失败；整目录搬也让 `validate()` 的「路径不得越出 runtime」约束不用改 |

---

## 1. 条目清单

列说明：**现状** = 现在存在哪儿 / 有没有 UI；**生效** = 改完什么时候起作用；**新增命令** = 是否要动 Rust。

### A. 外观与界面（纯前端，localStorage）

| 条目 | 现状 | 生效 | 新增命令 | 建议 |
|---|---|---|---|---|
| 主题（深 / 浅） | 已实现：`gsa.theme`（`src/stores/ui.ts:6`），顶栏有切换按钮 | 即时 | 否 | **v1**：设置里放同款分段控件，与顶栏双向同步 |
| 侧栏默认折叠 | 已实现：`gsa.sidebar`（`ui.ts:7`） | 即时 | 否 | **v1**：只是把已有开关挪一份进设置 |
| 界面缩放（0.9 / 1.0 / 1.1 / 1.25） | **无** | 即时 | 否 | v2：需要给 `<html>` 设基准字号并逐页核对，工作量在回归测试 |
| 校对区宽度 | **固定 `min-width: 520px`**（`ReviewPane.vue:123`），不可拖 | — | 否 | v2：属于交互改造（拖拽手柄），不是设置项 |

### B. 运行时与下载（Rust，写 `runtime/config.json` + 目录指针）

| 条目 | 现状 | 生效 | 新增命令 | 建议 |
|---|---|---|---|---|
| 运行时目录（显示 + 移动） | 写死 `app_data_dir()/runtime`，7 处调用 `resolve_runtime_dir()` | **重启** | **是**（`move_runtime_dir`） | **v1**（用户已定） |
| 下载源（自动 / 镜像 / 直连 / 自定义前缀） | 清单 `urls[]` 顺序 + 脚本 `-Mirror` 默认 `https://gh-proxy.com/` 写死（`bootstrap_runtime.ps1:56`） | 下次安装 | **是**（`install_runtime` 加参） | **v1** |
| 分段下载开关 / 分段大小 | 脚本 `-SegmentMB` 默认 64，UI 不可见 | 下次安装 | 是（同上） | v2（高级折叠区） |
| 首启自动准备（开 / 关） | 固定自动开跑（`SetupGate.vue` `onMounted`） | 下次启动 | 是（`runtime_status` 读配置） | v2 |
| 校验 / 重装运行时 | 准备页失败态有「重试」；欢迎页有「重装运行时」 | 即时 | 否（复用 `install_runtime`） | **v1**：设置里给一个入口 |
| 打开日志目录 | 无 | — | 是（`open_log_dir`，或前端用 shell 插件） | **v1**（排障必需，装不上时用户第一件事） |

### C. 引擎与设备（Rust，`runtime/config.json`）

| 条目 | 现状 | 生效 | 新增命令 | 建议 |
|---|---|---|---|---|
| MOSS 推理设备 | 已有 UI：打轴页「转写参数 → 推理设备」（`set_moss_device`） | 即时（重建 provider） | 否 | 设置里**只读显示**并给「去打轴页改」的跳转，避免双份真相 |
| FunASR 推理设备 | 同上（`set_funasr_device`） | 即时 | 否 | 同上 |
| OCR 推理设备 `ocr_device` | **无 UI**，但真的生效（`paddle.rs:187` → `GSA_OCR_DEVICE`） | 下次 OCR 启动 | 是（`set_ocr_device` 或通用 patch） | **v1** |
| OCR 模型 `ocr_model` | **无 UI**，生效（`paddle.rs:209` → `GSA_OCR_MODEL`） | 下次 OCR 启动 | 是 | v2（高级，写错就废） |
| MOSS 线程数 `moss_threads` | 无 UI，生效（`moss.rs:294`） | 即时 | 是 | **v1** |
| MOSS 超时 `moss_timeout_minutes` | 无 UI，生效（`moss.rs:296`） | 即时 | 是 | v2（长音频才需要） |
| FunASR 语言 `funasr_language` | 无 UI，生效（`funasr.rs:102`） | 即时 | 是 | v2 |
| FunASR 超时 `funasr_timeout_minutes` | 无 UI，生效（`funasr.rs:103`） | 即时 | 是 | v2 |
| 默认转写引擎 | 前端 `asrParams.engine`，**未持久化**（每次回 moss） | 即时 | 否（localStorage） | **v1** |
| 保留中间产物 `dev_debug` | 无 UI，生效（多文件） | 即时 | 是 | v2（诊断折叠区） |

> 死字段（除 `config.rs` 外无引用，不建议进面板）：`language`、`asr_provider`。

### D. OCR / 校对默认参数（前端，现按栏目存 localStorage）

现状：`gsa.ocrParams.corpus` / `gsa.ocrParams.asr`（`src/composables/ocrDefaults.ts:41`），逐字段与内置默认合并 —— **已经是"每栏目一套参数"的模型**。

条目：帧间隔、dhash 阈值、批大小、相似度合并阈值、最短时长、最短字数、标点规则（括号对 / 省略号 / 修正行尾 j）、术语表。

> 建议：设置面板**不要**复制一份这些参数，只放一行「默认值跟随当前栏目」+ 跳转按钮。理由：两套入口必然漂移，而它们的作用域本来就是"栏目"。

### E. 时间轴与编辑

| 条目 | 现状 | 生效 | 新增命令 | 建议 |
|---|---|---|---|---|
| 吸附默认开关 | `timeline.ts:64` 默认 true，**未持久化** | 即时 | 否 | **v1** |
| 分割策略默认 | 已持久化 `gsa.timeline.splitStrategy`（`punct/space/raw`） | 即时 | 否 | **v1** |
| 合并策略默认 | 已持久化 `gsa.timeline.mergeStrategy`（`latter/former/stack`） | 即时 | 否 | **v1** |
| 撤销深度 | 硬编码 `MAX_HISTORY = 60`（`project.ts:307`） | 重启 | 否 | v2 |
| 自动保存延迟 | 硬编码 1000 ms（`project.ts:273`） | 即时 | 否 | v2 |

### F. 快捷键（只读）

现状：`src/composables/shortcuts.ts:23-47` 全部硬编码，无自定义能力（22 个动作）。

> 建议 v1 只做**只读对照表**（用户查键位），自定义改键单独开一项（要处理冲突检测、平台差异、导入导出）。

---

## 2. 存储方案（建议）

| 类别 | 落点 | 谁读 | 理由 |
|---|---|---|---|
| 引擎 / 路径 / 线程 / 超时 / 下载源 | `runtime/config.json`（`RuntimeConfig`） | Rust | 已经是 Rust 的唯一配置源，且 `validate()` 已按「严格组 / 宽松组」校验 |
| 界面 / 默认策略 / 默认引擎 | localStorage（`gsa.*`） | 前端 | 纯展示层，零 IPC，跨版本兼容靠"读不到就回默认" |
| 运行时目录指针 | `app_data_dir()/runtime-location.json` | Rust（`resolve_runtime_dir` 最先读） | 必须独立于 runtime 目录本身 —— 搬走之后原路径就没东西可读了 |

## 3. 「移动 runtime 目录」实现要点

1. **指针文件优先**：`resolve_runtime_dir()` 当前顺序是 `GSA_RUNTIME_DIR`（须含 `config.json`）→ 从 exe 向上找 → `fallback`。插入第一级：`app_data_dir()/runtime-location.json` 里的 `runtime_dir`（目录存在才采纳，否则告警并回退）。
2. **7 处调用点改造**：现在都传 `app_data_dir()/runtime`，改为传 `app_data_dir()`，由 `resolve_runtime_dir` 自己拼 `runtime` 或读指针（`backend_probe.rs:808`、`config.rs:435/463`、`installed_state.rs:338/366`、`runtime_install.rs:499`、`setup.rs:257`）。
3. **移动流程**：目标目录必须为空或不存在 → 递归复制 → 逐文件比对大小（大文件可只比大小 + 抽查 sha）→ 写指针 → 提示「重启后生效，旧目录可手动删除」。
4. **失败回滚**：复制中途失败不写指针，保留旧目录；UI 报出失败的文件与原因。
5. **禁止目标在旧目录内**、禁止目标为盘符根、禁止目标是正在使用的 `%APPDATA%` 子路径（防递归复制）。

## 4. 需要新增的 Rust 命令（草案）

| 命令 | 作用 |
|---|---|
| `runtime_settings()` | 一次性返回：runtime 目录、可用磁盘、各引擎配置（脱敏只读值）、下载源、日志目录 |
| `update_runtime_settings(patch)` | 局部更新 `config.json` 的**非路径**字段（线程/超时/设备/下载源），走 `validate()` 后原子写 |
| `move_runtime_dir(target, delete_source: bool)` | 搬迁 + 校验 + 写指针；返回 `{ok, message, needs_restart}` |
| `open_log_dir()` | 用系统文件管理器打开 `runtime/logs` |

> 设备类沿用已有的 `set_moss_device` / `set_funasr_device`（它们会顺带重建 provider，别重复实现）。

## 5. 待确认

1. **v1 范围**：A+B+C（部分）+E 基础项够不够？还是先只做 B+C 运行时/引擎？
2. **下载源粒度**：三档（自动 / 强制镜像 / 直连）够用，还是要允许填自定义前缀 URL？
3. **移动语义**：复制成功后是否**自动删旧目录**？（默认建议：不自动删，提示用户手动删，避免误删正在被占用的文件）
4. **D 组处理**：认可「设置里只放跳转、不复制参数」吗？
