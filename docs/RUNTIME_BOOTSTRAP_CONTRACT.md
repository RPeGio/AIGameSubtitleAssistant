# runtime 安装器（bootstrap）↔ 前端/配置 契约

> **状态**：契约已由用户在 2026-10-21 逐条确认（会话 v4 的 m011250 及其后续澄清）。
> **本文用途**：把 installer 侧（另一会话负责）与前端/配置侧（本分支负责）之间的
> 接口固定下来，供两个会话并行开工。本文件是**前端侧视角的契约输入**；
> 若 installer 侧另写分发文档，以分发文档为准，本文件作为其附件与冲突仲裁依据。
>
> **文件所有权**（用户选 A 方案：跨界文件由对方改，我方只给补丁）：
>
> | 归属 | 文件 |
> |---|---|
> | 前端/配置侧（本分支） | `src-tauri/src/ai_runtime/config.rs`、`src-tauri/src/ai_runtime/backend_probe.rs`、`src-tauri/src/ai_runtime/moss.rs`、`src/` 全部 |
> | installer 侧（另一会话） | `scripts/*`、`src-tauri/tauri.conf.json`、`runtime/` 装配、分发文档 |
> | 跨界文件（**只有这一个**，双方都不直接改；需动时由本分支出补丁、installer 侧套用） | `src-tauri/src/ai_runtime/mod.rs` |
>
> **`src-tauri/src/lib.rs` 的归属变更（2026-10-22）**：用户授权本分支直接改，已在
> `invoke_handler` 里加好两行注册（`probe_moss_backend` / `set_moss_device`，见 §7）。
> installer 侧**不要重复添加**，合入时以本分支这两行为准。

---

## 1. 目标

用户机器首次启动时 runtime 为空。本期只做**一个条目**（`moss`），但安装器从第一天起
按「多条目 + 多 flavor」设计，后续条目（`deps-ocr` / `deps-funasr` / `deps-embed` /
`models-moss`）只加 manifest 条目、不改脚本逻辑。

分发形态（已达成共识）：**预构建二进制为主、自构建兜底**。MOSS 上游 LICENSE 实为 MIT，
可合法再分发；模型仍走原渠道（HF / ModelScope，带哈希）。

---

## 2. 安装器入口与参数

- **入口**：`scripts/bootstrap_runtime.ps1`
  （与既有 `bootstrap_{ocr,ocr_gpu,embed,funasr,moss,llm}.ps1` 家族命名保持一致；
  **不叫** `install_runtime.ps1`）
- **参数**：

| 参数 | 取值 | 说明 |
|---|---|---|
| `-Items <ids>` | 逗号分隔的条目 id | 本期 manifest 只有 `moss`；后续见 §4 |
| `-Flavor <auto\|cpu\|cuda\|vulkan>` | 默认 `auto` | `auto` = L0 机器能力探测，判定顺序同 `backend_probe`：**nvidia-smi → vulkaninfo → cpu** |
| `-Mirror <url>` | 默认 gh-proxy 代理前缀 | **代理前缀语义**（见 §4.1），非资源基址 |
| `-Force` | 开关 | 忽略已安装标记，强制重装 |
| `-Manifest <path>` | 默认 app 内置 manifest | 覆盖内置清单（离线/自测用） |

- **无参运行** = `-Items moss -Flavor auto`，即「首启默认动作」。

---

## 3. 进度输出协议

- **stdout**：**只输出 JSONL**，每行一个事件对象；无任何人类可读文本混入。
- **stderr**：人类可读的诊断信息（进度细节、错误堆栈、下载日志）。
- **最后一行 stdout 必为 `event:"done"`** 的汇总事件。

事件 schema（字段稳定，缺省字段省略）：

```json
{"event":"start","item":"moss","flavor":"cuda","detail":"开始安装 moss（cuda）"}
{"event":"progress","item":"moss","phase":"download","percent":42.3,"detail":"下载中"}
{"event":"item_done","item":"moss","flavor":"cuda","phase":"extract","detail":"已安装"}
{"event":"item_skip","item":"moss","detail":"已安装且版本一致"}
{"event":"item_fail","item":"deps-ocr","detail":"哈希不匹配"}
{"event":"error","item":"moss","detail":"磁盘空间不足"}
{"event":"done","ok":true,"installed":["moss"],"skipped":[],"failed":[]}
```

- `event` 取值集合（冻结）：`start` / `progress` / `item_done` / `item_skip` /
  `item_fail` / `error` / `done`。
