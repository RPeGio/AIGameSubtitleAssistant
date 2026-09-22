<#
bootstrap_ocr_gpu.ps1 —— 为 OCR worker 安装 **GPU（CUDA）版** paddlepaddle（仅需要 GPU 加速的用户）

背景与设计（对标 bootstrap_moss.ps1 的"默认 CPU、GPU 需自建"约定）：
  - CPU 版依赖装在 runtime/deps（默认，随安装包分发）；
  - 本脚本把 GPU 专有件装到**独立目录 runtime/deps_gpu**，避免覆盖 CPU 版；
  - 运行时 PYTHONPATH = "deps_gpu;deps"（GPU 件优先，其余依赖复用 deps）——
    由 src-tauri/src/ai_runtime/paddle.rs 在 GSA_OCR_DEVICE 非空时自动拼接。

只装两样东西（其余一律复用 deps/）：
  1) paddlepaddle-gpu 本体 + 7 个 nvidia-* CUDA 运行库（钉版本，来自 paddle 官方索引）；
  2) cuDNN 9.9.0.52（来自 PyPI，先装到临时目录再拷入）——版本必须对齐轮子的编译版本，
     且必须避开 pip 对共享 nvidia/ 命名空间目录的清理（三条约束都写在脚本 2 段注释里，
     改动前先读）。Windows 下 paddle 从 <paddle包目录>/../nvidia 逐个子目录挂 DLL
     （见 runtime/deps/paddle/__init__.py）。
  刻意不装纯 Python 依赖（numpy/protobuf/pillow…）：PYTHONPATH 前置会让它们
  覆盖 deps/ 里的基准版本，使 CPU/GPU 的精度对比无法归因到 CUDA 本身。

前置：
  - NVIDIA 显卡 + 驱动（用 nvidia-smi 检查）；CUDA 运行库由上述 nvidia-* pip 包
    提供，无需另装 CUDA Toolkit，只需驱动满足该 CUDA 版本的最低要求；
  - 磁盘 ~3.5GB（GPU 轮子 + CUDA 运行库 + cuDNN 体积大）。

参数：
  -Version    paddlepaddle-gpu 版本（默认 3.2.2，与 CPU 侧 paddle 3.2.2 对齐）
  -Cuda       轮子对应的 CUDA 版本档：cu126（默认）/ cu118 / cu129
  -Mirror     cuDNN 下载用的 PyPI 源（可选，如 https://mirrors.ustc.edu.cn/pypi/simple/）
  -Proxy      走 HTTP(S) 代理下载（可选，如 http://127.0.0.1:7897；会作用于全部包）
  -Force      已安装时强制重装
用法：
  powershell -ExecutionPolicy Bypass -File scripts/bootstrap_ocr_gpu.ps1
  之后设置环境变量 GSA_OCR_DEVICE=gpu:0 即启用（不设=CPU，行为与现状完全一致）
#>
param(
  [string]$Version = "3.2.2",
  [ValidateSet("cu126", "cu118", "cu129")]
  [string]$Cuda = "cu126",
  [string]$Mirror = "",
  [string]$Proxy = "",
  [switch]$Force
)

$ErrorActionPreference = "Stop"

$scriptDir = $PSScriptRoot
$root = (Resolve-Path (Join-Path $scriptDir "..")).Path
$runtime = Join-Path $root "runtime"
$python = Join-Path $runtime "python\python.exe"
$gpuDeps = Join-Path $runtime "deps_gpu"
$cpuDeps = Join-Path $runtime "deps"
# cuDNN 只在 PyPI 上有对齐版本；-Mirror 给了就换成国内 PyPI 镜像
$pypiIndex = if ($Mirror) { $Mirror } else { "https://pypi.org/simple" }

# ── 1. 前置检查 ──
if (-not (Test-Path $python)) {
  throw "找不到内嵌解释器 $python（请先完成应用安装/解压）"
}
if (-not (Test-Path $cpuDeps)) {
  throw "找不到 CPU 版依赖目录 $cpuDeps（请先运行 scripts/bootstrap_ocr.ps1）"
}
$smi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
if (-not $smi) {
  throw "未检测到 nvidia-smi：本机没有可用的 NVIDIA 驱动，无法使用 CUDA 版（继续用 CPU 版即可）"
}
Write-Host "==> 显卡信息："
& nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv | ForEach-Object { Write-Host "    $_" }

