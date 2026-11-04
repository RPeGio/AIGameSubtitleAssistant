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
按「多条目 + 多 flavor」设计，后续条目（`deps-ocr` / `deps-embed` / `models-moss`）只加
manifest 条目、不改脚本逻辑。（`deps-funasr` 已随 FunASR 退役作废，不要再为其登记条目。）

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
| `-Items <ids>` | 逗号分隔的条目 id | 缺省 `moss,models-moss,deps-ocr,deps-ocr-gpu,deps-embed`；`deps-ocr-cuda-libs` 是"要 CUDA 运行库"的开关，不是清单条目（§4.3） |
| `-Flavor <auto\|cpu\|cuda\|vulkan>` | 默认 `auto` | `auto` = L0 机器能力探测，判定顺序同 `backend_probe`：**nvidia-smi → vulkaninfo → cpu** |
| `-Mirror <url>` | 默认 gh-proxy 代理前缀 | **代理前缀语义**（见 §4.1），非资源基址 |
| `-Force` | 开关 | 忽略已安装标记，强制重装 |
| `-Manifest <path>` | 默认 app 内置 manifest | 覆盖内置清单（离线/自测用） |
| `-SegmentMB <n>` | 默认 `64`，`0` = 不分段 | 分段大小；见 §4.5 |
| `-NoResume` | 开关 | 不读、不留 `.downloads` 缓存（每次从头下） |
| `-MaxAttempts <n>` | 默认 `3` | 单段失败重试次数 |
| `-PipMirror <url>` | 默认华为云 | CUDA 运行库 pip 索引（§4.3）；`-SkipGpuLibs` 可整段跳过 |
| `-PipUseProxy` | 开关 | **默认清空 `HTTP(S)_PROXY`** 跑 pip（代理常拉慢 pip），加了才走代理 |