- `phase` 取值集合（冻结）：`download` / `verify` / `extract` / `configure`。
- `percent` 为 0–100 的浮点（仅 `progress` 事件带）。
- `done` 事件里的 `failed` 明细即退出码 3 的明细来源。

---

## 4. manifest 规范

- **落点**：`src-tauri/resources/runtime/manifest.json`，**随 app 发布并提交进仓库**。
  - **状态（2026-10-24）**：真清单已由本分支落库（内容由 installer 侧生成，`version:"1"`，
    三个 `id=moss` 条目 cpu / vulkan / cuda，`dest` 全为 `bin`）；`tauri.conf.json` 的
    `bundle.resources` 本分支已加一行 `"resources": ["resources/runtime/manifest.json"]`
    ——installer 侧**不要再重复加**（重复加会变成合并冲突）。
  - **打包落点**：数组形式会**保留目录结构** ⇒ 清单实际落在
    `$RESOURCES/resources/runtime/manifest.json`（不是 `$RESOURCES/runtime/…`）。
    Rust 侧候选路径已覆盖这个形状（§8.1），别照"平铺"的直觉去改。
  - **入库坑**：根 `.gitignore` 有一条 `runtime/`（指用户数据目录），它会顺带吃掉
    `src-tauri/resources/runtime/`，导致清单**静默不入库**。已在 `.gitignore` 加
    `!src-tauri/resources/runtime/` 例外（父目录被排除后无法只放行子文件，故整目录放行）。
    谁再动 `.gitignore`，用 `git check-ignore -v src-tauri/resources/runtime/manifest.json`
    确认没被重新吃掉。
- **脚本缺省读取 app 内置 manifest**，`-Manifest` 可覆盖。
- **schema（稳定字段冻结）**：

```json
{
  "version": "1",
  "generated_at": "2026-10-21T00:00:00Z",
  "items": [
    {
      "id": "moss",
      "flavor": "cuda",
      "url": "https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/moss-runtime-v1/moss-cuda.zip",
      "sha256": "…",
      "size": 123456789,
      "dest": "bin",
      "backend_marker": "cuda"
    }
  ]
}
```

| 字段 | 必填 | 语义 |
|---|---|---|
| `id` | ✔ | 条目 id（`moss` / `deps-ocr` / `deps-funasr` / `deps-embed` / `models-moss`） |
| `flavor` | ✖ | 缺省 = 该条目与 flavor 无关 |
| `url` | ✔ | 下载地址，**写绝对地址**；`-Mirror` 缺省时按 §4.1 拼接 |
| `sha256` | ✔ | 下载物哈希，校验不过 = 退出码 2 |
| `size` | ✔ | 字节数，用于磁盘预检 |
| `dest` | ✔ | 相对 runtime 目录的落地路径（**moss 三个 flavor 一律 `bin`**，见 §4.2） |
| `backend_marker` | ✖ | 写入落地目录的 `.backend` 标记（`cuda` / `vulkan`；cpu 版写 `cpu` 或不写） |

- **flavor 与 id 的组合**：本期只有 `moss` 一个条目，但 `moss` 条目按 flavor 拆成多条
  （`id=moss, flavor=cuda` / `id=moss, flavor=vulkan` / `id=moss, flavor=cpu`），
  由 `-Flavor` 选择。

### 4.1 `url` × `-Mirror` 的组合语义（已澄清）

`-Mirror` 是**代理前缀**，不是资源基址——`gh-proxy` 的正确用法是
`https://gh-proxy.com/https://github.com/...`，把它当基址去拼相对路径会拼出坏 URL。

installer 侧实现（已与前端侧对齐）：

| manifest `url` | `-Mirror` | 实际请求地址 |
|---|---|---|
| 绝对地址 | 空 | 绝对地址直连 |
| 绝对地址 | 非空（默认 gh-proxy） | `<mirror><url>`（前缀拼接，不插入分隔符） |
| 相对路径 | 必须是完整基址 | `<mirror>/<url>` |

⇒ **manifest 里一律写绝对地址**；相对路径只作自测/离线场景的兼容分支。

### 4.2 为什么 moss 三个 flavor 的 `dest` 必须都是 `bin`

三个硬约束叠在一起，只能装进同一个目录、靠 `.backend` 标记区分 flavor：