$want = "paddlepaddle-gpu==$Version"
# stamp 带布局后缀：区分"连纯 Python 依赖一起装"的旧布局与 cuDNN 9.5 的旧组合，
# 确保升级脚本后重跑会重装
$stampValue = "$want+$Cuda+cudnn9.9+staged"
$stamp = Join-Path $gpuDeps ".installed"
if ((Test-Path $stamp) -and -not $Force) {
  $cur = (Get-Content $stamp -Raw).Trim()
  if ($cur -eq $stampValue) {
    Write-Host "==> 已安装 $cur（-Force 重装）"
    exit 0
  }
  Write-Host "==> 版本/布局变更：$cur → $stampValue，重装"
}

# ── 2. 安装（--target 到独立目录，不污染 CPU 版 deps）──
New-Item -ItemType Directory -Force -Path $gpuDeps | Out-Null
$index = "https://www.paddlepaddle.org.cn/packages/stable/$Cuda/"

# CUDA 运行库：paddle Windows 版按 <paddle包目录>/../nvidia 下的固定子目录名挂 DLL，
# 这些包即对应的 nvidia 子目录（均为叶子包，无额外依赖）。
#
# 三条约束决定了下面的写法，改动前务必先读：
# 1) **不能分两次 pip 往 deps_gpu 里装**。这 8 个包共享同一个 nvidia/ 命名空间目录，
#    pip --target --upgrade 在覆盖安装前会先卸载并清理共享目录——分两次调用时，第二次
#    会把第一次装的那批从 nvidia/ 里一并清掉。实测（2026-09-22）：先装 7 个再单独装
#    cudnn → nvidia/ 下只剩 cudnn；反向操作 → cudnn 消失。本机因 PATH 里有系统 CUDA
#    Toolkit（D:\CUDA-Toolkit\bin）而掩盖了此问题，干净机器上会直接 DLL 加载失败。
# 2) **cuDNN 要 9.9，且只从 PyPI 有**。paddle 元数据钉的是 9.5.1.17（paddle 索引上
#    也只有它），但轮子实际按 cuDNN 9.9 编译，用 9.5 会触发 paddle 的兼容性警告：
#      "compiled with CUDNN 9.9, but CUDNN version in your machine is 9.5,
#       which may cause serious incompatible bug"
#    换 -Version 时若警告重现，需同步调整 $cudnnPin。
# 3) **不能靠 --extra-index-url 让 pip 去 PyPI 取 cudnn**。pip 没有源优先级：双源下它
#    会把 7 个 nvidia-* 也全从 PyPI 镜像拉，而国内镜像传大文件常中途停滞，且 pip 在
#    候选准备阶段的读超时**不受 --retries 影响**，会直接整轮失败（实测两次）。
# 综合 1+3：7 个包只走 paddle 官方索引一次装完；cuDNN 先装到临时目录，再把
# nvidia/cudnn 与 dist-info 拷进 deps_gpu——绕开 pip 的卸载逻辑，不碰已装的那批。
$cudaLibs = @(
  "nvidia-cublas-cu12==12.6.4.1",
  "nvidia-cuda-runtime-cu12==12.6.77",
  "nvidia-cufft-cu12==11.3.0.4",
  "nvidia-curand-cu12==10.3.7.77",
  "nvidia-cusolver-cu12==11.7.1.2",
  "nvidia-cusparse-cu12==12.5.4.2",
  "nvidia-nvjitlink-cu12==12.9.86"
)
$cudnnPin = "nvidia-cudnn-cu12==9.9.0.52"

# 覆盖安装前先清干净整个 nvidia/：9.x 各版本 DLL 同名（cudnn64_9.dll 等），
# 留旧文件会导致加载到混合版本
$nvidiaDir = Join-Path $gpuDeps "nvidia"
if (Test-Path $nvidiaDir) {
  Remove-Item -Recurse -Force $nvidiaDir
  Write-Host "    已清除旧 nvidia/ 目录（升级路径）"
}
Get-ChildItem $gpuDeps -Directory -Filter "nvidia_*" -ErrorAction SilentlyContinue |
  ForEach-Object { Remove-Item -Recurse -Force $_.FullName }

Write-Host "==> pip install --target deps_gpu $want + $($cudaLibs.Count) 个 CUDA 运行库（源：$index）"
Write-Host "    下载约 1.5GB，请耐心等待"
$pipArgs = @(
  "-m", "pip", "install", "--upgrade",
  "--target", $gpuDeps,
  # --no-deps：只装 GPU 专有件，纯 Python 依赖一律复用 deps/（见文件头说明）
  "--no-deps",
  # --no-cache-dir：轮子合计 2GB+，不留缓存副本（体积换时间，本脚本极少重跑）
  "--no-cache-dir",
  "--no-warn-script-location",
  "--timeout", "60",
  "--retries", "10",
  "-i", $index,
  $want
) + $cudaLibs
if ($Proxy) { $pipArgs += @("--proxy", $Proxy) }
& $python @pipArgs
if ($LASTEXITCODE -ne 0) {
  throw "安装失败：paddle 官方索引通常在国内较快，失败时检查网络或加 -Proxy"
}