- **无参运行** = `-Items <§2 表里的全部条目> -Flavor auto`，即「首启默认动作」（装齐运行时 + 模型 + OCR/向量运行时）。

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
  - **状态（2026-10-30 修订）**：清单共 **7 条**——`id=moss` 三条（cpu / vulkan / cuda，`dest` 全为
    `bin`）+ `id=models-moss` 一条（gguf 模型，**不带 flavor**，`dest=models/moss`）
    + `id=deps-ocr` 一条（OCR 运行时，`flavor=cpu`，**`dest="."`**，带 `unpacked_size`）
    + `id=deps-ocr-gpu` 一条（OCR 的 CUDA 增量，`flavor=cuda` + **`optional`**，`dest="."`）
    + `id=deps-embed` 一条（向量融合运行时，**不带 flavor**，`dest="."`，带 `unpacked_size`）。
  - **CUDA/cuDNN 运行库不进清单、不做资产**（原 `deps-ocr-cuda-libs` 条目已删除）：
    改由 `bootstrap_runtime.ps1` 在安装时用 `pip` 从 PyPI 镜像现拉（`nvidia-*` 共 8 个轮子，
    钉子版本 + 官方 sha256，见 §4.2）。理由两条：
    ① **许可**——CUDA Toolkit EULA 1.2.2 禁止把 SDK「作为独立产品分发」，1.1.2 要求可分发部分
    「仅你的应用可访问」；把裸 DLL 打成 zip 挂公开直链给任意人下载同时违反这两条。
    ② **体积**——`nvidia/` 一家 1.86 GB 打包 / 2.69 GB 解压，贴死 GitHub 单资产 **2 GiB** 上限
    （`gh` 会 422 `size must be less than 2147483648`）。
    脚本仍接受在 `-Items` 里出现 `deps-ocr-cuda-libs`（视作"要 CUDA 运行库"的开关），
    但它**不再是清单条目**，也不会产生对应下载。
    `tauri.conf.json` 的 `bundle.resources` 已有 `"resources/runtime/manifest.json"`
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
  "generated_at": "2026-10-30T03:01:00Z",
  "items": [
    {
      "id": "moss",
      "flavor": "cuda",
      "url": "https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/moss-runtime-v1/moss-cuda.zip",
      "sha256": "…",
      "size": 123456789,
      "dest": "bin",
      "backend_marker": "cuda"
    },
    {
      "id": "models-moss",
      "url": "https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/moss-runtime-v1/models-moss.zip",
      "sha256": "…",
      "size": 626397892,
      "dest": "models/moss"
    },
    {
      "id": "deps-ocr",
      "flavor": "cpu",
      "url": "https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/ocr-runtime-v1/ocr-runtime-cpu.zip",
      "sha256": "…",
      "size": 0,
      "unpacked_size": 0,
      "dest": "."
    },
    {
      "id": "deps-ocr-gpu",
      "flavor": "cuda",
      "optional": true,
      "url": "https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/ocr-runtime-v1/ocr-runtime-cuda-paddle.zip",
      "sha256": "…",
      "size": 502533301,
      "unpacked_size": 996251695,
      "dest": "."
    },
    {
      "id": "deps-embed",
      "url": "https://github.com/RPeGio/AIGameSubtitleAssistant/releases/download/embed-runtime-v1/embed-runtime.zip",
      "sha256": "…",
      "size": 157386205,
      "unpacked_size": 345868793,
      "dest": "."
    }
  ]
}
```

| 字段 | 必填 | 语义 |
|---|---|---|
| `id` | ✔ | 条目 id（`moss` / `deps-ocr` / `deps-ocr-gpu` / `deps-embed` / `models-moss`；`deps-funasr` 已随 FunASR 退役作废；`deps-ocr-cuda-libs` 已改为 pip 阶段，**不是清单条目**） |
| `flavor` | ✖ | 缺省 = 该条目与 flavor 无关 |
| `optional` | ✖ | `true` = 该条目在当前机器没有匹配 flavor 时**静默跳过**（不计失败）。OCR 的 CUDA 增量专用（AMD/Intel 机器 `-Flavor auto` 会选到 vulkan，而 OCR 没有 vulkan 版） |
| `url` | ✔ | 下载地址，**写绝对地址**；`-Mirror` 缺省时按 §4.1 拼接 |
| `urls` | ✖ | 多源候选（字符串数组，**顺序即优先级**）。存在时排在 `-Mirror` 拼接结果与 `url` 之前，见 §4.5。Rust 侧不消费该字段（`parse_manifest_items` 只取 `id/flavor/sha256/size/unpacked_size`），它是**下载器的输入** |
| `sha256` | ✔ | 下载物哈希，校验不过 = 退出码 2 |
| `size` | ✔ | 字节数，用于磁盘预检 |
| `unpacked_size` | ✖ | 解压后字节数。**有它就按它做预检**：`Σ下载 + Σ解压 + 最大单包解压 + 100 MB`（最后一项是临时目录与 dest 同时存在的那一份；早先漏了它，cuda 的 2.69 GB 包正好卡在这个差额上），没有才退回老的 `size × 3`。OCR 的 `deps` 有 24 065 个文件、压缩率只有 ~3 倍以内，`×3` 会低估 |
| `dest` | ✔ | 相对 runtime 目录的落地路径（**moss 三个 flavor 一律 `bin`**，见 §4.2；模型是 `models/moss`；**OCR 两个条目是 `"."`**，因为包内自带 `python/` `deps/` `models/paddleocr/` `worker/` 一级目录） |
| `backend_marker` | ✖ | 写入落地目录的 `.backend` 标记（`cuda` / `vulkan`；cpu 版写 `cpu` 或不写） |

- **`dest: "."` 的含义**：包内**保留自己的子目录结构**，直接展开到 runtime 根。
  这要求解压实现**保留目录结构**——早期实现按文件名平铺（`Join-Path $destDir $f.Name`），
  对 moss（11 个平铺文件）刚好没暴露，对 OCR 会把 `python/` `deps/` 压成同名文件堆。
  现实现用 `Copy-Item <tmp>\* -Destination <dest> -Recurse -Force`，对两种包都正确。
- **`-Items` 缺省 = `moss,models-moss,deps-ocr,deps-ocr-gpu,deps-embed`**：
  二进制、模型、OCR 三样缺一不可用（`probe_moss_install` 报 `model_missing`，OCR 则直接进不了
  项目）。`deps-ocr-gpu` 在缺省里但带 `optional`：非 NVIDIA 机器 `-Flavor auto` 选到
  cpu/vulkan，没有匹配 flavor ⇒ 自动跳过，不会让对方白下 1.5 GB。
  老调用方仍可显式把 `deps-ocr-cuda-libs` 放进 `-Items`，它只作为"要 CUDA 运行库"的开关，
  不查清单、不产生下载，由 §4.2 的 pip 阶段满足。

- **flavor 与 id 的组合**：
  - `moss` 按 flavor 拆成多条（`id=moss, flavor=cuda` / `vulkan` / `cpu`），由 `-Flavor` 选择；
  - `models-moss` **不带 flavor**：三种后端共用同一份 gguf，选择时直接命中，不参与设备探测；
  - 选 flavor 的可用集合 = **本次 `-Items` 选中条目的并集**（不是"第一个条目的"）。
    若选中的条目全都不带 flavor，`-Flavor auto` 直接跳过设备探测——曾经只看
    `items[0]`，导致 `-Items models-moss` 单装模型时被误判成"清单里没有可用的 flavor"。

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

### 4.3 CUDA/cuDNN 运行库：安装时 pip 现拉（不是清单条目）

`deps-ocr-gpu` 包里**只有 paddle 本体**；`nvidia/{cublas,cudnn,...}` 那 2.7 GB 由安装器在本地
用 OCR 基线自带的 `pip` 拉取，落到同一个 `runtime/deps_gpu`。这一段的契约：

| 项 | 值 |
|---|---|
| 触发条件 | `-Flavor` 解析结果为 `cuda`，且本次装了 `deps-ocr-gpu`（或调用方显式在 `-Items` 里写了 `deps-ocr-cuda-libs`） |
| 执行者 | `runtime/python/python.exe -m pip install`（因此**必须先装 `deps-ocr`**；缺 python 时该条目 `item_fail` 并给出提示） |
| 索引 | `-PipMirror`，缺省 `https://repo.huaweicloud.com/repository/pypi/simple`（实测 10.3 MB/s；阿里云 0.79、官方源 0.09；清华 TUNA 的目录接口会 403） |
| 钉版本 | 8 个 `nvidia-*` 轮子的 `==版本` + `--require-hashes`（sha256 取自 PyPI 官方元数据），清单在 `bootstrap_runtime.ps1` 的 `$GpuLibs` |
| 关闭方式 | `-SkipGpuLibs`（排障/离线）；`-PipUseProxy` 沿用系统代理（默认**临时清空** `HTTP(S)_PROXY`——国内镜像走代理反而慢） |
| 事件 | 与普通条目完全同形，`item = "deps-ocr-cuda-libs"`：`start` → 8×`progress`（phase=download，percent 12.5→100）→ `item_done`（detail 带落地文件数）|
| 幂等 | `.installed.json` 里记该条目的 `sha256` = **8 个包版本+哈希的内容摘要**；版本或任一哈希变化即视为需要重装 |
| 失败 | `item_fail` + 退出码 3（与其它条目一致） |

实测（2026-10-30，本机）：8 个轮子 1771 MB → 落地 `deps_gpu/nvidia` **2,568.4 MB / 258 文件**，
二次运行走 `item_skip`（0.5 s）。

### 4.4 交付物是 zip（不是 7z）、以及解压的三个坑

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

### 4.5 下载：多源候选 + 分段 + 断点续传（2026-10-30）

大陆到 GitHub Releases 实测 0.07–0.43 MB/s 且会断流，断流后旧实现（`WebClient.DownloadFileTaskAsync`）
**只能从头再来**——`deps-ocr-cuda-libs` 1.77 GB 连断三次就是在这个点上废掉的。现在改成
「按字节区间分段 + 段内续传 + 多源回退」，**不重打包资产、不加 shard 文件**（现有资产最大 502 MB，
重打包重传的代价远大于收益）。

