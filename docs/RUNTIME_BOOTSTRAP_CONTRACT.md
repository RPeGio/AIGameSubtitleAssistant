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
| `-Items <ids>` | 逗号分隔的条目 id | 缺省 `moss,models-moss,deps-ocr,deps-ocr-gpu`；`deps-ocr-cuda-libs` 是"要 CUDA 运行库"的开关，不是清单条目（§4.3） |
| `-Flavor <auto\|cpu\|cuda\|vulkan>` | 默认 `auto` | `auto` = L0 机器能力探测，判定顺序同 `backend_probe`：**nvidia-smi → vulkaninfo → cpu** |
| `-Mirror <url>` | 默认 gh-proxy 代理前缀 | **代理前缀语义**（见 §4.1），非资源基址 |
| `-Force` | 开关 | 忽略已安装标记，强制重装 |
| `-Manifest <path>` | 默认 app 内置 manifest | 覆盖内置清单（离线/自测用） |
| `-SegmentMB <n>` | 默认 `64`，`0` = 不分段 | 分段大小；见 §4.5 |
| `-NoResume` | 开关 | 不读、不留 `.downloads` 缓存（每次从头下） |
| `-MaxAttempts <n>` | 默认 `3` | 单段失败重试次数 |
| `-PipMirror <url>` | 默认华为云 | CUDA 运行库 pip 索引（§4.3）；`-SkipGpuLibs` 可整段跳过 |
| `-PipUseProxy` | 开关 | **默认清空 `HTTP(S)_PROXY`** 跑 pip（代理常拉慢 pip），加了才走代理 |

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
  - **状态（2026-10-30 修订）**：清单共 **6 条**——`id=moss` 三条（cpu / vulkan / cuda，`dest` 全为
    `bin`）+ `id=models-moss` 一条（gguf 模型，**不带 flavor**，`dest=models/moss`）
    + `id=deps-ocr` 一条（OCR 运行时，`flavor=cpu`，**`dest="."`**，带 `unpacked_size`）
    + `id=deps-ocr-gpu` 一条（OCR 的 CUDA 增量，`flavor=cuda` + **`optional`**，`dest="."`）。
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
- **`-Items` 缺省 = `moss,models-moss,deps-ocr,deps-ocr-gpu`**：
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
| 关闭续传 | `-NoResume`（每次清空 `.downloads`）；`-SegmentMB 0` 直接走单流 |
| 收尾 | 所有段拼成 `<zip>.joining` → 校验总长 → 改名后进 SHA256 校验；成功后删 `.downloads` |

实测（2026-10-30，本机 2.6 MB/s 限速服务 + 真实 gh-proxy）：

- 20 MB 资产 / 4 MB 段：5 段成功；不支持 Range 时正确回退单流并**换源成功**（镜像 404 → 直连）；
- 中途 `Stop-Process` 杀掉 → 重跑从断点继续（`续传：缓存里已有 3.0 MB，只需补 17.0 MB`），
  预置段文件做确定性验证时最终 SHA256 与清单一致；
- 真实网络：295.6 MB 的 `deps-ocr` 切 5 段经 gh-proxy 下载完成，`item_done`。

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
| P5 | `src-tauri/src/ai_runtime/backend_probe.rs` | 老卡（算力 < 7.5）不再走 CUDA：`MIN_CUDA_COMPUTE_CAP`、`MachineCaps.cuda_cc` / `cuda_runnable()`、`parse_compute_cap()`、`decide()` 老卡改指「换 vulkan」、`recommend()` 抽成纯函数、报告加 `cuda_compute_cap` / `cuda_runnable`（详见 §8.1） | **已套用**（`351cd5a`，= installer 侧补丁 0001；2026-10-28 `git am --3way` 无冲突） |
| P6 | `src-tauri/src/ai_runtime/installed_state.rs`（新增）<br>+ `ai_runtime/mod.rs`（`pub mod installed_state;`）<br>+ `src-tauri/src/lib.rs`（注册命令） | 已装运行时 vs 清单比对：`evaluate()` 七态 + `#[tauri::command] probe_moss_install`（详见 §8.4） | **已套用**（`80eda1d`，= installer 侧补丁 0002；同上） |

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
- 返回：`{ device: "cuda", message: "已保存 moss_device=cuda（下次启动生效）" }`
  （结构体 `MossDeviceSaveResult`，无 `ok` 字段——成功即返回，失败走 `Err`）。
- **生效时机**：MOSS provider 在 app 启动时 spawn 并绑定设备 ⇒ 改完配置
  **默认下次启动生效**；若 installer 侧套用补丁 P3，可做即时重置。

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