1. 配置文件只有**一个** `moss_binary` 字段，示例值 `"bin/moss-transcribe.exe"`
   （`config.rs:38-40`、`config.rs:414`）——三份二进制并列子目录时，app 不知道去哪个子目录找；
2. ggml 运行库**同名互斥**（`ggml.dll` / `ggml-base.dll` / `ggml-cpu.dll` 三 flavor 同名），
   分目录存放也会因 loader 只抓 exe 同目录而打架；
3. L1 资产探测 `probe_assets()` 固定读**二进制同目录**的 `.backend` 与
   `ggml-cuda.dll` / `ggml-vulkan.dll`（`backend_probe.rs:209-222`）。

### 4.3 交付物是 zip（不是 7z）、以及解压的三个坑

实际发布的资源是 **zip**（`moss-runtime-v1` 下的 `moss-cpu.zip` / `moss-vulkan.zip` /
`moss-cuda.zip`）。**不要按扩展名写死 `.7z`**——那是早期示例留下的，会 404。

installer 侧实测（2026-10-24）确认过、且容易踩的三点：

| # | 现象 | 要求 |
|---|---|---|
| 1 | zip 条目带 `./` 前导（`./moss-transcribe.exe`） | 解压实现必须先规整条目名再去 `./`。`Expand-Archive -DestinationPath runtime\bin` 实测是**直接平铺** 11 个文件，不会出现 `bin\bin` 嵌套；但自研解压（Rust `zip` crate / .NET）若直接拼接会落成 `bin\./xxx` ⇒ 一律去掉前导 `./` 与 `.\`，并且**按 `dest` 展开到 `runtime/<dest>`，不要当成自带一级目录的包** |
| 2 | 包内已有 `.backend` 标记（cpu 包内容 `cpu`、vulkan 包内容 `vulkan`） | 按 `backend_marker` 再写一次是**幂等覆盖**，不冲突；**但别指望它区分后端**——它只是校验用 |
| 3 | `.backend` 看起来能"选后端" | 真正的判据永远是 L2 实测（`moss-transcribe info` 的 `[mt I] backend:` 行）+ L1 资产；`.backend` 只是缓存提示，标记与请求不符时 `decide()` 会**拒绝**而不是信任它 |

⇒ 换 flavor = 覆盖 `runtime/bin` 下的同名文件 + 重写 `runtime/bin/.backend`。
这也是 installer 侧「装另一种 flavor 前需 -Force 或先清理旧 flavor」的原因。

---

## 5. 退出码表（写进分发文档的错误码表）

| 码 | 含义 | 说明 |
|---|---|---|
| 0 | 全成功 | 所有请求条目安装成功（或按 `item_skip` 跳过） |
| 1 | 环境 / 预检失败 | **磁盘空间预检不过在第一步就拒绝**；nvidia-smi / vulkaninfo 缺失且 `-Flavor` 显式要求该后端 |
| 2 | 校验失败 | 下载物 sha256 与 manifest 不符 |
| 3 | 部分条目失败 | 明细写在 stdout 最后一行 `done` 事件的 `failed` 里 |
| 4 | 未知错误 | 未分类异常 |

> 磁盘预检：按 `manifest` 中待装条目的 `size` 求和 + 落盘展开余量，
> 不足则**在下载任何东西之前**退出码 1 拒绝。

---

## 6. 前端 / 配置侧如何消费

### 6.1 设备下拉的取值过滤（第 12 项）

- 下拉候选 = `auto`（空值）/ `cuda` / `vulkan` / `cpu`。
- **只有 manifest 里存在的 flavor 才给选**：`items[]` 中 `id == "moss"` 的条目所覆盖的
  flavor。
- **manifest 读不到时 → 退化显示全部四项**，并在下拉下方标注「未读取到安装清单，
  暂显示全部设备选项」。
  - 清单到位前（2026-10-24 之前）**一直**走这个退化分支，属预期；清单到位后只在
    清单文件缺失/损坏时才触发。⇒ 过滤生效后要**重测一次下拉项**，别把退化表现当成新 bug。
  - 注意：当前发布的清单声明了 cpu / vulkan / cuda **三种** flavor，所以正常情况下列表
    仍是四项全可选、与退化分支**看起来一样**（差别只在有没有那行标注）。
    要验证过滤真的起作用：临时把清单里某个 flavor 条目删掉（或用 `-Manifest` 指向精简清单）
    → 对应项必须消失；已保存值不在清单里时仍要能看到它（带「manifest 未提供」后缀）。
- 配置文件字段 `moss_device` 白名单维持 `"" | cpu | cuda | vulkan`（`config.rs:307-316`），
  `"auto"` 归一为 `""` 的逻辑在 `backend_probe::normalize_device_request` 与
  `config::normalize_moss_device` 两处（前者判定用、后者写盘用）——
  **写盘一律写 `""` 而不是 `"auto"`**。
- 过滤由 Rust 命令完成（决策 D4-A）：`probe_moss_backend` 返回
  `manifest_found` + `manifest_flavors`，前端只做渲染，不自己读文件、不加 fs 插件权限。

### 6.2 需要 installer 侧提供的能力（前端侧依赖清单）

| 依赖 | 由谁提供 | 前端如何使用 |
|---|---|---|
| manifest 可读（id → flavor 集合） | **已就位**（2026-10-24：清单落库 + `bundle.resources` 已加） | 前端读 Rust 命令回传的 flavor 过滤下拉项；读不到则退化显示全部 |
| 装完一项的落盘标记 `runtime/.installed.json` | installer 侧**只负责写** | `{items:{<id>:{flavor, sha256, source_version, installed_at}}}`；**app 启动侧的比对逻辑本期不做**——它住在 `ai_runtime/mod.rs`（跨界文件，见 §7），等本分支出补丁、installer 侧套用 |
| 磁盘空间预检 / 退出码语义 | installer 侧 | 前端只展示引导文案，不自己算 |

### 6.3 三条风险（用户已全部采纳）

1. **磁盘空间预检** → installer 第一步做，不过则退出码 1 拒绝。
2. **`requirements.lock`** → Phase 2+ 的 `package_deps.ps1` 用 pip hash-checking mode
   （`--require-hashes`）锁索引与 wheel 哈希。
3. **`runtime/.installed.json` 版本门** → 每装完一项落盘（installer 侧）；app 启动时比对
   是 `ai_runtime/mod.rs` 的活（跨界文件，见 §7）——**本期只写文件，不做比对**，
   等本分支出补丁后接通。以 §7 为准，§6.2 此处若与 §7 冲突按 §7 执行。

---

## 7. 跨会话补丁清单（本分支只出补丁，installer 侧套用）

| # | 目标文件 | 补丁内容 | 状态 |
|---|---|---|---|
| P1 | `src-tauri/src/lib.rs` | `invoke_handler` 注册 `ai_runtime::backend_probe::probe_moss_backend,` | **已由本分支直接改**（用户 2026-10-22 授权） |
| P2 | `src-tauri/src/lib.rs` | `invoke_handler` 注册 `ai_runtime::config::set_moss_device,` | **已由本分支直接改**（同上） |
| P3 | `src-tauri/src/ai_runtime/mod.rs` | *（可选）* `AsrManager` 增加「重置 moss provider」的方法，用于改设备后**即时**生效 | 不做（决策 D3-A：下次启动生效），仅在将来要即时生效时补 |
| P4 | `src-tauri/tauri.conf.json` | `bundle.resources` 加 `"resources": ["resources/runtime/manifest.json"]` | **已由本分支直接改**（2026-10-24 用户指示落清单时一并加好）；installer 侧勿重复添加 |

P1/P2 的命令函数本体放在 `backend_probe.rs` / `config.rs`——这两个模块**已在
`ai_runtime/mod.rs` 里声明**（`pub mod backend_probe;` / `pub mod config;`），
所以**除 `mod.rs` 外，双方不需要在任何文件上有协同**；`mod.rs` 本期无人改动。

---

## 8. 命令契约（第 12 项用）

### 8.1 `probe_moss_backend`（只读探测）

```ts
invoke("probe_moss_backend", { device?: string })  // device 省略/空 = 用配置文件现值
```

返回：

```jsonc
{
  "configured_device": "",      // 配置文件现值（""=auto）
  "requested": "cuda",          // 归一后的请求值 "auto"|"cpu"|"cuda"|"vulkan"
  "available": false,           // 显式请求不可用 = false（硬报错，绝不静默降级）
  "backend": "CUDA0",           // 实际生效后端 / L1 推断值；不可用时为空串
  "reason": "已请求 cuda，…",    // 人话依据，自检面板直接展示
  "source": "l2",               // l2 实证 / l1 资产推断 / requested 显式指定 / reject 硬报错
  "recommendation": "cuda",     // recommend_from_assets 的建议（该装哪个后端）
  "recommendation_reason": "已装 CUDA 版 moss（.backend 标记 / ggml-cuda.dll）",
  "machine_cuda": true,         // L0：NVIDIA 驱动可用
  "machine_vulkan": true,       // L0：Vulkan loader + GPU 可枚举
  "gpus": ["NVIDIA GeForce RTX 4060 Laptop GPU"],   // vulkaninfo 枚举到的 GPU 名
  "manifest_found": false,      // 找到安装清单？false ⇒ 前端退化显示四项
  "manifest_flavors": ["cuda", "vulkan", "cpu"],    // id=="moss" 的条目声明的 flavor
  "error": null                 // available=false 时的硬报错文案（可直接展示）
}
```

实现要求：
- 复用 `backend_probe::probe_machine()`（L0）、`probe_assets()`（L1）、
  `parse_info_stderr()`（L2）、`decide()`、`recommend_from_assets()`——**不重写判定逻辑**。
- L2 实证需要拉起 `moss-transcribe info <model>` 子进程（约 0.5 s），命令应为异步或
  允许失败降级到 L1 推断（`source: "l1"`）。
- **manifest 查找顺序**（`backend_probe.rs::manifest_candidates_for`，第一个存在的胜出）：
  1. `runtime/manifest.json`（installer 可写，便于热修清单）
  2. `<exe 目录>/runtime/manifest.json`
  3. `<exe 目录>/resources/runtime/manifest.json`（**打包布局**：`bundle.resources` 数组形式保留目录结构）
  4. `<exe 目录>/../../resources/runtime/manifest.json`（dev 布局：`target/debug/` → `src-tauri/resources/`）
  5. `<resource_dir>/runtime/manifest.json`（平铺布局，兼容 map 形式的 `bundle.resources`）
  6. `<resource_dir>/resources/runtime/manifest.json`（与第 3 条同源，非 Windows 平台 resource_dir ≠ exe 目录时兜底）
  ⇒ 清单放在 `src-tauri/resources/runtime/manifest.json` 并加进 `bundle.resources` 后，
  dev 与打包两种形态都能读到；第 3/4/6 条各有 `manifest_candidates_find_*` 单测钉住。

### 8.2 `set_moss_device`（落盘）

```ts
invoke("set_moss_device", { device: "" | "cpu" | "cuda" | "vulkan" })
```

- 白名单校验复用 `normalize_moss_device()`（`""`/`auto`/`cpu`/`cuda`/`vulkan`，归一后只有
  空串或三种具体后端；非法值 → 报错，不落盘，`config.rs:356-366`）。
- 落盘目标：`runtime/config.json` 的 `moss_device` 字段（原子写 `RuntimeConfig::save`）；
  读不到/坏文件直接报错，**不会用默认值覆盖**。
- 返回：`{ device: "cuda", message: "已保存 moss_device=cuda（下次启动生效）" }`
  （结构体 `MossDeviceSaveResult`，无 `ok` 字段——成功即返回，失败走 `Err`）。
- **生效时机**：MOSS provider 在 app 启动时 spawn 并绑定设备 ⇒ 改完配置
  **默认下次启动生效**；若 installer 侧套用补丁 P3，可做即时重置。

### 8.3 `check_asr_engines`（已有，不改）

已返回 `device`（配置值）与 `effective_backend`（L2 实测值，仅 moss 有），
欢迎页 ASR 卡的「设备 · …（实测 …）」一行就是它渲染的。

---

## 9. UI 侧硬报错口径（行为契约，UI 必须照此）

| 情形 | 行为 |
|---|---|
| 显式选 `cuda` / `vulkan` 但本机不具备该后端 | **硬报错**，提示「装对应后端」或「改回 auto / cpu」，**绝不静默降级**（ggml 自己会静默降级且 exit 0 ⇒ 必须靠 L2 实证拦下） |
| `auto`（空值） | CUDA → Vulkan → CPU；**reason 必须在自检面板可见** |
| 裸 `vulkan` 落到核显（`uma: 1`） | 同族可用，只提示落点问题（实测：双 GPU 机器裸 `vulkan` 落 `Vulkan0` 核显，`auto` 才落 `Vulkan1` 独显）⇒ **UI 默认值必须是 auto，不要引导用户选裸 `vulkan`** |

实测性能参考（同一段 5:14.99 音频）：CPU 680.6 s / Vulkan 134.7 s / CUDA 76.2 s
（Vulkan ≈ 1.77× CUDA、5.0× CPU；CUDA 8.9× CPU）。