| 环节 | 规则 |
|---|---|
| 候选源顺序 | `urls[]`（若有）→ `-Mirror` 前缀拼接（§4.1）→ `url` 直连。去重且保序 |
| Range 探测 | 每个候选先 `curl -r 0-0` 看是否 `206` + `Content-Range`；**不支持就只用它走单流、且不做续传**（gh-proxy 对大文件忽略 Range，`curl -C -` 会重复追加，实测把 1770 MB 下成 2036 MB） |
| 分段 | `-SegmentMB`（默认 64）；按总长切段，逐段 `curl -r from-to` 落到 `<runtime>\.downloads\<id>-<sha8>\segNNN.bin` |
| 段内续传 | 每段先写 `<segNNN>.bin.curlpart`；**下次运行先把它并进段文件**再算剩余区间 ⇒ 跨进程从断点继续 |
| 失败重试 | 段级 `-MaxAttempts`（默认 3），间隔 2 s；单流路径逐个候选源回退，全败抛 `所有候选源都失败` |
| **按源类型决定是否分段**（2026-11-02 修正） | 候选带 `Kind`：`cdn`（清单 `urls[]`）/`direct`（`url`）→ **探 Range、走分段**；`mirror`（`-Mirror` 前缀拼接）→ **跳过探测、只作单流兜底**。理由：镜像对 Range 是"整包重取再切片"（实测按段 0.18 MB/s，同 URL 单流 6.7 MB/s），而 CDN 是真 Range（ESA 实测 206 + 分片缓存）。2026-11-01 曾写成"只要配了镜像就一律单流"，导致在 `urls[]` 里配上 CDN 后**依然得不到分段**（621.6 MB 单流 521 s = 1.19 MB/s），已改正 |
| 分段全败 → 单流兜底 | 纯直连分段用尽重试后**不再直接判失败**：清掉 `.downloads`，按候选顺序改单流重试（日志 `分段下载失败（…），改单流重试`） |
| 关闭续传 | `-NoResume`（每次清空 `.downloads`）；`-SegmentMB 0` 直接走单流 |
| 收尾 | 所有段拼成 `<zip>.joining` → 校验总长 → 改名后进 SHA256 校验；成功后删 `.downloads` |

实测（2026-10-30，本机 2.6 MB/s 限速服务 + 真实 gh-proxy）：

- 20 MB 资产 / 4 MB 段：5 段成功；不支持 Range 时正确回退单流并**换源成功**（镜像 404 → 直连）；
- 中途 `Stop-Process` 杀掉 → 重跑从断点继续（`续传：缓存里已有 3.0 MB，只需补 17.0 MB`），
  预置段文件做确定性验证时最终 SHA256 与清单一致；
- 真实网络：295.6 MB 的 `deps-ocr` 切 5 段经 gh-proxy 下载完成，`item_done`。

**2026-11-01 真机事故（两条规则都是它换来的）**：`moss-cuda.zip` 621.6 MB —— gh-proxy 那次探测
"不支持 Range"（被跳过），直连支持 Range 但分段两次都 `curl: (28) Could not connect to github.com:443` /
`Recv failure: Connection was reset`。旧实现段重试用尽即抛异常，**没有单流兜底** ⇒ 整次安装
`退出码 3`，7 个条目里只差这一个二进制。同一批次里 `models-moss.zip` 597 MB 因为两个源都不支持
Range 走了单流镜像，反而成功。

随后复测又量到第二件事：镜像其实**也**能回 206（探测时通过），但按段下载只有 0.18 MB/s，
同一 URL 单流 6.7 MB/s —— 说明镜像的 Range 是"整包重取再切片"。同一份 621.6 MB 走镜像单流
约 90 s 装完（`item_done`、`done ok:true`）。

⇒ 两条结论：① **分段与单流是同一条下载路径的两级**，不能"分段失败即失败"；
② **镜像前缀不该分段**（它只是兜底），但**CDN 应该分段**——见下一段的修正。

**2026-11-02 修正与 CDN 实测**（`urls[]` 配上阿里云 ESA 后重跑全量首装）：

- 上一版把"配了镜像就别分段"写成了"只要 `-Mirror` 非空就一律单流"，而 `-Mirror` 默认就是
  `https://gh-proxy.com/` ⇒ **CDN 主源也被强制走单流**：621.6 MB 单流 521 s（1.19 MB/s）。
  真因是判据错了（看"有没有配镜像"，而不是"这个候选是不是镜像"），已改为按候选 `Kind` 决定。
- 改为按源拆分后同一批资产的实测（ESA 命中缓存、64 MB 段、本机同一时段）：

  | 条目 | 大小 | 下载耗时 | 折算速率 |
  |---|---|---|---|
  | `moss`（cuda） | 621.6 MB | 61 s | **10.2 MB/s** |
  | `models-moss` | 597.4 MB | 365 s | 1.64 MB/s |
  | `deps-ocr`（cpu） | 295.5 MB | 28 s | **10.6 MB/s** |
  | `deps-ocr-gpu`（cuda） | 479.3 MB | 387 s | 1.24 MB/s |
  | `deps-embed` | 150.1 MB | 15 s | **10.0 MB/s** |
  | CUDA 运行库（pip，华为云） | 1771 MB | 132 s | 13.4 MB/s |

  全量 6 项 19.3 分钟装完（`done ok:true`），ESA 侧平均 4.8 MB/s。
- **注意速率不是常数**：同样命中缓存，`moss`/`deps-ocr`/`deps-embed` 稳定 10 MB/s，而
  `models-moss`/`deps-ocr-gpu` 只有 1.2–1.6 MB/s（推测与免费版"智能限频"、单连接节流有关）。
  即"ESA 快"只在**多数时候、每对象随机**成立，不能当 SLA 依赖 —— 这正是保留 `urls[]` 多源
  与镜像兜底的理由。