# ── 2b. cuDNN 装到临时目录后拷入（原因见上方约束 1、3）──
Write-Host "==> pip install $cudnnPin 到临时目录后拷入 deps_gpu（源：$pypiIndex，约 770MB）"
$stage = Join-Path $env:TEMP "gsa_ocr_gpu_cudnn_stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
$cudnnArgs = @(
  "-m", "pip", "install",
  "--target", $stage,
  "--no-deps",
  "--no-cache-dir",
  "--no-warn-script-location",
  # 国内 PyPI 镜像传大文件易中途停滞；pip 候选准备阶段的读超时不重试，多给几次机会
  "--timeout", "60",
  "--retries", "10",
  "-i", $pypiIndex,
  $cudnnPin
)
if ($Proxy) { $cudnnArgs += @("--proxy", $Proxy) }
& $python @cudnnArgs
if ($LASTEXITCODE -ne 0) {
  Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
  throw "cuDNN 安装失败：PyPI 直连受限时用 -Mirror 指定国内镜像（如 https://mirrors.ustc.edu.cn/pypi/simple/）或 -Proxy 走代理"
}
$stagedCudnn = Join-Path $stage "nvidia\cudnn"
if (-not (Test-Path $stagedCudnn)) {
  Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
  throw "临时目录里没有 nvidia\cudnn，cuDNN 轮子结构异常：$stage"
}
New-Item -ItemType Directory -Force -Path $nvidiaDir | Out-Null
Copy-Item $stagedCudnn (Join-Path $nvidiaDir "cudnn") -Recurse -Force
Get-ChildItem $stage -Directory -Filter "nvidia_cudnn_cu12-*" |
  ForEach-Object { Copy-Item $_.FullName $gpuDeps -Recurse -Force }
Remove-Item -Recurse -Force $stage

# ── 3. 校验 CUDA 可用 ──
Write-Host "==> 校验 CUDA 可用性（PYTHONPATH=deps_gpu;deps）..."
$env:PYTHONPATH = "$gpuDeps;$cpuDeps"
$env:PADDLE_PDX_DISABLE_MODEL_SOURCE_CHECK = "True"
# import paddle 必然往 stderr 打一行 "INFO: Could not find files..."（探测 ccache 之类），
# 而 PS 5.1 在 ErrorActionPreference=Stop 下会把原生命令的 stderr 抛成 NativeCommandError
# （即使已 2>&1）。故探测段临时降级，并把 stderr 落到文件：既不产生看着像失败的错误块，
# 真出问题时也还能把原因打出来
$prevEap = $ErrorActionPreference
$ErrorActionPreference = "Continue"
$probeErr = Join-Path $env:TEMP "gsa_ocr_gpu_probe.err"
$probeText = (& $python -c "import paddle;print('cuda_compiled',paddle.device.is_compiled_with_cuda());print('cuda_count',paddle.device.cuda.device_count())" 2>$probeErr | Out-String)
$probeCode = $LASTEXITCODE
$ErrorActionPreference = $prevEap
# 必须先合成单个字符串再匹配：数组上的 -notmatch 返回"不匹配的元素"，
# 双行输出里只要有任一行不匹配就为真，会把成功误判成失败
$probeText.Trim() -split "`r?`n" | ForEach-Object { Write-Host "    $_" }
if ($probeCode -ne 0) {
  if (Test-Path $probeErr) { Get-Content $probeErr | ForEach-Object { Write-Host "    $_" } }
  throw "CUDA 探测进程退出码 $probeCode（paddle 导入失败？检查 deps_gpu 是否完整）"
}
if ($probeText -notmatch "cuda_compiled True") {
  throw "GPU 轮子已装但 CUDA 不可用：检查驱动版本是否满足该 CUDA 档（或改用 -Cuda cu118）"
}
if ($probeText -notmatch "cuda_count [1-9]") {
  throw "CUDA 可用但未发现 GPU 设备：检查显卡是否被占用/禁用（nvidia-smi）"
}

Set-Content -Path $stamp -Value $stampValue -NoNewline
Write-Host "==> 完成。启用方式：设置环境变量 GSA_OCR_DEVICE=gpu:0（不设=CPU，行为不变）"