- 冷启动（对象未被边缘缓存）TTFB 约 2.5 s、6 MB/s，回源走 GitHub；免费版无预热、无 L2 缓存，
  每个边缘节点首访都要整份回源。

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
| P3 | `src-tauri/src/ai_runtime/mod.rs` | *（可选）* `AsrManager` 增加「重置 moss provider」的方法，用于改设备后**即时**生效 | **已实现**（2026-10-29 提交 `1fa8a77`，本分支直接改：`reload_engine(key)` + provider 换 `RwLock<Arc<dyn AsrProvider>>`，见 §8.2 / §8.7；旧决策 D3-A「下次启动生效」已反转） |
| P4 | `src-tauri/tauri.conf.json` | `bundle.resources` 加 `"resources": ["resources/runtime/manifest.json"]` | **已由本分支直接改**（2026-10-24 用户指示落清单时一并加好）；installer 侧勿重复添加 |
| P5 | `src-tauri/src/ai_runtime/backend_probe.rs` | 老卡（算力 < 7.5）不再走 CUDA：`MIN_CUDA_COMPUTE_CAP`、`MachineCaps.cuda_cc` / `cuda_runnable()`、`parse_compute_cap()`、`decide()` 老卡改指「换 vulkan」、`recommend()` 抽成纯函数、报告加 `cuda_compute_cap` / `cuda_runnable`（详见 §8.1） | **已套用**（`351cd5a`，= installer 侧补丁 0001；2026-10-28 `git am --3way` 无冲突） |
| P6 | `src-tauri/src/ai_runtime/installed_state.rs`（新增）<br>+ `ai_runtime/mod.rs`（`pub mod installed_state;`）<br>+ `src-tauri/src/lib.rs`（注册命令） | 已装运行时 vs 清单比对：`evaluate()` 七态 + `#[tauri::command] probe_moss_install`（详见 §8.4） | **已套用**（`80eda1d`，= installer 侧补丁 0002；同上） |
| P7 | `scripts/bootstrap_runtime.ps1`（新增 406 行）<br>+ `ai_runtime/runtime_install.rs`（新增 1100 行）<br>+ `ai_runtime/mod.rs`（`pub mod runtime_install;`）<br>+ `lib.rs`（manage + 注册三命令） | 安装器执行体与 Rust 侧子进程基础设施（详见 §8.5） | **已并入**（`7d3f8d5`，走分支合并而非补丁；本分支在此之上做前端接线 §8.6） |
| P8 | `tauri.conf.json` 的 `bundle.resources` + `scripts/sync_bundle_resources.mjs` + `package.json` 的 `build` | 让 `scripts/bootstrap_runtime.ps1` 随安装包发布（此前只声明了 manifest，真机上一键安装必然失败） | **已由本分支完成**（2026-10-30，见 §8.6）；分发侧不要再重复加 |

P1/P2 的命令函数本体放在 `backend_probe.rs` / `config.rs`——这两个模块**已在
`ai_runtime/mod.rs` 里声明**（`pub mod backend_probe;` / `pub mod config;`）。

> **状态更正（2026-10-28）**：原文写「除 `mod.rs` 外双方不需要协同、`mod.rs` 本期无人改动」，
> 现已被 P6 打破——补丁 0002 在 `mod.rs` 加了 `pub mod installed_state;`、在 `lib.rs` 加了
> `ai_runtime::installed_state::probe_moss_install,`，两处各一行。两个补丁已按交接文档
> `git am --3way` 套入本分支，`cargo test --lib` 302 项全过，故**本期不再有需双方协同的文件**。

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
  "cuda_compute_cap": "8.9",    // 首块 NVIDIA 卡算力（×10 记法转成 "8.9"）；探测不到 ⇒ null
  "cuda_runnable": true,        // 有 NVIDIA 卡且算力 ≥ 7.5（MIN_CUDA_COMPUTE_CAP=75）；算力未知时乐观放行
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
- **算力口径（P5 引入）**：预构建 cuda 版只含 `sm_75/86/89` 内核，PTX 只向后 JIT ⇒
  Pascal 及更老（GTX 10xx / MX，算力 6.1）加载 cuda 版是**直接失败**（`no kernel image is
  available`），不是变慢。故显式请求 `cuda` 但 `cuda_runnable=false` 时在 L1 就硬报错，
  `reason` 已写「该卡算力 X.Y 低于预构建版本要求 7.5」，并指引改 `moss_device=vulkan`；
  `recommendation` 会反向推荐 vulkan（无 vulkan 则 cpu）。**前端不要自己按卡型号判断**，
  一律以 `cuda_runnable` / `reason` / `recommendation` 为准。

### 8.2 `set_moss_device`（落盘）

```ts
invoke("set_moss_device", { device: "" | "cpu" | "cuda" | "vulkan" })
```

- 白名单校验复用 `normalize_moss_device()`（`""`/`auto`/`cpu`/`cuda`/`vulkan`，归一后只有
  空串或三种具体后端；非法值 → 报错，不落盘，`config.rs:356-366`）。
- 落盘目标：`runtime/config.json` 的 `moss_device` 字段（原子写 `RuntimeConfig::save`）；
  读不到/坏文件直接报错，**不会用默认值覆盖**。
- 返回：`{ device: "cuda", message: "已保存 moss_device=cuda（moss 已重新加载）" }`
  （结构体 `DeviceSaveResult`，无 `ok` 字段——成功即返回，失败走 `Err`）。
- **生效时机（2026-10-29 变更）**：保存后立即调 `reload_engine("moss")` 重建该 provider，
  **不需要重启**；转写进行中时拒绝切换（返回 `"转写进行中，完成后再切换"`）。
  重建失败不阻断落盘，message 变成 `"已保存 moss_device=…；但 {原因}（重启后生效）"`。

### 8.3 `check_asr_engines`（已有，不改）

已返回 `device`（配置值）与 `effective_backend`（L2 实测值，仅 moss 有），
欢迎页 ASR 卡的「设备 · …（实测 …）」一行就是它渲染的。

### 8.4 `probe_moss_install`（已装运行时 vs 清单，P6 引入）

```ts
invoke("probe_moss_install")   // 无参，只读，不改任何文件
```

返回 `InstallStatus`：

```jsonc
{
  "state": "ok",            // 七态，见下表
  "installed": {            // 无 .installed.json 记录 ⇒ null
    "flavor": "vulkan",
    "sha256": "e8488472…",
    "source_version": "1",
    "installed_at": "2026-10-24T…Z"
  },
  "declared_size": 16139894, // 清单里该 flavor 的 size；清单缺失/该 flavor 不在清单 ⇒ null
  "message": "已安装 vulkan 版，与清单一致"   // 可直接上屏的中文结论
}
```

| `state` | 含义 |
|---|---|
| `ok` | 已装且与清单 sha256 一致，**且模型在** |
| `update_available` | 已装但 sha256 与清单不同（清单换了版本） |
| `not_installed` | 无 `.installed.json` 记录（或有记录但二进制没装） |
| `file_missing` | 有记录、二进制却不在 ⇒ 优先报「能跑」问题，不比较版本 |
| `model_missing` | 二进制在、**gguf 不在** ⇒ 同样跑不了，优先于版本类结论（2026-10-30 新增） |
| `flavor_not_in_manifest` | 装的 flavor 当前清单里没有（清单被精简或换过发布） |
| `manifest_missing` | 找不到清单 |
| `unknown` | 坏 JSON / 缺 sha256 字段 ⇒ 当「无记录」处理，不让元数据问题挡住 ASR |

约束：判据**只认 sha256**（zip 不可字节复现，版本号证明不了内容一致）；二进制与模型
**都要查**（`model_missing` 的由来：模型 618 MB，此前不进清单、也没人查，于是"装完显示
ok、第一次转写才报 `MOSS 模型不存在`"）；`flavor` 与二进制同目录 `.backend` 标记不一致时
只在 `message` 里附注（以 L2 实测为准）。
⇒ 本命令的数据源是 installer 写的 `runtime/.installed.json`：在
`scripts/bootstrap_runtime.ps1` 落地前，它会稳定返回 `not_installed` / `unknown`，
这是**预期**，不是 bug。
⇒ **空字段 = 标准布局**：一键安装只解包、不写 `config.json`，所以 `moss_binary` /
`moss_model` 为空时必须回退到 `bin/moss-transcribe.exe` 与
`models/moss/moss-transcribe-q5_k.gguf`（`moss_paths()`）。否则 `resolve_path("")`
会解析成 runtime 目录本身，把刚装好的运行时判成 `file_missing`——装完反而显示坏了。

### 8.5 安装器命令（installer 侧 `runtime_install.rs` 交付，前端 11c 已接线）

```ts
invoke("precheck_runtime_disk")                                              // 无参，只读
invoke("install_runtime", { items: null, flavor: "auto", force: false })      // 起子进程
invoke("cancel_runtime_install")                                             // 杀进程树
```

- `precheck_runtime_disk` → `DiskPrecheck`：`{ ok, manifest_entries, required_bytes, free_bytes | null, message }`。
  `free_bytes = null` 表示读不到磁盘信息 ⇒ **不拦**（宁放行不误拦）。
  需求按**本次要装的全部条目求和**（`precheck_disk_multi`）：单条目取该 id 全部 flavor 里
  **最大**的 size 乘展开系数 3（cuda 622 MB ⇒ ≈1.97 GB），多条目再把各自需求累加，
  `SAFETY_MARGIN` 只加一次。「装全部」（清单 7 条）时总需求约 4 GB 级，别按单条目估。
  **预检要扣掉已是最新的条目**（2026-10-31 修正）：逐条目比对 `.installed.json` 的
  sha256 与清单，一致且非 `-Force` 的不计入需求（脚本侧对它们只会打 `item_skip`）。
  全部都已最新时 `required_bytes = 0`、`message` 为「全部 N 个运行时条目已是最新：无需
  下载与解压，本次不占用额外磁盘」。不扣的话会出现"重装报磁盘不足、其实一个字节都不用下"
  （真机：C 盘剩 4.24 GB，全量估算 9.46 GB ⇒ 直接拒绝）。脚本侧同口径：`Test-Pending()`
  （`-Force` / 无记录 / sha 不同 / `dest` 目录为空 ⇒ 待装），CUDA 运行库已拉过也不再计入。
- `install_runtime` 的 `items` 是**逗号分隔字符串**（`null`/省略 = **清单里的全部条目**，
  与 §2 的 `-Items` 缺省一致）；`flavor` 省略 = `"auto"`；`force = false` 时已装且哈希一致就跳过、不重下。
  显式传 `items` 时只装这些 id，磁盘预检也按这些 id 算。
- `cancel_runtime_install` → `bool`：没有在跑的任务时返回 `false`。
- 进度事件 `runtime-install-progress`，payload = `InstallEvent`：
  `{ event, item?, flavor?, phase?, percent?, detail?, ok?, installed?, skipped?, failed? }`。
  `event ∈ start | progress | item_skip | item_done | item_fail | error | done`（与 §3 同一套），
  只有 `done` 才带 `ok / installed / skipped / failed`；`percent` 为 0–100 浮点。
- 失败语义要分清：**预检不过不抛 Err**，而是发 `error` + `done` 事件并返回
  `InstallSummary{ ok: false, exit_code: 1, message }`；**找不到脚本 / 已有任务并发**才抛 `Err(String)`。
  两者前端都是一条提示，但前者不能当异常处理（否则会掩盖「磁盘不够」这种可操作原因）。
- 事件是**全局**的（不分窗口），前端在 store 顶层监听一次即可；同一时刻只允许一个安装任务。

### 8.6 前端接线口径（第 11 项 11c，已实现）

- 按钮出现条件：MOSS 未就绪 **或** `probe_moss_install.state ∈ {update_available, file_missing, flavor_not_in_manifest, model_missing}`；
  文案「安装运行时」/「更新运行时」；`state === "ok"` 却仍未就绪时改「重装运行时」（带 `-Force`）。
- 点击顺序：`precheck_runtime_disk`（不 ok 只提示、不启动）→ `install_runtime({ flavor: "auto" })`
  → 事件驱动进度条 → 结束（成功/取消/失败）后重跑 `check_asr_engines` + `probe_moss_backend` + `probe_moss_install`。
- **自检卡要会等后台预热**：provider 构造不再做就绪握手（见 §8.6.1），首开时 OCR / MOSS 的
  message 会是「正在启动 / 正在探测」。前端要隔一会儿重探（当前：OCR 8 × 1.2 s、ASR 8 × 1.5 s），
  否则用户永远停在「未就绪」。
- **预热期必须显示「探测中…」而不是「未就绪」**（2026-10-31 修正）：`setProbe(key, ready, msg,
  device, backend, warming)` 的第 6 个参数为真时 state 置 `probing`；`probeOcr` / `probeAsr` 在
  `!ready && isWarming(message)` 时传 `true`，重试条件也放宽到 `notready | probing`。
  否则首帧是"未就绪"、几秒后自己变"就绪"，看起来像故障。
- `flavor` 传 `"auto"`：脚本 `Resolve-AutoFlavor`（cuda 算力 ≥7.5 → vulkan → cpu）与 Rust
  `recommend_from_assets` 口径一致，与卡面「建议改用 cuda 版运行时」不矛盾。
- **不做首启静默下载**（有意偏离该待办原标题）：cuda 包 622 MB，静默后台拉不可接受，失败时用户
  什么也看不到；由自检卡负责说清「缺什么、该装哪个后端」。形态可逆。
- **脚本随包（2026-10-30 补齐）**：`scripts/sync_bundle_resources.mjs` 在 `pnpm build` 前置把
  `scripts/bootstrap_runtime.ps1` 拷进 `src-tauri/resources/scripts/`，`bundle.resources` 声明
  `resources/scripts/*` ⇒ 打包后落在 `<exe 目录>/resources/scripts/bootstrap_runtime.ps1`，正是
  `bootstrap_candidates()` 的候选之一（`resources/scripts/` 那条）。已验证 NSIS/MSI 内确实带该文件。
  该目录是**构建产物**：`.gitignore` 只忽略 `*.ps1`，`README.md` 常驻（目录空掉会让 Tauri 的
  `resources/scripts/*` 通配失配，构建直接失败）。
- 安装失败时看 `runtime/logs/install.log`（脚本路径 / flavor / 退出码 / stderr 尾部 4000 字符），
  `InstallSummary.message` 会自动追加「（日志：<路径>）」；`message` 里还会带上脚本 stderr 中
  **最后一条含「失败：」的行**（截断 160 字），否则用户只看到「moss 失败」还得自己翻日志。
- **准备页的「重试」不带 `-Force`**（2026-11-01 真机修正）：`-Force` 会把已装好的条目也重算进
  磁盘预检，一次部分失败之后重试必然卡在「磁盘空间不足」——真机上就是 5 项已装、只剩 moss
  621 MB 没装，却因为 force 把 5 项重算成 8.77 GB > 可用 5.21 GB 而彻底走不动。语义分工：
  「重试」= 补缺（`force=false`），「重装运行时」（自检卡、`state==="ok"` 却不就绪时）= 强制重来。

### 8.6.1 子进程与命令的硬约束（2026-10-30 真机回归后新增）

真机装机实测暴露过两个体验级故障，均已定位并修复；后续任何新增命令/子进程**必须**遵守：

| 约束 | 为什么 | 做法 |
|---|---|---|
| 会起子进程或读大文件的 `#[tauri::command]` 一律 `async` + `tauri::async_runtime::spawn_blocking` | 同步命令跑在 Tauri **主线程**上，`probe_moss_backend`（`moss info` 加载几百 MB 模型 + `nvidia-smi` + `vulkaninfo`）、`check_ffmpeg`、`get_video_metadata` 曾把整个窗口冻住，连拖窗口都卡 | `probe_moss_backend` / `check_ffmpeg` / `get_video_metadata` 已改；`precheck_runtime_disk` / `install_runtime` 本来就是 async |
| 任何 `Command` 都必须经 `crate::subprocess::hidden()` / `quiet()` | GUI 进程起控制台子进程会弹窗，真机上表现为「命令行窗口疯狂闪」 | `CREATE_NO_WINDOW`（`0x0800_0000`），全仓库 28 处已统一；单测钉住「静默但仍能取到 stdout」 |
| 构造 manager 时不要做重活 | `AsrManager::new` / `OcrManager::new` 都在 `setup` 里跑，曾经在构造函数里同步做就绪握手，启动即卡 | `MossProvider` 惰性：`ensure_probed()` + 后台线程 `warm_up()`；`PaddleProvider` 同构：`spawn` 只到「子进程起来了」就返回（实测 6.43 s → 40 ms），`Inner` 用 `Arc` 共享 + `probe_lock` 串行化首次握手，首次 `recognize_batch` 等锁而不是往管道写半条请求；`describe()` 未探测完时给「正在启动」 |

### 8.6.2 路径解析与就绪判定：**空配置 ≠ 未安装**（2026-10-31 真机反馈后新增）
一键安装（`bootstrap_runtime.ps1`）**只解包文件、不写 `runtime/config.json`**，所以"文件都装好了、
配置字段还全空"是正常状态。任何就绪判定都必须按下面的口径来，否则会出现"文件明明在盘上、
卡片却说没装"（真机 2026-10-31 的 MOSS 与 e5-small 就是这个症状）：

| 约束 | 为什么 | 做法 |
|---|---|---|
| 判定就绪必须走 `*_paths()`（`moss_paths` / `ocr_paths` / `embed_paths` / `funasr_paths`）的**标准布局回退**，不能只判 `config.xxx` 字符串是否为空 | 空字段是"没配过"，不是"没装"。安装器只解包、不生成 config | `EmbedManager::status()` 改为 `embed_paths()` 解析后逐项 `is_file()/is_dir()`；`build_moss()` 不再因空字段短路返回 `AsrNoneProvider` |
| 未就绪原因必须**指出具体路径** | 只说"缺 embed_worker / embed_model / …"等于让用户猜 | 「嵌入模型不存在: `<绝对路径>`（点「安装运行时」可补齐）」 |
| provider 不能用"空配置就返回 None provider"代替真实实现 | None provider 往往没有 `describe()`，界面只能显示一句空白的"未就绪"，把真正原因藏起来 | `MossProvider::spawn` 解析完路径后若文件缺失，报「MOSS 未就绪：可执行文件 `<路径>` 不存在」 |
| `validate()`：**模型类只读数据允许绝对路径指向 runtime 之外** | C 盘紧张的机器要把几百 MB 的 gguf 放到别的盘 | 严格组（必须在内）：`worker_script` / `deps_dir` / `moss_binary` / `llm_binary` / `funasr_worker` / `funasr_deps` / `embed_worker` / `embed_deps_dir`；宽松组（允许绝对路径，相对路径仍查越界）：`model_dir` / `moss_model` / `llm_model` / `funasr_model_dir` / `embed_model` / `embed_tokenizer` |

标准布局（安装器落地位置，回退逻辑即按此拼）：

```
<runtime>/bin/<moss 二进制 + ggml/cuda DLL + .backend>
<runtime>/models/moss/<gguf>      models/embed/<model>/{model*.onnx, tokenizer.json}
<runtime>/worker/{ocr_worker.py, embed_worker.py, fuse_core.py}
<runtime>/deps/  deps_gpu/  deps_embed/   python/
```

### 8.6.3 「探测中」必须与「未就绪」区分，装完必须热加载（2026-11-01 真机反馈后新增）

真机 2026-10-31 的三条症状都出自"进程内状态与世界不一致"，产品行为上必须固化：

| 症状 | 机制 | 硬约束 |
|---|---|---|
| 启动瞬间卡片显示「未就绪」，几秒后自己变「就绪」 | 探测标志在**开始**就置真，`describe()` 于是返回空串（既不是"正在探测"也不是错误）⇒ 前端判 notready、且失去继续轮询的理由 | 探测状态机分四态 `Idle / Running / Ready / Failed(Instant)`；`Running` 与 `Idle` 一律返回「正在探测…」（前端据此显示「探测中」并每 1.5 s 重试，最多 20 次） |
| 探测失败一次后就永久「未就绪」，只能重启应用 | 旧实现 `probed` 是单向闩，失败也置真，之后再无重试路径 | `Failed` 带失败时刻；**过了冷冻期允许自动重试**（15 s 起按连续失败次数线性退避，封顶 120 s），`check_asr_engines` 每次轮询到未就绪就在后台触发一次 `retry_warm_up()`；转写入口用 `force=true` 忽略冷冻期 |
| 一键安装脚本跑完，四张卡全是「未就绪」，必须重启应用 | provider 是 `setup` 阶段按当时的目录一次性构造的（缺文件时退化成 `None`/`Broken` 占位），安装只往磁盘写文件 | 安装成功后前端必须调 `reload_runtimes`（见 §8.6.4）再重探全部卡片 |

### 8.6.4 `reload_runtimes`（2026-11-01 新增）

```ts
invoke("reload_runtimes")   // 无参数
// → { asr: string[], ocr: string }   每项是一句可直接显示的中文
```

- 语义：从磁盘重读 `runtime/config.json`（缺失则沿用当前配置）并**重建** ASR 双引擎与 OCR 的 provider；
  MOSS 重建会重新跑 `info`，因此放在 `spawn_blocking` 里，不占命令线程。
- 返回的是"结论句"而非状态码：`"moss 已重新加载"` / `"OCR 已重新加载但未就绪（provider: paddle）：…"`，
  前端把含「未就绪」的行拼成一条 info 提示即可。
- 边界：转写进行中时 ASR 侧会被拒绝（返回 `"moss: 转写进行中，完成后再切换"`），配置已落盘、重启后仍生效。
- 调用时机：`install_runtime` 返回 `ok=true` **之后、重探卡片之前**；用户手动点「重新探测」只重探不重载。

### 8.7 `set_funasr_device`（落盘，2026-10-29 引入）

```ts
invoke("set_funasr_device", { device: "cuda" | "cpu" })   // 也接受 "" / "auto"，均归一为 cuda
```

- 返回 `DeviceSaveResult`（与 §8.2 同一结构体），message 形如
  `"已保存 funasr_device=cuda（funasr 已重新加载）"`。
- 白名单只有 `{"", "cpu", "cuda"}`——**FunASR 走 torch，没有 Vulkan 后端**，写 `vulkan` 会被
  `RuntimeConfig::validate()` 打回（手改 `config.json` 也一样，启动时报错而非静默忽略）。
- **默认 `cuda`、但软回退**：`scripts/funasr_worker.py:46 pick_device` 在 CUDA 不可用（缺库/无卡）时
  自动落 CPU 并跑通 ⇒ UI **不做硬报错**，只在尾注写明回退行为（与 §9 里 MOSS 的「绝不静默降级」相反，
  因为两者后果不同：MOSS 静默降级会跑出一个假 GPU 结论，FunASR 只是慢）。
- 生效时机同 §8.2：保存后立即 `reload_engine("funasr")` 重建 worker，转写中拒绝切换。
- 旧决策 D5-A「本期只做 moss」已由用户 m01567 反转——FunASR 设备下拉已补
  （`src/views/AsrView.vue` 的 `FUNASR_DEVICE_OPTIONS`），两块 UI 互斥出现（选 MOSS 显示 MOSS 设备、
  选 FunASR 显示 FunASR 设备）。

### 8.8 `runtime_status`（首启门控判定，2026-11-01 新增）

```ts
invoke("runtime_status")   // 无参数
// → RuntimeStatus（见下）
```

```jsonc
{
  "ready": false,               // 必需项齐 + 安装脚本在位，才为 true
  "skip": false,                // 开发期跳过（GSA_SKIP_SETUP=1），为 true 时前端直接放行
  "runtime_dir": "C:\\Users\\…\\runtime",
  "script": "D:\\app\\resources\\scripts\\bootstrap_runtime.ps1",  // null = 打包漏了脚本，补不上
  "items": [
    { "id": "moss", "label": "MOSS 转写运行时", "flavor": "cuda", "size": 651834262,
      "state": "ready", "required": true, "note": "" }
  ],
  "missing_required": [],       // 缺的必需项 id
  "pending_bytes": 0,           // 还要下载的字节数（不含解压余量）
  "disk": { "ok": true, "manifest_entries": 7, "required_bytes": 0, "free_bytes": 1234, "message": "…" },
  "error": null                 // 非 null 时前端按"判定失败"处理（例如清单缺失）
}
```

- **必需项口径**（用户 2026-11-01 拍板）：`REQUIRED_IDS = ["moss", "models-moss", "deps-ocr", "deps-embed"]`。
  `moss` 只要 CPU 或 Vulkan 任一 flavor 就够（两者 `dest` 都是 `bin`，装不下两份），CUDA 版与
  `deps-ocr-gpu`、`deps-ocr-cuda-libs` 属可选加速项，缺了只降速、不拦门。
- `state` 三态：`ready`（文件在且 sha 与清单一致）/ `update`（文件在但 sha 不同）/ `missing`。
  flavor 由 `pick_entry` 决定：**已装记录优先**，否则按 `cuda_runnable()` → cuda、`caps.vulkan` → vulkan、最后 cpu。
- **`ready` 还要求 `script.is_some()`**：脚本没随包发布时点"安装"也只会报找不到脚本，不如直接拦住并提示重装应用。
- 放行条件：`ready || skip`。`ready=false` 且 `skip=false` 时前端渲染准备页；**已就绪的老用户完全不出现这层**。
- `disk` 复用 §8.5 的 `precheck_pending`，只统计**还没装/需要更新**的条目（已装条目不重复计），
  所以"装完再点一次"不会因为空间不足被误拦。
- 开发期跳过：设 `GSA_SKIP_SETUP=1`（或任意非 `"0"` 值）→ `skip=true`。

### 8.9 首启准备页的进度口径（`item_index` / `item_total`）

- `runtime-install-progress` 事件新增两个可选字段，**由 Rust 按本次请求的条目顺序补上**，
  脚本 stdout 协议（§3）不变，installer 侧无需改动。
- 语义：`item_index` 从 1 起，`item_total` = 本次请求的条目数（不是清单总数）。
- 前端总进度：`(item_index - 1 + percent / 100) / item_total`；没有这两个字段时退化成"当前条目百分比"。
- 为什么要它：一次装 5 个条目（含 622 MB 的 cuda 包与 597 MB 的模型）时，只报单条目百分比，
  用户看到的是"进度条反复从 0 走到 100"，观感上等于没有总进度。

### 8.10 设置命令与 runtime 目录搬家（2026-11-02 新增，第 19 项）

五个命令都在 `src-tauri/src/ai_runtime/config.rs`，均已注册到 `lib.rs` 的 `invoke_handler`。

| 命令 | 入参 | 返回 | 说明 |
|---|---|---|---|
| `runtime_settings` | 无 | `SettingsSnapshot` | 读设置 + 运行时现场（目录、可用空间、日志目录、清单是否找到、待删旧目录） |
| `update_runtime_settings` | `{ patch }` | `SettingsSnapshot` | 只改 patch 里出现的字段；落盘前过 `validate()` |
| `move_runtime_dir` | `{ target }` | `MoveResult` | 复制 + 对账 + 写指针；**重启生效** |
| `delete_old_runtime_dir` | 无 | `string` | 删指针里的 `previous` 目录 |
| `open_log_dir` | 无 | `string` | 建目录并用系统文件管理器打开 `runtime/logs` |

`SettingsPatch` 允许的字段：`ocr_device` / `moss_threads`（≤256）/ `moss_timeout_minutes`（≤1440）/
`dev_debug` / `download_source` / `segment_mb`（≤1024，0 = 关分段）/ `setup_auto`。
`ocr_device` 变更后**当场重建 OCR provider**（失败不阻断，下次启动生效）。

**runtime 目录指针**：`<app_data>/runtime-location.json`，形如
`{ "runtime_dir": "D:\\GSA\\runtime", "previous": "C:\\Users\\...\\runtime", "moved_at": 1793600000 }`

- `resolve_runtime_dir` 的入参语义是**应用数据目录**（不是 runtime 目录），解析顺序：
  指针文件 → `GSA_RUNTIME_DIR`（须含 `config.json`）→ 从 exe 向上找 `runtime/config.json` → `<app_data>/runtime`。
  指针指向的目录不存在时（外接盘掉线）退回后面的兜底，**不返回死路径**。
- 搬家规则：目标不能是当前目录 / 其子目录 / 其上级，不能是盘符根，**必须是空目录**；复制时跳过根上的
  `.downloads` 临时缓存；复制完按**相对路径逐条对账**，缺一条就报错且**不写指针**（半搬状态比不搬更糟）。
- 旧目录**一律保留**，由用户在设置里点「删除旧目录」——搬完立刻删，出问题就没法回退。
- 只搬 runtime：`recent_projects.json` 等仍在 app_data，不受影响。

**2026-11-04 补记（真机搬家失败后的两条硬约束）**

1. **调用方必须传 `app_data_dir()` 本身**。`lib.rs` 的 `setup` 原来写的是
   `resolve_runtime_dir(app_data_dir().map(|d| d.join("runtime")))`——调用方先拼了 `runtime`，
   `resolve_runtime_dir` 内部再拼一次，指针文件永远读不到；搬家"成功"但所有管理器仍盯着旧目录。
   真机后果：三个运行时全部未就绪，而设置面板显示的是新目录，两边自相矛盾。
   现在 `setup` 只有一行 `let runtime_dir = resolve_runtime_dir(app.path().app_data_dir().ok());`，
   其余调用点（`backend_probe` / `installed_state` / `runtime_install` / `setup`）同样只传 `app_data_dir()`。
2. **指针悬空必须可见**。`SettingsSnapshot.location_warning`（`Option<String>`）在指针目标目录不存在时
   给出人话解释并说明已回退到哪个目录；前端在「runtime 目录」行下方以警示色整行显示。
   指针里的路径是人手打/粘贴来的，拼错一个字母就会让应用"什么都没装"，这条信息不能再只留在日志里。

**下载源三档**（`RuntimeConfig.download_source` → 脚本 `-Source`）

| 值 | 脚本行为 |
|---|---|
| `auto`（默认） | 清单 `urls[]` 顺序：ESA 镜像 → 镜像前缀（gh-proxy）→ GitHub 直链 |
| `mirror` | 镜像前缀排在最前（强制走镜像） |
| `direct` | 只用 GitHub 直链（`urls[]` 里过滤 `://github.com/`） |

**分段**（`RuntimeConfig.segment_mb` → 脚本 `-SegmentMB`）：0 = 一律单流；>0 = 逐候选探 Range，
支持才分段。注意 §4.5 的实测结论：镜像源的 Range 是"整包重取再切片"，所以镜像候选不参与分段探测，
只作单流兜底。Rust 侧**显式传参**，不依赖脚本默认值。

---

## 9. UI 侧硬报错口径（行为契约，UI 必须照此）

| 情形 | 行为 |
|---|---|
| 显式选 `cuda` / `vulkan` 但本机不具备该后端 | **硬报错**，提示「装对应后端」或「改回 auto / cpu」，**绝不静默降级**（ggml 自己会静默降级且 exit 0 ⇒ 必须靠 L2 实证拦下） |
| `auto`（空值） | CUDA → Vulkan → CPU；**reason 必须在自检面板可见** |
| 裸 `vulkan` 落到核显（`uma: 1`） | 同族可用，只提示落点问题（实测：双 GPU 机器裸 `vulkan` 落 `Vulkan0` 核显，`auto` 才落 `Vulkan1` 独显）⇒ **UI 默认值必须是 auto，不要引导用户选裸 `vulkan`** |
| 老卡（算力 < 7.5，如 GTX 10xx / MX 6.1）显式选 `cuda` | **硬报错**，文案已由后端写成「该卡算力 X.Y 低于预构建版本要求 7.5 ⇒ 改 `moss_device=vulkan`」；`recommendation` 自动反向推荐 vulkan / cpu。**前端不要按卡型号自行判断**，只读 `cuda_runnable` / `reason` / `recommendation` |

实测性能参考（同一段 5:14.99 音频）：CPU 680.6 s / Vulkan 134.7 s / CUDA 76.2 s
（Vulkan ≈ 1.77× CUDA、5.0× CPU；CUDA 8.9× CPU）。
